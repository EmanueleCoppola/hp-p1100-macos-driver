# JBIG (ISO/IEC 11544 / ITU-T T.82)

This is the compression the P1102 expects for a page image. Everything below is
**CONFIRMED** against a working reference implementation, not recalled from memory.

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

The open implementation that is verified against the P1102 passes these to the reference
library for this model:

    order   = ILEAVE | SMID              = 3
    options = DELAY_AT | LRLTWO | TPDON | TPBON | DPON   -> BIH byte (options & 0x7f) = 0x5c
    L0      = 128
    MX      = 0        (no adaptive template movement on this path)
    MY      = 0

`MX = 0` is the significant simplification: the adaptive template pixel never moves, so
none of the ATMOVE machinery is needed.

## What we emit

The simplest configuration that is still conforming and decodable:

    DL = 0, D = 0, planes = 1
    order   = 3
    options = 0        (no typical prediction, no differential layers, no LRLTWO)
    L0      = 128
    MX = MY = 0

A conforming decoder must handle every option combination, so `options = 0` is legal, and
it drops the two most intricate parts of the algorithm (typical prediction and the LRL
two-line template). If the printer turns out to dislike it, typical prediction can be
added later; the framing and the coder do not change.

## Verification available now

A native ARM64 reference implementation can be installed **for testing only**, with no
compiler, by unpacking the Homebrew bottle for `jbigkit` 2.1. It provides both directions:

    pbmtojbg    reference encoder   (bitmap -> BIE)
    jbgtopbm    reference decoder   (BIE -> bitmap)

Use `scripts/jbig-oracle.sh` to fetch it, and `scripts/jbig-oracle.sh compare` to check our
output. Two checks matter, and the byte-exact one is the strong one:

1. **Byte-exact**: our BIE for a given bitmap must equal `pbmtojbg -p 0 -m 0 -s 128` output
   for the same bitmap. This is what actually proves the arithmetic coder and the context
   model are right.
2. **Round trip**: `jbgtopbm` on our output must reproduce the original pixels.

### Result

`scripts/jbig-encode.py` - a from-the-standard implementation of the same algorithm the
Rust encoder will use - is **byte-identical to the reference encoder** across a sweep of 22
cases: blank, all-black, uniform random noise, sparse patterns, banded patterns, odd widths
(13, 1000), a single row, a single column, an exact stripe boundary (129 lines), and widths
from 1 to 1200, at both L0 = 128 and L0 = 64. Random noise is the hardest case for an
arithmetic coder, so the QM coder and the context model are both exercised properly.

One trap worth recording: `pbmtojbg` silently enables differential layers for images larger
than its default lowest-layer size of 640x480, which changes the BIH and produces a stream
our single-layer encoder must not be compared against. Always pass **-d 0** when comparing;
`jbig-oracle.sh encode` does.

## The implementation

`crates/p1005-jbig` is a port of that verified prototype into pure Rust, with no
dependencies. Its output is byte-identical to the reference for the same configuration.
Four vectors from the reference encoder are embedded as golden tests and run on every
build: 8x2, 16x4, an 800x600 image spanning five stripes, and a degenerate 1x200 single
column. `scripts/jbig-encode.py` remains as the prototype and as an independent
cross-check of the Rust code.

`p1005-job` feeds it the same bitmap the uncompressed path would use, and the JBIG line
length is asserted to equal the raster row stride - a mismatch there would silently shear
every page rather than fail.

### Licensing

`jbigkit` is **GPL**. It is used here strictly as a test oracle - the same role foo2zjs
plays for the ZjStream container - and is never committed, never linked, and never shipped.
Our encoder is written from the standard. See `docs/research.md`.
