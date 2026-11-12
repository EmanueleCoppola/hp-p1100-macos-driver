/* Read-only PJL protocol and transaction policy; no platform dependencies. */
#ifndef P1100_INQUIRY_H
#define P1100_INQUIRY_H
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

enum inquiry_status {
    INQUIRY_OK = 0, INQUIRY_USAGE = 2, INQUIRY_NO_DEVICE = 3,
    INQUIRY_BUSY = 4, INQUIRY_TIMEOUT = 5, INQUIRY_UNSUPPORTED = 6,
    INQUIRY_IO = 7, INQUIRY_IDENTITY = 8
};
enum { INQUIRY_COUNT = 3, INQUIRY_CAPACITY = 4096, INQUIRY_TIMEOUT_MS = 1000 };
enum inquiry_page { INQUIRY_PAGE_DEMO, INQUIRY_PAGE_CONFIGURATION,
                    INQUIRY_PAGE_SUPPLY_STATUS, INQUIRY_PAGE_CLEANING };
static const char *const inquiry_names[] = {
    "QUIETMODE", "POWERSAVETIME", "INACTIVEOFFTIME"
};
static const char inquiry_uel[] = "\033%-12345X";
struct inquiry_target { char product[256], serial[256]; };

static bool inquiry_decode(const char *input, size_t n, char *output, size_t capacity) {
    size_t used = 0;
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)input[i];
        if (c == '%') {
            unsigned value = 0;
            if (i + 2 >= n) return false;
            for (int j = 0; j < 2; j++) {
                unsigned char h = (unsigned char)input[++i];
                unsigned digit;
                if (h >= '0' && h <= '9') digit = h - '0';
                else if (h >= 'a' && h <= 'f') digit = h - 'a' + 10;
                else if (h >= 'A' && h <= 'F') digit = h - 'A' + 10;
                else return false;
                value = value * 16 + digit;
            }
            c = (unsigned char)value;
        }
        /* Identities here are printable ASCII, never control characters or truncation. */
        if (c < 32 || c > 126 || used + 1 >= capacity) return false;
        output[used++] = (char)c;
    }
    output[used] = 0;
    return used != 0;
}

static bool inquiry_target_from_uri(const char *uri, struct inquiry_target *target) {
    const char *path;
    if (!strncmp(uri, "usb://Hewlett-Packard/", sizeof "usb://Hewlett-Packard/" - 1))
        path = uri + sizeof "usb://Hewlett-Packard/" - 1;
    else if (!strncmp(uri, "usb://HP/", 9)) path = uri + 9;
    else return false;
    const char *query = strchr(path, '?');
    if (!query || !inquiry_decode(path, (size_t)(query - path), target->product,
                                  sizeof target->product)) return false;
    if (strncmp(target->product, "HP LaserJet Professional P110",
                sizeof "HP LaserJet Professional P110" - 1)) return false;
    bool serial_seen = false;
    for (const char *part = query + 1; *part;) {
        const char *end = strchr(part, '&');
        if (!end) end = part + strlen(part);
        /* Reject all unknown routing options, especially interface overrides. */
        if ((size_t)(end - part) <= 7 || strncmp(part, "serial=", 7) || serial_seen)
            return false;
        if (!inquiry_decode(part + 7, (size_t)(end - part - 7), target->serial,
                            sizeof target->serial)) return false;
        serial_seen = true;
        if (!*end) break;
        part = end + 1;
        if (!*part) return false;
    }
    return serial_seen;
}

static bool inquiry_matches(const struct inquiry_target *target, const char *product,
                            const char *serial, int vendor, int cls, int sub, int proto) {
    return product && serial && vendor == 0x03f0 && cls == 7 && sub == 1 && proto == 2
        && !strcmp(product, target->product) && !strcmp(serial, target->serial);
}

/* A complete response must name the requested variable and end with a form feed.
 * Stale status, echoes without values, missing delimiters and extra frames fail closed. */
static enum inquiry_status inquiry_parse(unsigned index, const unsigned char *bytes,
                                         size_t size, char value[32]) {
    if (index >= INQUIRY_COUNT || size >= INQUIRY_CAPACITY || memchr(bytes, 0, size))
        return INQUIRY_IO;
    char text[INQUIRY_CAPACITY];
    memcpy(text, bytes, size);
    text[size] = 0;
    char *p = text;
    if (!strncmp(p, inquiry_uel, sizeof inquiry_uel - 1)) p += sizeof inquiry_uel - 1;
    while (*p == '\r' || *p == '\n') p++;
    char header[96];
    snprintf(header, sizeof header, "@PJL INQUIRE %s", inquiry_names[index]);
    size_t length = strlen(header);
    if (strncmp(p, header, length)) return INQUIRY_IO;
    p += length;
    if (*p == '\r') p++;
    if (*p++ != '\n') return INQUIRY_IO;
    char *end = strchr(p, '\f');
    if (!end) return INQUIRY_IO;
    char *tail = end + 1;
    while (*tail == '\r' || *tail == '\n') tail++;
    if (!strncmp(tail, inquiry_uel, sizeof inquiry_uel - 1)) tail += sizeof inquiry_uel - 1;
    while (*tail == '\r' || *tail == '\n') tail++;
    if (*tail) return INQUIRY_IO;
    while (end > p && (end[-1] == '\r' || end[-1] == '\n')) end--;
    *end = 0;
    if (!strcmp(p, "?") || !strcmp(p, "NA")) return INQUIRY_UNSUPPORTED;
    if (!index) {
        if (strcmp(p, "ON") && strcmp(p, "OFF")) return INQUIRY_IO;
    } else {
        /* Preserve numeric readback, including firmware values absent from the UI. */
        if (!*p || strlen(p) > 9) return INQUIRY_IO;
        for (char *digit = p; *digit; digit++)
            if (*digit < '0' || *digit > '9') return INQUIRY_IO;
    }
    if (strlen(p) >= 32) return INQUIRY_IO;
    strcpy(value, p);
    return INQUIRY_OK;
}

