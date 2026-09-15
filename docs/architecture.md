# Architecture of the replacement

## What has to be true

The replacement must: drive the P1102; be ARM64 with no Rosetta; be buildable from
source here; keep the raster/protocol logic separate from CUPS, USB and packaging; and
not sit on something macOS is actively deleting.

## The decisive constraint we measured

The blocker that actually killed HP's driver here is the **architecture**. `rastertozjs` is
x86_64, Rosetta is not installed, and there is no x86_64 dyld shared cache, so the process
cannot launch at all: `bad CPU type in executable` is what it prints.

An earlier version of this document gave a second reason that turned out to be wrong. It
claimed `/usr/lib/libcups.2.dylib` no longer exists, which would make any dependency on the
CUPS library fatal on its own. That is false here: Apple's own `cgpdftoraster`,
`cgpdftopdf`, `cgpdftops` and the `usb` backend all link `libcups.2.dylib` and run, so dyld
resolves it from the shared cache. The mistake was checking `/usr/lib` for the file, which is
not where modern macOS keeps dylibs.

Not linking `libcups` is still the right decision, for a weaker and more durable reason: our
filter then depends on nothing but the raster stream and the Rust standard library, so a CUPS
ABI or path change cannot break it, and it compiles without CUPS headers. That is
independence rather than necessity, and it is worth stating as such.
Measured, not assumed: both of our binaries load exactly one library,

    /usr/lib/libSystem.B.dylib

with no libcups, no frameworks, no private frameworks and no rpaths. HP's filter loads four
(libcups, libcupsimage, libstdc++.6 and libSystem). The entire dependency surface of the
replacement is the C library, which is what makes it hard for a macOS release to break it.

One cosmetic artefact of the rust-lld fallback route: the recorded SDK version in the load
command comes out as 11.0.0 rather than the real SDK, because rustc cannot ask xcrun for the
SDK path while the licence is unaccepted. It is informational and does not affect the linked
behaviour - the minimum OS is 11.0 either way. Building through the Xcode toolchain directly,
which `scripts/build-native.sh` now prefers, records the real SDK, because the SDK is handed
to the linker explicitly.



## Options considered

### Option A - traditional CUPS PPD plus a native ARM64 filter

Smallest change: the queue already exists, and the PPD already points at exactly one
filter for raster and one for commands. The filter is a plain executable reading stdin
and writing stdout, so it is trivially ARM64 and testable offline.

The objection is that it inherits a deprecated path. The CUPS PPD API has been marked
deprecated since CUPS 1.6 / macOS 10.8 with removal warned of, and Apple's own tracker
carries a "Deprecate printer drivers" item.

What deprecation means in practice was measured on this machine rather than assumed from a
version number. It runs **macOS 27.0** (build 26A428); `lpadmin` and `cupsctl` work; five of the
queues here are driven by PPDs in `/etc/cups/ppd`; and Apple still ships **419** PPDs under
`/Library/Printers/PPDs/Contents/Resources`, including the `hp1100.ppd.gz` this printer's own
queue was built from. So the path is deprecated and alive, and every existing queue on this Mac
uses it - which is what makes building on it the shortest route to a working queue rather than a
bet on a dead mechanism. The deprecation is an argument for keeping the core separable, which is
what the decision below does; it is not an argument for avoiding a path that still works.

### Option B - CUPS 3.x / OpenPrinting Printer Application

OpenPrinting's stated direction is driverless IPP Everywhere, with legacy printers that
need a driver served by a separate Printer Application. This is the architecture the
ecosystem is converging on. It is a larger change: it replaces the queue, needs a
packaged application, and on macOS it does not map onto the existing
`/etc/cups/ppd` + `cupsFilter` mechanism at all.

### Option C - standalone local IPP daemon advertising IPP Everywhere

Most future-proof: the printer becomes an IPP endpoint the OS sees as driverless. Also
the most work, and it still needs the same ZjStream encoder underneath. Note that the
CUPS web interface on `localhost:631` serves pages but answers "Web Interface is Disabled" for
the parts that configure printers - measured here on macOS 27 with CUPS 2.3.4 - so a daemon must
not assume that surface is usable.

## Decision

Build the core as a **transport-agnostic library**, and host it behind a **CUPS filter
first** (Option A), with Option B/C kept open by construction.

Reasons, in order of weight:

1. The expensive, risky, irreversible work is the *protocol and raster engine*, and that
   work is identical in all three options. It is now written with no CUPS, USB or
   packaging dependency, so re-hosting it later is cheap and does not invalidate it.
2. `lpadmin` continues to work even where the CUPS web UI does not, so a PPD queue is
   still creatable on the macOS this was measured on (27.0). Option A restores printing
   soonest.
3. Options B and C need a JBIG encoder and a working USB path before they can do anything
   a filter cannot, so choosing them first would add packaging work ahead of the actual
   unknown.
4. Because we do not link `libcups`, Option A does not inherit the specific failure that
   broke HP's driver; if Apple eventually removes the PPD path entirely, the same core
   drops into a Printer Application without being rewritten.

