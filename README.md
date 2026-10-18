# p1100-drivers

A native Apple Silicon replacement for the Intel-only HP driver of the
**HP LaserJet Professional P1100** family: an open-source macOS driver implementation for
the HP LaserJet Professional P1100 printer family. Its generic implementation was
reverse-engineered against the installed driver artifacts and reference encoder for an
**HP LaserJet Professional P1102**. P1102 is the reference target; its USB transport and
native CUPS queue were validated on hardware on 2026-09-16.

Status: the existing stack is documented, the printer protocol is understood and
specified, and a dependency-free Rust stack exists with the raster-to-job pipeline, a JBIG
compressor and a CUPS filter wired end to end. All of it is tested, and the job the filter
writes is **byte-identical to the reference driver's** - wrapper, framing, item sets,
geometry, compressed image and padding - for every option combination
`scripts/reference-compare.py` covers; the compressed image is separately byte-identical to
the reference encoder of the standard. The two-bit path HP's own driver uses for this printer is
built too - its tone tables read out of the installed driver, the screening rule read out of its
code, and the whole page checked against the reference encoder by `scripts/two-bit-compare.py` -
behind an option, with the one-bit path still the default. Hardware validation accepted PJL
and no-PJL jobs through Apple's USB backend and completed a document through the installed
native CUPS queue; the remaining device questions are recorded in `docs/differences.md`.

## Models

The family is the series HP's own driver bundle covers with one PPD and one filter:
`hp1100.ppd` declares P1102, P1104, P1106, P1107, P1108 and P1109, and CUPS picks the model
by matching the attached device's IEEE 1284 identity against those lines. Nothing in the
pipeline here is tied to one model - the PPD carries the device description, the raster
header carries the page setup, and the protocol libraries are separate from CUPS.
That structure permits later model support, but shared HP packaging does not establish
identical firmware, USB behavior, or capabilities across those models.

**Offline validation targets P1102 only.** Measurements use its PPD and installed HP
filter bundle; compatibility with other models is unverified. The independent audit,
fixes, validation results, and remaining release gates are in [docs/audit.md](docs/audit.md).

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
thin x86_64 Mach-O binaries; this project replaces both paths with native ARM64 Rust binaries:

- Rosetta 2 is available on this Mac (`arch -x86_64 /usr/bin/true` succeeds), so the old
  filter can be launched only through translation. That still violates the native ARM64
  requirement and leaves the queue dependent on an obsolete compatibility layer.
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

    crates/p1100-protocol/   ZjStream encoder + decoder. Pure, no dependencies.
    crates/p1100-raster/     CUPS raster reader and mono conversion. No libcups.
    crates/p1100-job/        CUPS raster stream -> complete printer job. No CUPS.
    crates/p1100-jbig/       JBIG (T.82) compressor for the page image. Pure.
    crates/p1100-filter/     The CUPS filter binary: a thin wrapper around p1100-job.
    crates/p1100-cli/        Offline tooling: inspect rasters, dump streams, encode a job.
    scripts/                 inspection tools, test runner, PPD generator, installer.
    assets/p1100.icns        the printer icon the queue shows, built by scripts/make-icns.py
                             (docs/installation.md says from what, and how closely the checked-in
                             file matches a fresh build).
    docs/                    findings, written as they are confirmed.
    fixtures/inputs/         tiny deterministic source documents.
    LICENSE                      MIT licence

Dependencies point one way only, so every layer below the filter can be exercised without
CUPS and without a printer.

## Building and testing

    cargo check --workspace        # type-check only, no linker needed
    ./scripts/test-native.sh       # the full test suite on this machine
    ./scripts/test-wasm.sh         # the same suite on wasm, for a machine with no linker
    ./scripts/build-native.sh      # the real native ARM64 binaries
    ./scripts/verify-all.sh        # everything in this repository, and what each check said

`scripts/test-native.sh` runs the whole suite on aarch64-apple-darwin, which is the real target;
it links with Apple's toolchain, or with Xcode's clang called directly when the licence has
not been accepted. `scripts/test-wasm.sh` runs the identical suite on wasm32-wasip1, which
links with Rust's own bundled linker and needs no SDK, so it works even with no native
toolchain at all.

