# Test environment, toolchain constraints, fixtures

## This machine

```text
macOS 27.0 (build 26A428), Darwin 27.0.0, Apple M2 Max, arm64
```

## Toolchain blockers found (CONFIRMED)

1. **Xcode licence not accepted - at the shims, not at the tools.**
   `/usr/bin/{clang,cc,otool,lipo,nm,strings,python3,make}` are shims that refuse to run:

   ```text
   You have not agreed to the Xcode license agreements.
   Please run 'sudo xcodebuild -license' from within a Terminal window …
   ```

   Full Xcode 26.3 is installed at `/Applications/Xcode.app` and
   `/Library/Preferences/com.apple.dt.Xcode.plist` records agreement to `26.3`, but
   macOS 27 ships a newer licence text, so re-acceptance is required. Re-accepting needs
   an admin password and therefore the user.

   **The gate is in the shim, not in the toolchain.** The real binaries behind it run
   fine when called directly with an explicit SDK:

   ```bash
   XC=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin
   SDK=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk
   $XC/clang --version                 # Apple clang 21.0.0
   $XC/nm  rastertozjs | head          # symbols, including HP's C++ class names
   $XC/otool -tvV -p <symbol> <file>   # disassembly
   echo 'int main(void){return 0;}' > t.c && $XC/clang -isysroot $SDK t.c -o t && ./t
   ```

   `/Applications/Xcode.app/Contents/Developer/usr/bin/make` is the same story. This is
   how the reference driver was built, how HP's filter was disassembled, and how
   `scripts/foo2zjs-oracle.sh` and `scripts/build-native.sh` decide what to use. It is a
   toolchain path, not a licence circumvention: nothing is accepted, installed or
   modified, and accepting the licence remains the supported route. If it were revoked,
   the fallbacks below still work.

   `scripts/macho.py` and `scripts/elfinfo.py` remain as the pure-Python readers that need
   no Apple tooling at all.

2. **No Rosetta 2.** `arch -x86_64 /usr/bin/true` -> `Bad CPU type in executable`.
   Only `RosettaLinux` is present. No x86_64 dyld shared cache exists.

3. **`libstdc++.6.dylib` is not under `/usr/lib`**, and HP's filter needs it. The other two
   HP-era libraries are *not* missing: `libcups.2.dylib` and `libcupsimage.2.dylib` resolve
   out of the shared cache, which is proven by Apple's own filters - which link them -
   running here. An earlier version of this file said otherwise, because it checked
   `/usr/lib` for the file; that is not a valid test for a dylib on modern macOS.

4. **No CUPS job history to read.** `/var/log/cups/` contains only `access_log`; there is no
   `error_log`. cupsd's messages do reach the unified log - `log show --predicate
   'process == "cupsd"'` returns its start-up chatter - but a filter invocation in it would
   need a job to have run, and no job has run in the retained window, so the argv cupsd
   actually passes could not be observed from here. `/etc/cups/printers.conf` is mode 600
   root:_lp and was not read. After a real print, the unified log is the first place to look;
   `docs/installation.md` has the command.

5. `sudo` is not passwordless (`sudo -n true` -> `a password is required`).

### Building natively

`scripts/build-native.sh` picks one of three routes and prints which one it used: Apple's
toolchain through the `/usr/bin` shims when the licence has been accepted; Xcode's clang
called directly with an explicit SDK, which is the route that works on this machine today;
or `rust-lld` against a rewritten stub SDK, described next.

Rust ships `rust-lld` in its sysroot, so linking without Apple's `cc` was attempted long
before it worked, and the first diagnosis of why it failed was wrong. The symptom was
`undefined symbol: _opendir`, `_pthread_mutexattr_destroy` and so on; the earlier note here
blamed a missing plain `arm64-macos` target in `libSystem.tbd`. That is not the cause.

The real cause is a token. macOS 27's SDK describes some libraries with an architecture the
LLVM inside the current Rust toolchain does not know, and lld rejects the whole TAPI file
rather than the entry:

```text
rust-lld: error: could not load TAPI file at .../libSystem.tbd: malformed file
  libSystem.tbd:4:20: error: unknown architecture
        arm64e.x1-macos, arm64e.x1-maccatalyst ]
        ^~~~~~~~~~~~~~~
