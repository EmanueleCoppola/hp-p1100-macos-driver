# Printer protocol

Confidence tags: **CONFIRMED** (observed on this machine or corroborated by two
independent sources), **LIKELY** (strong inference), **UNKNOWN** (open).

## Headline

**CONFIRMED** The HP LaserJet Professional P1102 speaks **Zenographics ZjStream**
("ZJS"), framed in PJL, with **JBIG (ISO/IEC 11544 / ITU-T T.82)** bi-level compression
for the page raster.

Two independent sources agree on the format:

1. HP's own macOS filter `rastertozjs` - its C++ RTTI and symbols are namespaced
   `zjs::`, `zeno::`, `zcups::`, `jbig::`, and include `ZjsMonoFormatter`, `zjs::Chunk`,
   `zjs::ItemHeader`, `zjs::SignatureChunk`, `zjs::RawBandWriter`,
   `zjs::JbigBandWriter`, `ZJBIG.JBIG_Start()`.
2. OpenPrinting **foo2zjs** (`zjs.h`, `foo2zjs.c`, `zjsdecode.c`), which supports
   `HP-LaserJet_Pro_P1102.ppd` explicitly. Its decoder independently validates the two
   magic values that HP's binary also produces.

foo2zjs was read for understanding only. No foo2zjs (GPL) code is copied into this
repository; see `docs/research.md`.

## Job framing

The reference framing for the P1102-class models (HP1020 / HP Pro P1102 / P1566 /
P1606dn) is a PJL wrapper around one ZJS stream:

```text
ESC % - 1 2 3 4 5 X @PJL JOB\n
@PJL SET JAMRECOVERY=OFF\n
@PJL SET DENSITY=<int>\n
@PJL SET ECONOMODE=ON|OFF\n
@PJL SET RET=MEDIUM\n
@PJL INFO STATUS\n
@PJL USTATUS DEVICE = ON\n
@PJL USTATUS JOB = ON\n
@PJL USTATUS PAGE = ON\n
@PJL USTATUS TIMED = 30\n
@PJL SET JOBATTR="JobAttr4=<YYYYMMDDHHMMSS>"\0
ESC % - 1 2 3 4 5 X
"JZJZ"                        <- 4-byte ZJS stream signature
<ZJT_START_DOC chunk + items>
  <ZJT_START_PAGE chunk + items>
  [ <ZJT_START_PLANE chunk + ZJI_PLANE item> ]
  <ZJT_JBIG_BIH chunk>         <- payload is a 20-byte JBIG BIH
  <ZJT_JBIG_BID chunk(s)>      <- JBIG strip data
  <ZJT_END_JBIG>
  [ <ZJT_END_PLANE> ]
  <ZJT_END_PAGE>
<ZJT_END_DOC>
ESC % - 1 2 3 4 5 X @PJL EOJ\n
ESC % - 1 2 3 4 5 X
```

`ESC%-12345X` is the PJL Universal Exit Language sequence.

**CONFIRMED on this machine**, HP's macOS `rastertozjs` contains exactly these PJL
tokens, which is what pins the framing down:

```text
%-12345X        @PJL JOB NAME=%s        @PJL ENTER LANGUAGE=%s
@PJL EOJ        @PJL USTATUSOFF         @PJL INFO STATUS
@PJL USTATUS JOB=ON   @PJL USTATUS PAGE=ON   @PJL USTATUS DEVICE=ON
cupsPJL  pjlDensity  pjlJamRecovery  pjlDuplexOffset
```

and the PPD carries `*cupsPJL pjlDensity: "@PJL SET DENSITY = %s%n"`.

**Differences matter**: HP's macOS driver uses `@PJL JOB NAME="..."`, adds
`@PJL ENTER LANGUAGE="..."` and `@PJL EOJ`, and its literals do not show the
`JOBATTR`/`RET`/`TIMED` lines. **LIKELY** the `ENTER LANGUAGE` argument is `ZJS`. So the
two implementations are semantically equivalent but not textually identical, which is
exactly the class of difference the golden-master comparison has to tolerate.

## Record format

**CONFIRMED** A ZJS stream is a flat sequence of *chunks*. Each chunk starts with a
16-byte, **big-endian** header:

```text
offset size field      meaning
  0     4   size       total chunk byte count, including this 16-byte header
  4     4   type       ZJ_TYPE opcode
  8     4   items      number of item records that follow this header
 12     2   reserved   housekeeping, see below
 14     2   signature  always 0x5a5a
```

`reserved` was observed to equal the total byte length of the following item records,
i.e. `12 * items` for all-`uint32` item sets, and `0` when there are no items. The
P1102-class path uses `reserved = 12 * items`. **LIKELY** it is a consistency field.

`items` acts as a count for most record types; the JBIG records
(`ZJT_JBIG_BIH`, `ZJT_JBIG_BID`, `ZJT_END_JBIG`) always pass `items = 0` and put their
byte count in `size` instead.

Each item is an 8-byte header plus a payload:

```text
offset size field    meaning
  0     4   size     total item byte count, including this 8-byte header
  4     2   item     ZJ_ITEM tag
  6     1   type     ZJ_ITEM_TYPE
  7     1   param    general purpose, 0 in all observed uses
```

| type | name | payload | total item size |
|---|---|---|---|
| 1 | `ZJIT_UINT32` | 4-byte big-endian value | 12 |
| 2 | `ZJIT_INT32` | 4-byte big-endian signed value | 12 |
| 3 | `ZJIT_STRING` | NUL-terminated bytes, padded to a 4-byte multiple | 8 + 4*ceil((len+1)/4) |
| 4 | `ZJIT_BYTELUT` | 4-byte count, then that many byte entries | 12 + n |

## Chunk opcodes (`ZJ_TYPE`)

| value | name | meaning |
|---|---|---|
| 0 | `ZJT_START_DOC` | start of document |
| 1 | `ZJT_END_DOC` | end of document |
| 2 | `ZJT_START_PAGE` | start of page |
| 3 | `ZJT_END_PAGE` | end of page |
| 4 | `ZJT_JBIG_BIH` | payload is a 20-byte JBIG BIH |
| 5 | `ZJT_JBIG_BID` | payload is a JBIG strip / data block |
| 6 | `ZJT_END_JBIG` | end of the compressed image |
| 7 | `ZJT_SIGNATURE` | signature chunk |
| 8 | `ZJT_RAW_IMAGE` | "full uncompressed plane follows" |
| 9 | `ZJT_START_PLANE` | start of a colour plane |
| 10 | `ZJT_END_PLANE` | end of a colour plane |
| 11 | `ZJT_2600N_PAUSE` | HP 2600n specific |
| 12 | `ZJT_2600N` | HP 2600n specific |
| 13,14,15 | `ZJT_ZX_0x0d/0e/0f` | ZX specific |

For our monochrome P1102 only 0-10 are relevant.

## Item tags (`ZJ_ITEM`)

Document scope, inside `ZJT_START_DOC`:

| tag | name | notes |
|---|---|---|
| 0 | `ZJI_PAGECOUNT` | page pairs, 0 if unknown |
| 1 | `ZJI_DMCOLLATE` | 0 |
| 2 | `ZJI_DMDUPLEX` | 1 = off, 2 = long edge, 3 = short edge, 4/5 = manual |
| 15 | `ZJI_QUANTITY` | copy count |

Page scope, inside `ZJT_START_PAGE`. Value ranges come from the reference encoder's
DEVMODE / DMPAPER enumerations:

| tag | name | notes |
|---|---|---|
| 2 | `ZJI_DMDUPLEX` | duplex mode |
| 3 | `ZJI_DMPAPER` | 1 = Letter, 9 = A4, ... |
| 4 | `ZJI_DMCOPIES` | copies |
| 5 | `ZJI_DMDEFAULTSOURCE` | 1 = upper/only, 4 = manual, 7 = auto |
| 6 | `ZJI_DMMEDIATYPE` | 1 = standard, ... |
| 7 | `ZJI_NBIE` | bi-level image entities; 1 for monochrome |
| 8,9 | `ZJI_RESOLUTION_X/Y` | dpi |
| 10,11 | `ZJI_OFFSET_X/Y` | top-left corner; emitted only when non-zero |
| 12,13 | `ZJI_RASTER_X/Y` | raster dimensions in pixels |
| 16,17,18 | `ZJI_VIDEO_BPP/X/Y` | bit depth and video dimensions |
| 19 | `ZJI_INTERLACE` | 0 or 1 |
| 20 | `ZJI_PLANE` | 1 = C, 2 = M, 3 = Y, 4 = K; unused on the `-P` path |
| 21 | `ZJI_PALETTE` | translation table |
| 22 | `ZJI_RET` | HP Resolution Enhancement Technology |
| 23 | `ZJI_ECONOMODE` | 0 = off, 1 = on |

