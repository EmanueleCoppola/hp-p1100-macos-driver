# External references and licensing

Confidence tags as in `docs/protocol.md`: **CONFIRMED** / **LIKELY** / **UNKNOWN**.

## 1. Is this a known protocol?

**CONFIRMED yes.** The installed driver is a ZjStream (ZJS) implementation, not a
bespoke HP format. See `docs/protocol.md` for the local evidence (`zjs::` / `zeno::`
namespaces, `ZjsMonoFormatter`).

What the public material says:

- OpenPrinting's ZjStream page describes ZJS as Zenographics' raster protocol, notes
  that HP never published a reference implementation, and that the format was
  reconstructed by the OpenPrinting/foo2zjs work rather than disclosed.
  Source: <https://www.openprinting.org/driver/zjs>
- The `zjs(4)` manual page documents the driver's feature matrix (supported page
  sizes, resolution, duplex, paper handling) and is the closest thing to a public
  description of the driver surface.
  Source: <https://www.opennet.ru/man.shtml?topic=zjs&category=4> (mirror of the same man page)

=> the protocol is **reverse-engineered, not specified**. There is no authoritative
specification to code against, which is exactly why the plan in the README puts capture
experiments ahead of writing the encoder.

## 2. Reference implementations that already support the P1102

- **foo2zjs** (OpenPrinting) supports the HP LaserJet Professional P1102 / P1102w,
  among many ZJS printers. This is the important one: an existing open implementation
  for our exact model already exists and can be used to *validate* and *compare* our
  encoder.
  Source: <https://packages.fedoraproject.org/pkgs/foo2zjs/foo2zjs/>
- `foo2zjs-wrapper` is the documented entry point; its man page lists the knobs that
  map onto things we independently found inside HP's macOS binary:

  | foo2zjs option | meaning | matches in HP's `rastertozjs` |
  |---|---|---|
  | `-p paper` | paper code (`-p1` Letter, `-p9` A4) | `Paper`, `Letter`, `A4` handling |
  | `-r resolution` | resolution | `0x118` = 600, 600 |
  | `-d duplex` | duplex | `duplex`, `sides`, `DupUnit` |
  | `-n copies` | copies | `copies` |
  | `-z model` | printer family | `cupsModelNumber: 256` |
  | `-P` | suppress `START_PLANE` codes for some monochrome printers | **`startPlane`**, `closePrevious` |
  | `-g WxH`, `-u`, `-l`, `-L`, `-X` | page geometry, offsets, clipping, padding | `marginX`, `marginY`, `minAtMove`, `maxAtMove`, `ZxBits` |

  Source for options: <https://manpages.ubuntu.com/manpages/noble/man1/foo2zjs-wrapper.1.html>

  The `START_PLANE` correspondence is the strongest external confirmation we have that
  HP's macOS filter and foo2zjs encode the same protocol.

  **It is also runnable here.** `scripts/foo2zjs-oracle.sh` fetches the upstream tarball
  (checksum pinned), builds it with Xcode's toolchain and runs it, so the comparison in
  `scripts/reference-compare.py` is against the real driver rather than against a reading of
  its source. The OpenPrinting database it ships is a primary source in its own right: the
  `foo2zjs-z2` driver's `prototype` is `foo2zjs-wrapper -z2 -P -L0 %A`, and its record for
  `HP-LaserJet_Pro_P1102` carries the printer's own IEEE 1284 identity
  (`CMD:ZJS,PJL,ACL,HTTP;...;FWVER:20090924`), both of which are quoted in
  `docs/protocol.md`.

## 3. Capture tooling

The standard way to obtain ground truth for an undocumented USB raster protocol is to
capture the USB bulk traffic while printing controlled pages:

- Linux: `usbmon` + `tcpdump -i usbmon1 -w zjs.pcap`, or `lsusb -v` for descriptors.
- Windows: USBPcap with Wireshark.
- Here on macOS: the equivalent is a debug-quiet approach — we can capture the filter's
  *output* if we can run the encoder at all, and for the wire we need the device. Note
  that Apple's `usb` backend does not provide a trivial tee; see the plan in the README.

Sources: usbmon documentation and general USBPcap/Wireshark USB-capture write-ups
(multiple low-authority pages found; treat as tooling hints, not as protocol evidence).

One lower-quality source (a personal blog about adding an HP printer on Linux) also
recommends capturing USB traffic and diffing a blank page against a page with a black
square. The *method* is sound and matches what we intend to do; the page itself is not
authoritative and is not used as evidence here.

## 4. Licensing implications

This project must be clean. Concretely:

| component | licence | consequence for us |
|---|---|---|
| **foo2zjs / foo2xqx** | GPL-2.0 | We may read it to understand the protocol and to produce *comparison* output. We must **not** copy code into this project unless we deliberately licence this project GPL-2.0-compatible. |
| **Markus Kuhn's portable free JBIG library**, statically linked into HP's filter | GPL-2.0-or-later, with a note offering other terms on request; plus a caveat that it ships no patent licence | We must not copy it. If we need JBIG we implement or link it deliberately, with the licence chosen on purpose. |
| **HP `.acl` device firmware** (P1005/P1006/P1505/1020) | proprietary HP | Not redistributable. Not needed for the P1102 (no firmware blob in its bundle). If firmware is ever needed, the driver must point at the user's own installed copy rather than bundling it. |
| **HP `rastertozjs` / `usbink`** | proprietary | Golden reference only. Must not be committed to the repo, and must not be redistributed. |
| **CUPS** | Apache-2.0 (Apple's fork ships with macOS) | Fine to link to / call. Our filter is a separate program. |

### Test oracles

Two GPL implementations are used **only** to check our own output. Neither is committed,
linked or shipped, and neither is a source of copied code:

| oracle | used for | how |
|---|---|---|
| **foo2zjs** | validating the *whole job* - wrapper, chunk framing, items, geometry, image and padding - byte for byte | `scripts/foo2zjs-oracle.sh` builds it; `scripts/reference-compare.py` compares |
| **jbigkit 2.1** | validating our JBIG *compressed image*, byte for byte | `scripts/jbig-oracle.sh`, installed from a prebuilt Homebrew bottle |

`jbigkit` is Markus Kuhn's reference implementation of the standard - the same library HP
bundled in its own driver - which is precisely what makes it useful. Having it turns
"write a JBIG encoder and hope" into "write a JBIG encoder and prove it byte for byte
against the reference".

That "the same library" is now evidence rather than inference. HP's `rastertozjs` for this
printer exports jbigkit's own internal table symbols - `_jbg_nlps`, `_jbg_nmps`,
`_jbg_dptable`, `_jbg_resred` - so it is the library itself, statically linked and wrapped
in HP's `jbig::` C++ classes, not a reimplementation. One curiosity worth recording: the
**P1102's** bundle carries no licence file at all, while the sibling `hplaserjetzjs.bundle`
(the P1005/P1006/P1505/1020 driver) does, and that file is the GPL notice for Markus Kuhn's
library, `jbig.c,v 1.1 2007`.

**The licence we chose, and why.** This project is licensed under the MIT licence, whose
canonical text is in `LICENSE`. That is possible because the implementation is our own:
written from the ZjStream format, the T.82 standard and observed behaviour. The reference
implementations were read to *understand* the protocols and are used only as test oracles -
never linked, never copied, never shipped - which is what keeps the GPL of foo2zjs and jbigkit
out of this codebase.

Worth being precise about one thing rather than overstating it: "read to understand" is not
the same as a strict clean-room, where the original is never seen. The formats are facts and
the code is ours. A permissive MIT licence was chosen and recorded here because the manifest
declares it.

**The one thing that is necessarily shared** is the normative T.82 arithmetic coder table in
crates/p1100-jbig: a table of constants that every conforming JBIG implementation has to
contain, published in the standard, not copied implementation code.

**JBIG is implemented, not deferred.** That supersedes the fallback reasoning this section
used to carry: the encoder is written from the standard and is byte-identical to the
reference implementation for the configuration it emits (one resolution layer, one bit
plane, typical prediction and the lowest-resolution layer's two-line template on, no
adaptive template movement - the configuration both working drivers use for this family).
The patent caveat in the JBIG library's licence text
refers to arithmetic-coding patents from the 1980s and 1990s that are **LIKELY** expired, but
that is unverified and worth confirming before any commercial distribution. See
docs/jbig.md for the format and docs/differences.md for the parameters we emit.


## 5. Open questions

Resolved since this list was first written:

1. **ZjStream chunk encoding.** Fully specified in `docs/protocol.md`, corroborated by an
   independent implementation, and covered by golden byte-level tests.
2. **CUPS raster layout.** Reconciled against CUPS's own `cups_page_header2_t`; the earlier
   note about it not matching was an artefact of assuming a `Magic` field that this CUPS
   revision no longer has.
3. **Apple's PPD-based printing path.** PPDs are deprecated but still shipping for macOS
   26.x, and a filter based on a PPD still runs. The architecture decision and its
   migration trigger are in `docs/architecture.md`.

Still open, and all blocked on hardware or on further implementation:

4. P1102 USB vendor/product id and interface class. **UNKNOWN** until the printer is
   attached.
5. Whether the P1102 needs a host-side firmware download after power-on. **LIKELY no** - no
   firmware blob ships in its bundle - but unverified.
6. Whether the printer accepts `ZJT_RAW_IMAGE`. If it does, no JBIG encoder is needed at
   all.
7. The JBIG encoder itself. The format is now documented in `docs/jbig.md` and a
   byte-exact oracle exists, so this is an implementation task rather than a research one.
8. Whether the JBIG and arithmetic-coding patent caveats in the JBIG licence text still
   apply. Those patents date from the 1980s-90s and are **LIKELY** expired, but that
   should be confirmed before shipping an encoder rather than assumed.
