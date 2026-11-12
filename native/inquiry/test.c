#include "inquiry.h"
#include <assert.h>
#include <stdlib.h>

struct fake {
    unsigned guards, opens, writes, reads, closes;
    enum inquiry_status guard_error, open_error, write_error, read_error, close_error;
    unsigned reject_guard;
    size_t chunk, offset;
    uint64_t time, advance;
    const char *reply;
    bool zero_read, over_report, page;
    enum inquiry_page page_kind;
};
static enum inquiry_status fake_guard(void *ctx) {
    struct fake *f = ctx;
    f->guards++;
    return f->guards == f->reject_guard ? f->guard_error : INQUIRY_OK;
}
static enum inquiry_status fake_open(void *ctx) {
    struct fake *f = ctx;
    f->opens++;
    return f->open_error;
}
static enum inquiry_status fake_write(void *ctx, const void *bytes, uint32_t n, uint32_t ms) {
    struct fake *f = ctx;
    assert(ms == INQUIRY_TIMEOUT_MS);
    if (f->page) {
        char expected[128];
        size_t length = inquiry_page_job(f->page_kind, expected, sizeof expected);
        assert(length == n && !memcmp(bytes, expected, n));
        assert(!memchr(bytes, '\f', n));
        f->writes++;
        return f->write_error;
    }
    assert(f->writes < INQUIRY_COUNT);
    char expected[96];
    snprintf(expected, sizeof expected, "\033%%-12345X@PJL INQUIRE %s\n\033%%-12345X",
             inquiry_names[f->writes]);
    assert(n == strlen(expected) && !memcmp(bytes, expected, n));
    assert(!memchr(bytes, '\f', n)); /* No page or rendering payload. */
    f->writes++;
    f->offset = 0;
    return f->write_error;
}
static enum inquiry_status fake_read(void *ctx, void *bytes, uint32_t *n, uint32_t ms) {
    struct fake *f = ctx;
    f->reads++;
    assert(ms > 0 && ms <= INQUIRY_TIMEOUT_MS);
    if (f->read_error) return f->read_error;
    if (f->over_report) { (*n)++; return INQUIRY_OK; }
    if (f->zero_read) { *n = 0; return INQUIRY_OK; }
    char reply[128];
    snprintf(reply, sizeof reply, "@PJL INQUIRE %s\r\n%s\r\n\f",
             inquiry_names[f->writes - 1], f->writes == 1 ? "OFF" : "15");
    const char *source = f->reply ? f->reply : reply;
    size_t length = strlen(source) - f->offset;
    if (length > *n) length = *n;
    if (f->chunk && length > f->chunk) length = f->chunk;
    memcpy(bytes, source + f->offset, length);
    f->offset += length;
    *n = (uint32_t)length;
    f->time += f->advance;
    return INQUIRY_OK;
}
static enum inquiry_status fake_close(void *ctx) {
    struct fake *f = ctx;
    f->closes++;
    return f->close_error;
}
static uint64_t fake_now(void *ctx) { return ((struct fake *)ctx)->time; }
static enum inquiry_status run(struct fake *f) {
    struct inquiry_transport io = {
        f, fake_guard, fake_open, fake_write, fake_read, fake_close, fake_now
    };
    char values[INQUIRY_COUNT][32] = {{0}};
    enum inquiry_status status = inquiry_run(&io, values);
    if (!status) {
        assert(!strcmp(values[0], "OFF"));
        assert(!strcmp(values[1], "15") && !strcmp(values[2], "15"));
    }
    return status;
}

static enum inquiry_status run_page(struct fake *f, enum inquiry_page page) {
    struct inquiry_transport io = {
        f, fake_guard, fake_open, fake_write, fake_read, fake_close, fake_now
    };
    f->page = true;
    f->page_kind = page;
    return inquiry_send_page(&io, page);
}