There are also HP-specific tags (`ZJI_HP_CDOTS`..`ZJI_HP_KWHITE`, 0x8200-0x8207) and ZX
tags (0x6c-0x6f). Their mapping onto HP's macOS property names `ZxBits`, `colorFormat`,
`colorSize`, `minAtMove`, `maxAtMove` is **LIKELY** but unverified.

## What the P1102 actually needs

The reference implementation drives the P1102 as model `2` with flags `-P -z2 -L0`:

- `-P` suppresses `ZJT_START_PLANE` / `ZJT_END_PLANE` and the `ZJI_PLANE` item entirely.
  This is the important simplification: **a P1102 page has no plane records at all.**
  HP's macOS binary has the matching `startPlane` / `closePrevious` concepts and a
  `RawBandWriter`, so it clearly handles both shapes.
- Geometry is emitted as `ZJI_VIDEO_X`, `ZJI_VIDEO_Y`, `ZJI_VIDEO_BPP`, `ZJI_RASTER_X`,
  `ZJI_RASTER_Y`, optional offsets, `ZJI_NBIE`, `ZJI_RESOLUTION_X/Y`, `ZJI_RET`,
  `ZJI_DMDEFAULTSOURCE`, `ZJI_DMCOPIES`, `ZJI_DMPAPER`, `ZJI_DMMEDIATYPE`.
- The raster handed to JBIG is **width-aligned up to a multiple of 128 pixels**
  (`w = (w + 127) & ~127`), while `ZJI_RASTER_X` keeps the true unaligned pixel width.
  Bitmap scanlines are padded to a 16-byte (128-bit) multiple.
- `ZJI_VIDEO_X` is `raster_width / bits_per_pixel`; `ZJI_VIDEO_BPP` is 1 for monochrome.

### Page size codes (`ZJI_DMPAPER`)

CUPS puts the PPD's `*PageSize` choice name into the raster header - measured by running
every size this PPD declares through `cupsfilter` - so the mapping from a job to a code is
name-based, and the names are the PPD's own: `B5JIS`, `EnvDL`, `16K197x273mm`, `8.5x13Custom`
and so on. The codes come from the reference driver's table, which is keyed on the same names,
and each entry below was checked against it **by page geometry** rather than by name.

| PPD `PageSize` | code | | PPD `PageSize` | code |
|---|---|---|---|---|
| `Letter` | 1 | | `Env10` | 20 |
| `Legal` | 5 | | `EnvDL` | 27 |
| `Executive` | 7 | | `EnvC5` | 28 |
| `A4` | 9 | | `EnvISOB5` | 34 |
| `A5` | 11 | | `Monarch` | 37 |
| `B5JIS` | 13 | | `Postcard` | 43 |
| `A6` | 70 | | `DoublePostcard` | 82 |
| `16K197x273mm` | 257 | | `16K184x260mm` | 263 |
| `16K195x270mm` | 264 | | `8.5x13Custom` | 258 |
| `Custom` (not a PPD choice; a custom media arrives as this) | 256 | | | |

Geometry is what resolves the near-misses: `EnvISOB5` is 499x709 pt, the 176x250 mm envelope
rather than ISO B5 paper (259), and `8.5x13Custom` is 612x936 pt, the German legal fanfold
size (258) rather than an arbitrary custom size. `DoublePostcard` at 567x420 pt is the only
entry that rests on an orientation convention rather than on a size, so it is **LIKELY** 82.

Eight of these names were missing from the first version of our table, and a job on A5,
B5JIS, EnvDL, EnvISOB5, a 16K size or 8.5x13Custom would have been sent to the printer as
Letter. The reference comparison is what caught it, and it now covers seven of these sizes
end to end; `docs/differences.md` item 6 covers the media *type* codes, which are a separate
matter.

## JBIG

**CONFIRMED** compression is standard **ISO/IEC 11544 / ITU-T T.82 JBIG**:

- `ZJT_JBIG_BIH` carries a **20-byte Bi-level Image Header** (the standard JBIG BIH size;
  the reference encoder hard-asserts `len == 20`). Bytes 4..7 are image width and bytes
  8..11 are height, big-endian.
