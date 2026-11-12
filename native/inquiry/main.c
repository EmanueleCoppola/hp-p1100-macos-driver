/* Standalone macOS helper. Uses the same IOUSBLib path as p1100-usbink,
 * with SDK declarations instead of duplicating its Rust vtable layout. */
#include "inquiry.h"
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOCFPlugIn.h>
#include <IOKit/IOKitLib.h>
#include <IOKit/usb/IOUSBLib.h>
#include <cups/cups.h>
#include <stdlib.h>
#include <sys/socket.h>

struct session {
    http_t *cups;
    const char *queue;
    struct inquiry_target target;
    IOUSBInterfaceInterface182 **usb;
    UInt8 input, output;
};

static enum inquiry_status usb_status(IOReturn status) {
    if (status == kIOReturnSuccess) return INQUIRY_OK;
    fprintf(stderr, "p1100-inquire: USB status 0x%08x\n", (unsigned)status);
    if (status == kIOReturnExclusiveAccess) return INQUIRY_BUSY;
    if (status == kIOReturnNoDevice || status == kIOReturnNotAttached) return INQUIRY_NO_DEVICE;
    if (status == kIOReturnTimeout || status == kIOUSBTransactionTimeout) return INQUIRY_TIMEOUT;
    return INQUIRY_IO;
}

static enum inquiry_status queue_target(struct session *s, struct inquiry_target *target) {
    cups_dest_t *dest = cupsGetNamedDest(s->cups, s->queue, NULL);
    if (!dest) return INQUIRY_IDENTITY;
    const char *uri = cupsGetOption("device-uri", dest->num_options, dest->options);
    const char *state = cupsGetOption("printer-state", dest->num_options, dest->options);
    enum inquiry_status result = INQUIRY_OK;
    if (!uri || !inquiry_target_from_uri(uri, target)) result = INQUIRY_IDENTITY;
    else if (!state) result = INQUIRY_IO;
    else if (strcmp(state, "3")) result = INQUIRY_BUSY; /* IPP idle only. */
    cupsFreeDests(1, dest);
    return result;
}

static enum inquiry_status guard(void *context) {
    struct session *s = context;
    struct inquiry_target current;
    enum inquiry_status status = queue_target(s, &current);
    if (status) return status;
    if (strcmp(current.product, s->target.product) || strcmp(current.serial, s->target.serial))
        return INQUIRY_IDENTITY;
    cups_job_t *jobs = NULL;
    /* All local queues and users: aliases of the same USB device must also be idle.
     * ponytail: unrelated queued jobs also block; filter by device only if needed. */
    int count = cupsGetJobs2(s->cups, &jobs, NULL, 0, CUPS_WHICHJOBS_ACTIVE);
    cupsFreeJobs(count > 0 ? count : 0, jobs);
    if (count < 0 || cupsLastError() >= IPP_STATUS_ERROR_BAD_REQUEST) return INQUIRY_IO;
    return count ? INQUIRY_BUSY : INQUIRY_OK;
}

static bool string_property(io_service_t service, CFStringRef key, char *out, size_t capacity) {
    CFTypeRef value = IORegistryEntryCreateCFProperty(service, key, NULL, 0);
    if (!value) return false;
    bool ok = CFGetTypeID(value) == CFStringGetTypeID()
        && CFStringGetCString(value, out, (CFIndex)capacity, kCFStringEncodingUTF8);
    CFRelease(value);
    return ok;
}

static int number_property(io_service_t service, CFStringRef key) {
    CFTypeRef value = IORegistryEntryCreateCFProperty(service, key, NULL, 0);
    int number = -1;
    if (value) {
        if (CFGetTypeID(value) == CFNumberGetTypeID())
            CFNumberGetValue(value, kCFNumberIntType, &number);
        CFRelease(value);
    }
    return number;
}

