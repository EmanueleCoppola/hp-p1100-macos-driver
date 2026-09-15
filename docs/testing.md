# Test environment, toolchain constraints, fixtures

## This machine

```text
macOS 27.0 (build 26A428), Darwin 27.0.0, Apple M2 Max, arm64
```

## Toolchain blockers found (CONFIRMED)

1. **Xcode licence not accepted.** `/usr/bin/{clang,cc,otool,lipo,nm,strings,python3,make}`
   are shims that refuse to run:

   ```text
   You have not agreed to the Xcode license agreements.
   Please run 'sudo xcodebuild -license' from within a Terminal window …
   ```

   Full Xcode 26.3 is installed at `/Applications/Xcode.app` and
   `/Library/Preferences/com.apple.dt.Xcode.plist` records agreement to `26.3`, but
   macOS 27 ships a newer licence text, so re-acceptance is required. This needs an
   admin password and therefore the user; it blocks compiling Rust and blocks Apple's
   Mach-O tools.

   Workaround in place: `scripts/macho.py` and `scripts/elfinfo.py` are pure-Python and
   run under Homebrew Python, which is independent of Apple's tools.

2. **No Rosetta 2.** `arch -x86_64 /usr/bin/true` -> `Bad CPU type in executable`.
   Only `RosettaLinux` is present. No x86_64 dyld shared cache exists.

3. **`libstdc++.6.dylib` is not under `/usr/lib`**, and HP's filter needs it. The other two
   HP-era libraries are *not* missing: `libcups.2.dylib` and `libcupsimage.2.dylib` resolve
   out of the shared cache, which is proven by Apple's own filters - which link them -
   running here. An earlier version of this file said otherwise, because it checked
   `/usr/lib` for the file; that is not a valid test for a dylib on modern macOS.

4. **No CUPS logging and no readable registry of past jobs.** `/var/log/cups/` contains
   only `access_log`; there is no `error_log`. `/etc/cups/printers.conf` is mode 600
   root:_lp and was not read.

5. `sudo` is not passwordless (`sudo -n true` -> `a password is required`).

### Building natively without Apple's toolchain

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
`~/.cache/p1005-stub-sdk`, nothing outside that directory is touched. For an arm64 link those
entries never apply anyway, so the symbol sets are the ones Apple's linker would use. Then:

```bash
scripts/build-native.sh          # picks the route and says which one it used
```

produces ordinary native binaries:

```text
target/release/p1005:        Mach-O 64-bit executable arm64
target/release/p1005-filter: Mach-O 64-bit executable arm64
```

Linking against stubs only describes the library at link time; the binary imports the real
runtime `libSystem` from the OS. The output runs natively and is verified against the
reference decoder exactly as the wasm build was.

Accepting the Xcode licence remains the supported route, and it also restores `otool`,
`nm`, `lipo`, `strings` and `brew`. This exists so the project can be built and verified
without it, not as a replacement for it.

### Working workaround: run tests on wasm32-wasip1


### Working workaround: run tests on wasm32-wasip1

Linking wasm needs no SDK, so pure-logic tests do run today:

```bash
rustup target add wasm32-wasip1
./scripts/test-wasm.sh
```

That script passes an absolute runner path via `--config`, because cargo runs test
binaries with the *package* directory as cwd, so a relative runner path in
`.cargo/config.toml` does not resolve. Tests execute under `node`'s WASI implementation
(`scripts/wasi-test-runner.mjs`), serially, since wasm has no threads.

The runner also forces `--nocapture`. wasm has no unwinding, so libtest cannot catch a
panic and therefore never regains control to print a failing test's captured output;
without `--nocapture` a failure shows up only as an opaque `RuntimeError: unreachable`
with no indication of the assertion. With it, the panic message reaches the real stderr.

This matters in practice: three test-authoring mistakes in this repo were found only
because the assertion text became visible that way.

This covers everything that matters most and is pure: the ZjStream byte layout, item
ordering, `reserved` consistency, JBIG chunk padding rules, the PJL framing and its
stripping, the CUPS raster page walk and header round trip, the full raster-to-job chunk
sequence, and the filter's option parsing. It does not cover anything that needs a
linker-produced binary, the filesystem, USB, or the printer.

## Tools available right now

| tool | status |
|---|---|
| `rustc` / `cargo` 1.98.0 | present, but linking currently fails behind the Xcode licence gate |
| `/opt/homebrew/bin/python3.13`, `python3.10` | present and working |
| `file`, `xxd`, `hexdump`, `codesign`, `plutil`, `gzcat`, `cupsfilter`, `lpstat` | present and working |
| our `scripts/macho.py`, `scripts/elfinfo.py`, `scripts/cupsraster.py` | working |
| `otool`, `nm`, `lipo`, `strings`, `clang`, `/usr/bin/python3` | blocked by Xcode licence |

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
the library HP itself bundled, which is what makes it the right reference.