For the native build, scripts/build-native.sh picks a route and says which one it used:

- **Apple's toolchain.** Xcode's own `clang` and `ld` run even when the `/usr/bin` shims in
  front of them refuse because the licence has not been accepted, so the script calls them
  directly with an explicit SDK. The output is linked by Apple's linker against the real
  SDK; accepting the licence remains the supported route and changes nothing else.
- **rust-lld with a rewritten stub SDK**, if even that is unavailable. macOS 27's SDK describes some
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

    lp -d HP_LaserJet_P1100_native /path/to/file.pdf
    lpstat -o HP_LaserJet_P1100_native                  # job state
    log show --predicate 'process == "cupsd"' --last 5m # the job log, including filter stderr

Remove it again, leaving HP's installation untouched:

    ./scripts/uninstall.sh --dry-run
    sudo ./scripts/uninstall.sh

That installs the filter, the printer icon, a generated PPD and the tone tables it reads into /Library/Printers/p1100, and creates a new
queue alongside HP's. HP's installation is read but never written, SIP is untouched, and no
kernel or system extension is involved.


## Using the command line tool and inspection scripts

Homebrew Python is needed for the scripts, because Apple's /usr/bin/python3 is behind the
same licence gate.

    p1100 raster-info     <cups-raster-file>
    p1100 raster-to-mono1 <cups-raster-file> <out.raw>
    p1100 raster-to-zjs   <cups-raster-file> <out.zjs> [--no-pjl]
    p1100 zjs-dump        <zjs-file>
    p1100 send <job.zjs> [--device URI] [--timeout SECS]   # through Apple's backend
    p1100 wrap --bih <file> --bid <file> ... --width W --height H -o out.zjs

    PY=/opt/homebrew/bin/python3.13
    $PY scripts/macho.py info   <file>          # architecture, min OS, uuid, entry
    $PY scripts/macho.py libs   <file>          # linked dylibs and rpaths
    $PY scripts/macho.py syms   <file> 'zjs'    # symbol table, filtered
    $PY scripts/macho.py strings <file> 8       # printable strings with offsets
    $PY scripts/hp-tone-table.py                # HP's grey tone tables: checked, and --convert
                                                # screens a page with them into two bits
    $PY scripts/elfinfo.py      <firmware.acl>  # ELF identity of HP device firmware
    $PY scripts/cupsraster.py dump <file>       # CUPS raster pages, validated against EOF
    $PY scripts/make-test-job.py --list         # deterministic bring-up test pages
    ./scripts/jbig-oracle.sh fetch              # reference JBIG encoder/decoder (test only)
    $PY scripts/jbig-sweep.py                   # our encoder vs the reference: 48 comparisons
    ./scripts/foo2zjs-oracle.sh fetch           # the reference driver, built here (test only)
    $PY scripts/reference-compare.py            # our whole job vs the reference driver's
    $PY scripts/zjs-compare.py a.zjs b.zjs      # semantic job comparison
    ./scripts/verify-native.sh                  # ARM64 and dependency check for our binaries
    $PY scripts/rehearse-job.py                 # rehearse the CUPS -> filter boundary
    $PY scripts/preflight-usb.py                # what the USB bus and CUPS see, read-only

The real tool can also be built and run without a compiler, because wasm links with Rust's
own linker and needs no SDK:

    cargo build --target wasm32-wasip1 -p p1100-cli --offline
    node scripts/wasi-run.mjs target/wasm32-wasip1/debug/p1100.wasm test-page square -o /tmp/x.zjs

That is how the Rust implementation was checked against the reference without the licence:
all eight test pages come out byte-identical to the reference-verified Python path, and the
raster path's image decodes with the reference JBIG decoder back to the expected bitmap.
See the wasm section in docs/testing.md, including a caveat about node's WASI.

## Generating the CUPS raster the filter consumes

    /usr/sbin/cupsfilter -m application/vnd.cups-raster \
        -p /etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd \
        fixtures/inputs/text-1p.txt > /tmp/1p.raster
    $PY scripts/cupsraster.py dump /tmp/1p.raster

## Framing an image the reference encoder compressed