```

With the file rejected, `libSystem`, `libc` and `libm` all fail to load, which is what the
undefined symbols were really reporting. Apple's `ld64` understands the token, which is why
clang works and rust-lld does not.

**The fix, and it works.** `scripts/make-stub-sdk.py` writes a link-only copy of the SDK's
stubs with the offending entries removed from their target lists - non-destructive, in
`~/.cache/p1102-stub-sdk`, nothing outside that directory is touched. For an arm64 link those
entries never apply anyway, so the symbol sets are the ones Apple's linker would use. Then:

```bash
scripts/build-native.sh          # picks the route and says which one it used
```

produces ordinary native binaries:

```text
target/release/p1102:        Mach-O 64-bit executable arm64
target/release/p1102-filter: Mach-O 64-bit executable arm64
```

Linking against stubs only describes the library at link time; the binary imports the real
runtime `libSystem` from the OS. The output runs natively and is verified against the
reference decoder exactly as the wasm build was.

Accepting the Xcode licence remains the supported route, and it also restores `otool`,
`nm`, `lipo`, `strings` and `brew`. This exists so the project can be built and verified
without it, not as a replacement for it.

### Running the tests: natively first, wasm as the fallback

`scripts/test-native.sh` runs the whole suite on aarch64-apple-darwin - the real target -
using Apple's toolchain, or Xcode's clang passed to rustc as the linker when the licence has
not been accepted. Both routes run the same suite:

```bash
./scripts/test-native.sh                 # native; the same suite test-wasm.sh runs
./scripts/test-native.sh -p p1102-jbig   # anything cargo test takes
```

`scripts/test-wasm.sh` runs the same suite on wasm32-wasip1, which links with Rust's own
bundled rust-lld and needs no SDK. It is the fallback for a machine with no usable native
toolchain, and it is what the CI-style checks used before the toolchain binaries were found.

It passes an absolute runner path via `--config`, because cargo runs test
binaries with the *package* directory as cwd, so a relative runner path in
`.cargo/config.toml` does not resolve. Tests execute under `node`'s WASI implementation
(`scripts/wasi-test-runner.mjs`), serially, since wasm has no threads.

The runner also forces `--nocapture`. wasm has no unwinding, so libtest cannot catch a
panic and therefore never regains control to print a failing test's captured output;
without `--nocapture` a failure shows up only as an opaque `RuntimeError: unreachable`
with no indication of the assertion. With it, the panic message reaches the real stderr.

This matters in practice: three test-authoring mistakes in this repo were found only
because the assertion text became visible that way.

The suite covers the ZjStream byte layout, item ordering, `reserved` consistency, JBIG chunk
padding rules, the PJL framing and its stripping, the CUPS raster page walk and header round
trip, the full raster-to-job chunk sequence, and the filter's option parsing. Neither route
covers what needs the printer, which is what the hardware sections below are for.

## One command that runs everything

`scripts/verify-all.sh` runs every check in this repository and prints what each one said:

```text
build and tests:
  native build                             ok
  tests, native, whole suite               ok
  tests, wasm, the same suite              ok
  clippy                                   ok
  rustfmt                                  ok

what the machine can see:
  native ARM64 check, with HP control      ok
  pre-flight self-test                     ok

correctness against the references:
  whole job vs the reference driver        ok
  JBIG encoder vs the reference            ok

the CUPS boundary, and the shape of the install:
  rehearsal, JBIG path                     ok
  rehearsal, uncompressed path             ok
  rehearsal, with PPD options              ok
  PPD the installer would write            ok
  uninstall, dry run                       ok

the scripts themselves:
  python scripts parse                     ok
  shell scripts parse                      ok
  node scripts parse                       ok

everything passed.
```

It needs no privileges and writes nothing outside the repository and one scratch directory, so
it is safe to run at any point. `--quick` skips the two reference comparisons, which are the slow
part - a minute or so becomes a few seconds - and it exits non-zero if anything fails, showing the
last few lines of what the failing check said.

Two things it deliberately does not cover, because they need the printer or an administrator:
attaching the device, and installing the queue. Both are further down this document.

## Tools available right now

| tool | status |
|---|---|
| `rustc` / `cargo` 1.98.0 | present; links with Xcode's toolchain directly, or with `rust-lld` and the stub SDK |
| `/opt/homebrew/bin/python3.13`, `python3.10` | present and working |
| `file`, `xxd`, `hexdump`, `codesign`, `plutil`, `gzcat`, `cupsfilter`, `lpstat` | present and working |
| our `scripts/macho.py`, `scripts/elfinfo.py`, `scripts/cupsraster.py` | working |
| Xcode's own `clang`, `nm`, `otool`, `lipo`, `strings`, `make` | working when called directly with an SDK; see above |
| `/usr/bin/{clang,cc,otool,nm,lipo,strings,make,python3}` | blocked: these are the gated shims |

## Generating fixtures

### Deterministic test pages

`scripts/make-test-job.py` builds a complete, printer-ready ZjStream job for the patterns
the plan called for - blank, a black square, a horizontal line, a vertical line, the text
`TEST 123`, a grayscale ramp, a checkerboard and a border - at the measured 600 dpi page
geometry (Letter 4900x6400, A4 4758x6817):

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/make-test-job.py --list
$PY scripts/make-test-job.py --pattern square --page a4 -o /tmp/square.zjs --verify
$PY scripts/make-test-job.py --pattern text123 --raw -o /tmp/raw.zjs --verify
```