static enum inquiry_status find_interface(struct session *s, io_service_t *found) {
    io_iterator_t iterator = IO_OBJECT_NULL;
    CFMutableDictionaryRef matching = IOServiceMatching("IOUSBHostInterface");
    if (!matching || IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iterator))
        return INQUIRY_IO;
    *found = IO_OBJECT_NULL;
    unsigned matches = 0;
    io_service_t service;
    while ((service = IOIteratorNext(iterator))) {
        char product[256], serial[256];
        bool match = string_property(service, CFSTR("USB Product Name"), product, sizeof product)
            && string_property(service, CFSTR("USB Serial Number"), serial, sizeof serial)
            && inquiry_matches(&s->target, product, serial,
                number_property(service, CFSTR("idVendor")),
                number_property(service, CFSTR("bInterfaceClass")),
                number_property(service, CFSTR("bInterfaceSubClass")),
                number_property(service, CFSTR("bInterfaceProtocol")))
            && number_property(service, CFSTR("bAlternateSetting")) == 0;
        if (match && ++matches == 1) *found = service;
        else IOObjectRelease(service);
    }
    bool valid = IOIteratorIsValid(iterator);
    IOObjectRelease(iterator);
    if (!valid || matches > 1) {
        if (*found) IOObjectRelease(*found);
        *found = IO_OBJECT_NULL;
        return INQUIRY_IDENTITY;
    }
    return matches ? INQUIRY_OK : INQUIRY_NO_DEVICE;
}

static enum inquiry_status open_interface(void *context) {
    struct session *s = context;
    /* Plain exclusive open. No seize, detach, reset, alternate-setting or fallback. */
    IOReturn opened = (*s->usb)->USBInterfaceOpen(s->usb);
    if (opened) return usb_status(opened);
    UInt8 cls = 0, sub = 0, proto = 0, count = 0;
    UInt16 vendor = 0;
    enum inquiry_status result = INQUIRY_IO;
    if ((*s->usb)->GetInterfaceClass(s->usb, &cls)
        || (*s->usb)->GetInterfaceSubClass(s->usb, &sub)
        || (*s->usb)->GetInterfaceProtocol(s->usb, &proto)
        || (*s->usb)->GetDeviceVendor(s->usb, &vendor)
        || cls != 7 || sub != 1 || proto != 2 || vendor != 0x03f0
        || (*s->usb)->GetNumEndpoints(s->usb, &count)) goto failed;
    for (unsigned pipe = 1; pipe <= count; pipe++) {
        UInt8 direction, number, type, interval;
        UInt16 packet;
        if ((*s->usb)->GetPipeProperties(s->usb, (UInt8)pipe, &direction, &number,
                                        &type, &packet, &interval)) goto failed;
        if (type != kUSBBulk) continue;
        if (!packet) goto failed;
        if (direction == kUSBIn && !s->input) s->input = (UInt8)pipe;
        else if (direction == kUSBOut && !s->output) s->output = (UInt8)pipe;
        else goto failed; /* Ambiguous endpoints are not guessed. */
    }
    if (s->input && s->output) return INQUIRY_OK;
failed:
    (*s->usb)->USBInterfaceClose(s->usb);
    return result;
}

static enum inquiry_status write_inquiry(void *context, const void *data, uint32_t size,
                                        uint32_t timeout) {
    struct session *s = context;
    return usb_status((*s->usb)->WritePipeTO(s->usb, s->output, (void *)data, size,
                                            timeout, timeout));
}

static enum inquiry_status read_reply(void *context, void *data, uint32_t *size,
                                     uint32_t timeout) {
    struct session *s = context;
    return usb_status((*s->usb)->ReadPipeTO(s->usb, s->input, data, size, timeout, timeout));
}

static enum inquiry_status close_interface(void *context) {
    struct session *s = context;
    return usb_status((*s->usb)->USBInterfaceClose(s->usb));
}

static uint64_t now_ms(void *unused) {
    (void)unused;
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (uint64_t)now.tv_sec * 1000 + (uint64_t)now.tv_nsec / 1000000;
}