- `ZJT_JBIG_BID` carries JBIG strip / data blocks from the encoder.
- `ZJT_END_JBIG` terminates the image. On the P1102 path it carries no items, unless
  duplex is active, in which case it carries one `ZJI_DMDUPLEX` item.
- The final `ZJT_JBIG_BID` payload carries the reference implementation's padding: 16 zero
  bytes on top of rounding the chunk up to a 4-byte boundary, so a 120-byte image arrives as
  a 136-byte payload. Earlier BID chunks are not padded at all. **CONFIRMED** by reading the
  code and by measuring its output; the padding is inert (a decoder stops once it has the
  announced number of lines) but we emit it so a captured job stays comparable byte for
  byte.

Our encoder and the parameters it emits are documented in `docs/jbig.md`. It is verified
byte-for-byte against the reference implementation of the standard.

### HP's encoder, read out of its binary

This became readable once Xcode's toolchain could run (`docs/testing.md`): `rastertozjs`
keeps its symbols, so `nm` and `otool -tvV` name what they are. **CONFIRMED**:

- `jbig::AbstractEncoder` has `setTemplate(int)`, `setOrderBits(int)`, `setStripeSize(int)`,
  `setTypicalLinePrediction(bool)`, `setEndOfStripeReset(bool)`, `setAtMove(int,int,int)`,
  `setLineStride(int)`, `setBitsPerPixel(int)` and `getBIH(jbig::BIH*)`. That is the same
  parameter set T.82 defines, so HP is not doing anything exotic.
- The encoder underneath those classes is **jbigkit** itself, statically linked: the binary
  exports Markus Kuhn's internal tables as `_jbg_nlps`, `_jbg_nmps`, `_jbg_dptable` and
  `_jbg_resred`. The same library is what `scripts/jbig-oracle.sh` fetches, which is what
  makes it a fair reference for our own encoder; `docs/research.md` covers the licensing
  side of that fact.
- `jbig::ZjbigEncoder`'s constructor writes the BIH defaults directly:
  `movb $0x5c, 0x93(%rbx)`, `movb $0x3, 0x92(%rbx)`, `movb $0x10, 0x90(%rbx)`. Offsets 0x90
  to 0x93 are the four bytes `getBIH` writes into BIH bytes 16..19 in order, so HP's
  defaults are `MX = 16`, `MY = 0`, `order = 3`, `options = 0x5c`.
- `zjs::JbigBandWriter::start` then reads the properties `atmove`, `minAtMove` and
  `maxAtMove` off its `zeno::TagArg`, which is where the adaptive template from the
  constructor can be overridden. That is also the source of the property names this
  document already listed as LIKELY.

The one thing this does not settle is whether the running driver leaves `MX` at 16 or
overrides it to 0; `docs/differences.md` item 2 records what we emit and why.

## Record format, cross-checked against the reference driver

**CONFIRMED** every chunk type, item tag, item order and `reserved` value this document
describes is reproduced byte for byte by `foo2zjs`, which supports this printer, for the
same page and options - see `scripts/reference-compare.py`. That comparison is what turned
these tables from inference into measured agreement; it is also what caught the padding
rule above, which an earlier version of this document described as 4-byte alignment only.

## Uncompressed mode

**Partly confirmed, and worth testing on hardware.** `ZJT_RAW_IMAGE` (opcode 8) is documented
as "full uncompressed plane follows", and HP's filter has the path: `zjs::RawBandWriter` sits
beside `zjs::JbigBandWriter` and implements the same interface. **CONFIRMED** from its
disassembly:

- `RawBandWriter::start` opens a chunk with opcode **8** and no items, and does not write a
  BIH - so a raw image is a single chunk rather than a BIH plus data chunks.
- `RawBandWriter::writeScans(const void*, n, m)` passes `n * m` bytes straight from the
  caller's buffer to the stream. There is no per-scanline header, no bit reordering and no
  compression: the payload is the packed scanlines as the caller holds them.
- `RawBandWriter::end` closes the chunk and nothing else.