By default the page image is compressed with the **reference** encoder, so the payload is
known-good and the job can be sent to the printer before our own JBIG encoder exists. That
separates "is the framing, item set and transport right?" from "is our compressor right?",
which is the difference between one unknown and two at a time. `--raw` produces the same
job with `ZJT_RAW_IMAGE` instead, for testing whether the printer accepts an uncompressed
image at all; for one Letter page that is 3,994,175 bytes against 1,003 bytes compressed.

`--verify` walks the produced job back and checks the chunk sequence, that every chunk
size adds up, that `reserved` equals the item bytes, and that the compressed image
round-trips through the split into `JBIG_BIH` and `JBIG_BID` chunks.

### Reference JBIG oracle

`scripts/jbig-oracle.sh` unpacks the Homebrew `jbigkit` 2.1 bottle - a **prebuilt**
ARM64 binary, so no compiler is needed - and uses it for two things: producing the
known-good payloads above, and checking our own encoder byte for byte once it exists.

```bash
./scripts/jbig-oracle.sh fetch                       # download, verify sha256, unpack
./scripts/jbig-oracle.sh encode in.pbm out.jbg       # our target parameters
./scripts/jbig-oracle.sh verify ours.jbg expected.pbm  # decode ours, compare pixels
```

Pinned bottle: `sha256 a7cc930c8a3652ca...`, arm64_tahoe, jbigkit 2.1. It is GPL and is
used only as an oracle: never committed, never linked, never shipped. `jbigkit` is exactly
the library HP itself links into `rastertozjs` for this printer - its internal table symbols
are visible in that binary - which is what makes it the right reference.

`encode` passes the parameters we target, `-d 0 -p 92 -m 0 -s 128 -o 3`, and appends any
extra arguments, so `encode in.pbm out.jbg -p 0` also produces the no-options
configuration. `scripts/jbig-sweep.py` runs 21 bitmaps through both implementations in
both configurations - 42 comparisons - and requires byte identity; it is the check behind
the claim in `docs/jbig.md`.

### The reference driver, as a runnable golden master

HP's filter cannot run here, but `foo2zjs` - the open driver that supports this printer -
can, and a job it produces is the closest available statement of what a working driver
sends. `scripts/foo2zjs-oracle.sh` fetches and builds it out of a cache directory using
Xcode's toolchain directly (see the toolchain note above).

```bash
./scripts/foo2zjs-oracle.sh fetch        # idempotent; GPL, test-only, never committed
./scripts/foo2zjs-oracle.sh which        # where the binary landed
./scripts/foo2zjs-oracle.sh -h           # it takes the usual foo2zjs options

PY=/opt/homebrew/bin/python3.13
$PY scripts/reference-compare.py                      # every case
$PY scripts/reference-compare.py --only a4 --keep /tmp/reference
```

Each case goes through the whole real path on both sides: the raster comes from Apple's
own `cgpdftoraster` via `cupsfilter` with the PPD's real options, our side is the actual
filter binary with the argv shape cupsd uses, and the reference is fed the same page
bitmaps with the equivalent flags. The pass condition is **byte identity after removing
the job-attribute line**, which carries the clock; a failure prints the differing items by
name, because `scripts/zjs-compare.py` is what is doing the explaining.

Our side also has `PPD` set to the installed PPD, because CUPS sets it for a real job and the
filter reads the defaults the job does not name from it: the reference is given the same
choices as flags, so a default the filter failed to pick up would show as a difference rather
than pass unnoticed.

Every case leaves a raster of tens of megabytes behind, so a run that passes removes its
scratch directory and a run that fails keeps it - and says where - for inspection. `--keep`
keeps it either way. `scripts/jbig-sweep.py` does the same; between them they had quietly
accumulated twenty-five gigabytes before that was noticed.

That line's *value* is expected to differ - ours is UTC and the reference's is local time, two
hours apart here - so the harness removes it and then checks that both jobs still carry a
fourteen-digit one, and prints both stamps. A regression in the format would otherwise hide
behind the removal that makes the comparison possible; `docs/differences.md` item 7 has the
reason the bases differ.

The case list covers print options (defaults, A4, econo mode, density, manual feed, copies,
two pages, both resolutions) and **every page size the PPD declares**, plus a custom one.
The page-size cases exist because the raster's page size name decides `ZJI_DMPAPER`, and
comparing them is how eight names that our table did not carry - and which therefore went out
as Letter - were found.