static void pages(void) {
    const char *names[] = { "demo", "configuration", "supplyStatus", "cleaning" };
    const char *jobs[] = {
        "\033%-12345X@PJL SET LANG=\n@PJL SET TESTPAGE=DEMOPAGE\n\033%-12345X",
        "\033%-12345X@PJL SET LANG=\n@PJL SET TESTPAGE=SELFTEST\n\033%-12345X",
        "\033%-12345X@PJL SET LANG=\n@PJL SET TESTPAGE=SUPPLIES\n\033%-12345X",
        "\033%-12345X@PJL CLEANPRINTER\n\033%-12345X"
    };
    for (unsigned i = 0; i < 4; i++) {
        enum inquiry_page page = INQUIRY_PAGE_DEMO;
        assert(inquiry_page_parse(names[i], &page) && (unsigned)page == i);
        char actual[128];
        size_t length = inquiry_page_job(page, actual, sizeof actual);
        assert(length == strlen(jobs[i]) && !memcmp(actual, jobs[i], length));
        struct fake f = {0};
        assert(run_page(&f, page) == INQUIRY_OK);
        assert(f.guards == 2 && f.opens == 1 && f.writes == 1 && !f.reads && f.closes == 1);
    }
    const char *invalid[] = { "", "Demo", "clean", "demo\n@PJL SET X=Y", "configuration " };
    enum inquiry_page page;
    for (size_t i = 0; i < sizeof invalid / sizeof *invalid; i++)
        assert(!inquiry_page_parse(invalid[i], &page));
    char job[128];
    assert(!inquiry_page_job((enum inquiry_page)99, job, sizeof job));
    struct fake f = {0};
    assert(run_page(&f, (enum inquiry_page)99) == INQUIRY_USAGE);
    assert(!f.guards && !f.opens && !f.writes && !f.closes);

    f = (struct fake){.reject_guard = 1, .guard_error = INQUIRY_BUSY};
    assert(run_page(&f, INQUIRY_PAGE_DEMO) == INQUIRY_BUSY && !f.opens && !f.writes && !f.closes);
    f = (struct fake){.reject_guard = 2, .guard_error = INQUIRY_IDENTITY};
    assert(run_page(&f, INQUIRY_PAGE_DEMO) == INQUIRY_IDENTITY && f.opens == 1 && !f.writes && f.closes == 1);
    f = (struct fake){.write_error = INQUIRY_TIMEOUT};
    assert(run_page(&f, INQUIRY_PAGE_DEMO) == INQUIRY_TIMEOUT && f.writes == 1 && f.closes == 1);
    f = (struct fake){.open_error = INQUIRY_BUSY};
    assert(run_page(&f, INQUIRY_PAGE_DEMO) == INQUIRY_BUSY && f.opens == 1 && !f.writes && !f.closes);
    f = (struct fake){.close_error = INQUIRY_IO};
    assert(run_page(&f, INQUIRY_PAGE_DEMO) == INQUIRY_IO && f.writes == 1 && f.closes == 1);
}

static void identities(void) {
    struct inquiry_target target;
    const char *uri = "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=ABC%201+2";
    assert(inquiry_target_from_uri(uri, &target));
    assert(!strcmp(target.product, "HP LaserJet Professional P1102"));
    assert(!strcmp(target.serial, "ABC 1+2"));
    assert(inquiry_matches(&target, target.product, "ABC 1+2", 0x03f0, 7, 1, 2));
    assert(!inquiry_matches(&target, target.product, "WRONG", 0x03f0, 7, 1, 2));
    assert(!inquiry_matches(&target, "Other printer", target.serial, 0x03f0, 7, 1, 2));
    assert(!inquiry_matches(&target, target.product, target.serial, 0x03f0, 255, 2, 16));
    assert(!inquiry_matches(&target, target.product, target.serial, 1, 7, 1, 2));
    assert(!inquiry_matches(&target, target.product, NULL, 0x03f0, 7, 1, 2));
    assert(!inquiry_matches(&target, target.product, target.serial, 0x03f0, 7, 1, 1));
    const char *bad[] = {
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=X&serial=Y",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=X&interface=1",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=X&",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=%00",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=%0A",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=%XX",
        "usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=%1",
        "usb://Other/HP%20LaserJet%20Professional%20P1102?serial=X",
        "usb://HP/Samsung?serial=X", "ipp://printer/queue", ""
    };
    for (size_t i = 0; i < sizeof bad / sizeof *bad; i++)
        assert(!inquiry_target_from_uri(bad[i], &target));
    char oversized[1024];
    memset(oversized, 'X', sizeof oversized);
    oversized[sizeof oversized - 1] = 0;
    char decoded[256];
    assert(!inquiry_decode(oversized, strlen(oversized), decoded, sizeof decoded));
}

