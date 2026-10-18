# Independent engineering audit (2026-09-16)

This records the independent audit pass over the implementation and the first hardware pass:
what was verified rather than trusted, what changed and why, and what remains unproven.

## Overall assessment

The architecture is sound and unusually well evidenced for a reverse-engineering project.
The crate boundaries (raster, JBIG, protocol, job, filter) are clean, the filter is a
standard CUPS stdout filter with no USB code of its own, and the protocol work is checked
against two independent references (foo2zjs and jbigkit) rather than against itself.

The audit found no defect in production wire settings. A dormant JBIG option bug
(0x80 incorrectly selecting SDRST) was fixed without changing production output. It found real input-safety defects at the
trust boundaries (raster header, ZjStream decoder), one streaming-output defect that could
send a partial job after a valid first page failed to encode, and one destructive installer
defect (recursive delete on an arbitrary absolute directory). All are fixed with focused
diffs and regression tests; no component was rewritten.

## Hardware validation (2026-09-16)

The attached device reported Hewlett-Packard `03f0:002a`, product `HP LaserJet Professional
P1102`, serial `000000000Q80E182PR1a`, USB 2.0 High Speed, and a bidirectional Printer
interface `7/1/2` with two endpoints. The CUPS USB backend listed the same URI and the queue
serial matched.

Nine direct jobs were accepted through Apple's backend: A4 square with PJL (747 bytes), the
same A4 square without PJL (452 bytes), Letter text (819 bytes), Legal border (791 bytes),
A4 grey (1111 bytes), and A4 blank, horizontal-line, vertical-line and checker patterns
without PJL (440, 452, 448 and 1432 bytes). Each backend run wrote the complete job and
returned after the device exchange. Text submitted through `HP_LaserJet_P1100_native`
completed as CUPS job 402; the Screened2 job 403 and two-page duplex job 404 also completed
and left the queue idle. A further CUPS job 408 using the PPD's `MediaType=Heavy111-130g`
choice also completed; the printer's paper handling and the visual result still need a human
check.

The native queue installed under `/Library/Printers/p1100` uses the ARM64 filter and the
correct device URI. The HP queue remains present with its original x86_64 filters. These
results establish transport and job completion; they do not by themselves establish visual
page quality. The raw A4 job (4,145,016 bytes) was also accepted by the backend, but its
image interpretation remains unverified. Duplex ordering, non-plain media rendering and
power-cycle firmware behaviour remain open; the Heavy111-130g job's paper handling still
needs a human check.

The attached crash reports provide a second, direct observation of the old filter. They show
HP `rastertozjs` version 6.0 as `X86-64 (Translated)` on macOS 27. One run with the normal
CUPS raster reaches `selectScreenTable` with a null model pointer and dies with
`EXC_BAD_ACCESS`; a second direct invocation without `DEVICE_URI` reaches `initReadThread`
and dies while trying to inspect that missing environment value. With the complete CUPS
environment and the real USB URI supplied, the filter gets past both guards, writes only a
437-byte PJL/ZjStream prologue, then aborts on its own assertion in
`CupsRaster.h:getBandedPixels` because the input byte count does not match its expected
banding. It never produces a complete old-driver job. This explains why the HP executable
is a useful disassembly reference but not a runnable golden master on this OS; the report
does not justify changing the native implementation.

## Critical issues found and fixed

1. Raster geometry was trusted before validation. A malformed 1796-byte header could
   request a multi-gigabyte pixel allocation before any pixel byte arrived. payload_len
   now returns Result, headers are validated before allocation, raw payloads and
   converted bitmap allocations are separately capped at 128 MiB host-resource limits,
   and arithmetic is checked so the wasm32 target cannot wrap usize.
2. The ZjStream decoder reserved memory from an untrusted item count. A 16-byte chunk
   claiming u32::MAX items triggered a Vec reservation of that size. Counts are now
   bounded by the bytes actually available, and offsets use subtraction so pos + size
   cannot wrap on a 32-bit target.
3. A valid first page could leave a partial job on the wire. The PJL prologue was flushed
   after the first page was merely read; if conversion then failed (for example a missing
   screening tone table), the printer had already received a prologue and document header.
   Nothing is now written until the first page is read, validated, and encoded.
4. uninstall.sh could recursively delete an arbitrary directory. --dest accepted any
   absolute path and ran rm -rf on it. Uninstall now deletes only the six files this
   installer owns, refuses destinations without an ownership marker (or a legacy
   recognizable PPD plus filter), resolves symlinked parents, rechecks protected paths
   after canonicalization, and preserves unrelated files, removing the directory only when empty.

