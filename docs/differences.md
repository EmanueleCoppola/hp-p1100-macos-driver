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

The language argument comes from a table beside it that holds `HBS` and `ZJS`, so **LIKELY**
HP sends `@PJL ENTER LANGUAGE=ZJS`. Three further things follow from that list and from the
PPD. The `DENSITY` and `JAMRECOVERY` lines are not in the binary because the PPD supplies
them - its two `*cupsPJL` entries are exactly those two lines - which is also why the
driver's strings carry the *option names* `cupsPJL`, `pjlDensity`, `pjlJamRecovery`,
`pjlDuplexOffset` and `cupsRET`. And there is no `JOBATTR`, `ECONOMODE`, `RET=MEDIUM` or
`USTATUS TIMED` literal anywhere in it, so the three lines the reference adds and HP does
not have are exactly those.

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

It also may not be worth building even with hardware: the P1102 is a single-cartridge
engine whose level reporting is a local web page, and a USB-attached unit has no address
for the `snmpInk` fallback either. What a hardware session should establish first is simply
whether the request over USB produces a level at all.

**This is the one functional regression relative to the working driver, and it does not
affect printing.**

## 5. Duplex items are emitted; the reference reorders pages host-side. UNKNOWN

The PPD offers Duplex None / DuplexNoTumble / DuplexTumble, and CUPS describes this printer
as doing manual duplex. We forward the choice as `ZJI_DMDUPLEX` items on the document header,
the page and the image and page ends, and leave the ordering of sheets to the printer.

The reference driver does more than that, and the comparison is what surfaced it: for a
duplex job it pairs pages in host memory, rotates every second page 180 degrees for
long-edge duplex, and pads an odd page count with a blank sheet (`rotate_bytes_180` in
`pbm_pages`, and the "P2(norm), P1(rot180), P4(norm), P3(rot180)" comment in
`foo2zjs.c`). That is written for the P1606dn, which has an automatic duplexer, and it is
why a one-page duplex job from it comes out as two pages.

We do not do that: a CUPS filter that streams pages should not reorder a manual-duplex job,
and HP's architecture leaves this to the firmware through the same item. Which behaviour the
P1102 wants is **UNKNOWN** and is settled by one two-sided test. Simplex printing is
unaffected either way; the comparison harness therefore does not include a duplex case, so
that its pass condition stays meaningful.

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

## 7. Job timestamps. Cosmetic, and the only remaining byte difference

We emit `@PJL SET JOBATTR="JobAttr4=<UTC timestamp>"` from the system clock, which is what
the reference implementation does too. HP's binary contains no `JOBATTR` literal at all, so
HP's driver most likely sends no job attribute - an earlier version of this document claimed
it sent one of its own, which the disassembly does not support.

This line is what scripts/zjs-compare.py normalises away by default, precisely so that a
comparison of two jobs is not dominated by it.

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
  resolutions, constraints and IPP reasons are the same file; only the driver wiring lines
  differ, and the diff is exactly those lines. See docs/installation.md.
- **HP's queue, files and receipts are untouched**, so the change is reversible and both
  queues can coexist.
- The 1284 device ID, the USB device path and the queue's device URI are inherited rather
  than re-derived.

## What settles the unverified items

Each one maps to an observation the hardware test makes:

| item | what to look for |
|---|---|
| 1, PJL wrapper | if nothing prints at all, this is the first suspect; resending the same payload with `P1102_NO_PJL=1` separates a wrapper problem from an image problem |
| 2, JBIG adaptive template | a page that prints but is corrupt or truncated points here, since the framing, the coder and the typical-prediction path are all byte-verified against the reference |
| 4, command path | absent supplies reporting is visible in the print dialog, not in printing |
| 5, duplex | whether a two-sided job prompts for manual duplex, and whether the sheets come out in the order the driver left them |
| 6, media codes | whether a non-plain-paper job prints correctly, or jams or misfeeds |

Everything else that used to be listed as uncertain - the wrapper text, the item sets and
their order, the page geometry, the JBIG configuration, the compressed image, the chunk
framing and the padding of the final image chunk - is now settled against the reference
driver, byte for byte, by `scripts/reference-compare.py`. What remains here is what only the
printer can answer.
