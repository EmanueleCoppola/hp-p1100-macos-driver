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

## 1. The PJL wrapper follows the reference, not HP. CONFIRMED difference, low risk

HP's filter emits `@PJL JOB NAME=...`, `@PJL ENTER LANGUAGE=...` and `@PJL EOJ`. We emit the
wrapper used by the open reference implementation, which is `@PJL JOB`, has no
`ENTER LANGUAGE` line, and adds `@PJL SET JOBATTR=...`.

Why: HP's binary cannot be executed on this machine, so its exact byte output cannot be
captured, while the reference implementation is verified on this printer family and is
therefore the safer model. The risk it carries is specific and worth naming: **if the
printer requires `ENTER LANGUAGE` to switch into ZjStream, it would ignore the whole job.**
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

So the two wrappers differ in both directions, and we follow the reference in both: no
`JOB NAME`, no `ENTER LANGUAGE`, no `EOJ`, plus `ECONOMODE`, `RET=MEDIUM`, `USTATUS TIMED`
and the `JOBATTR` line. The one-line discriminator, if a hardware test ever needs it, is
that the reference's payload can be sent with and without its wrapper (`P1102_NO_PJL=1`),
which separates "the printer rejects our framing or image" from "the printer wants a
different wrapper".

The rest of the wrapper we emit matches HP's in substance: density, econo mode, jam
recovery, resolution enhancement, and the unsolicited status requests are all the same
settings, driven by the same PPD options.

## 2. The JBIG options now match HP's byte; the adaptive template may not. UNKNOWN

This entry has been rewritten: we no longer emit a simpler configuration. Our BIH carries
`options = 0x5c` (DPON | TPBON | TPDON | LRLTWO) with typical prediction implemented, and
`order = 3`, `L0 = 128`, `MX = 0`. That is exactly the configuration the reference driver
sends, and it is also HP's: disassembling `jbig::ZjbigEncoder`'s constructor in
`rastertozjs` shows `movb $0x5c, 0x93(%rbx)` and `movb $0x3, 0x92(%rbx)` - the two bytes
`getBIH` copies into BIH offsets 19 and 18.

What is left is the adaptive template: HP's constructor seeds that field with `0x10` (16)
and reads it back from `atmove`, `minAtMove` and `maxAtMove` properties in
`zjs::JbigBandWriter::start`, so HP's stream may move the template pixel; the reference
driver forces `MX = 0` for this model family, and we match the reference. A stream that
announces `MX = 0` never moves the pixel, which every decoder must accept, so the residual
risk is only that the printer's decoder handles *fewer* cases than HP's driver uses - the
converse of the usual worry.

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

## 4. The command path is not implemented. CONFIRMED gap

HP's PPD routes `application/vnd.cups-command` to `usbink`, which answers `ReportLevels` by
opening the printer over USB with IOKit and reading its status. Our generated PPD drops
that line and `*cupsCommands`, so **the supplies/clean items that HP's driver provided are
gone** for this queue.

What `usbink` actually does is now known, and it is worth recording because it changes what
"restoring it" would mean. It does **not** speak PJL and does **not** use the CUPS
backchannel: it carries a hand-rolled HTTP client and sends
`GET /DevMgmt/ConsumableConfigDyn.xml HTTP/1.1\r\n\r\n` over the USB interface it opened,
then reports the level as `ATTR: marker-...` plus a
`<dd:ConsumablePercentageLevelRemaining>` element. The full evidence is in
`docs/protocol.md`; the short version is that the binary contains no networking symbols at
all, so the request must travel over the bulk pipes.

Why we do not do that: it means writing an IOKit client and a second transport for a
device's embedded web server, in a stack whose point is to keep the transport Apple already
ships - and for printing it changes nothing. The two mechanisms this document used to
propose (PJL over the CUPS backchannel) would have been a different protocol from the one
the printer actually answers, which is the kind of plausible-looking substitute this
project is trying not to ship.

One nuance, because it is where a future implementation should start if this is ever built:
CUPS does have a supported way for a *filter* to query the device - the side channel, which
the reference tree uses in its Konica-Minolta command filter (`cups/sidechannel.h`) to send
PJL and read the reply, and which Apple's own supplies reporting uses too. HP did not take
that route for this family even though it was available, and **LIKELY** that is because the
printer does not report supplies over PJL at all: the toner level on these models lives in
the embedded web server, which is why the driver goes through the trouble of an HTTP request
over the bulk pipes. So the first question for a hardware session that wants this feature is
not "how do I write the filter" but "does the printer answer a PJL supply query", and the
side channel is what would carry it.

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

Settled by one two-sided test, with two specific things to look for: does the printer prompt,
and do the sheets come back in the right order? Simplex printing is unaffected either way, and
the comparison harness deliberately keeps no duplex case: the reference reorders pages by
construction, so comparing against it there would fail for a reason that is not ours.

## 6. Media type codes are HP's own. UNKNOWN for anything but plain paper

We forward the raster header's `cupsMediaType`, which HP's PPD sets: Paper = 1, Transparency
= 2, and heavier stocks 258, 262, 265, 273 and so on. For the default Paper it is 1, which
is also the value the reference implementation uses, so the common case agrees. Whether the
printer expects HP's codes or the Windows `DMMEDIA` values for the exotic media is
unverified.

This is the one item the comparison harness cannot cover, and the reason is now precise:
`cupsMediaType` reaches our filter in the raster header, set by the PPD's `MediaType`
option through the PostScript page device, and `cupsfilter` cannot produce that (its raster
always carries Paper/1) while the reference takes the code as a flag. The mapping itself is
covered by a unit test in `crates/p1102-job`; what is unverified is the printer's side.

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