It also covers the things CUPS does to a job *before* rasterising it, because those reach the
filter only as pixels: landscape, a page rotated 180 degrees, and two pages on one sheet. Each
is a case where the driver has nothing to do and a chance to get it wrong anyway.

A document that mixes page sizes is covered too - a Letter cover and A4 body pages, say - since
each raster page carries its own header and therefore its own geometry. A unit test keeps each
page's `Zji_Dmpaper` and raster dimensions apart, and the same thing was checked end to end
through the filter by splicing two real rasters ("Letter" then "A4"): the job came out with two
`START_PAGE` chunks, `RASTER_X` 4900 with paper code 1 and `RASTER_X` 4758 with code 9. The
splice is needed because `cupsfilter` cannot build a mixed-size job here - its chain has no
PostScript-to-raster path, so the PostScript that would describe one cannot be rendered.

Duplex has its own measurement, because it is the one case where a filter could plausibly do
something and should not. The PPD declares `*cupsBackSide: "Rotated"`, and CUPS acts on it
before any filter runs: a two-page job submitted with `-o sides=two-sided-long-edge` comes out of
`cupsfilter` with page 2 upside down - its ink sits at rows 6314..6393 of 6400 where simplex
printing puts it at 7..86. So passing the pixels through is correct, and a filter that rotated
the back side itself would print it the wrong way up. The comparison harness has no duplex case
for this reason: the reference driver *does* rotate and reorder, by construction, so comparing
against it there would fail for a reason that is not ours.

Two things are deliberately *not* in the case list, and both are recorded in
`docs/differences.md`: duplex (the reference reorders and rotates pages host-side, we leave
that to the printer) and the media code (it reaches us in the raster header, which is set
through the PostScript page device - something `cupsfilter` cannot reproduce).

### Reading HP's filter

The same toolchain path makes HP's own binaries skimmable without running them:

```bash
XC=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin
F=/Library/Printers/hp/laserjet/p1100_1560_1600series/hpRaster.bundle/Contents/MacOS/rastertozjs
$XC/nm "$F" | $XC/c++filt | grep 'jbig::'          # its own encoder's surface
$XC/otool -tvV -p __ZN4jbig12ZjbigEncoderC1EiiRN4zeno7IWriterE "$F"
```

That is how the BIH defaults `0x5c` / order `3` / `MX 16` in `docs/protocol.md` were read
off, and how the PJL literal list in `docs/differences.md` was checked. `c++filt` is a
harness from the same toolchain directory: `/usr/bin/c++filt` is gated too.

## Hardware bring-up with no build at all

### Pre-flight, the moment the printer is attached

`scripts/preflight-usb.py` reports what the machine can see, read-only and with no
privileges, so the first hardware moment is diagnostic rather than guesswork:

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/preflight-usb.py
$PY scripts/preflight-usb.py --all        # every USB device, not just printers and hubs
$PY scripts/preflight-usb.py --selftest   # check the report without a printer
```

It answers the questions that decide what to do next:

- is a USB Printer-class device attached at all (interface class 7, or a Hewlett-Packard
  `03f0:` vendor id);
- does its serial number match the one the queue's device URI asks for? The `usb` backend
  matches on exactly that, so a second printer with the same name is reported as a
  mismatch instead of silently printing nowhere;
- which interfaces and how many endpoints it presents. Two on a bidirectional printer
  interface is what the backchannel needs; fewer is called out;
- which filters the queue names, whether they exist, and whether their architectures can
  run here - which is the question the whole project exists to answer.

- whether CUPS's own side can see it: does the `usb` backend list the device the queue's URI
  names? That is the closest thing to an answer to "will a job reach the printer at all", and
  the report gives it in one line per queue, taken from the same executable a job is handed to
  in its listing mode - which needs no privileges either.

Interfaces are matched back to their device by `locationID`. Endpoints are reported as a
count only: IOKit does not publish endpoint addresses as registry entries, so the script
does not invent them. It reads `ioreg -a` rather than `system_profiler SPUSBDataType`,
which returns nothing on this machine.

The tool exists for one moment, so its success path is checkable without one: `--selftest`
runs the report against a registry built by hand - a P1102 answering with the serial the queue
asks for, then with the wrong serial, then a keyboard - and checks what it says in each case.
That is the difference between the first hardware session being this tool's first run and its
eleventh. `--from FILE` reads either a saved `ioreg -a` dump or a report this script wrote
with `--json`, so a dump from another machine can be read here, and today's report can be
re-read later.

Exit status is 0 when a printer-class device is attached, 1 when none is, 2 on failure, so
it can be waited on:

```bash
until $PY scripts/preflight-usb.py >/dev/null; do sleep 5; done
```

Whether the printer re-enumerates - or comes back with a different VID/PID or serial -
after a power cycle or a firmware download is answered by diffing two JSON runs, which is
what `--json` is for:

```bash
$PY scripts/preflight-usb.py --json > /tmp/before.json
# power cycle the printer
$PY scripts/preflight-usb.py --json > /tmp/after.json
diff /tmp/before.json /tmp/after.json
```

With no printer attached it exits 1 and says so, but still reports the CUPS side, which is
useful on its own:

```text
  HP_LaserJet_Professional_P1102
    device   usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=000000000Q80E182PR1a
    wants    serial 000000000Q80E182PR1a
    filter   x86_64   .../hpRaster.bundle/Contents/MacOS/rastertozjs  <- Intel-only, cannot run here
  no USB Printer-class device and no Hewlett-Packard device is attached.
