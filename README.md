# p1102-drivers

A native Apple Silicon replacement for the Intel-only HP driver that drives the
**HP LaserJet Professional P1102**.

Status: the existing stack is documented, the printer protocol is understood and
specified, and a dependency-free Rust stack exists with the raster-to-job pipeline, a JBIG
compressor and a CUPS filter wired end to end. All of it is tested, and the compressor is
byte-identical to the reference implementation. **Nothing has been validated on hardware
yet** - the printer has never been attached during this work.

scripts/make-test-job.py builds complete, reference-correct ZjStream jobs independently of
the filter, which is useful for bring-up: it can emit either a payload compressed by the
reference encoder or the uncompressed ZJT_RAW_IMAGE form, over our own chunk framing, item
sets and PJL wrapper.

Those jobs can be sent to the printer **without building or installing anything**, by
pointing a raw queue at the device and pushing the file straight through Apple's own USB
backend - see the bring-up section in docs/testing.md. That is the fastest way to find out
whether the printer accepts our framing before committing to an install.

## Why this exists

The queue on this Mac is

    HP_LaserJet_Professional_P1102 -> usb://Hewlett-Packard/HP LaserJet Professional P1102?serial=000000000Q80E182PR1a

and its PPD routes raster data through HP's rastertozjs, with usbink for commands. Both are
thin x86_64 Mach-O binaries and neither can run here:

- Rosetta 2 is not installed and there is no x86_64 dyld shared cache (arch -x86_64
  /usr/bin/true -> "Bad CPU type in executable"), so no x86_64 process can launch at all.
- libstdc++.6.dylib, which rastertozjs also needs, is not present under /usr/lib.

The architecture is the decisive blocker. Being precise about the rest, because an earlier
version of this file was not: libcups.2.dylib and libcupsimage.2.dylib are **not** missing.
Apple's own cgpdftoraster and the cupsd backends link libcups.2.dylib and run here, so dyld
resolves it out of the shared cache rather than from a file on disk. We still avoid linking
it, for independence rather than necessity; see docs/architecture.md.

So the queue is already broken, not merely at risk. This replaces the part that actually
needs replacing: a native ARM64 filter that turns a CUPS raster into the ZjStream the
printer wants, with no libcups dependency and no Rosetta.

## Documentation

| document | contents |
|---|---|
| docs/existing-driver.md | the current HP stack end to end, with paths and architectures |
| docs/protocol.md | the printer protocol: framing, records, opcodes, items, JBIG, the CUPS raster input |
| docs/architecture.md | the replacement's architecture, and why it was chosen |
| docs/jbig.md | the JBIG (T.82) page image format: BIH layout, stripe framing, parameters |
| docs/installation.md | what gets installed, where, why, and how to remove it |
| docs/differences.md | every known difference from HP's driver, and what settles each one |
| docs/research.md | external references and licensing implications |
| docs/testing.md | environment constraints, how tests run today, fixtures |

## The pipeline being replaced

    Application -> macOS print system -> cupsd
                -> cgpdftoraster        (Apple, arm64e)   PDF -> CUPS raster (RaS3)
                -> rastertozjs          (HP, x86_64)      CUPS raster -> ZjStream   <- ours
                -> usb backend          (Apple, arm64e)   USB bulk -> printer

The transport and the PDF-to-raster stage are already native and Apple-signed. Only the
middle stage is ours.

## Repository layout

    crates/p1102-protocol/   ZjStream encoder + decoder. Pure, no dependencies.
    crates/p1102-raster/     CUPS raster reader and mono conversion. No libcups.
    crates/p1102-job/        CUPS raster stream -> complete printer job. No CUPS.
    crates/p1102-jbig/       JBIG (T.82) compressor for the page image. Pure.
    crates/p1102-filter/     The CUPS filter binary: a thin wrapper around p1102-job.
    crates/p1102-cli/        Offline tooling: inspect rasters, dump streams, encode a job.
    scripts/                 inspection tools, test runner, PPD generator, installer.
    docs/                    findings, written as they are confirmed.
    fixtures/inputs/         tiny deterministic source documents.
    LICENSE, LICENSE-MIT, LICENSE-APACHE   dual MIT / Apache-2.0

Dependencies point one way only, so every layer below the filter can be exercised without
CUPS and without a printer.

## Building and testing

    cargo check --workspace        # type-check only, no linker needed
    rustup target add wasm32-wasip1
    ./scripts/test-wasm.sh         # the full test suite, no linker needed
    ./scripts/build-native.sh      # the real native ARM64 binaries