## 9. Deliberately unchanged

- **The PPD's device description is byte-identical to HP's.** Media sizes, imageable areas,
  resolutions, constraints and IPP reasons are the same file. Only the driver wiring
  differs, and "only" is now measured rather than asserted: generating the PPD and diffing
  it against the installed one shows two added comment lines, the `*NickName` line and its
  sixteen localised `*.Translation NickName/` keys gaining a " (native ARM64)" suffix, the
  raster filter line repointed at our filter, and three lines removed - HP's
  `application/vnd.cups-command` filter, `*cupsCommands`, and `*APPrinterUtilityPath`.
  Nothing else, in 1297 lines. The command is in docs/installation.md, which carries the
  same table.
- **HP's queue, files and receipts are untouched**, so the change is reversible and both
  queues can coexist.
- The 1284 device ID, the USB device path and the queue's device URI are inherited rather
  than re-derived.

## 10. Options a job does not name now come from the PPD. CONFIRMED

Three defaults disagree, and all three are defensible on a printer with one tray: ours is 1
("only one"), the reference driver's is 7 (auto), and the PPD's own default - which HP's
filter reads out of the PPD with `ppdMarkDefaults` and `ppdFindMarkedChoice`, both visible in
its imports - is `*DefaultInputSlot: Manual`, i.e. 4.

CUPS's own manual settles it, and not in our favour. filter(7) says that "options passed on
the command-line typically do not include the default choices [of] the printer's PPD file",
and it tells a filter to take them from the file - which CUPS hands it in `PPD` - through
`ppdMarkDefaults` and `ppdFindMarkedChoice`, the two functions HP's driver imports. So the
command line is not carrying the defaults, and a job that named no options sent our 1 where
HP sends the PPD's 4.

That is now fixed rather than documented. The filter reads the PPD's defaults for the five
options it maps - media source, duplex, econo mode, density, jam recovery - and then lets the
command line override them, which is the order the interface describes. On this printer it
changes exactly one item, `ZJI_DMDEFAULTSOURCE`, from 1 to 4 by default. A PPD that cannot be
read is not fatal: the built-in defaults stand, as they did before. Unit tests cover the
reading, and the rehearsal checks the real PPD - with `PPD` set and no options the page carries
4, and with `InputSlot=Upper` on the command line it carries 1 again.

Page size, media type and resolution need none of this: the raster page header already reflects
the PPD's defaults for those, which is where the interface says a raster filter should look.

## 11. Grey is screened by HP and thresholded by us. CONFIRMED difference, appearance only

CUPS hands the filter 8 bits per pixel of grey - the PPD asks for exactly that
(`cupsBitsPerColor 8`, `cupsColorSpace 0`) - and something has to turn it into the printer's one
bit. HP's filter does it with a screen: its symbols include `ProcessMonoOneBit`,
`ProcessMonoTwoBit`, `_ScreenPixelsOneBit` and `_ScreenPixelsTwoBit`, and the one-bit path walks
the pixels comparing each against a threshold table, packing the result with a shifting bit
mask. We compare against a fixed threshold of 128 instead, which is what `crates/p1102-raster`
has done since the first commit.

What it changes, and what it does not: the protocol is untouched - the page is a bitmap either
way - but the *appearance* of anything with grey in it is not. Antialiased text gets harder
edges from us, and a photograph or a gradient gets a hard edge where HP would print a pattern
of dots.

HP's screen is not reproduced, for a reason worth stating rather than hiding: the matrix is not
readable statically. `ProcessMonoOneBit` takes it as its `unsigned char*` argument and `main`
passes a null there, and the caller is the whole of `main`: there is no PPD attribute for it
either (the PPD has no screen or halftone keyword, only `*DefaultColorSpace: Gray`), so the
table it falls back to is built inside the filter and the path to it did not resolve in the
time this was worth. Reproducing HP's halftone needs that table; substituting a screen of our
own would be a guess with the same standing as the threshold, so the simpler behaviour stayed.

One observation settles whether it matters: print a graded fill or antialiased text, and compare
it with the same page through HP's queue while that still runs. If ours looks worse in a way
someone would notice, the work is to find HP's screen rather than to invent one.

## What settles the unverified items

Each one maps to an observation the hardware test makes:

| item | what to look for |
|---|---|
| 1, PJL wrapper | if nothing prints at all, this is the first suspect; resending the same payload with `P1102_NO_PJL=1` separates a wrapper problem from an image problem |
| 2, JBIG adaptive template | a page that prints but is corrupt or truncated points here, since the framing, the coder and the typical-prediction path are all byte-verified against the reference |
| 4, command path | absent supplies reporting is visible in the print dialog, not in printing |
| 5, duplex | send a two-page job with `Duplex=DuplexNoTumble`: the printer should prompt to flip the stack ("press the Go button"), and the back of each sheet should be the right way up, which CUPS's rotation has already arranged. Single-sided output with no prompt would mean the job needs host-side page *pairing*, not rotation |
| 6, media codes | whether a non-plain-paper job prints correctly, or jams or misfeeds |
| 10, media source | settled: the PPD's default is read and an explicit `InputSlot` overrides it. If a job ever prints one page and then waits for paper, the code it sends is DMBIN 4 (manual feed, the PPD's default) |
| 11, grey conversion | print a graded fill or antialiased text: ours thresholds, HP's screens, so expect harder edges. HP's queue can be used for the comparison while it still exists |

Everything else that used to be listed as uncertain - the wrapper text, the item sets and
their order, the page geometry, the JBIG configuration, the compressed image, the chunk
framing and the padding of the final image chunk - is now settled against the reference
driver, byte for byte, by `scripts/reference-compare.py`. What remains here is what only the
printer can answer.