/* A narrow test seam around the native transport. Callbacks cannot choose commands. */
struct inquiry_transport {
    void *context;
    enum inquiry_status (*guard)(void *);
    enum inquiry_status (*open)(void *);
    enum inquiry_status (*write)(void *, const void *, uint32_t, uint32_t);
    enum inquiry_status (*read)(void *, void *, uint32_t *, uint32_t);
    enum inquiry_status (*close)(void *);
    uint64_t (*now_ms)(void *);
};

static bool inquiry_page_parse(const char *name, enum inquiry_page *page) {
    static const char *const names[] = { "demo", "configuration", "supplyStatus", "cleaning" };
    if (!name) return false;
    for (unsigned i = 0; i < sizeof names / sizeof *names; i++)
        if (!strcmp(name, names[i])) { *page = (enum inquiry_page)i; return true; }
    return false;
}

static size_t inquiry_page_job(enum inquiry_page page, char *job, size_t capacity) {
    static const char *const commands[] = {
        "SET LANG=\n@PJL SET TESTPAGE=DEMOPAGE",
        "SET LANG=\n@PJL SET TESTPAGE=SELFTEST",
        "SET LANG=\n@PJL SET TESTPAGE=SUPPLIES",
        "CLEANPRINTER"
    };
    if ((unsigned)page >= sizeof commands / sizeof *commands) return 0;
    int length = snprintf(job, capacity, "%s@PJL %s\n%s", inquiry_uel,
                          commands[page], inquiry_uel);
    return length > 0 && (size_t)length < capacity ? (size_t)length : 0;
}

static enum inquiry_status inquiry_send_page(struct inquiry_transport *io,
                                              enum inquiry_page page) {
    char job[128];
    size_t length = inquiry_page_job(page, job, sizeof job);
    if (!length) return INQUIRY_USAGE;
    enum inquiry_status result = io->guard(io->context);
    if (result) return result;
    result = io->open(io->context);
    if (result) return result;
    result = io->guard(io->context);
    if (!result) result = io->write(io->context, job, (uint32_t)length, INQUIRY_TIMEOUT_MS);
    enum inquiry_status closed = io->close(io->context);
    return result ? result : closed;
}

static enum inquiry_status inquiry_run(struct inquiry_transport *io,
                                       char values[INQUIRY_COUNT][32]) {
    enum inquiry_status result = io->guard(io->context);
    if (result) return result;
    result = io->open(io->context);
    if (result) return result; /* Never close an interface we did not open. */
    for (unsigned i = 0; i < INQUIRY_COUNT; i++) {
        result = io->guard(io->context); /* Includes identity revalidation while owned. */
        if (result) break;
        char request[96];
        int n = snprintf(request, sizeof request, "%s@PJL INQUIRE %s\n%s",
                         inquiry_uel, inquiry_names[i], inquiry_uel);
        result = io->write(io->context, request, (uint32_t)n, INQUIRY_TIMEOUT_MS);
        if (result) break; /* No retries: a failed write might have sent a prefix. */
        uint64_t deadline = io->now_ms(io->context) + INQUIRY_TIMEOUT_MS;
        unsigned char response[INQUIRY_CAPACITY];
        size_t used = 0;
        for (;;) {
            uint64_t now = io->now_ms(io->context);
            if (now >= deadline) { result = INQUIRY_TIMEOUT; break; }
            uint32_t available = (uint32_t)(sizeof response - 1 - used);
            if (!available) { result = INQUIRY_IO; break; }
            uint32_t received = available;
            result = io->read(io->context, response + used, &received,
                              (uint32_t)(deadline - now));
            if (result) break; /* Ignore partial bytes on any transfer failure. */
            if (io->now_ms(io->context) >= deadline) { result = INQUIRY_TIMEOUT; break; }
            if (!received || received > available) { result = INQUIRY_IO; break; }
            used += received;
            if (memchr(response, '\f', used)) {
                result = inquiry_parse(i, response, used, values[i]);
                break;
            }
        }
        if (result) break;
    }
    enum inquiry_status closed = io->close(io->context);
    return result ? result : closed;
}
#endif