static enum inquiry_status parse(unsigned index, const char *reply) {
    char value[32];
    return inquiry_parse(index, (const unsigned char *)reply, strlen(reply), value);
}
static void replies(void) {
    assert(parse(0, "@PJL INQUIRE QUIETMODE\r\nON\r\n\f") == INQUIRY_OK);
    assert(parse(0, "\033%-12345X@PJL INQUIRE QUIETMODE\nOFF\n\f\r\n\033%-12345X") == INQUIRY_OK);
    assert(parse(1, "@PJL INQUIRE POWERSAVETIME\n0\n\f") == INQUIRY_OK);
    assert(parse(2, "@PJL INQUIRE INACTIVEOFFTIME\n1440\n\f") == INQUIRY_OK);
    assert(parse(1, "@PJL INQUIRE POWERSAVETIME\n7\n\f") == INQUIRY_OK);
    assert(parse(0, "@PJL INQUIRE QUIETMODE\n?\n\f") == INQUIRY_UNSUPPORTED);
    assert(parse(2, "@PJL INQUIRE INACTIVEOFFTIME\nNA\n\f") == INQUIRY_UNSUPPORTED);
    const char *bad[] = {
        "", "@PJL INQUIRE QUIETMODE", "@PJL INQUIRE QUIETMODE\n",
        "@PJL INQUIRE QUIETMODE\nON\n", "@PJL INQUIRE QUIETMODE\nON\n\fgarbage",
        "@PJL INQUIRE QUIETMODE\nON\n\f@PJL INQUIRE QUIETMODE\nOFF\n\f",
        "@PJL INQUIRE QUIETMODE\nON\nOFF\n\f", "@PJL USTATUS JOB\nEND\n\f",
        "@PJL INQUIRE POWERSAVETIME\n15\n\f", "@PJL INQUIRE QUIETMODE\nTRUE\n\f"
    };
    for (size_t i = 0; i < sizeof bad / sizeof *bad; i++) assert(parse(0, bad[i]) == INQUIRY_IO);
    assert(parse(1, "@PJL INQUIRE POWERSAVETIME\n-1\n\f") == INQUIRY_IO);
    assert(parse(1, "@PJL INQUIRE POWERSAVETIME\n1.5\n\f") == INQUIRY_IO);
    assert(parse(1, "@PJL INQUIRE POWERSAVETIME\n9999999999\n\f") == INQUIRY_IO);
    char value[32];
    const unsigned char nul[] = "@PJL INQUIRE QUIETMODE\nON\0\n\f";
    assert(inquiry_parse(0, nul, sizeof nul - 1, value) == INQUIRY_IO);
    assert(inquiry_parse(INQUIRY_COUNT, nul, 0, value) == INQUIRY_IO);
    unsigned char large[INQUIRY_CAPACITY] = {0};
    assert(inquiry_parse(0, large, sizeof large, value) == INQUIRY_IO);
}

static void transactions(void) {
    struct fake f = {0};
    assert(run(&f) == INQUIRY_OK && f.writes == 3 && f.closes == 1);
    for (size_t chunk = 1; chunk < 40; chunk++) {
        f = (struct fake){.chunk = chunk};
        assert(run(&f) == INQUIRY_OK && f.closes == 1);
    }
    enum inquiry_status blocked[] = { INQUIRY_BUSY, INQUIRY_IDENTITY, INQUIRY_IO };
    for (size_t i = 0; i < sizeof blocked / sizeof *blocked; i++) {
        f = (struct fake){.reject_guard = 1, .guard_error = blocked[i]};
        assert(run(&f) == blocked[i] && !f.opens && !f.writes && !f.closes);
        f = (struct fake){.reject_guard = 2, .guard_error = blocked[i]};
        assert(run(&f) == blocked[i] && f.opens == 1 && !f.writes && f.closes == 1);
        f = (struct fake){.reject_guard = 3, .guard_error = blocked[i]};
        assert(run(&f) == blocked[i] && f.writes == 1 && f.closes == 1);
    }
    enum inquiry_status errors[] = { INQUIRY_BUSY, INQUIRY_NO_DEVICE, INQUIRY_TIMEOUT, INQUIRY_IO };
    for (size_t i = 0; i < sizeof errors / sizeof *errors; i++) {
        f = (struct fake){.open_error = errors[i]};
        assert(run(&f) == errors[i] && f.opens == 1 && !f.writes && !f.reads && !f.closes);
        f = (struct fake){.write_error = errors[i]};
        assert(run(&f) == errors[i] && f.writes == 1 && !f.reads && f.closes == 1);
        f = (struct fake){.read_error = errors[i]};
        assert(run(&f) == errors[i] && f.writes == 1 && f.reads == 1 && f.closes == 1);
    }
    f = (struct fake){.reply = "@PJL INQUIRE QUIETMODE\n?\n\f"};
    assert(run(&f) == INQUIRY_UNSUPPORTED && f.writes == 1 && f.closes == 1);
    f = (struct fake){.reply = "@PJL USTATUS JOB\nEND\n\f"};
    assert(run(&f) == INQUIRY_IO && f.writes == 1 && f.closes == 1);
    f = (struct fake){.zero_read = true};
    assert(run(&f) == INQUIRY_IO && f.reads == 1 && f.closes == 1);
    f = (struct fake){.over_report = true};
    assert(run(&f) == INQUIRY_IO && f.closes == 1);
    f = (struct fake){.chunk = 1, .advance = 600};
    assert(run(&f) == INQUIRY_TIMEOUT && f.reads == 2 && f.writes == 1 && f.closes == 1);
    f = (struct fake){.advance = INQUIRY_TIMEOUT_MS + 1};
    assert(run(&f) == INQUIRY_TIMEOUT && f.reads == 1 && f.writes == 1 && f.closes == 1);
    f = (struct fake){.close_error = INQUIRY_IO};
    assert(run(&f) == INQUIRY_IO && f.writes == 3 && f.closes == 1);
    char huge[5000];
    memset(huge, 'A', sizeof huge);
    huge[sizeof huge - 1] = 0;
    f = (struct fake){.reply = huge, .chunk = 512};
    assert(run(&f) == INQUIRY_IO && f.writes == 1 && f.closes == 1);
}

int main(void) {
    identities();
    replies();
    pages();
    transactions();
    puts("inquiry tests passed: identity, responses, four fixed pages, no retries, fragmentation, guards and cleanup");
    return 0;
}