This is a deliberate, reversible sequencing choice, not a preference for the smallest
possible change. The migration trigger is explicit: if the PPD filter path stops being
invoked, move the same library behind an IPP Printer Application.

## Shape of the code

```text
crates/p1102-protocol   ZjStream encoder + decoder. Pure. No I/O, no deps.
                        Owns the wire format and its golden tests.
crates/p1102-raster     CUPS raster reader + 8-bit gray -> 1 bpp. Pure. No libcups.
crates/p1102-job        Raster stream -> printer job, a page at a time. Pure. No CUPS.
crates/p1102-filter     The CUPS filter binary: a thin argv/stdin/stdout wrapper
                        around p1102-job. This is the only CUPS-aware code.
crates/p1102-cli        Offline tooling: inspect rasters, dump streams, encode a job
                        without involving CUPS at all.
```

The dependency arrow only ever points one way - filter depends on job, job depends on
raster and protocol, and raster and protocol depend on nothing. That is what lets every
layer below the filter be exercised with a printer and a CUPS installation nowhere in
sight, which is the property that made this testable on this machine at all.

`p1102-usb` does not exist and is not currently needed: Apple's `usb` backend is already
native, and the filter writes to stdout for CUPS to hand to it.

**The filter streams.** It reads the raster one page at a time and hands each page's chunks to
the backend as soon as they are encoded, so peak memory is one page plus its compressed form
rather than the whole job. That matters at these sizes: a Letter page is 31 MB of raster at
600 dpi, so a fifty page document is over a gigabyte of raster whose entire compressed output
would be a few kilobytes.

```text
raster      buffering version (measured then)   streaming filter (measured now)
 31 MB              35.6 MB                              37.3 MB
 94 MB             103.1 MB                              37.4 MB
314 MB             312.4 MB                              37.5 MB
```

The "before" column is the version that accumulated the whole job, measured against it at the
time; those numbers track the raster exactly. The "after" column is the shipped filter,
measured the same way for all three sizes - a ten times larger job costs 0.3% more memory, so
the property is flatness rather than a single figure. Reproduce it with a multi-page raster:

```bash
/usr/bin/time -l target/release/p1102-filter 1 manu m 1 '' /tmp/ten-page.raster > /dev/null
```

The output was byte-identical before and after that refactor, which is how it was checked.



## Installation shape

The filter and a generated PPD go into `/Library/Printers/p1102/`, and a **new** queue is
created alongside HP's rather than replacing it. HP's installation is read but never
written. See `docs/installation.md`.

## Remaining technical risk

**The compressor is no longer the gap.** JBIG is implemented in `crates/p1102-jbig` and its
output is byte-identical to the reference implementation of the standard: four embedded
golden vectors, a 42-comparison sweep, and - the one that covers the whole path - the page
images inside jobs that `scripts/reference-compare.py` matches against the reference driver
byte for byte. It emits the configuration both working drivers use for this family (`0x5c`,
typical prediction on, the two-line lowest-resolution template, `L0` = 128, no adaptive
template movement); the parameters and their provenance are in `docs/jbig.md`.

**Neither is the wrapper.** The job our filter writes - PJL wrapper, chunk sequence, item
sets and their order, geometry, compressed image and final-chunk padding - is byte-identical
to the reference driver's for the same page and options, across twenty-eight cases: print
options including a two-page job, both resolutions the PPD offers, every page size it declares,
and the layout CUPS applies before rasterising. The comparison is
`scripts/reference-compare.py`, and what it cannot cover is listed in `docs/differences.md`.

The uncompressed `ZJT_RAW_IMAGE` path is also implemented and selectable with
`P1102_RAW=1`, because whether the printer accepts it is still an open hardware question
and it is a useful fallback to have. `p1102-job` prepares the same 1 bpp bitmap either way,
so choosing between the two encoders touches no other layer.

What remains is therefore hardware validation, not design or implementation:

1. Whether the printer accepts our framing, item sets and PJL wrapper at all.
2. Whether it accepts our JBIG stream. The stream is byte-identical to the reference
   driver's, so a rejection here would mean a *printer-specific* constraint rather than an
   encoder bug, and the adaptive template (item 2 of `docs/differences.md`) is the one
   parameter where HP's driver may differ from ours.
3. Whether `ZJT_RAW_IMAGE` works, and whether the 128-pixel-aligned row stride is what it
   expects.
4. Whether a power cycle needs a host-side firmware download. **LIKELY no**, but unverified.

## Open items

Every known difference from HP's driver, with what would settle each one, is listed in
docs/differences.md.

- No hardware validation has happened yet: the printer is not attached, so USB identity,
  endpoint layout, framing acceptance and the raw-image question are all unverified.
- Whether a filter-based queue survives future macOS releases is a migration question
  rather than a design one; the core is positioned so that it does not matter.
- Whether our *specific* JBIG configuration is accepted, and whether HP's adaptive template
  settings matter to the printer's decoder. Both are byte-verified against the reference
  driver and unverified against the device.
