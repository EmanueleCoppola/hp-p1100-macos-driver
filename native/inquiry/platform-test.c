/* Exercise the actual SDK-based endpoint/open policy without touching USB or CUPS. */
#define main inquiry_cli_main
#include "main.c"
#undef main
#include <assert.h>
#include <unistd.h>

static unsigned opened, closed;
static IOReturn open_result, property_result;
static UInt8 class_value = 7, endpoint_count = 2;
static UInt16 vendor_value = 0x03f0;
static bool duplicate_output, no_input;
static IOReturn test_open(void *self) { (void)self; opened++; return open_result; }
static IOReturn test_close(void *self) { (void)self; closed++; return 0; }
static IOReturn test_class(void *self, UInt8 *out) { (void)self; *out = class_value; return 0; }
static IOReturn test_sub(void *self, UInt8 *out) { (void)self; *out = 1; return 0; }
static IOReturn test_protocol(void *self, UInt8 *out) { (void)self; *out = 2; return 0; }
static IOReturn test_vendor(void *self, UInt16 *out) { (void)self; *out = vendor_value; return 0; }
static IOReturn test_count(void *self, UInt8 *out) { (void)self; *out = endpoint_count; return 0; }
static IOReturn test_properties(void *self, UInt8 pipe, UInt8 *direction, UInt8 *number,
                                UInt8 *type, UInt16 *packet, UInt8 *interval) {
    (void)self;
    *direction = pipe == 1 || duplicate_output ? kUSBOut : kUSBIn;
    *number = pipe;
    *type = no_input && pipe == 2 ? kUSBInterrupt : kUSBBulk;
    *packet = 64;
    *interval = 0;
    return property_result;
}

static void endpoint_policy(void) {
    IOUSBInterfaceInterface182 table = {0};
    table.USBInterfaceOpen = test_open;
    table.USBInterfaceClose = test_close;
    table.GetInterfaceClass = test_class;
    table.GetInterfaceSubClass = test_sub;
    table.GetInterfaceProtocol = test_protocol;
    table.GetDeviceVendor = test_vendor;
    table.GetNumEndpoints = test_count;
    table.GetPipeProperties = test_properties;
    IOUSBInterfaceInterface182 *pointer = &table;
    struct session s = {.usb = &pointer};
    assert(open_interface(&s) == INQUIRY_OK && s.input == 2 && s.output == 1);
    assert(opened == 1 && !closed);
    assert(close_interface(&s) == INQUIRY_OK && closed == 1);

    s.input = s.output = 0;
    open_result = kIOReturnExclusiveAccess;
    assert(open_interface(&s) == INQUIRY_BUSY && opened == 2 && closed == 1);
    assert(!s.input && !s.output);
    open_result = kIOReturnNoDevice;
    assert(open_interface(&s) == INQUIRY_NO_DEVICE && closed == 1);
    open_result = 0;
    class_value = 255; /* The EWS interface must never reach bulk I/O. */
    assert(open_interface(&s) == INQUIRY_IO && closed == 2);
    class_value = 7;
    vendor_value = 0x1234;
    assert(open_interface(&s) == INQUIRY_IO && closed == 3);
    vendor_value = 0x03f0;
    property_result = kIOReturnNoDevice;
    assert(open_interface(&s) == INQUIRY_IO && closed == 4);
    property_result = 0;
    endpoint_count = 1;
    assert(open_interface(&s) == INQUIRY_IO && closed == 5);
    s.input = s.output = 0;
    endpoint_count = 2;
    duplicate_output = true;
    assert(open_interface(&s) == INQUIRY_IO && closed == 6);
    s.input = s.output = 0;
    duplicate_output = false;
    no_input = true;
    assert(open_interface(&s) == INQUIRY_IO && closed == 7);
    assert(usb_status(kIOReturnTimeout) == INQUIRY_TIMEOUT);
    assert(usb_status(kIOUSBTransactionTimeout) == INQUIRY_TIMEOUT);
    assert(usb_status(kIOReturnNotAttached) == INQUIRY_NO_DEVICE);
    assert(usb_status(kIOReturnNotPermitted) == INQUIRY_IO);
}

static void json_encoding(void) {
    FILE *capture = tmpfile();
    assert(capture);
    int original = dup(STDOUT_FILENO);
    assert(original >= 0);
    fflush(stdout);
    assert(dup2(fileno(capture), STDOUT_FILENO) >= 0);
    json_string("ABC\"\\123");
    fflush(stdout);
    assert(dup2(original, STDOUT_FILENO) >= 0);
    close(original);
    rewind(capture);
    char actual[64] = {0};
    assert(fread(actual, 1, sizeof actual - 1, capture) > 0);
    assert(!strcmp(actual, "\"ABC\\\"\\\\123\""));
    fclose(capture);
}

int main(void) {
    endpoint_policy();
    json_encoding();
    puts("native tests passed: SDK vtable, exclusive-open refusal, endpoints, close ownership, JSON escaping");
    return 0;
}
