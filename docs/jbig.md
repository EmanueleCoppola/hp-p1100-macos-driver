# JBIG (ISO/IEC 11544 / ITU-T T.82)

This is the compression the printer expects for a page image. It was measured on the P1102,
the model this driver was developed against, and it is what the driver emits. Everything
below is **CONFIRMED** against a working reference implementation, not recalled from
memory.

## Why not JBIG2

The printer wants **T.82**, the original JBIG standard. JBIG2 is a different, later format
and is not what ZjStream carries. crates.io has JBIG2 implementations but no T.82 encoder,
which is why one has to be written.

## Where it sits in the job

    ZJT_START_PAGE                  page geometry items
    ZJT_JBIG_BIH                    20-byte BIH, exactly
    ZJT_JBIG_BID  (1..n)            the BIE body, split into <=64 KiB chunks
    ZJT_END_JBIG
    ZJT_END_PAGE

The BIH and the body together are one "bi-level image entity" (BIE). The ZjStream layer
does not interpret it; it just splits the body into chunks of at most 65536 bytes, the
limit the reference encoder uses ("Printer does strange things otherwise").

## BIH layout, all 20 bytes

Confirmed from the reference implementation's own header assembly and reproduced
byte-for-byte by experiment:

| offset | field | meaning |
|---|---|---|
| 0 | `DL` | first (lowest) resolution layer stored in this BIE |
| 1 | `D` | highest resolution layer stored, i.e. layer count - 1 |
| 2 | `planes` | number of bit planes |
| 3 | - | always 0 |
| 4..7 | `XD` | image width in pixels, big-endian u32 |
| 8..11 | `YD` | image height in pixels, big-endian u32 |
| 12..15 | `L0` | lines per stripe, big-endian u32 |
| 16 | `MX` | maximum adaptive-template horizontal offset |
| 17 | `MY` | maximum adaptive-template vertical offset |
| 18 | `order` | SMID=0x01, ILEAVE=0x02, SEQ=0x04, HITOLO=0x08 |
| 19 | `options` | DPON=0x04, TPBON=0x08, TPDON=0x10, LRLTWO=0x40, masked with 0x7f |

Note `XD`/`YD`/`L0` are **four** bytes each, not one: `L0` occupies bytes 12..15, which is
easy to get wrong and produces a stream no decoder accepts.

`JBG_DELAY_AT` (0x100) and `JBG_SDRST` (0x200) are library-level switches, not BIH fields,
which is why the options byte is masked with 0x7f before being written.

`planes` is 1 in the BIH the default path sends, and 1 is what the reference driver sends. HP's
own driver writes **2** for this printer, because the `cupsCompression` markers in its PPD put it
on the two-bit path; item 11 of `docs/differences.md` has that reading and the 256-entry tone
tables behind it.

The encoder here writes either. `JbigEncoder::planes(n)` - and the prototype's `--planes n` -
emits `n` planes, each one bit of the pixel value with the most significant plane first,
stripe-major and plane-minor, with a probability state table per plane carried across that
plane's stripes. That is what the reference encoder writes for a PGM when it is told to use
binary words rather than Gray-coded ones, and `scripts/jbig-sweep.py` checks six two-plane
pages against it byte for byte, including a stripe boundary.

## Body layout

