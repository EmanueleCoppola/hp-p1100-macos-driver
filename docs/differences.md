# Differences from HP's driver

The goal was to reproduce the working driver's behaviour as closely as is reasonable, and to
explain every difference where it is not exact. This is that list. Confidence tags as in
docs/protocol.md: **CONFIRMED**, **LIKELY**, **UNKNOWN**.

## 1. The PJL wrapper text differs. CONFIRMED difference, low risk

HP's filter emits `@PJL JOB NAME=...`, `@PJL ENTER LANGUAGE=...` and `@PJL EOJ`. We emit the
wrapper used by the open reference implementation, which is `@PJL JOB`, has no
`ENTER LANGUAGE` line, and adds `@PJL SET JOBATTR=...`.

Why: HP's binary cannot be executed on this machine, so its exact byte output cannot be
captured, while the reference implementation is verified on this printer family and is
therefore the safer model. The risk it carries is specific and worth naming: **if the
printer requires `ENTER LANGUAGE` to switch into ZjStream, it would ignore the whole job.**
A raw-queue test distinguishes that from an image problem, because the same test can send a
job whose compressed image is known-good.

The rest of the wrapper we emit matches HP's in substance: density, econo mode, jam
recovery, resolution enhancement, and the unsolicited status requests are all the same
settings, driven by the same PPD options.

## 2. Our JBIG parameters are simpler than HP's. LIKELY equivalent, UNKNOWN on hardware

We emit one resolution layer, one bit plane, no typical prediction, no two-line low
resolution template, no adaptive template movement, and 128 lines per stripe. HP's filter
uses its own encoder with parameters we cannot read out of a binary we cannot run.

Two things make this defensible rather than hopeful: every conforming decoder must handle
every option combination, and our output is byte-identical to the reference implementation
of the standard for the configuration we emit. The residual risk is a printer-side decoder
that only accepts what HP's driver happens to send, which is not something a standard
guarantees against.

If it turns out to matter, typical prediction is the most likely candidate to add, and it
does not change the framing or the coder.

## 3. The compressed image bytes are ours, not HP's. Expected, both valid

Both are T.82 JBIG, but the exact bytes depend on encoder parameters, so they will never
match HP's output byte for byte even if the printer accepts both. The requirement was
semantic equivalence, and that is what is verified: the image decodes back to exactly the
bitmap the raster describes, using the standard's own decoder.

## 4. The command path is not implemented. CONFIRMED gap

HP's PPD routes `application/vnd.cups-command` to `usbink`, which answers `ReportLevels` by
opening the printer over USB with IOKit and reading its status. Our generated PPD drops
that line and `*cupsCommands`, so **the supplies/clean items that HP's driver provided are
gone** for this queue.

Why: `usbink` is an x86_64 binary that talks to the device itself, and this project
deliberately keeps transport in Apple's native usb backend instead of adding a second
implementation of the same thing. That is an architectural choice, not a limitation of the
approach: the transport is native and signed by Apple, and duplicating it would be more code
to maintain for no gain in printing.

Options for restoring it, in order of preference: a command filter that speaks PJL over the
CUPS backchannel, which needs no libcups since the backchannel is just a file descriptor
CUPS hands the filter; or a small IOKit helper. Neither is written yet because neither is
verifiable without the printer attached, and writing untested device code against a protocol
whose responses we have never seen is how you get a component that looks finished and is not.

**This is the one functional regression relative to the working driver, and it does not
affect printing.**

## 5. Duplex items are emitted, but the printer has no duplexer. UNKNOWN

The PPD offers Duplex None / DuplexNoTumble / DuplexTumble, and CUPS describes this printer
as doing manual duplex. We forward the choice as `ZJI_DMDUPLEX` items on the document header,
the page and the image and page ends. Whether the P1102 acts on them, prompting for manual
duplex, or ignores them is unverified. Simplex printing is unaffected either way.

## 6. Media type codes are HP's own. UNKNOWN for anything but plain paper

We forward the raster header's `cupsMediaType`, which HP's PPD sets: Paper = 1, Transparency
= 2, and heavier stocks 258, 262, 265, 273 and so on. For the default Paper it is 1, which
is also the value the reference implementation uses, so the common case agrees. Whether the
printer expects HP's codes or the Windows `DMMEDIA` values for the exotic media is
unverified.

## 7. Job timestamps. Cosmetic

We emit `@PJL SET JOBATTR="JobAttr4=<UTC timestamp>"` from the system clock. HP's driver
emits a job attribute of its own. This is the difference scripts/zjs-compare.py normalises
away by default, precisely so that a comparison of two jobs is not dominated by it.

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
| 1, PJL wrapper | if nothing prints at all, this is the first suspect; the reference-correct payload rules out our encoder |
| 2, JBIG parameters | a page that prints but is corrupt or truncated points here, since the framing and the bitmap are both independently verified |
| 4, command path | absent supplies reporting is visible in the print dialog, not in printing |
| 5, duplex | whether a two-sided job prompts for manual duplex |
| 6, media codes | whether a non-plain-paper job prints correctly, or jams or misfeeds |