The test suite runs on wasm32-wasip1 because that target links with Rust's own bundled
linker and needs no SDK at all, so tests work regardless of the machine's toolchain state.

For the native build, scripts/build-native.sh picks a route and says which one it used:

- **Apple's toolchain**, if the Xcode licence is accepted. This is the supported path.
- **rust-lld with a rewritten stub SDK**, if it is not. macOS 27's SDK describes some
  libraries with an architecture token, `arm64e.x1`, that the LLVM inside the current Rust
  toolchain does not recognise, so rust-lld rejects those .tbd files as malformed and cannot
  resolve libSystem. The script builds a link-only copy of the stubs without that token
  (scripts/make-stub-sdk.py, non-destructive, in ~/.cache) and links against it. The output is
  an ordinary native arm64 binary linked against the real runtime libSystem; only the link
  step differed.

Either route produces binaries that `scripts/verify-native.sh` reports as
`Mach-O 64-bit executable arm64`.

## Installing, printing, removing


Build and test:

    ./scripts/build-native.sh
    rustup target add wasm32-wasip1 && ./scripts/test-wasm.sh

Install (see docs/installation.md for what lands where, and why):

    ./scripts/install.sh --dry-run    # prints every command it would run, needs no privileges
    sudo ./scripts/install.sh

Print:

    lp -d HP_LaserJet_P1102_native /path/to/file.pdf
    lpstat -o HP_LaserJet_P1102_native                  # job state
    log show --predicate 'process == "cupsd"' --last 5m # the job log, including filter stderr

Remove it again, leaving HP's installation untouched:

    ./scripts/uninstall.sh --dry-run
    sudo ./scripts/uninstall.sh

That installs the filter and a generated PPD into /Library/Printers/p1102 and creates a new
queue alongside HP's. HP's installation is read but never written, SIP is untouched, and no
kernel or system extension is involved.


## Using the command line tool and inspection scripts

Homebrew Python is needed for the scripts, because Apple's /usr/bin/python3 is behind the
same licence gate.

    p1102 raster-info     <cups-raster-file>
    p1102 raster-to-mono1 <cups-raster-file> <out.raw>
    p1102 raster-to-zjs   <cups-raster-file> <out.zjs> [--no-pjl]
    p1102 zjs-dump        <zjs-file>
    p1102 wrap --bih <file> --bid <file> ... --width W --height H -o out.zjs

    PY=/opt/homebrew/bin/python3.13
    $PY scripts/macho.py info   <file>          # architecture, min OS, uuid, entry
    $PY scripts/macho.py libs   <file>          # linked dylibs and rpaths
    $PY scripts/macho.py syms   <file> 'zjs'    # symbol table, filtered
    $PY scripts/macho.py strings <file> 8       # printable strings with offsets
    $PY scripts/elfinfo.py      <firmware.acl>  # ELF identity of HP device firmware
    $PY scripts/cupsraster.py dump <file>       # CUPS raster pages, validated against EOF
    $PY scripts/make-test-job.py --list         # deterministic bring-up test pages
    ./scripts/jbig-oracle.sh fetch              # reference JBIG encoder/decoder (test only)
    $PY scripts/zjs-compare.py a.zjs b.zjs      # semantic job comparison
    ./scripts/verify-native.sh                  # ARM64 check for our binaries
    $PY scripts/rehearse-job.py                 # rehearse the CUPS -> filter boundary
    $PY scripts/preflight-usb.py                # what the USB bus and CUPS see, read-only

The real tool can also be built and run without a compiler, because wasm links with Rust's
own linker and needs no SDK:

    cargo build --target wasm32-wasip1 -p p1102-cli --offline
    node scripts/wasi-run.mjs target/wasm32-wasip1/debug/p1102.wasm test-page square -o /tmp/x.zjs

That is how the Rust implementation was checked against the reference without the licence:
all eight test pages come out byte-identical to the reference-verified Python path, and the
raster path's image decodes with the reference JBIG decoder back to the expected bitmap.
See the wasm section in docs/testing.md, including a caveat about node's WASI.

## Generating the CUPS raster the filter consumes

    /usr/sbin/cupsfilter -m application/vnd.cups-raster \
        -p /etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd \
        fixtures/inputs/text-1p.txt > /tmp/1p.raster
    $PY scripts/cupsraster.py dump /tmp/1p.raster

## What is verified, and what is not

Verified by execution, on real captured data where it matters:

- The ZjStream byte layout: chunk and item headers, big-endian field placement,
  reserved = 12 x items, item ordering for every chunk the P1102 path uses, JBIG chunk
  padding rules, the 4-byte JZJZ stream signature, and the PJL framing.