## Important improvements

- Streaming output now writes directly from the encoder buffer and reuses that allocation,
  instead of copying each page through a second full-page Vec.
- write_all behavior is pinned by tests: interrupted writes are retried, zero-byte and
  broken-pipe writes are reported as Error::Io rather than treated as success.
- Direct converters reject short pixel buffers before indexing; supported input models are
  explicit (chunked, single-channel W/SW at 8 bits and K at 1 bit), so unsupported color
  models fail loudly instead of being thresholded with the wrong ink polarity.
- Generated bitmap jobs validate geometry, resolution, and exact length before allocating,
  on both JBIG and raw paths; the test-page command and its dpi option inherit the guard.
- The filter accepts exactly the CUPS calling convention (job, user, title, copies,
  options, and an optional filename), and a new process test exercises stdin and file
  input, identical jobs, INFO on stderr, and the write-nothing-on-invalid-input mode.
- The JBIG sweep now also runs every bi-level case through the shipped Rust CLI, not only
  the Python prototype, and covers the final BID chunk trailing padding.
- Installer: --no-queue no longer queries the source queue; queue deletion verifies the
  queue PPD points at our filter; the source HP queue can never be selected; destinations
  are canonicalized and rechecked; build-native.sh resolves tools from PATH and preserves
  extra argument boundaries; a new guard-regression script stubs lpstat and lpadmin.
- Documentation now distinguishes the completed P1102 hardware checks from the remaining
  paper-handling and power-cycle questions.

## Performance

Measured with /usr/bin/time -l as the median of three runs on generated 600-dpi Letter
raster (31.4 MB per page, repeated gradient):

- JBIG, one page: 37.36 MB before, 37.34 MB after.
- JBIG, ten pages: 37.50 MB before, 37.49 MB after.
- Raw, one page: 41.35 MB before, 41.32 MB after.
- Raw, ten pages: 45.38 MB before, 41.37 MB after.

The change matters on the raw and poorly compressible multi-page path, where the second
full-page buffer was retained alongside the reused encoder buffer. The JBIG path is
unchanged within noise. Ten-page elapsed times were 0.45 s before / 0.46 s after for
JBIG and 0.47 s in both raw runs; no throughput improvement is claimed.

## Independently confirmed reverse engineering

- ZjStream framing, big-endian chunk header, item layout, and the JZJZ signature are
  confirmed by independent foo2zjs comparisons; HP binary metadata and symbols were
  inspected, but do not independently prove every field. Our
  whole job is byte-identical to foo2zjs for the same page and options across 31 cases.
- JBIG parameters (order 3, options 0x5c, L0 128, MX and MY 0) and stripe encoding are
  confirmed against jbigkit; 69 comparisons are byte-identical, including 21 cases run
  through the shipped Rust CLI with its 128-pixel alignment.
- CUPS raster layout (1796-byte cups_page_header2_t, native-endian RaS3 sync) is confirmed
  against Apple cgpdftoraster output and the CUPS source; our reader accepts RaS3 only.
- HP's stack is x86_64-only and links libstdc++.6; ours links only libSystem, verified by
  reading load commands and rechecked by verify-native.sh on every run.
- The generated PPD passes CUPS's own cupstestppd and differs from HP's only in driver
  wiring and the documented screening option, checked offline with --no-queue.

## Remaining uncertainty

- USB identity, endpoint behaviour, PJL/no-PJL framing acceptance, compressed JBIG transfer
  and the native CUPS queue are confirmed on hardware. Duplex behaviour, non-plain media
  rendering, raw-image visual correctness and power-cycle firmware behaviour remain
  unverified.
- The legacy translated `usbink` status probe ran against the attached P1102 but returned
  `levels is -1` without marker attributes, so toner reporting remains unavailable.
- ZJT_RAW_IMAGE was accepted as a complete transfer (4,145,016 bytes); its payload layout and
  the resulting page image remain hypotheses.