## Hardware bring-up with no build at all

Both blockers on hardware testing - the Xcode licence and the missing printer - are worth
clearing, but the first test needs neither, because a complete job can be produced without
compiling and delivered without installing:

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/make-test-job.py --pattern square --page a4 -o /tmp/square.zjs --verify

DEV=$(lpstat -v HP_LaserJet_Professional_P1102 | sed -n 's/^device for [^:]*: //p')
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

`scripts/cupsraster.py` implements the same walk as `crates/p1005-raster` and is used as
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
    2. our filter produced a 987 byte job
    3. chunk sequence: START_DOC START_PAGE JBIG_BIH JBIG_BID END_JBIG END_PAGE END_DOC
    4. extracted the compressed image: 395 bytes (BIH 20 + body 375)
    5. reference decoder vs the independently computed bitmap: pixels identical (4992x6400, 3993600 bytes)

It covers the things that are easy to get wrong and hard to notice: that the filter reads
its raster from stdin as well as from a file argument, that PPD options reach both the page
items and the PJL, and that the uncompressed path's payload is byte-for-byte the bitmap the
raster describes. Pass --raster to rehearse a captured or synthesised raster instead of a
generated one. Each case below was judged by the reference decoder:

| case | result |
|---|---|
| generated one-page Letter raster | passes |
| PPD options: duplex, manual feed, econo mode, density, jam recovery | passes, and the 13 expected semantic changes appear |
| raster on stdin instead of a file argument | identical job |
| captured two-page raster, 62 MB | both pages decode to the expected bitmaps |
| captured three-page raster, 94 MB | all three pages decode to the expected bitmaps |
| uncompressed path on the three-page raster | all three payloads match, each with its own ink count |
| synthesised 1 bit per pixel raster | passes, both compressed and uncompressed |
| raster with no pages | filter exits 1, no output |
| raster truncated mid-page | filter exits 1, naming the page and the shortfall |

It runs the filter from its wasm build, so it needs wasmtime or node; pass P1005_WASMTIME to
point at a wasmtime binary that is not on PATH.


## Golden-master strategy, and why it is currently blocked


The intended reference pipeline was:

```text
deterministic page -> CUPS raster -> HP rastertozjs -> old-driver-output.bin
same page          -> CUPS raster -> our encoder      -> new-driver-output.bin
```

`rastertozjs` cannot be executed on this machine (x86_64 + no Rosetta + missing
`libcups`). Three ways forward, in order of preference:

1. **Differential capture against the printer.** Send our own encoder's output,
   capture what the device tolerates, and compare against foo2zjs output generated
   off-machine. Keeps us honest without needing the old binary to run.
2. **Run the old filter under a harness.** Install Rosetta 2, then supply a tiny
   `libcups`/`libcupsimage` compatibility shim (`cupsRasterOpen`, `cupsRasterReadHeader2`,
   `cupsRasterReadPixels`, …) so the reference filter can read our raster and emit its
   ZjStream. This is legitimate for *test fixture generation* only; it is explicitly not
   the deliverable, and it does not fix the missing `libstdc++.6.dylib`.
3. **Capture from a machine where the driver still works** (an Intel Mac, or a Linux box
   running foo2zjs) and bring the `.bin` back as a fixture.

## Running the real code without a compiler (wasm)

The licence gate blocks linking *native* binaries, not wasm: wasm links with Rust's own
bundled rust-lld and needs no SDK. So the real command line tool can be built and executed
here, which is how the Rust implementation was checked against the reference rather than
only unit tested.

```bash
cargo build --target wasm32-wasip1 -p p1005-cli --offline
node scripts/wasi-run.mjs target/wasm32-wasip1/debug/p1005.wasm test-page square --page a4 -o /tmp/out.zjs
```

scripts/wasi-run.mjs exposes the host root at the guest root, so the tool sees the same
files the native build would. A second runtime is available as a fallback, and is the more
reliable of the two:

```bash
wasmtime run --dir / target/wasm32-wasip1/debug/p1005.wasm raster-info big.raster
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
P1005_WASI_DEBUG=1 to report the guest's exit code, which is what distinguished "the guest
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
  p1005                  native   Mach-O 64-bit executable arm64
  p1005-filter           native   Mach-O 64-bit executable arm64
  OK: nothing Intel-only found.
```

The native binaries were then checked against the same oracle as everything else, rather
than being trusted because they link:

- the CLI's generated test page is **byte-identical** to the job from the reference-verified
  Python path;
- the filter, given a real 31 MB CUPS raster produced by Apple's own cgpdftoraster, emits a
  job whose image the **reference decoder** decodes back to exactly the expected bitmap
  (4992x6400, 3,993,600 bytes);
- the native and the wasm builds produce **semantically identical** jobs, so the verification
  done through wasm earlier carries over to the artifact that would actually be installed.

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