`p1100 wrap` builds a complete job around a JBIG image our own encoder did not produce. That is
what separates "the printer rejects our framing" from "the printer rejects our compression": a
page the reference implementation compressed goes out as a normal job, and the pieces come
straight out of that encoder's output, because its BIH is byte-identical to ours for the same
page and sits in the first twenty bytes:

    $PY scripts/make-test-job.py --pattern square --page letter \
        -o /tmp/ours.zjs --keep-pbm /tmp/ref-in.pbm
    ./scripts/jbig-oracle.sh encode /tmp/ref-in.pbm /tmp/ref.jbg
    dd if=/tmp/ref.jbg bs=1 count=20 of=/tmp/ref.bih
    dd if=/tmp/ref.jbg bs=1 skip=20 of=/tmp/ref.bid
    p1100 wrap --bih /tmp/ref.bih --bid /tmp/ref.bid --width 4900 --height 6400 \
        -o /tmp/ref.zjs
    p1100 zjs-dump /tmp/ref.zjs

`--width` is the page's true pixel width; the 128-pixel alignment the printer expects is added
by the wrapper. The result is an ordinary job file, so it goes to the printer the same way any
other one does.

## Making a two-bit page, the way HP's driver would

HP's driver screens grey into two bits per pixel through a 12x12 tone table (docs/differences.md
item 11), and that page can be built here, which is how the difference gets settled on a printer:

    PY=/opt/homebrew/bin/python3.13
    $PY scripts/hp-tone-table.py --convert /tmp/1p.raster --pages /tmp/twobit
    ./scripts/jbig-oracle.sh encode /tmp/twobit/page-1.pgm /tmp/twobit/page.jbg -b -p 28
    dd if=/tmp/twobit/page.jbg bs=1 count=20 of=/tmp/twobit/page.bih
    dd if=/tmp/twobit/page.jbg bs=1 skip=20 of=/tmp/twobit/page.bid
    p1100 wrap --bih /tmp/twobit/page.bih --bid /tmp/twobit/page.bid \
        --bits 2 --width 4900 --height 6400 -o /tmp/twobit/page.zjs

The same job comes out of one command, through this repository's own conversion and two-plane
encoder rather than the tool plus the reference encoder, and `scripts/two-bit-compare.py`
requires the two to agree:

    $PY scripts/hp-tone-table.py --out /tmp/twobit
    p1100 raster-to-zjs /tmp/1p.raster /tmp/twobit/ours.zjs --2bit \
        --tone-table /tmp/twobit/sdtahs10.bin

The screening uses the tables out of the installed HP driver, so it is what that driver would
have sent; the pages are written at the aligned width, and `-b` selects binary rather than
Gray-coded words in the two planes, which a decoder has to be told because the BIE does not say.
`--bits 2` is what makes the page items read `ZJI_VIDEO_BPP` 2, `ZJI_NBIE` 2 and
`ZJI_VIDEO_X` = width/2, which is our reading of HP's own item emission and needs the printer
to confirm. A page of only black and white screens to itself - the tables take 0 to level 0 and
255 to level 3, and `--convert` says so when it sees such a page - so a one-bit job and a
two-bit job of the same document are the same page, which is what makes comparing them on a
printer mean anything.

## How the plan maps onto this repository

The request had eleven phases. Where each one landed, and what its evidence is:

| phase | artifact | state |
|---|---|---|
| 1. inventory the installed stack | `docs/existing-driver.md` | done, from `lpstat`, `ioreg`, the registry, `pkgutil` and the files themselves |
| 2. find the obsolete Intel component | `docs/existing-driver.md` component table | done: 337 Mach-O binaries in the HP installation, none with an arm64 slice |
| 3. determine the protocol | `docs/protocol.md`, `crates/p1100-protocol` | done, and the encoder's output is byte-identical to the reference driver's for the same page |
| 4. check for a known open protocol | `docs/research.md` | done: the reference implementation is built and run here as a test oracle |
| 5. firmware handling | `docs/protocol.md` | resolved for this bundle (no firmware blob, none uploaded); the device side needs the printer |
| 6. design the replacement | `docs/architecture.md` | done: PPD plus filter, chosen explicitly over a Printer Application and re-hosting kept cheap |
| 7. Rust implementation | `crates/` | done: four pure libraries, a CLI and the filter, no dependencies beyond libSystem |
| 8. golden-master comparison | `scripts/reference-compare.py` | 31 cases against the open reference; HP's translated filter emits only a partial prologue and aborts on the current CUPS raster |
| 9. real printer testing | `scripts/make-test-job.py`, `docs/testing.md` | done for USB identity, deterministic raster patterns, compressed JBIG jobs, PJL/no-PJL framing, raw transfer and the native CUPS queue; physical duplex order, media rendering and power-cycle checks remain |
| 10. native ARM64 verification | `scripts/verify-native.sh` | done for the binaries and the native queue; the post-print process check found no p1100/rastertozjs/usbink helper running |
| 11. installation | `scripts/install.sh`, `scripts/uninstall.sh` | done: the privileged install created the side-by-side queue, strict cupstestppd passed, and the rehearsal executes the reversible file path in a scratch directory |

Phase 9 now has hardware evidence. The remaining items are deliberately limited to behaviours
that require observing the printer's paper handling or a power cycle.

## What is verified, and what is not

Verified by execution, on real captured data where it matters:

- The ZjStream byte layout: chunk and item headers, big-endian field placement,
  reserved = 12 x items, item ordering for every chunk the driver emits, JBIG chunk
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
- **The generated PPD, as CUPS itself reads it.** `scripts/ppd-compare.py` generates the file
  install.sh would install - from both PPDs install.sh can start from, the queue's and Apple's -
  drives `cupsfilter` with it for six option combinations each (defaults, three page sizes, the
  resolution choice that moves the `cupsCompression` marker, and a media type that moves the
  header's media string) and requires the raster to be byte-identical to the source PPD's for
  the same document. All twelve comparisons match, and `/usr/bin/cupstestppd`, CUPS's own
  conformance test, reports no failures on either. This is also the comparison that caught the
  file being unopenable: a PPD whose first line is a comment makes cupsfilter fall back to its
  built-in conversion rules - 100 dpi, one bit per pixel, 119,500 bytes for the test page,
  against 31,361,800 for the real thing - instead of refusing, so a queue installed from it
  would have printed the fallback page rather than reporting a problem.
- **The install and the uninstall, executed.** `scripts/rehearse-install.sh` runs install.sh
  with `--dest` in a scratch directory and `--no-queue`, so the copies and their modes, the two
  tone tables read out of HP's driver, the PPD written into the destination and the checks on it,
  and then the uninstall that removes the lot, all happen for real. Twenty-three assertions, which
  include that nothing was created at the system destination or in the print system, and the
  refusals around a recursive delete: an HP location, an Apple location, `/etc/cups`, `/`, a
  relative path, an empty one, and the system destination without root. What this does not reach
  is `lpadmin` and the strict cupstestppd run, and both need root.
- The PPD option mapping, tested against the real value names from the installed PPD:
  `Duplex=DuplexNoTumble`/`DuplexTumble`/`None` and the IPP `sides=` spellings of the same
  choices, `InputSlot=Manual` to the tray code, `pjlDensity` clamped to the 1..5 the PPD
  offers, `EconoMode` and `pjlJamRecovery`, and the raster's `cupsMediaType` forwarded as the
  media code. Unrecognised keys and values are ignored rather than guessed.
  Options a job does not name are taken from the PPD, the way CUPS's filter interface expects
  and the way HP's driver does it: CUPS passes only what the user changed, so the file's
  `*Default*` keywords fill the rest and the command line overrides them.
- **The bring-up tool, against the shipped framing and against CUPS.**
  `scripts/bringup-compare.py` builds each test page with `scripts/make-test-job.py` - the tool a
  hardware session uses first, whose Python framing mirrors `crates/p1100-protocol` - has
  `p1100 wrap` frame the same reference-compressed image with the Rust code, and asks
  `p1100 test-page` for the same pattern. All three agree in all ten cases, byte for byte, so a
  printer that rejects a test page is rejecting what the filter sends. The same script checks what
  each test page claims about the page against the job the filter builds from a real CUPS raster of
  that size, which is how both Rust and Python tables were found to be giving Legal 8100 px where a
  real Legal job carries 8200.
- The streaming filter: peak memory is flat in the job size. Measured now with
  `/usr/bin/time -l`, peak RSS is 37.3 MB for a 31 MB one-page raster, 37.4 MB for 94 MB and
  37.5 MB for 313 MB - a ten times larger job for 0.3% more memory - against 312 MB for the
  last of those when the filter buffered the whole job. Output was byte-identical across the
  change, and the reference decoder still reproduces every page.
- **The JBIG compressor**, in both configurations it supports, and in one plane or two. Its
  output is byte-identical to the reference encoder of the standard across 48 comparisons
  (`scripts/jbig-sweep.py`: random noise, odd widths, single rows and columns, stripe
  boundaries, widths 1 to 1200, and six two-plane pages), and five oracle-derived vectors are
  embedded as golden tests that run on every build. The
  job layer is separately tested to reassemble exactly what the compressor produced, and the
  raster stride is asserted to equal the JBIG line length, since a mismatch there would
  silently shear every page.
- **The two-bit screening**, as far as it can be without the printer: a page whose greys are
  only 0 and 255 screens to itself, so a one-bit job and a two-bit job of the same document are
  the same page, and a screened page survives the whole round trip - screen, encode into two
  planes, frame, reassemble, decode with the reference decoder - with identical samples. The
  screening rule itself is read out of HP's driver (item 11 of `docs/differences.md`); what it
  looks like on paper and whether the printer takes that framing are the observations left.
- **The whole job, against a working driver.** `scripts/reference-compare.py` builds the
  open driver that supports this printer and requires our filter's output to be
  byte-identical to its, for the same page and options, once the clock-bearing
  job-attribute line is removed. Thirty-one cases pass: the print options (defaults, econo
  mode, density, manual feed, three copies, two pages), both resolutions the PPD offers, every
  page size it declares plus a custom one, and the layout CUPS applies before rasterising
  (landscape, upside down, two pages per sheet). Between them they cover the wrapper, the
  chunk sequence, every item and its order, the geometry, the page size code, the compressed
  image and the final padding.
- **HP's own encoder constants**, read out of its binary rather than assumed: the BIH
  options byte `0x5c`, order `3` and the adaptive-template default are visible in
  `jbig::ZjbigEncoder`'s constructor, and every PJL literal the filter contains is listed in
  `docs/differences.md`.

Not verified:

- Duplex ordering, non-plain media codes, the uncompressed image layout and power-cycle
  firmware behaviour remain unverified on the printer.
- Whether the printer acts on non-plain media codes, keeps duplex pages in the expected
  physical order, accepts the raw-image fallback and needs firmware after a power cycle.

## Remaining gates

1. Duplex ordering, non-plain media handling, raw-image compatibility and power-cycle
   firmware behaviour need further physical observations.
2. Optional, and only a convenience: sudo xcodebuild-license accept restores the `/usr/bin`
   shims and `brew`. Nothing here depends on it any more - the toolchain's own binaries are
   called directly - but it is the supported route, and the build says which one it used.

## Next steps

1. Run the duplex, media and power-cycle recipes in `docs/testing.md` while the P1102 is
   attached.
2. If the compressed image ever proves unsuitable, `P1100_RAW=1` switches the filter to the
   uncompressed ZJT_RAW_IMAGE form, whose layout is the one thing here with no independent
   confirmation.

## Safety

Nothing in /Library/Printers/hp, /Library/Printers/PPDs, /etc/cups or /usr/libexec/cups has
been modified. All inspection is read-only, and cupsfilter writes job data to /tmp only.
The installer adds a new queue and a new directory and never writes over an Apple or HP
file; see docs/installation.md.

This project is licensed under the MIT licence; see LICENSE.

A permissive licence was chosen deliberately and the reasoning is in docs/research.md. In
short: the implementation is our own, written from the documented formats, the published
standards and observed behaviour; the GPL reference implementations were read to understand
the protocols and are used only as test oracles, never linked, copied or shipped. The one
thing necessarily shared is the normative T.82 arithmetic coder table, which every conforming
JBIG implementation must contain.

The copyright holder is Emanuele Coppola, as stated in LICENSE.