That is exactly the shape we emit: one opcode-8 chunk holding the same 1 bpp bitmap the JBIG
path would compress, at the same 128-pixel-aligned stride. What is still **UNKNOWN** is
whether the device accepts it - the reference driver does not emit raw images, so nothing
runnable confirms it end to end - and whether HP adds an item we do not, since the call
that opens the chunk passes only the opcode. If the P1102 accepts raw, it removes the JBIG
dependency for a first version; that is a hardware question, and `make-test-job.py --raw` and
`P1102_RAW=1` both exist to answer it.

## Byte order caveat

HP's macOS binary exposes an `endianness` property on its stream writer and a
`zjs::SignatureChunk`. The reference implementation is strictly big-endian and writes
the 4-byte `JZJZ` stream signature before `ZJT_START_DOC`. **LIKELY** our models accept
big-endian only, and `endianness` exists for other engine families.

## Status / error vocabulary

**CONFIRMED** state strings in HP's filter: `Readytoprint`, `Nopagesfound`,
`Printingpage`, `Complete`; plus the PPD's `cupsIPPReason` set:

```text
CoverOpen  TonerMissing  DoorOpen  InputJam  OutputJam  OutOfPaper
ManualFeed  ManualDuplex  EngineFuser  EngineScanner  EngineComm
```

**CONFIRMED** the command filter `usbink` answers `ReportLevels`: it locates the device
through IOKit by `USB Product Name` + `USB Serial Number`, opens an interface matched by
class/subclass/protocol, and does synchronous bulk reads and writes (`mBulkOutPipe`,
`mBulkInPipe`). **LIKELY** USB Printer class (0x07/0x01/0x02); needs the device attached.

## The command path: HTTP over USB, not PJL

**CONFIRMED by disassembly and by the literals in the binary.** `usbink` does not speak PJL
to the printer, and it does not use the CUPS backchannel. It carries a small hand-rolled
HTTP client - the strings `HTTP/`, `: `, `Transfer-Encoding: `, `chunked` and
`Content-Length: ` sit next to the one request it sends - and it sends exactly this over the
USB interface it opened:

```text
GET /DevMgmt/ConsumableConfigDyn.xml HTTP/1.1\r\n\r\n
```

There is no networking code in the binary at all: the undefined symbols are IOKit
(`IOServiceGetMatchingServices`, `IOCreatePlugInInterfaceForService`, `IORegistryEntryCreateCFProperty`),
Foundation, libcups and libc, with no `socket`, `connect`, `CFStream` or URL loading. The
"web server" is therefore reached through the device's USB interface, which is what the
`CMD:...HTTP...` entry in the printer's own IEEE 1284 identity is advertising.

It reads `DEVICE_URI` from the environment, parses it with `httpSeparateURI`, and reports a
failure rather than guessing when it is absent. For devices it cannot reach that way it
falls back to Apple's helper at `/System/Library/Printers/Libraries/snmpInk` through
`popen`, which is the network-printer path (the same bundle serves the wireless P1102w).

The answer comes back as the supplies level and is handed to CUPS in the command-filter
output format:

```text
ATTR: marker-colors=#000000 marker-levels=%d marker-names=Black marker-types=ink
<dd:ConsumablePercentageLevelRemaining>%d</dd:ConsumablePercentageLevelRemaining>
```

with `<key>prtMarkerSuppliesLevel</key>` / `<integer>%d</integer>` assembled for the device
description. That is the whole interface, and it is why `docs/differences.md` records the
command path as a deliberate gap rather than an oversight: reproducing it means writing an
IOKit client that tunnels HTTP over the bulk pipes, which is a second transport in a stack
whose point is to keep the one Apple already ships.

## Model identity

**CONFIRMED** `rastertozjs` contains the strings `HP LaserJet Professional P1560` and
`HP LaserJet Professional P1600dn`; the PPD declares
`MODEL:HP LaserJet Professional P1102` (also P1104/P1106/P1107/P1108/P1109) with
`*cupsModelNumber: 256`. One bundle serves the whole P1100/P1560/P1600 family, so the
model is selected by PPD / `1284DeviceID`, not by a different binary.

**CONFIRMED** the same family is driven by the open reference implementation as model 2
with `-P -z2 -L0`. That string is not folklore: it is the `prototype` line of the
`foo2zjs-z2` driver in the OpenPrinting database, whose printer record for
`HP-LaserJet_Pro_P1102` also carries the device's own IEEE 1284 identity, captured from a
real unit:

```text
MFG:Hewlett-Packard;MDL:HP LaserJet Professional P1102;
CMD:ZJS,PJL,ACL,HTTP;CLS:PRINTER;DES:HP LaserJet Professional P1102;FWVER:20090924;
```

`CMD:ZJS,PJL` is independent confirmation of the headline finding from the device side,
and `ACL` is the firmware-upload command set, which is worth remembering when reading the
firmware question below.

## Firmware

**LIKELY not applicable to the P1102.** Its bundle contains no firmware blob. The
firmware images on this system (`HP1005.acl`, `HP1006.acl`, `HP1505.acl`, `hp1020.acl`;
each an ELF with an ASCII date prefix - `HP1005.acl` is `20080505` then `\x7fELF`,
32-bit big-endian `ET_EXEC`) belong to the *other* bundle `hplaserjetzjs.bundle`, i.e.
the P1005/P1006/P1505/1020 family. **UNKNOWN** without hardware: whether the P1102 ever
needs a host-side firmware download after a power cycle. First thing to observe once the
printer is attached.

"No firmware blob" is an exhaustive statement rather than a spot check: searching
`/Library/Printers` for `*.acl` or for anything shaped like firmware (`.rfu`, `.bin`,
`.hex`, `.img`, `.fw`, `.dl`) returns exactly those four files, all in that one sibling
bundle, and the P1102's own tree holds only its two Mach-O filters, an `Info.plist`, one
strings file and the utility app. The other side of the question - whether the *device*
wants firmware pushed to it after a power cycle - is what only the hardware can answer, and
the reference driver does not push any either.

One caveat against that "no": the device advertises `CMD:...ACL...` in its own IEEE 1284
identity (above), so it can accept the firmware-upload command set even though this
machine's driver never uploads anything to it. The reference implementation does not upload
firmware to this model either, which is why nothing in this stack does.

## CUPS raster, the input side

Our filter is handed `application/vnd.cups-raster`. Measurements from
`/usr/sbin/cupsfilter -m application/vnd.cups-raster -p <p1102 ppd>` on one-page text
(CONFIRMED):

| quantity | value |
|---|---|
| magic | `RaS3` (u32 0x52615333, native endian) |
| Letter job, total | 31,361,800 bytes |
| pixel payload | 4,900 x 6,400 = 31,360,000 bytes |
| depth | 8 bits per pixel, grayscale, uncompressed |
| A4 job, total | 32,437,086 bytes, payload 4,758 x 6,817 |
| 2-page job | 62,723,596 bytes |
| 3-page job | 94,085,392 bytes |

**RESOLVED, and reconciled against CUPS itself.** The stream is a 4-byte sync word
written **once per stream**, then a sequence of pages, each being a **1,796-byte
`cups_page_header2_t`** followed by `bytes_per_line x height` bytes of pixel data.

The sync word lives in `cupsRasterOpenIO`, which writes it when the stream is opened and
reads it back once (`cups/raster-stream.c`), which is why it does not repeat per page -
this is not an Apple quirk. `CUPS_RASTER_SYNC` is `0x52615333` (`"RaS3"`) and is what
`cgpdftoraster` produces here; CUPS also defines `CUPS_RASTER_SYNCv1` `0x52615374`
(`"RaSt"`), `CUPS_RASTER_SYNCv2` `0x52615332` (`"RaS2"`) and, for Apple,
`CUPS_RASTER_SYNCapple` `0x554E4952` (`"UNIR"`).

Critically, CUPS's current `cups_page_header2_t` has **no `Magic`/`Version`/`HeaderSize`
members at all** - the sync word carries that role out of band. That is why the first
three words of the header read as zero, which is what made this look wrong at first.

Verified by walking all three pages of a 3-page job:

```text
page 1 header @ 4            w=4900 h=6400 bpc=8 bpp=8 bpl=4900 colors=1 "Letter" "Paper"
page 2 header @ 31,361,800   identical header
page 3 header @ 62,723,596   identical header
walk lands on EOF at 94,085,392 = exact file size
```

So the page stride is `1796 + bytes_per_line * height`, and only the first page is
preceded by the magic. A parser reads the magic once, then iterates 1,796-byte headers,
and validates that the geometry accounts for the rest of the file exactly.

Header field offsets are `offsetof()` into that struct, relative to the start of the
1,796-byte header. Every one below was independently confirmed by differencing a Letter
job against an A4 job, and then matched against CUPS's own header definition:

| offset | meaning | Letter | A4 |
|---|---|---|---|
| 128 | `MediaType`, 64-byte string | `"Paper"` | `"Paper"` |
| 372 | width in pixels | 4900 | 4758 |
| 376 | height in pixels | 6400 | 6817 |
| 276 | `HWResolution[2]` | 600, 600 | 600, 600 |
| 312 | `Margins[2]` in points | 12, 12 | 12, 12 |
| 340 | `NumCopies` | 1 | 1 |
| 372 | `cupsWidth` in pixels | 4900 | 4758 |
| 376 | `cupsHeight` in pixels | 6400 | 6817 |
| 380 | `cupsMediaType` | 1 | 1 |
| 384 | `cupsBitsPerColor` | 8 | 8 |
| 388 | `cupsBitsPerPixel` | 8 | 8 |
| 392 | `cupsBytesPerLine` | 4900 | 4758 |
| 396 | `cupsColorOrder` | 0 (chunked) | 0 |
| 400 | `cupsColorSpace` | 0 (`W`, grayscale) | 0 |
| 420 | `cupsNumColors` | 1 | 1 |
| 424 | `cupsBorderlessScalingFactor` (f32) | 1.0 | 1.0 |
| 428, 432 | `cupsPageSize[2]` in points (f32) | 612.0, 792.0 | 595.0, 842.0 |
| 436..448 | `cupsImagingBBox[4]` in points (f32) | 12, 12, 600, 780 | 12, 12, 583, 830 |
| 1732 | `cupsPageSizeName`, 64-byte string | `"Letter"` | `"A4"` |

**One field is deliberately ignored.** `cupsCompression` (offset 404) reads **401** in
every real page here, which is not a CUPS compression code. It turns out that is
harmless: `cupsCompression` is only ever populated from the PostScript page device
dictionary (`cups/raster-interpret.c`) and libcups never consults it when reading
pixels - page data is always `cupsBytesPerLine x cupsHeight` raw bytes. Gating on it
would reject valid jobs, so the reader does not. The geometry/byte-count check is the
real validation instead: a genuinely compressed stream cannot match it, and the reader
fails loudly in that case.

**Where 401 comes from, confirmed.** The P1100 PPD itself contains the values: its two
`Resolutions` options carry `cupsCompression 401` (the default, `600x400dpi`) and
`cupsCompression 1` (for `600x600dpi`) inside their `setpagedevice` snippets. Selecting
each option and measuring the result shows the header field following the PPD exactly,
while the payload stays raw in both cases:

```text
default (600x400dpi)    4900x6400  payload 31,360,000 = w*h  cupsCompression 401
Resolutions=600x600dpi  4900x6400  payload 31,360,000 = w*h  cupsCompression 1
```

So it is HP's own marker travelling through the page device dictionary, not a compression
instruction. Note that `600x600dpi` would look like RLE (`1` *is* `CUPS_COMPRESSION_RLE`)
while the data is plainly raw - which is precisely why gating on this field would break a
valid job.

The same mechanism carries the media type: the PPD's `MediaType` options set
`cupsMediaType` (`Paper` = 1, `TransparencyPaper` = 2, and heavier stocks get HP-specific
codes such as 258 and 262), and that value reaches us at offset 380. We forward it as
`ZJI_DMMEDIATYPE` rather than re-deriving it from the option name, because it is the code
the PPD declares for the medium that was actually selected. For the default `Paper` it is
1, which is also the value the reference implementation uses. **UNKNOWN:** whether the
printer agrees with HP's codes for the non-default media; that needs hardware.

Input format for our filter is therefore **8 bits per pixel, 1 colour, colour space 0
(grayscale)**: thresholding gray to 1 bpp, no colour handling and no dithering needed.

## USB

**CONFIRMED** the PPD's `1284DeviceID` is
`MANUFACTURER:Hewlett-Packard;MODEL:HP LaserJet Professional P1102`; CUPS addresses the
device as `usb://Hewlett-Packard/HP LaserJet Professional P1102?serial=000000000Q80E182PR1a`.
**UNKNOWN, needs hardware:** vendor/product id, endpoint addresses and packet sizes, and
whether the interface is Printer class (0x07/0x01/0x02) or vendor-specific.
