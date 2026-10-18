# Differences from HP's driver

The goal was to reproduce the working driver's behaviour as closely as is reasonable, and to
explain every difference where it is not exact. This is that list. Confidence tags as in
docs/protocol.md: **CONFIRMED**, **LIKELY**, **UNKNOWN**.

## How this list is measured now

Two things became possible late in the work and are what most of the entries below now rest
on. First, Xcode's own toolchain binaries run on this machine even though the `/usr/bin`
shims behind them refuse (the licence prompt is in the shim, not in the tools), so the
reference driver can be **built and run** here, and HP's filter can be **disassembled**.
Second, that made a real comparison possible:

```bash
scripts/foo2zjs-oracle.sh fetch     # builds the reference driver, test-only, never committed
scripts/reference-compare.py        # our filter's job vs the reference driver's job
```

That comparison runs both implementations over the same page bitmaps with the same print
options and requires the two jobs to be byte-identical once the job-attribute line (which
carries the clock) is removed. As of this writing it passes for every case it covers:

    letter, defaults   PASS   1 pages, 31361800 raster bytes, ours 1043 bytes, reference 1043 bytes
    a4                 PASS   1 pages, 32437086 raster bytes, ours 1051 bytes, reference 1051 bytes
    econo mode         PASS   1 pages, 31361800 raster bytes, ours 1054 bytes, reference 1054 bytes
    density 5          PASS   1 pages, 31361800 raster bytes, ours 1043 bytes, reference 1043 bytes
    manual feed        PASS   1 pages, 31361800 raster bytes, ours 1043 bytes, reference 1043 bytes
    three copies       PASS   1 pages, 31361800 raster bytes, ours 1043 bytes, reference 1043 bytes
    two pages          PASS   2 pages, 62723596 raster bytes, ours 1795 bytes, reference 1795 bytes

That is the comparison Phase 8 of the plan asked for, in the only form this machine allows:
not against HP's binary, which cannot run, but against the other implementation that is
known to drive this printer family. What it does not cover is listed at the end.

One scope limit belongs here rather than only in item 11: the page the reference receives is
**1-bit**. `scripts/reference-compare.py` derives it by thresholding the CUPS raster at the
same cut our filter uses, so what the two jobs share is the page bitmap as *this harness*
defines it - which means the comparison covers the job encoding (the item sets, the chunk
framing, the geometry, the JBIG image) and not the 8 bits per pixel to 1 bit conversion. A
fixture with grey in it would silently compare our thresholding against itself, so the script
now refuses to run one. The grey path is item 11, and what settles it is an observation rather
than this comparison.

## 1. The PJL wrapper follows HP's language switch. CONFIRMED on the P1102

HP's filter emits `@PJL JOB NAME=...`, `@PJL ENTER LANGUAGE=ZJS` and `@PJL EOJ`. The native
filter keeps the reference implementation's `@PJL JOB` and `JOBATTR` setup, but now emits the
same explicit `ENTER LANGUAGE=ZJS` switch required by the attached P1102.

Why: HP's binary can only be launched under Rosetta on this machine, and feeding it the real
CUPS raster ends in `SIGSEGV` before it emits bytes. The attached P1102 then provided the
missing discriminator: the old reference framing was accepted by USB but produced a blank
page and attention state, while adding `ENTER LANGUAGE=ZJS` printed `TEST 123`.
A raw-queue test distinguishes that from an image problem, because the same test can send a
job whose compressed image is known-good.

What is now measured rather than assumed: our wrapper is **byte-identical** to the
reference's for every option combination the comparison covers, and the difference from HP
is now readable in its binary. Every PJL literal `rastertozjs` contains is:

```text
@PJL JOB NAME=%s%n        @PJL ENTER LANGUAGE=%s%n      @PJL EOJ%n
@PJL USTATUSOFF%n         @PJL INFO STATUS%n
@PJL USTATUS JOB=ON%n     @PJL USTATUS PAGE=ON%n        @PJL USTATUS DEVICE=ON%n
```

The order they are sent in is readable too, because the emission is straight-line code that
calls `zcups::Formatter::sendPJL` once per line. HP's `startJob` sends this:

```text
@PJL USTATUS JOB=ON     @PJL USTATUS PAGE=ON     @PJL USTATUS DEVICE=ON
@PJL INFO STATUS        @PJL JOB NAME=<the job title>
<the PPD's own *cupsPJL lines, with the marked choice substituted>
@PJL ENTER LANGUAGE=<ZJS>
```

and its `endJob` sends `@PJL EOJ` and then `@PJL USTATUSOFF`. The whole prologue is gated on
`getModelNumber() & 0x100`, and this printer's PPD declares `*cupsModelNumber: 256` - so HP
does send one here. The language argument is chosen by `model & 1`: with 256 that bit is clear
and the argument is the literal **`ZJS`**, which makes it CONFIRMED rather than likely. The
only other candidate in the code is the literal `HBS`, which no model this PPD covers
selects: it declares one `*cupsModelNumber` for the whole family.

Three further things follow from that and from the PPD. The `DENSITY` and `JAMRECOVERY` lines
are not in the binary because the PPD supplies them: the code looks them up with
`ppdFindAttr("cupsPJL", key)` and `ppdFindMarkedChoice(key)`, so the text comes from the PPD's
template - `@PJL SET DENSITY = %s%n`, spaces and all - rather than from the driver. That is
also why the driver's strings carry the *option names* `cupsPJL`, `pjlDensity`,
`pjlJamRecovery`, `pjlDuplexOffset` and `cupsRET`. And there is no `JOBATTR`, `ECONOMODE`,
`RET=MEDIUM` or `USTATUS TIMED` literal anywhere in it, so the three lines the reference adds
and HP does not have are exactly those.

The native wrapper still differs from HP in job naming, `EOJ`, and the extra status/settings
lines, but it now uses the language switch that the hardware visibly requires. The same test
page that previously produced a blank sheet printed `TEST 123` after this change.

The rest of the wrapper we emit matches HP's in substance: density, econo mode, jam
recovery, resolution enhancement, and the unsolicited status requests are all the same
settings, driven by the same PPD options.

## 2. The JBIG options match HP's, adaptive template included. CONFIRMED

Our BIH carries `options = 0x5c` (DPON | TPBON | TPDON | LRLTWO), `order = 3`, `L0 = 128`
and `MX = 0`, and it is byte-identical to the reference driver's for every case in
`scripts/reference-compare.py`. The same configuration is readable in HP's binary, and so is
the part of it that used to be guesswork: the adaptive template.

`jbig::ZjbigEncoder::start` configures its encoder from four getters on the wrapper, through a
call that is HP's version of jbigkit's own `jbg_enc_options(s, order, options, l0, mx, my)` -
the parameter order and the clamp rules are the same (the state's `mx` is stored only when it
is at most 127, `my` only when at most 255). The values are the wrapper's fields:

| argument | getter, and the field it returns | value here |
|---|---|---|
| `order` | `getOrderBits`, the byte at 0x92 | 3 |
| `options` | `getOptionBits`, 0x93 | 0x5c |
| `l0` | `getStripeSize`, 0x8c (the band writer's stripes, 128) | 128 |
| `mx` | `getAtMove`, 0x94 | **0** |
| `my` | passed as a constant | 0 |

`mx` is the ATMOVE window, and it is 0 because `atmove` is: the band writer's constructor
initialises it to 0 (`movl $0x0, 0xc4(%rdi)`), `JbigBandWriter::start` reads it from the
*job's* `atmove`, `minAtMove` and `maxAtMove` keys with defaults of 0, `jbig::AbstractEncoder::
setAtMove` maps a move of 0 to a window of 0 and otherwise clamps into `min..max` (defaults 5
and 128), and nothing in HP's driver adds those keys: the only references to those names
anywhere in the binary are the reads in `start` and one other applying helper, and the PPD
declares no such option. `setTemplate` - the only other thing that touches the adaptive
template - merely toggles the options byte's `LRLTWO` bit (`0x40`), which is what the
`template` 2-or-3 property means; it never moves `mx`.

So the question this entry used to leave open is answered: **HP's driver does not move the
adaptive template for a job it produced, and neither do we.** The 0x10 that the constructor
stores (`movb $0x10, 0x90(%rbx)`) is a byte of the wrapper's *own* BIH buffer, a different
field from the encoder's `mx`, and the encoder - which is what writes the stream - is
configured with 0. If the stream's MX byte were taken from that buffer instead of from the
encoder, HP would announce `MX = 16` where we announce `MX = 0`; both decode, because MX is
only the *maximum* ATMOVE window and a stream that never signals a move is handled by any
decoder that implements the standard. Ours claims the least, and it is what the reference
sends, so that is what we keep.

The typical-prediction work is verified byte for byte: the encoder reproduces
`pbmtojbg -p 92 -m 0 -s 128 -o 3` exactly, across the sweep in `scripts/jbig-sweep.py` and
on a full 6400-line page through the real filter path.

## 3. The compressed image bytes are ours, not HP's, but match the reference's. CONFIRMED

Both implementations emit T.82 JBIG, and with the same parameters the bytes are identical:
for every page in `scripts/reference-compare.py` our compressed image equals the reference
driver's, which is why the whole jobs match. HP's own bytes would differ if its filter
moves the adaptive template (item 2), and that is not something that can be checked without
running its binary.

Independently of the reference: the image decodes back to exactly the bitmap the raster
describes, using the standard's own decoder, and the encoder reproduces the reference
encoder's bytes for the same bitmap in both supported configurations.

## 4. The command path is implemented. CONFIRMED

The generated PPD keeps `*cupsCommands: "ReportLevels"` and routes
`application/vnd.cups-command` to `p1100-usbink`, a native ARM64 Rust command filter. It
locates the attached P1102's USB EWS interface by product name, serial number, and interface
class `ff/02/10`, then uses macOS's IOKit interface client to send
`GET /DevMgmt/ConsumableConfigDyn.xml HTTP/1.1\r\n\r\n` through its bulk pipes.

The filter uses `IOUSBHost` for direct invocations and the IOKit legacy interface client when
it is launched by CUPS; the latter matches HP's original transport and avoids a macOS crash in
`IOUSBLib` under cupsd. The reply is HTTP with chunked transfer encoding. The filter reconstructs its XML, extracts
`<dd:ConsumablePercentageLevelRemaining>`, and emits CUPS marker attributes. A hardware run
against the attached P1102 returned `marker-levels=20` for the black toner. This is direct USB
EWS communication; it does not use PJL or CUPS's printer backchannel.

One nuance, because it is where a future implementation should start if this is ever built:
CUPS gives a filter two different ways to reach the device, and the SDK's headers keep them
apart.

- The **side channel**, file descriptor 4 (`CUPS_SC_FD` in `cups/sidechannel.h`), carries a
  *fixed* command set: soft reset, drain, bidirectional capability, IEEE-1284 device ID, device
  state, SNMP get and get-next, connectivity. There is **no command that writes arbitrary bytes
  to the device**, so it could not carry HP's HTTP request whatever else it could do. Two of its
  commands come close to what this feature wants: `GET_STATE` reports the standard condition
  bits, media and marker low or empty among them, and `SNMP_GET` asks for a Printer MIB OID such
  as `prtMarkerSuppliesLevel`. Both depend on Apple's `usb` backend asking the printer, which is
  the thing to establish first. An earlier version of this note said the side channel would
  carry a PJL supply query; the header says otherwise.
- The **backchannel** (`cupsBackChannelRead` and `cupsBackChannelWrite`), which is the device's
  byte stream. That is what the reference tree's Konica-Minolta command filter uses to send PJL
  and read the reply, and it is what could carry an HTTP request to the printer's embedded web
  server the way HP's driver does. Whether Apple's `usb` backend lets a filter write *to* the
  device out of band is not something this machine can answer.

So the first question for a hardware session that wants supplies reporting is not how to write
the filter but what the printer answers: whether `lpstat -p` shows a paper or toner reason when
the printer is in that state, which would mean the backend is already asking it.

It also may not be worth building even with hardware: the P1102 is a single-cartridge
engine whose level reporting is a local web page, and a USB-attached unit has no address
for the `snmpInk` fallback either. What a hardware session should establish first is simply
whether the request over USB produces a level at all.

**This is the one functional regression relative to the working driver, and it does not
affect printing.**

A second, smaller gap sits next to it, in the *print* path rather than the command path: HP's
filter reports printer state reasons - `STATE: media-empty`, `media-needed`, `media-jam` and the
rest of the strings listed under "Status / error vocabulary" - which it can only do by reading
the backchannel while it works. We do not read the backchannel, so we report no state reasons
of our own. Whether the queue loses anything by that is **UNKNOWN** and worth one observation:
does the queue show a reason (an empty tray, a jam) when the printer is in that state? If it
does, Apple's usb backend is reporting them and this gap costs nothing; if it does not, adding
a backchannel read to the filter is the work.

Part of the answer is already visible: Apple's `usb` backend reports state itself. Run it by
hand with no printer attached and it says `STATE: +offline-report` and `INFO: The printer is
offline.`, which is where connectivity reasons come from. What it cannot know without asking
the device is the paper and toner conditions, which is precisely what HP reads the backchannel
for - so the open question is narrower than it first looks: **do paper and toner reasons appear
on this queue?**

## 5. Duplex: CUPS rotates the back side itself; the ordering is the open part. LIKELY right

The PPD offers Duplex None / DuplexNoTumble / DuplexTumble, and CUPS describes this printer
as doing manual duplex. We forward the choice as `ZJI_DMDUPLEX` items on the document header,
the page and the image and page ends, and leave the ordering of sheets to the printer.

**Measured: CUPS rotates the back side itself, before the filter ever sees the page.** The PPD
declares `*cupsBackSide: "Rotated"`, and a two-page job submitted with
`sides=two-sided-long-edge` comes out of `cupsfilter` with page 2 upside down: its ink sits at
rows 6314..6393 of 6400 where simplex printing puts it at 7..86, a clean 180 degrees. A filter
that rotated the back side therefore would *undo* CUPS's work and print it the wrong way up,
which makes "emit the items and pass the pixels through" the right behaviour here rather than
the least-supported one.

It also explains the reference driver. It rotates every second page itself (`rotate_bytes_180`
in `pbm_pages`, with the "P2(norm), P1(rot180), P4(norm), P3(rot180)" comment) because the
architecture it was written for renders its own raster rather than going through CUPS's
page-setup filters - the same reason it pairs pages and pads an odd count with a blank sheet,
which is why a one-page duplex job from it comes out as two pages.

HP's filter has page-handling machinery of its own (`despoolDuplex`, `addBlankPage`,
`getJobDuplex`, a `zjs::JobSaver`, `_flipLine`, the property names `DupUnit` and
`pjlDuplexOffset`, and three named manual-duplex paths), and it runs at job end rather than per
page: hold pages, pad an odd count with a blank, then write them out. Whether it takes that
path for a P1102 in a given duplex mode is not something the disassembly says on its own.

What is still open is therefore only the *ordering*. Two things point the same way: the printer
prompts for manual duplex itself - the PPD carries its own instruction as `*cupsIPPReason
ManualDuplex`, "remove the paper stack… press the Go button" - and a printer that prompts has
its own idea of page order, which for a flip-the-stack flow is document order. The PPD also
declares no `*cupsManualDuplex`, so CUPS is not splitting the job either. We send document
order, which is what both facts point at.

The two-page `DuplexNoTumble` job was accepted and eventually completed as CUPS job 404 after
the printer-side wait, but the prompt and the physical sheet order were not observed in the
session. The protocol's duplex ordering therefore remains UNKNOWN. Simplex printing is
unaffected either way, and the comparison harness deliberately keeps no duplex case: the
reference reorders pages by construction, so comparing against it there would fail for a
reason that is not ours.

## 6. Media type codes are HP's own. The path to the item is checked; the printer's side is not

The PPD declares sixteen `MediaType` choices, and each one writes its code into the page
device as `cupsMediaType`: Paper 1, TransparencyPaper 2, Light60-74g 258, BondPaper 260,
Heavy111-130g 262, RoughPaper 263, LabelsPaper 265, EnvelopePaper 267, VellumPaper 273,
Mid-Weight96-110g 282, ExtraHeavy131-175g 283, ColorPaper 512, LetterheadPaper 513,
PreprintedPaper 514, PrepunchedPaper 515, RecycledPaper 516. cgpdftoraster copies that code
into the raster header (offset 380 of `cups_page_header2_t`), and we forward the *header's*
value as `ZJI_DMMEDIATYPE` rather than re-deriving it from the option name, because the header
is where the interface says a raster filter should look.

An earlier version of this entry called media type the one thing the comparison harness could
not cover, on the grounds that "cupsfilter cannot produce that (its raster always carries
Paper/1)". That is wrong, and measuring it is what showed the entry up: `cupsfilter -o
MediaType=TransparencyPaper` puts 2 in the header, and a job built from that raster carries
`ZJI_DMMEDIATYPE` 2. What cupsfilter ignores is a name the PPD does not declare -
`-o MediaType=HeavyPaper` stays Paper, because the choice is called `Heavy111-130g` - which is
why everything below uses the file's own names.

So the path is now measured rather than asserted, in two places:

| what is checked | where |
|---|---|
| the PPD's code reaches the raster header | `scripts/reference-compare.py` asks cupsfilter for `TransparencyPaper`, and the case passes only if our job carries the code the header holds |
| the header's code reaches the printer's item | the same comparison, against the reference driver's `-m`: 2 (transparency), 265 (labels) and 282 (mid-weight, a code the reference's help text does not list but its table accepts) |
| every declared choice is forwarded | `scripts/rehearse-job.py` sweeps all sixteen choices of the installed PPD, patching each code into a small page cut out of a real raster and checking `ZJI_DMMEDIATYPE` in the job the real filter binary produces |

What remains unverified is the printer's side: whether it acts on 282, 283, 260 or 512-516 the
way this PPD implies, or wants the Windows `DMMEDIA` values instead. The native queue has not
yet run a non-plain-paper job; the last table in this document says what a failure would look
like.

## 7. Job timestamps. Cosmetic, on a different clock basis

We emit `@PJL SET JOBATTR="JobAttr4=<timestamp>"` from the system clock, in the same
fourteen-digit `YYYYMMDDHHMMSS` shape the reference implementation uses. Its value is ours to
choose, with one measured difference and one unknown.

**The clock basis differs: ours is UTC, the reference's is local time.** A run at 21:25 local
gave `20260915212516` from the reference and `20260915192515` from us, two hours apart.
`scripts/reference-compare.py` removes this line's value - that is the pass condition - but
checks its fourteen-digit shape in both jobs and prints both stamps, so the gap stays visible
instead of being papered over.

HP's binary contains no `JOBATTR` literal at all, so HP's driver most likely sends no job
attribute - an earlier version of this document claimed it sent one of its own, which the
disassembly does not support. What HP's accounting used instead is **UNKNOWN**.

This is also the line `scripts/zjs-compare.py` normalises away by default, precisely so that
a comparison of two jobs is not dominated by it. Nothing on the printer consumes the field,
and matching the reference's local basis would mean reaching for local time - libc's
`localtime_r` through `unsafe` FFI, with a `cfg` so the wasm test build still compiles -
inside a stack that is otherwise pure Rust. That is not a trade worth making for a field the
printer ignores, so the difference is recorded rather than removed.

It is now the *only* difference the reference comparison finds: every job it covers is
byte-identical once this line is removed, and the two jobs are the same length.

## 8. Geometry is taken from the raster rather than chosen. CONFIRMED improvement

Width, height, resolution, media type and copy count come from the CUPS raster header, so a
resolution or paper choice made in the print dialog flows through unchanged. Measured: the
PPD's `600x400dpi` and `600x600dpi` both set `HWResolution[600 600]`, and the raster is
4900x6400 either way. We emit the JBIG image at the 128-pixel aligned width (4992) while
`RASTER_X` keeps the true 4900, which is the distinction the reference implementation
documents as the horizontal pixel alignment.

Orientation needs nothing from the filter either, and that is measured rather than assumed:
`cupsfilter -o landscape` produces the same 4900x6400 Letter header as the portrait job, with
the *content* rotated inside it - the ink that occupies rows 7..86 of columns 2..472 in
portrait occupies rows 5927..6397 of columns 6..85 in landscape, 8,631 ink pixels against
8,629. The geometry and the option stream are the same either way, so a landscape job is the
same job to this filter: CUPS's page-setup filter has already done the rotation.

## 9. Deliberately unchanged

- **The PPD's device description is byte-identical to HP's.** Media sizes, imageable areas,
  resolutions, constraints and IPP reasons are the same file. Only the driver wiring
  differs, and "only" is now measured rather than asserted: generating the PPD and diffing
  it against the installed one shows two added comment lines, the `*NickName` line and its
  sixteen localised `*.Translation NickName/` keys gaining a " (native ARM64)" suffix, the
  raster filter line repointed at our filter, `*APPrinterIconPath` repointed at the icns we
  install, and three lines removed - HP's
  `application/vnd.cups-command` filter, `*cupsCommands`, and `*APPrinterUtilityPath`.
  Nothing else, in 1297 lines. The command is in docs/installation.md, which carries the
  same table.
- **HP's queue, files and receipts are untouched**, so the change is reversible and both
  queues can coexist.
- The 1284 device ID, the USB device path and the queue's device URI are inherited rather
  than re-derived.

## 10. Options a job does not name now come from the PPD. CONFIRMED

The reference driver's default is 7 (auto), our built-in default is 1 (only one), and the
installed PPD labels its only source `Manual`, i.e. 4. On the attached P1102 that PPD marker
triggers manual-feed attention for an ordinary tray job, so the native filter keeps source 1
unless the user explicitly selects `InputSlot=Manual`.

CUPS's manual says that options passed on the command line typically omit the PPD's default
choices, which is why the filter reads the file through `PPD`. The native queue makes one
deliberate hardware-specific exception for the PPD's misleading `Manual` source marker.

The filter still reads the PPD defaults for duplex, econo mode, density, jam recovery and
screening, then lets the command line override them. It deliberately skips only the PPD's
`InputSlot=Manual` default, leaving the built-in source 1; an explicit `InputSlot=Manual`
still carries source 4. Unit tests and the rehearsal cover both paths.

Page size, media type and resolution need none of this: the raster page header already reflects
the PPD's defaults for those, which is where the interface says a raster filter should look.

**Which spelling arrives in argv[5] is worth being exact about**, because it decides what the
filter has to understand. filter(7) says the options are "encoded from the corresponding IPP
attributes used when the job was submitted" and points a filter at `cupsMarkOptions()` to map
them onto PPD options - which is what HP's driver does, since it imports `ppdMarkDefaults`,
`cupsMarkOptions` and `ppdFindMarkedChoice`. Two consequences follow, and both are handled:

- For attributes that have a standard IPP spelling, the name differs from the PPD's: `sides`
  for `Duplex`, `media-source` for `InputSlot`. The filter accepts both spellings of duplex,
  and a unit test pins that down, because accepting only the PPD one would silently ignore a
  two-sided request.
- `media-source` cannot change the installed PPD's displayed source choice, which is a property
  of the file rather than a guess: it declares exactly one `InputSlot` choice, `Manual`. The
  native default nevertheless uses source 1, while an explicit `InputSlot=Manual` remains
  source 4. For HP's custom options (`pjlDensity`, `pjlJamRecovery`, `EconoMode`) the IPP
  attribute name *is* the PPD option name, which is why reading the PPD names works at all.

The check that no option can be accepted and ignored is `scripts/rehearse-job.py`, which reads
the `*OpenUI` groups of the installed PPD and fails if one of them is in neither list:

    mapped by the filter:  Duplex, InputSlot, pjlDensity, pjlJamRecovery, EconoMode
    carried by the raster: PageSize, PageRegion, MediaType, Resolutions

Nine groups, all accounted for, on the PPD this queue uses. It is a check rather than a
paragraph because the realistic way to break it is to regenerate the PPD from a different HP
bundle - a P1505 or a P1560 offers other slots and other groups - and an option in neither list
is a setting the queue would show, accept and never pass to the printer.


## 11. HP sends two bits per pixel through a tone table; we send one. CONFIRMED difference

CUPS hands the filter 8 bits per pixel of grey (`cupsBitsPerColor 8`, `cupsColorSpace 0`), and
something has to turn it into what the printer draws. We compare against a fixed threshold of
128 and pack one bit per pixel, which is what `crates/p1100-raster` has done since the first
commit and what the reference driver sends. HP does something else, and the difference is not a
screen versus a threshold - it is a different bit depth:

- `main` derives two values from the raster header's `cupsCompression`: `bits = (compression &
  1) + 1`, and `vdpi = compression & 0x3ffe` (falling back to `HWResolution[1]` only when that
  is zero). It calls `ProcessMonoTwoBit` when `bits == 2` and `ProcessMonoOneBit` when
  `bits == 1`.
- This PPD's two `Resolutions` choices set `cupsCompression` 401 and 1. Both are odd, so `bits`
  is 2 either way: **every job this queue produces takes HP's two-bit path.** 401 & 0x3ffe is
  400, which is also the "vertical dpi" that `selectScreenTable` is given, so the default
  resolution selects `_sdtahs10`; the `600x600dpi` choice, whose marker is 1, gives 0 there and
  falls back to the raster's 600, selecting `_sdtahr10`.
- `_ScreenPixelsTwoBit` indexes the table by the *pixel value* - `table[pixel * 48]` - and the
  byte it finds holds four 2-bit values, one for each column of a group of four pixels. The row
  comes from the call's line counter, and that counter is the band's *remaining* line count as it
  counts down, so the cell's vertical phase steps by one row per line and restarts at the next
  band - the height is rounded up to a multiple of 12 before it is passed in, which fixes the
  phase's starting point. Each table is therefore 256 entries of 12 rows by 4 bytes: 12288
  bytes, exactly the distance between the two symbols.
- Decoded that way the tables are what they should be, and `scripts/hp-tone-table.py` reads them
  out of the installed driver and checks it: entry 0 is all black, entry 255 all white, and the
  entries between them are clustered-dot cells that get lighter as the grey value rises. On this
  machine `_sdtahs10` has 0, 8, 16, 32, 41, 48, 64, 80 and 144 of its 144 cells at the lightest
  level for grey 0, 32, 64, 96, 128, 160, 192, 224 and 255.

So HP's pages carry **two bits per pixel** - four levels per pixel, from a 12x12 clustered-dot
cell chosen by grey level - while ours carry one, thresholded at 128. The protocol is untouched
(the page is a bitmap either way), but the appearance of anything with grey in it is not:
antialiased text and photographs get a hard edge from us where HP would print a dot pattern, and
the printer would have four levels to work with instead of two.

What matching it would take, and where it stands. The encoder is done: `p1100-jbig` writes
`planes(n)`, and its two-plane output is byte-identical to the reference encoder's for six pages
in `scripts/jbig-sweep.py`, with goldens for a one-stripe and a two-stripe page in the crate's
own tests.

The conversion is implemented twice over, once in `scripts/hp-tone-table.py --convert` and
once in the library (`p1100_raster::to_two_bit_planes`, reachable as
`p1100 raster-to-zjs --2bit --tone-table FILE`), and the two are held together by
`scripts/two-bit-compare.py`: the same page built through our conversion, our two-plane encoder
and our framing, and through the tool plus the reference encoder, has to come out the same job.
That check earned its keep immediately - it caught the aligned columns past the page width being
padded black rather than white, where the one-bit path's padding is white and a zeroed two-bit
margin is not. It is in the filter too, behind the `p1100Grey` option - `Mono1` by default, `Screened2`
to ask for this - which reads the two tables from beside its own binary or from `P1100_TONES`.
The installer does both halves: `scripts/make-ppd.py` injects the option group, with `Mono1` as its
default, and `scripts/install.sh` reads the two tables out of the installed HP driver and installs
them beside the filter. A machine that has no HP driver left still gets a working queue: only
the option that needs them fails, and the filter says so.

The item side is done with it: `PageGeometry`'s `bits_per_pixel` is what `ZJI_VIDEO_BPP`,
`ZJI_VIDEO_X` and `ZJI_NBIE` are computed from, and the job layer sets it from the same option
that selects the screening, so a two-bit page announces itself consistently.

That marker also picks the table: `ToneTables::for_page` takes the 400 dpi cell when `cupsCompression & 0x3ffe` is 400 and the 600 dpi one otherwise, which is what the bullet
above says HP does with it, and the job layer tests both halves: the page marked 401 comes out
exactly as it would with the 400 dpi table in both slots.

The framing has now crossed the printer path: the native queue completed job 403 with
`p1100Grey=Screened2`, including the two plane records. HP's page items come from
`zcups::Formatter::startPage`, which takes the depth as its third argument and reads a
`bitsPerPixel` property, and whether plane records are written at all is behind three
`zjs::BandWriter` flags - `sendPlaneItem`, `sendStartPlaneChunk` and `sendEndPlaneChunk` -
which are what a two-bit implementation has to settle. The tone tables are HP's own data, so
a legitimate implementation reads them out of the user's installed driver:
`scripts/hp-tone-table.py --out DIR` writes them. The transfer is accepted; the threshold
remains the default until the printed grey quality is compared with HP's output.

So the compression is verified here and the conversion is written and checked against the
tables, the page items follow from the depth, and the two-bit framing is accepted by the
attached printer. Only the visual comparison waits for human inspection.

One observation settles whether the two-bit difference matters in practice: print a graded fill
or antialiased text and compare it with the same page through HP's queue while that still runs.
If ours looks worse in a way someone would notice, the work is the two-bit path above - and the
same comparison can be made against our own queue once the two-bit page can be sent, because a
bi-level page screens to itself: the tables take grey 0 to level 0 and grey 255 to level 3, so a
one-bit job and a two-bit job of the same document are the same page and the difference between
them is the depth and nothing else.

## 12. The job now ends when the printer says the page is out. CONFIRMED in the reference

With HP's driver, a job sits in Print Center until the sheet has been printed; the native
filter used to end the job as soon as the bytes had been handed to the USB backend, so the row
disappeared while the printer was still working. The reference filter does not do that: at the
end of a job it writes `@PJL EOJ`, joins its backchannel reader, and only then sends
`@PJL USTATUSOFF` and exits, waiting for the printer's `USTATUS JOB` `END`. CUPS keeps a job
open while one of its filters is still running, which is why that wait is what the row in the
print dialog measures. `docs/protocol.md` has the disassembly evidence.

The native filter now follows the same sequence, with one addition the reference does not
need: because a filter that waits can hold a queue open, the wait is bounded. It gives up
after 30 seconds if the printer has said nothing at all about the job, after 90 seconds
between pages - the timed device reports the prologue also asks for do not count as news -
and after five minutes in total. The job then ends with a `WARNING:` rather than disappearing
quietly. The filter also writes nothing after the job: HP sends `@PJL USTATUSOFF` there, but
that opens a second PJL session after the job and is not worth the risk on a printer that is
already waiting.

`crates/p1100-filter/tests/process.rs` covers it without a printer: the test hands the filter
fd 3 as a socket pair, asserts the process is still running after the job bytes have been
written, that a job start is not an end, and that the `USTATUS JOB` `END` is what lets it
finish. The physical question - whether the row now survives until the paper comes out - is
the one thing this needs the attached printer for.

## What settles the unverified items

Each one maps to an observation the hardware test makes:

| item | what to look for |
|---|---|
| 1, PJL wrapper | both PJL and no-PJL A4 jobs were accepted with final back-channel responses; visual output remains a separate observation |
| 2, JBIG configuration | settled, and not a suspect: the options, the order, L0 and the adaptive template all match what HP's encoder is configured with, and the compressed image is byte-identical to the reference's. A corrupt or truncated page would point at the framing or at the printer's own decoder |
| 4, command path | settled on hardware: the command filter answers `ReportLevels` and the print dialog's Supply Levels tab shows the printer's own percentage instead of "Information Not Available" |
| 5, duplex | job 404 completed after the printer-side wait; inspect the prompt and physical sheet order on a future run to settle whether host-side pairing is needed |
| 6, media codes | whether a non-plain-paper job prints correctly, or jams or misfeeds |
| 10, media source | settled on hardware: the PPD's `Manual` marker caused attention for ordinary jobs, so the native default is DMBIN 1 and explicit `InputSlot=Manual` remains DMBIN 4 |
| 11, grey conversion | job 403 with `p1100Grey=Screened2` completed through CUPS; visual tone/edge quality still needs inspection against HP's queue |
| 12, job completion | that the row in Print Center stays until the sheet is out, and that a printer which reports nothing still ends the job, after the 90-second silence warning |

Everything else that used to be listed as uncertain - the wrapper text, the item sets and
their order, the page geometry, the JBIG configuration, the compressed image, the chunk
framing and the padding of the final image chunk - is now settled against the reference
driver, byte for byte, by `scripts/reference-compare.py`. What remains here is what only the
printer can answer.