- Whether duplex needs host-side page pairing (as foo2zjs does) or printer-side DMDUPLEX
  suffices (as HP's architecture suggests) can only be settled by a two-sided print.
- Whether a power cycle needs a host-side firmware download is unresolved; evidence says
  likely not.
- The chunk header reserved field's semantics are inferred from observation, not specified.
- The 128 MiB caps are host resource limits, not printer limits.

## P1102-specific behavior intentionally preserved

PJL density default 3, EconoMode, JAMRECOVERY and RET settings, DMPAPER codes, media and
source codes, the two 12288-byte HP tone tables and their cupsCompression marker selection,
the 0x5c and 0x1c JBIG option split, and the P1102 queue and device identity are unchanged.
No protocol constant was altered without evidence.

## Intentionally not changed

- Production protocol constants, item sets, and the crate architecture: no defect found that
  justified touching them.
- The filter now catches CUPS cancellation signals, stops before its next output write, and
  sends the documented CUPS side-channel soft-reset request to Apple's USB backend. Whether
  the P1102 firmware discards a job whose bytes already reached its buffer still needs a
  physical cancellation check.
- The Mac's default destination was initially the untouched HP queue; it is now set to
  `HP_LaserJet_P1100_native` so ordinary print-dialog jobs use the ARM64 filter.
- No speculative P1104, P1106, P1107, P1108, or P1109 support was added; family claims are
  now scoped to what is actually verified.

## Validation performed

Commands and results:

- scripts/verify-all.sh with the local foo2zjs and jbigkit oracles: every check ok,
  including reference job comparison, JBIG sweep, two-bit comparison, rehearsals, PPD
  checks, memory flatness, and script guard regressions.
- scripts/test-native.sh in debug mode: 115 tests passed. The same suite with --release:
  115 passed.
- scripts/test-wasm.sh: 111 passed; two CLI tests and two filter process tests are native-only.
- cargo clippy --workspace --all-targets --all-features -- -D warnings: clean.
- cargo fmt --check and cargo check --workspace: clean.
- scripts/build-native.sh: ARM64 release binaries; verify-native.sh confirms only
  libSystem is linked.
- Offline installer rehearsal and guard script: ok; no queue created and no system path
  written.
- Plain cargo test still fails on this machine because /usr/bin/cc is gated by the Xcode
  licence; the repository's documented native route works, so this is environmental.

## Files deserving human review

- crates/p1100-raster/src/lib.rs: the new validation policy and 128 MiB caps.
- crates/p1100-job/src/lib.rs: first-page ordering and bitmap guards.
- scripts/install.sh and scripts/uninstall.sh: ownership and queue guards.
- docs/differences.md: remains the authoritative list of hardware-open questions.

## Tests added or strengthened

Regression coverage includes malformed raster geometry and colors, short pixel buffers,
bounded aligned allocation, short/interrupted reads, excessive protocol item counts and
lengths, JBIG geometry overflow and reset-marker semantics, generated bitmap lengths and
tray selection, first-page output atomicity, truncated second pages, interrupted/zero/
broken-pipe writes, CLI dimensions, filter process arguments and copies, and installer
ownership/path guards. Independent comparisons now exercise the Rust CLI directly.

## Deployment and adversarial review

The pipeline is CUPS PDF rasterization → native RaS3 → Rust filter → PJL/ZjStream →
Apple's USB backend. Transport endpoint handling, USB partial writes, retries, disconnect
recovery and status remain Apple's backend responsibility, not functionality implemented
by this filter. The CLI bounds backend waiting using elapsed time; no device-response
parser or host firmware loader is present.

Each job is a separate process with local encoder state; no mutable cross-job state was
introduced. Page memory is bounded by validated page geometry, not document page count.
A failed later page can leave an incomplete physical job; tests ensure it reports failure
and does not append a misleading successful END_DOC. Physical recovery remains unknown.

Another Mac needs a usable Rust/native build toolchain and an HP source PPD; optional
grey screening also needs extracted HP tone tables. The current host's Xcode licence
gate requires the documented direct-clang build route. No CI workflow, signed/notarized
installer package or clean-Mac deployment was validated. The privileged queue installation
was validated on this host; the scratch rehearsal still verifies the reversible file path.
These are remaining release-readiness limits.

## Confidence based on evidence

- Protocol correctness: strong offline evidence for production bi-level JBIG framing
  and encoding from independent whole-job and compression comparisons, backed by accepted
  compressed jobs on the P1102. Raw image visual correctness, reserved semantics and duplex ordering
  remain open.
- macOS/CUPS integration: strong for local ARM64 builds, CUPS rasterization, PPD generation,
  process I/O, privileged installation and queued printing. Clean-Mac deployment remains
  unverified.
- P1102 compatibility: compressed one-bit and two-bit jobs, PJL/no-PJL framing and the native
  queue have been accepted by the attached device/backend; visual quality and paper handling
  still need direct observation.
- P1100-family extensibility: existing small crate boundaries can accommodate additional
  confirmed model parameters. No other family model is claimed supported; capability
  and quirk data should be added only with device evidence.