- The CUPS raster reader, including a header round trip, and that decoded geometry
  accounts for every byte of the stream.
- The raster layout itself: reconciled against CUPS's own cups_page_header2_t definition,
  with every field cross-checked by differencing a Letter job against an A4 job. 1-, 2-
  and 3-page captures all walk to an exact EOF.
- The full raster-to-job pipeline: chunk sequence, per-page framing, the uncompressed
  image payload and its stride, paper code derivation from the page size name, and PJL
  framing on and off.
- The filter's option parsing, and the PPD generator (its output differs from HP's PPD by
  exactly the intended lines).
- The PPD option mapping, tested against the real value names from the installed PPD:
  `Duplex=DuplexNoTumble`/`DuplexTumble`/`None` (CUPS sends the PPD spelling, not the IPP
  `sides=` one), `InputSlot=Manual` to the tray code, `pjlDensity` clamped to the 1..5 the
  PPD offers, `EconoMode` and `pjlJamRecovery`, and the raster's `cupsMediaType` forwarded
  as the media code. Unrecognised keys and values are ignored rather than guessed.
- The job builder and the JBIG oracle: all eight deterministic test pages build, walk back
  with the expected chunk sequence, and round-trip their compressed image through the
  JBIG_BIH / JBIG_BID split unchanged.
- The streaming filter: peak memory is flat in the job size (43 MB for a 299 MB raster,
  against 312 MB when it buffered the whole job), with byte-identical output before and
  after, and the reference decoder still reproducing every page.
- **The JBIG compressor.** Its output is byte-identical to the reference implementation
  across a 22-case sweep (random noise, odd widths, single rows and columns, stripe
  boundaries, widths 1 to 1200, two stripe sizes), and four of those vectors are embedded
  as golden tests that run on every build. The job layer is separately tested to reassemble
  exactly what the compressor produced, and the raster stride is asserted to equal the
  JBIG line length, since a mismatch there would silently shear every page.

Not verified:

- Anything requiring the printer: USB identity, endpoint layout, whether the framing and
  the uncompressed image are accepted, and whether a power cycle needs a firmware download.
- A native ARM64 build of the final binaries, pending the licence fix above.

## Blockers that need the user

1. **Hardware.** The P1102 is not attached, so USB descriptors and on-printer behaviour
   cannot be observed. This is the one that matters: every remaining question is about what
   the printer accepts.
2. **Administrator access, to install.** Writing to /Library/Printers and creating a queue
   needs sudo. The build itself no longer needs anything beyond scripts/build-native.sh.
3. Optional, and only a convenience: sudo xcodebuild -license accept restores Apple's
   otool, nm, lipo and brew, and makes the native build take the supported route rather than
   the rust-lld fallback described above.

## Next steps

1. Re-accept the Xcode licence and confirm cargo build emits an ARM64 binary.
2. Attach the P1102 and run `scripts/preflight-usb.py`. It reports VID/PID, serial,
   interface class and endpoint count, checks the serial against the one the queue asks
   for, and - by diffing two `--json` runs across a power cycle - whether the device
   re-enumerates with a different identity.
3. Send a minimal page. The default path is now JBIG-compressed, so the first question is
   simply whether a page prints; `P1102_RAW=1` switches the filter to the uncompressed
   ZJT_RAW_IMAGE form if the compressed one is rejected.
4. Only then look at golden-master comparison. The old filter cannot run here, so
   reference output has to come from a capture or from another machine; options are in
   docs/testing.md.

## Safety

Nothing in /Library/Printers/hp, /Library/Printers/PPDs, /etc/cups or /usr/libexec/cups has
been modified. All inspection is read-only, and cupsfilter writes job data to /tmp only.
The installer adds a new queue and a new directory and never writes over an Apple or HP
file; see docs/installation.md.

MIT OR Apache-2.0, at your option: see LICENSE, LICENSE-MIT and LICENSE-APACHE. Both texts
are the canonical SPDX versions.

A permissive licence was chosen deliberately and the reasoning is in docs/research.md. In
short: the implementation is our own, written from the documented formats, the published
standards and observed behaviour; the GPL reference implementations were read to understand
the protocols and are used only as test oracles, never linked, copied or shipped. The one
thing necessarily shared is the normative T.82 arithmetic coder table, which every conforming
JBIG implementation must contain.

Say the word if you would rather it were GPL-2.0, or if you want a specific copyright holder
named instead of "the p1102-drivers authors"; both are one-line changes.