```

### Bring-up with no build and no install

Both blockers on hardware testing - the Xcode licence and the missing printer - are worth
clearing, but the first test needs neither, because a complete job can be produced without
compiling and delivered without installing:

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/make-test-job.py --pattern square --page a4 -o /tmp/square.zjs --verify

# straight through Apple's own backend: no queue, no privileges
DEV=$(lpstat -v HP_LaserJet_Professional_P1102 | sed -n 's/^device for [^:]*: //p')
DEVICE_URI="$DEV" /usr/libexec/cups/backend/usb 1 "$USER" bring-up 1 '' /tmp/square.zjs
```

The backend line is the same transport a queue would use. A CUPS backend takes its device
URI from `argv[0]` - or, as here, from `DEVICE_URI` - and reads the job from the file argument
or stdin, all documented in backend(7), and Apple's `usb` backend is world-executable, so
**sending a job needs no administrator** and creates no queue to clean up. With no printer
attached it says so plainly (`STATE: +offline-report`, `INFO: The printer is offline.`) and
keeps retrying - measured at fifty seconds and still going, so Ctrl-C it - which is itself the
check that the invocation is right before anything is plugged in. The same binary with no
arguments lists the USB devices CUPS can see, which is worth comparing with what
`scripts/preflight-usb.py` says from the registry side.

What that path does not cover is the wiring: it hands bytes to the backend, so it tests the job
and the transport but not CUPS's chain from the PPD through the filter. That is what the raw
queue below is for, and it does need administrator access:

```bash
sudo lpadmin -p P1102_raw -E -v "$DEV" -m raw
lp -d P1102_raw -o raw /tmp/square.zjs
lpstat -o P1102_raw
sudo lpadmin -x P1102_raw          # remove the test queue again
```

`-m raw` selects the "Raw Queue" model: no PPD, no filters, and the bytes go to the printer
verbatim through Apple's own `usb` backend. That is the same transport the real filter will
use, so this tests the printer protocol, the framing and the USB path in isolation from
CUPS's filter chain - and it needs no build, no install and no filter.

What each result tells you:

| observation | meaning |
|---|---|
| a page prints correctly | framing, item sets, PJL wrapper, JBIG stream, transport and polarity are all right |
| nothing happens at all | the printer is not accepting the job; try `--raw` next |
| `--raw` prints but JBIG does not | the printer dislikes our compressed stream specifically; it is byte-identical to the reference encoder, so this would point at a printer-side constraint such as the option set, not at the coder |
| it prints but the image is inverted | the ink polarity is the wrong way round; `1` is currently ink, following the reference implementation's plane convention |
| it prints the wrong size or is sheared | the geometry or the row stride is wrong, which the `RASTER_X` versus aligned-width distinction in `docs/protocol.md` explains |
| a two-sided job prints single-sided, in document order, with no prompt | duplex needs the host-side page handling the other two drivers have; see item 5 of `docs/differences.md`, which also says what to try |

`scripts/make-test-job.py` compresses with the **reference** encoder by default. That is
deliberate: if a page fails to print, a reference-quality payload has already ruled out our
own compressor, so the remaining suspects are much fewer.

Fixtures are **generated**, not committed, because a single uncompressed Letter page at
600 dpi is ~31 MB.

```bash
PPD=/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd

# tiny deterministic inputs
printf 'TEST 123\n'          > fixtures/inputs/text-1p.txt
printf 'PAGE ONE\fPAGE TWO\n' > fixtures/inputs/text-2p.txt

# CUPS raster, exactly what the printer's raster filter consumes
/usr/sbin/cupsfilter -m application/vnd.cups-raster -p "$PPD" fixtures/inputs/text-1p.txt > /tmp/1p.raster
/usr/sbin/cupsfilter -m application/vnd.cups-raster -p "$PPD" -o PageSize=A4 fixtures/inputs/text-1p.txt > /tmp/a4.raster

# inspect
python3 scripts/cupsraster.py dump /tmp/1p.raster
python3 scripts/cupsraster.py scan /tmp/1p.raster
```