After the 20-byte BIH, for each stripe:

    <arithmetic-coded stripe data, 0xFF-escaped>
    0xFF 0x02        SDNORM, the normal stripe terminator
                     (0xFF 0x03 is SDRST, used with the reference's -r option)
    0xFF 0x00        escapes a literal 0xFF byte inside the coded data

There is no explicit per-stripe header: a stripe's extent is implied by the terminator,
which is why 0xFF inside the coded data has to be escaped.

Verified by encoding a 64x200 bitmap with `-s 64` (four stripes, 64/64/64/8 rows): the file
is `0000 0100 | 0000 0040 | 0000 00c8 | 0000 0040 | 08 00 03 1c` followed by four coded
groups each ending `ff 02`.

## Parameters for this printer family

Two independent sources agree on this configuration, and it is what we emit:

- The open implementation that supports the P1102 passes these to the reference library
  for this model (`JbgOptions` in `foo2zjs.c`).
- HP's own filter has the same byte compiled into it. `jbig::ZjbigEncoder`'s constructor in
  `rastertozjs` writes `0x5c` to the field `getBIH` copies into BIH byte 19, and `0x03` to
  byte 18, which is readable in the disassembly as `movb $0x5c, 0x93(%rbx)`.

    order   = ILEAVE | SMID              = 3
    options = DPON | TPBON | TPDON | LRLTWO   -> BIH byte (options & 0x7f) = 0x5c
    L0      = 128
    MX      = 0        (no adaptive template movement on this path)
    MY      = 0

Options 0x5c is where HP's byte and the reference disagree with the prose: the reference
also sets `JBG_DELAY_AT`, but that is a library switch that never reaches the header, which
is why the options byte is masked with 0x7f.

`MX = 0` is the significant simplification: the adaptive template pixel never moves, so
none of the ATMOVE machinery is needed. That is HP's configuration too, not only the
reference driver's: `zjs::JbigBandWriter::start` configures HP's encoder with `mx` taken
from its `atmove` field, which is 0 for every job its own driver produces, and
`setTemplate` only switches the options byte's `LRLTWO` bit. `docs/differences.md` item 2
has the reading, and the one detail left open.

## What each option bit does here

Only two of the four bits change the coded data of a single-layer, single-plane image, and
both are implemented:

- **TPBON** (typical prediction): one bit per line, coded with a context the standard
  reserves for it, saying whether the line's typicality *changed*. A line that is identical
  to the one above is then not coded at all. This is the mechanism that makes long runs of
  identical lines cost one bit each, and the one worth getting right.
- **LRLTWO**: the layer we code is the lowest resolution layer - with no differential
  layers it is also the only one - so this bit does two things here: it selects the
  *two-line* template (the current and the previous line) rather than the standard
  three-line one, and it selects which of the two reserved typical-prediction contexts the
  TPBON bit is coded with. Getting only the context half of that wrong produces a stream
  that is still valid but no longer byte-identical to the reference, which is how this was
  caught.

**TPDON** and **DPON** do not change this path at all: TPDON belongs to the
low-resolution image of differential layers, which we do not emit, and DPON is a
decoder-side switch. They are written into the BIH because both sources write them.

`options = 0` remains supported (`JbigEncoder::with_options`) and is still checked against
the reference, because it takes the other template and skips typical prediction entirely -
which makes it a useful second path to keep honest.

## Verification available now

A native ARM64 reference implementation can be installed **for testing only**, with no
compiler, by unpacking the Homebrew bottle for `jbigkit` 2.1. It provides both directions:

    pbmtojbg    reference encoder   (bitmap -> BIE)
    jbgtopbm    reference decoder   (BIE -> bitmap)

Use `scripts/jbig-oracle.sh` to fetch it. Two checks matter, and the byte-exact one is the
strong one:

1. **Byte-exact**: our BIE for a given bitmap must equal `pbmtojbg -p 92 -m 0 -s 128 -o 3`
   output for the same bitmap (`-p 92` is 0x5c). This is what actually proves the
   arithmetic coder, the template and the typical-prediction state machine are right.
2. **Round trip**: `jbgtopbm` on our output must reproduce the original pixels.

### Result

`scripts/jbig-sweep.py` runs 21 deterministic bitmaps through both implementations in both
configurations, plus six two-plane pages against the reference encoder's multi-plane
configuration, and requires byte identity: 48 comparisons, and it reports every one identical.
The set covers blank and solid images, uniform noise (the hardest case for an arithmetic
coder), a single row, a single column, odd widths (7, 9, 63, 65, 127, 255, 1000), exact
stripe boundaries, and widths either side of the 128-pixel alignment.

The sweep drives `scripts/jbig-encode.py`, a from-the-standard implementation of the same
algorithm in Python, so it does not depend on being able to build the Rust code. The Rust
encoder is pinned to the reference separately, by the golden vectors below and by
`scripts/reference-compare.py`, which requires the *whole job* our filter writes - including
this compressed image - to be byte-identical to the reference driver's for the same page.

One trap worth recording: `pbmtojbg` silently enables differential layers for images larger
than its default lowest-layer size of 640x480, which changes the BIH and produces a stream
our single-layer encoder must not be compared against. Always pass **-d 0** when comparing;
`jbig-oracle.sh encode` does.

## The implementation

`crates/p1100-jbig` is a port of that verified prototype into pure Rust, with no
dependencies. Four vectors from the reference encoder are embedded as golden tests and run
on every build: 8x2, 16x4, an 800x600 image spanning five stripes, and a degenerate 1x200
single column. The two small ones are checked in **both** configurations, so the template
switch and the typical-prediction path each have a golden. `scripts/jbig-encode.py` remains
as the prototype and as an independent cross-check of the Rust code.

`p1100-job` feeds it the same bitmap the uncompressed path would use, and the JBIG line
length is asserted to equal the raster row stride - a mismatch there would silently shear
every page rather than fail.

### Licensing

`jbigkit` is **GPL**. It is used here strictly as a test oracle - the same role foo2zjs
plays for the ZjStream container - and is never committed, never linked, and never shipped.
Our encoder is written from the standard. It is also, as it happens, the same library HP
links into its own filter: `rastertozjs` exports jbigkit's internal tables (`_jbg_nlps`,
`_jbg_nmps`, `_jbg_dptable`, `_jbg_resred`), which is the strongest reason to trust it as
the reference. See `docs/research.md`.