static const char *no_password(const char *prompt, http_t *http, const char *method,
                              const char *resource, void *unused) {
    (void)prompt; (void)http; (void)method; (void)resource; (void)unused;
    return NULL; /* Read failures never open an authentication prompt. */
}

static const char *status_message(enum inquiry_status status) {
    switch (status) {
    case INQUIRY_OK: return "ok";
    case INQUIRY_NO_DEVICE: return "no matching USB printer is attached";
    case INQUIRY_BUSY: return "printer interface or CUPS is busy; try later";
    case INQUIRY_TIMEOUT: return "inquiry timed out; values remain unknown";
    case INQUIRY_UNSUPPORTED: return "firmware did not support an inquiry";
    case INQUIRY_IDENTITY: return "queue/device identity missing, changed, or ambiguous";
    default: return "inquiry failed; values remain unknown";
    }
}

static void json_string(const char *s) {
    putchar('"');
    for (; *s; s++) {
        if (*s == '"' || *s == '\\') putchar('\\');
        putchar(*s); /* URI decoding already excludes control/non-ASCII bytes. */
    }
    putchar('"');
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--help")) {
        puts("Usage: p1100-inquire --queue NAME\nReads QUIETMODE, POWERSAVETIME and INACTIVEOFFTIME; never changes settings.");
        return 0;
    }
    if (argc != 3 || strcmp(argv[1], "--queue") || !*argv[2] || strlen(argv[2]) > 127) {
        fputs("Usage: p1100-inquire --queue NAME\n", stderr);
        return INQUIRY_USAGE;
    }
    /* Pin to the local scheduler; ignore CUPS_SERVER and remote client.conf defaults. */
    cupsSetPasswordCB2(no_password, NULL);
    struct session s = {0};
    s.queue = argv[2];
    s.cups = httpConnect2("/private/var/run/cupsd", 631, NULL, AF_UNSPEC,
                         HTTP_ENCRYPTION_IF_REQUESTED, 1, 1500, NULL);
    enum inquiry_status status = INQUIRY_IO;
    if (!s.cups) goto done;
    httpSetTimeout(s.cups, 1.5, NULL, NULL);
    status = queue_target(&s, &s.target);
    if (status) goto done;
    status = guard(&s);
    if (status) goto done;
    io_service_t service = IO_OBJECT_NULL;
    status = find_interface(&s, &service);
    if (status) goto done;
    IOCFPlugInInterface **plugin = NULL;
    SInt32 score = 0;
    IOReturn created = IOCreatePlugInInterfaceForService(service,
        kIOUSBInterfaceUserClientTypeID, kIOCFPlugInInterfaceID, &plugin, &score);
    IOObjectRelease(service);
    if (created || !plugin) {
        if (plugin) IODestroyPlugInInterface(plugin);
        status = created ? usb_status(created) : INQUIRY_IO;
        goto done;
    }
    HRESULT queried = (*plugin)->QueryInterface(plugin,
        CFUUIDGetUUIDBytes(kIOUSBInterfaceInterfaceID182), (LPVOID *)&s.usb);
    IODestroyPlugInInterface(plugin);
    if (queried || !s.usb) { status = INQUIRY_IO; goto done; }
    char values[INQUIRY_COUNT][32] = {{0}};
    struct inquiry_transport transport = {
        &s, guard, open_interface, write_inquiry, read_reply, close_interface, now_ms
    };
    status = inquiry_run(&transport, values);
    /* Emit a complete result only after every response and a successful close. */
    if (!status) {
        fputs("{\"serial\":", stdout);
        json_string(s.target.serial);
        printf(",\"QUIETMODE\":\"%s\",\"POWERSAVETIME\":%lu,\"INACTIVEOFFTIME\":%lu}\n",
               values[0], strtoul(values[1], NULL, 10), strtoul(values[2], NULL, 10));
    }
done:
    if (s.usb) (*s.usb)->Release(s.usb);
    if (s.cups) httpClose(s.cups);
    if (status) fprintf(stderr, "p1100-inquire: %s\n", status_message(status));
    return status;
}