Measured results are recorded in `docs/protocol.md`.

`scripts/cupsraster.py` implements the same walk as `crates/p1102-raster` and is used as
a cross-check against real captured rasters; both agree, and both land exactly on EOF for
1-, 2- and 3-page jobs, which is what validates the empirically derived header offsets.

### Rehearsing the whole boundary in one command

scripts/rehearse-job.py runs the pipeline the way CUPS will drive it, with no printer:

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/rehearse-job.py                                  # default options
$PY scripts/rehearse-job.py --options "Duplex=DuplexNoTumble EconoMode=True"
$PY scripts/rehearse-job.py --raw                            # the uncompressed path
```

Each step is real rather than simulated: the raster comes from Apple's own cgpdftoraster via
cupsfilter, the job is produced by our actual filter binary with the argv shape cupsd uses,
and the result is judged by the reference decoder. A passing run:

    rehearsal passed
    1. cgpdftoraster (Apple, native) produced a 31361800 byte CUPS raster
    2. our native filter produced a 1043 byte job
       the same job comes out with the raster on stdin
    3. chunk sequence: START_DOC START_PAGE JBIG_BIH JBIG_BID END_JBIG END_PAGE END_DOC
    4. the job carries 1 compressed page image(s) for 1 raster page(s)
    5. reference decoder vs the independently computed bitmap: pixels identical (4992x6400, 3993600 bytes)
    6. bring-up levers, the overrides the hardware notes tell you to reach for:
       P1102_NO_PJL=1: the job starts at the signature, with no wrapper
       P1102_DENSITY=5: the prologue carries DENSITY=5
       P1102_ECONOMODE=1: the page item and the PJL line both say so
       P1102_RAW=1: covered by the payload check when the rehearsal runs --raw

It covers the things that are easy to get wrong and hard to notice: that the filter reads
its raster from stdin as well as from a file argument, that PPD options reach both the page
items and the PJL, and that the uncompressed path's payload is byte-for-byte the bitmap the
raster describes. Pass --raster to rehearse a captured or synthesised raster instead of a
generated one. Each case below was judged by the reference decoder:

That second point is asserted rather than assumed: every PPD option named on the command line
is checked against the item or the PJL line it is supposed to change, so an option whose name
stopped matching would fail the rehearsal instead of quietly doing nothing. Simplex and duplex
are asserted on the chunks that actually carry them - the document for `Duplex=None`, the page,
image end and page end for a duplex mode - because that is where the reference driver puts
them too.

It prefers the native filter - the artifact that would be installed - and falls back to the
wasm build when there is none, which links with Rust's own bundled linker and needs no SDK.
Both routes were checked the same way: the whole rehearsal, levers and stdin comparison
included, was run with the native binary moved aside, and the wasm build carried it.

| case | result |
|---|---|
| generated one-page Letter raster | passes |
| PPD options: duplex (none, long edge, short edge), manual feed, econo mode, density, jam recovery | passes, and each option's effect is asserted on the item or PJL line it should change |
| raster on stdin instead of a file argument | identical job |
| the four bring-up overrides: no PJL, density, econo mode, raw | each changes the job the way the hardware notes say it does |
| captured two-page raster, 62 MB | both pages decode to the expected bitmaps |
| captured three-page raster, 94 MB | all three pages decode to the expected bitmaps |
| uncompressed path on the three-page raster | all three payloads match, each with its own ink count |
| synthesised 1 bit per pixel raster | passes, both compressed and uncompressed |
| raster with no pages | filter exits 1, no output |
| raster truncated mid-page | filter exits 1, naming the page and the shortfall |
| a hand invocation with fewer than seven arguments, and `--help` | exits 2 with the usage text on stderr, or 0 with it on stdout, and writes no job either way |
| `PPD` set in the environment, which is how CUPS passes the queue's PPD | the media source becomes the PPD's default (Manual, 4), and an explicit `InputSlot` on the command line still overrides it |

### What the rehearsal does not cover, and why

The rehearsal invokes the filter with the argv and environment CUPS documents. It cannot
invoke it *through* CUPS, and the reason is worth recording so nobody assumes otherwise:

- **cupsfilter does not run a PPD's printer filter.** It builds its chain from the MIME
  conversion table (`/usr/share/cups/mime/mime.convs`), so asking it for a destination type
  it does not know fails with `Unknown destination MIME type`, and asking for
  `printer/*` selects the built-in PostScript path. The last filter in a PPD is only ever
  run by cupsd.
   Three variants were tried before that was accepted as final: a `*cupsFilter2` line declaring
   the filter's output type, which is ignored (`No filter to convert from text/plain to
   application/vnd.cups-raw`); the same with `application/vnd.cups-postscript` as the output,
   where the shorter built-in text-to-PostScript path wins instead; and an explicit input type
   with `-i application/vnd.cups-raster`, which still looks the conversion up in the MIME
   database rather than in the PPD (`... from application/vnd.cups-raster to ...`).
- **cupsd cannot be started as a non-root user here.** `/usr/sbin/cupsd` is mode
  `0500 root:wheel`, so a private instance on a high port is not possible. Driving our filter
  through CUPS requires a queue, which requires the install step, which requires sudo.

What can be checked without a queue is the other half of the contract, and one piece of it
comes from CUPS itself rather than from documentation. cupsfilter prints the argv shape it
uses for a filter:

```text
DEBUG: argv[0]="cupsfilter"   DEBUG: argv[1]="1"        DEBUG: argv[2]="manu"
DEBUG: argv[3]=<title>        DEBUG: argv[4]="1"        DEBUG: argv[5]=<options>
DEBUG: argv[6]=<input file>
```

That is exactly what p1102-filter implements: options at argv[5], the input file at argv[6],
and stdin when there is no file. The rehearsal now runs the raster both ways and requires the
same job from each, which is what would catch a dependence on the input being seekable. The
option strings come from the printer's real PPD, so the remaining untested piece is narrow:
whether cupsd reaches the filter at all with our PPD, which the first real print answers.

Run the filter by hand and it tells you this: `--help` prints the calling convention, and a
short argument list prints it on stderr and exits 2 without writing a job. Before that guard,
a mistyped invocation wrote half a job to stdout and then complained about the raster.




## Golden-master comparison: what was intended, and what was done

The plan for this comparison was:

```text
deterministic page -> CUPS raster -> HP rastertozjs -> old-driver-output.bin
same page          -> CUPS raster -> our encoder      -> new-driver-output.bin
```

HP's own `rastertozjs` still cannot be executed here - x86_64, no Rosetta 2, no x86_64 dyld
shared cache - and running it would need a `libstdc++.6.dylib` that does not exist on this
machine either. So HP's *bytes* remain uncapturable, and that has not changed.

What did change is that the reference **implementation** turned out to be buildable here,
because the toolchain gate is a shim (see the first section). That turned option 3 of the
original list - capture from a machine where the driver works - into something that runs
locally: `scripts/foo2zjs-oracle.sh` builds the open driver that supports this printer, and
`scripts/reference-compare.py` compares whole jobs against it, byte for byte, over a set of
real option combinations. That is the comparison described under "The reference driver, as a
runnable golden master" above, and it is what `docs/differences.md` now reports against.
The page both sides see there is 1-bit: the harness derives it by thresholding the raster the
way the filter does, so what is compared is the job encoding rather than the grey conversion
(item 11 of that list, and the script now refuses a fixture with grey in it).

For HP's own output specifically, the binary can still be *read* even though it cannot be
run, which is how its BIH defaults and its complete PJL literal list were determined. Where
the two implementations are known to differ - the wrapper, the adaptive template - HP's side
is documented from that reading rather than measured, and tagged accordingly.

## Running the real code without a native linker (wasm)

wasm links with Rust's own bundled rust-lld and needs no SDK at all, which is why the test
suite runs there (`scripts/test-wasm.sh`): it works whatever state the machine's toolchain is
in, and it was the only way to run Rust at all before the toolchain binaries were found.
Native builds work now, so this is a convenience rather than a necessity - but it is also
what makes the job pipeline testable on a machine with no linker at all.

```bash
cargo build --target wasm32-wasip1 -p p1102-cli --offline
node scripts/wasi-run.mjs target/wasm32-wasip1/debug/p1102.wasm test-page square --page a4 -o /tmp/out.zjs
```

scripts/wasi-run.mjs exposes the host root at the guest root, so the tool sees the same
files the native build would. A second runtime is available as a fallback, and is the more
reliable of the two:

```bash
wasmtime run --dir / target/wasm32-wasip1/debug/p1102.wasm raster-info big.raster
```

This is a development and verification path, not the deliverable: what ships is the native
ARM64 binary. But it is what made these two checks possible without the licence:

1. **Every deterministic test page is byte-identical** to the job produced by the Python
   path, which is itself verified against the reference JBIG encoder. Eight patterns, all
   eight identical byte for byte, comparing the real Rust raster and pattern code, the
   encoder, the chunk framing and the PJL wrapper against the reference-verified
   implementation.
2. **The raster path is judged by the reference decoder.** raster-to-zjs on a captured 31 MB
   Letter raster produces an image that jbgtopbm decodes back to exactly the bitmap an
   independent computation from the same raster expects: 4992x6400, 3,993,600 bytes, 8,629
   ink pixels. That covers the CUPS raster reader, the 8 bits per pixel thresholding, the
   128-pixel width alignment, the JBIG encoding and the chunk framing in a single check.

### A caveat about node's WASI

node's WASI implementation crashes (SIGSEGV, no message) inside its own start call for some
invocations: raster-info on rasters above roughly 29 MB, reproducibly, while raster-to-mono1
and raster-to-zjs handle the same 31 MB file fine. It is not our code, and it is not a
memory ceiling:

- the guest never returns an exit code, so node dies before the program finishes;
- a ten-line wasm program that reads the same 30 MB file and prints to stdout is fine;
- a ten-line wasm program that allocates and touches 64 MB is fine;
- **wasmtime runs the identical module and input to completion with correct output.**

Use wasmtime for large inputs and node for everything else. scripts/wasi-run.mjs accepts
P1102_WASI_DEBUG=1 to report the guest's exit code, which is what distinguished "the guest
exited 139" from "node died".

## Verifying "native ARM64"


`scripts/verify-native.sh` automates the Phase 10 checks. It uses `file(1)`, which is not
gated behind the Xcode licence, and it is enough to tell a native binary from an Intel-only
one:

```bash
./scripts/verify-native.sh                     # our built and installed binaries
./scripts/verify-native.sh --include-hp        # also show HP's, as a control
./scripts/verify-native.sh --processes         # flag any x86_64 process in the print path
./scripts/verify-native.sh <file>...           # check specific files
```

It exits non-zero if anything Intel-only is found. The detection has been self-tested both
ways: HP's `rastertozjs` and `usbink` are reported `INTEL` and fail, while Apple's universal
`cups/backend/usb` is reported native and passes.

### Result, executed

```text
./scripts/verify-native.sh
  p1102                  native   Mach-O 64-bit executable arm64
  p1102-filter           native   Mach-O 64-bit executable arm64
  OK: nothing Intel-only found.
```

The native binaries were then checked against the same oracle as everything else, rather
than being trusted because they link:

- the CLI's generated test page is **byte-identical** to the job from the reference-verified
  Python path, and the wasm CLI's is byte-identical to both;
- the filter, given a real 31 MB CUPS raster produced by Apple's own cgpdftoraster, emits a
  job whose image the **reference decoder** decodes back to exactly the expected bitmap
  (4992x6400, 3,993,600 bytes);
 - the native and the wasm builds agree **byte for byte**, not merely semantically, so the
   verification done through wasm earlier carries over to the artifact that would be installed.

The three-way comparison is exact rather than approximate because the CLI's test page stamps
the epoch instead of the clock, which makes its output reproducible; the filter is the one that
takes the real time. `make-test-job.py`'s `--timestamp` defaults to the same value, so
reproducing it needs no flags:

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/make-test-job.py --pattern square --page a4 --timestamp 19700101000000 -o /tmp/py.zjs
target/release/p1102 test-page square --page a4 -o /tmp/native.zjs
node scripts/wasi-run.mjs target/wasm32-wasip1/debug/p1102.wasm test-page square --page a4 -o /tmp/wasm.zjs
cmp /tmp/py.zjs /tmp/native.zjs && cmp /tmp/py.zjs /tmp/wasm.zjs
```

What remains unverified about the native path is only what needs the printer: no page has
ever been sent to one.

## Comparing two jobs, old driver against ours


Phase 8 asks for tooling that compares outputs semantically rather than byte for byte.
`scripts/zjs-compare.py` does that:

```bash
scripts/zjs-compare.py captured-from-hp.zjs ours.zjs --ignore-jobattr
scripts/zjs-compare.py ours.zjs ours.zjs --show-normalised
```

It normalises away the two things that may legitimately differ - the PJL `JOBATTR`
timestamp, and how the compressed image happens to be divided between `JBIG_BID` chunks,
since a 64 KiB chunk boundary is transport and not a printer command - and then compares
what actually matters: every other PJL setting, the ordered chunk sequence, the item set of
each chunk **by tag name** (with duplex modes spelled out), and each page image's BIH
fields and compressed body. It exits non-zero when the two differ, so it works as a check.

Each of those behaviours is exercised: a different image, a flipped bit inside the image
body, an added `EconoMode` item, a duplex change, and the timestamp case are all detected;
the same image re-chunked into different `JBIG_BID` boundaries is correctly reported
`SEMANTICALLY IDENTICAL` despite differing byte sizes.

## Safety rules observed during reverse engineering

- Nothing under `/Library/Printers/`, `/etc/cups/`, or `/usr/libexec/cups/` was modified.
- All inspection was read-only (`file`, `codesign`, `plutil`, `gzcat`, our own parsers).
- `cupsfilter` was used to generate job data in `/tmp`; it does not touch the queue.
