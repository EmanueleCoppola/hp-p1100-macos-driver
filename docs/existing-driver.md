# The existing (HP) driver stack

Target device: **HP LaserJet Professional P1102**, connected over USB.

> Note on scope. The original request named an HP LaserJet P1005. This machine has
> no P1005 queue and no P1005 device record anywhere in CUPS; the only HP LaserJet
> queue is a P1102. The user confirmed the P1102 is the real target. The P1005 is a
> *different* driver bundle on this system (see "P1005 vs P1102" below) and is out of scope.

## Queue configuration (CONFIRMED)

From `lpstat -v` / `lpstat -t`:

```text
printer HP_LaserJet_Professional_P1102
device  usb://Hewlett-Packard/HP%20LaserJet%20Professional%20P1102?serial=000000000Q80E182PR1a
```

PPD in use: `/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd`

```text
*PCFileName:  "hp1100.ppd"
*FileVersion: "6.9"
*ModelName:   "HP LaserJet Professional P1100"
*Product:     "(HP LaserJet Professional P1102)"  (also P1104/P1106/P1107/P1108/P1109)
*1284DeviceID: "MANUFACTURER:Hewlett-Packard;MODEL:HP LaserJet Professional P1102"
*cupsVersion: 1.6
*cupsModelNumber: 256
*cupsManualCopies: False
*cupsCommands: "ReportLevels"
*cupsFilter: "application/vnd.cups-command 100 /Library/Printers/hp/laserjet/p1100_1560_1600series/hpRaster.bundle/Contents/MacOS/usbink"
*cupsFilter: "application/vnd.cups-raster  100 /Library/Printers/hp/laserjet/p1100_1560_1600series/hpRaster.bundle/Contents/MacOS/rastertozjs"
```

The same PPD is also shipped by Apple at
`/Library/Printers/PPDs/Contents/Resources/hp1100.ppd.gz` (identical `cupsFilter`
targets, `FileVersion 6.9`), so the installed queue and Apple's template agree on
the driver path.

The PPD carries six `*1284DeviceID` lines, one per model in the series it covers
(P1102, P1104, P1106, P1107, P1108, P1109) and a matching `*Product` line for each. One
PPD therefore serves the whole family, and the model is selected by matching the attached
device's own 1284 identity against those lines rather than by a different file. Only the
P1102 line matters here, and it is the one the queue's device URI agrees with.

## The actual pipeline (CONFIRMED)

```text
Application
  |
  v
macOS print system  (Print panel / CG / PDF)
  |
  v
cupsd  -> /usr/libexec/cups/daemon/cupsd
  |   content type on entry: application/pdf
  v
cgpdftoraster            Apple, universal (x86_64 + arm64e + arm64e.x1), signed "macOS Software Signing"
  |   -> application/vnd.cups-raster   (RaS3, uncompressed 8-bit gray in our captures)
  v
rastertozjs              *** HP, x86_64 ONLY ***   com.hp.HPM1100_1560_1600Raster
  |   -> Zenographics ZjStream, framed in PJL
  v
usb backend              Apple, universal (x86_64 + arm64e + arm64e.x1), /usr/libexec/cups/backend/usb
  |   -> USB bulk transfers, USB Printer class
  v
HP LaserJet Professional P1102
```

Secondary (status/utility) path, used for `ReportLevels`, `ReportStatus`, `Clean`:

```text
cupsd -> application/vnd.cups-command -> usbink (HP, x86_64 ONLY)
            uses IOKit directly (IOUSBDevice / IOUSBInterface, IOUSBInterfaceStruct182),
            matches the device by USB Product Name + serial number, and sends
            "GET /DevMgmt/ConsumableConfigDyn.xml HTTP/1.1" over that USB interface -
            a hand-rolled HTTP client, with no networking code in the binary at all.
            Reports the result as ATTR: marker-... plus <dd:ConsumablePercentageLevel...>
```

`rastertozjs` does not open a USB device itself. It writes the encoded ZjStream to
stdout and CUPS hands that to the backend. It *does* read the CUPS backchannel
(`[…] creating read thread using cups backchannel`) when status reporting is active.

The two paths therefore do not share a mechanism: printing goes through CUPS's backends,
and supplies reporting is a USB-tunnelled HTTP request from the filter itself. That is why
`docs/differences.md` treats the command path as a deliberate gap rather than something to
bolt on cheaply.

## Component inventory (CONFIRMED)

| path | arch | binary type | role | invoked by |
|---|---|---|---|---|
| `/usr/libexec/cups/filter/cgpdftoraster` | universal x86_64 + arm64e + arm64e.x1 | Mach-O executable, Apple-signed | PDF -> CUPS raster | cupsd |
| `/usr/libexec/cups/backend/usb` | universal x86_64 + arm64e + arm64e.x1 | Mach-O executable, Apple-signed | USB transport | cupsd |
| `/Library/Printers/hp/laserjet/p1100_1560_1600series/hpRaster.bundle/Contents/MacOS/rastertozjs` | **x86_64** | Mach-O executable in bundle, minos 10.6, SDK 10.7, HP Developer ID (6HB5Y2QTA3) | CUPS raster -> ZjStream encoder | cupsd, via PPD `cupsFilter` |
| `/Library/Printers/hp/laserjet/p1100_1560_1600series/hpRaster.bundle/Contents/MacOS/usbink` | **x86_64** | Mach-O executable, minos 10.6, SDK 10.7, HP Developer ID | command filter / status ink, direct IOKit USB | cupsd, via PPD `cupsFilter` |
| `/Library/Printers/hp/laserjet/p1100_1560_1600series/HP LaserJet Professional Utility.app` | (app bundle) | | GUI utility | user |

### Dependencies of every component (CONFIRMED)

Read with scripts/macho.py libs, which needs no Apple developer tools. Only the libraries
that matter for the print path are listed; every one of these also links libSystem and
CoreFoundation.

| component | libraries |
|---|---|
| cgpdftoraster | libcups.2, libcupsimage.2, CoreGraphics, Quartz, ApplicationServices, ColorSync, CoreText, Accelerate, libc++.1, libobjc |
| cgpdftopdf | libcups.2, CoreGraphics, AppKit, ApplicationServices, CoreText, libobjc |
| cgpdftops | libcups.2, ApplicationServices, QuartzFilters, ColorSync, CoreGraphics, CoreText, libobjc |
| backend/usb | libcups.2, IOKit, SystemConfiguration, Security, libresolv.9, libiconv.2, libz.1 |
| HP rastertozjs | **libcups.2, libcupsimage.2, libstdc++.6**, Foundation, Accelerate, libobjc |
| HP usbink | libcups.2, IOKit, AppKit, Foundation, libobjc |
| our p1102-filter | **only `/usr/lib/libSystem.B.dylib`** |

Three things this table shows that are worth stating plainly.

**Every component links libcups except ours.** That is deliberate, but it is a choice about
independence rather than a necessity - see docs/architecture.md, which records the earlier
and wrong claim that libcups was missing.

**Only the HP filters need libstdc++.6.dylib**, which is the one library in this list that is
not on disk. Combined with their x86_64-only architecture, they cannot run here.

**Ours links one library.** Read with the same tool as the rest of this table, both of our
binaries load only `/usr/lib/libSystem.B.dylib` - no libcups, no frameworks, no private
frameworks, no rpaths. That is the evidence for the claim that the replacement minimises its
reliance on the platform: the entire dependency surface is the C library.

### Census of the whole installation (CONFIRMED)

Judging two filters would understate the problem, so every Mach-O binary in the HP
installation was classified, plus the Image Capture devices and the Samsung installation for
comparison. Done with scripts/macho.py, so no Apple tooling was needed: 13,561 files walked,
337 Mach-O binaries found.

| architecture | count |
|---|---|
| x86_64, thin | 245 |
| universal i386 + x86_64 | 86 |
| universal ppc + i386 + x86_64 | 6 |
| **arm64-capable** | **0** |

**Not one binary in the HP installation contains an ARM64 slice.** So the compatibility
warning is not about a stray component: the whole stack HP shipped predates Apple Silicon,
and with Rosetta absent nothing in it can run here.

The sweep also covers the directories Phase 1 asked for specifically:

- **/Library/Image Capture/Devices/** holds three HP scanner components - HP M1130_M1210
  Scanner.app, HP Scanner 3.app and HPScanner.app. They are scanner-side and serve other
  devices; the P1102 is print-only, so they are outside this project's scope.
- **/Library/LaunchAgents/ and /Library/LaunchDaemons/ contain no HP entries**, so there is
  no HP background service to disable and nothing for an uninstaller to remove there.
- **/Library/Application Support/ has no HP entry.**
- **There is no HP package receipt.** Neither pkgutil --pkgs nor /var/db/receipts lists one,
  so an uninstaller cannot rely on a receipt to find these files - which is why our own
  install does not try to.

## What actually triggers the "Intel-only" warning (CONFIRMED)

rastertozjs, usbink, and (for the P1005 PPD) rasterToHPZJS / commandToHPZJS are **thin
x86_64 Mach-O executables**. They are the components that break printing; everything on
Apple's side of the chain is already universal, which is why only the filter stage needs
replacing.


## The stack is already non-functional on this Mac (CONFIRMED)

This is more severe than "will stop working in a future release": it cannot run now.

1. **Rosetta 2 is not installed.**
   `arch -x86_64 /usr/bin/true` -> `Bad CPU type in executable`; `sysctl.proc_translated` = 0;
   the only Rosetta component present is `/Library/Apple/usr/libexec/oah/RosettaLinux`.
   There is no x86_64 dyld shared cache — only `dyld_shared_cache_arm64e*`.
2. **`libstdc++.6.dylib` is not present on disk**, and `rastertozjs` needs it. The other two
   HP-era dependencies are **not** missing, which an earlier version of this document
   claimed wrongly. Apple's own `cgpdftoraster`, `cgpdftopdf`, `cgpdftops` and the `usb`
   backend all link `/usr/lib/libcups.2.dylib` and run on this machine, so dyld resolves it
   out of the arm64e shared cache rather than from a file. Checking `/usr/lib` for a dylib is
   not a valid test on modern macOS.

   ```text
   /usr/lib/libcups.2.dylib       -> resolvable (Apple's filters link it and run)
   /usr/lib/libcupsimage.2.dylib  -> resolvable (cgpdftoraster links it and runs)
   /usr/lib/libstdc++.6.dylib     -> not under /usr/lib
   ```

   Executing the filter directly yields `bad CPU type in executable`, which is the decisive
   failure: there is no x86_64 runtime for it to use.

Consequences for the plan:

- A PPD queue that points at `rastertozjs` will fail at the filter stage.
- A golden master cannot be captured *from HP's filter* on this machine, because it cannot
  be executed here. That is not the same as having no reference: the open driver that
  supports this printer **can** be built and run here, and `scripts/reference-compare.py`
  uses it to hold the whole job to byte identity. What HP's binary could not be run for, it
  was still read for: its disassembly is where the BIH defaults and the PJL literal list in
  this documentation come from.

## P1005 vs P1102 (CONFIRMED they are different driver bundles)

Both are ZjStream-generation HP host-based LaserJets, but macOS ships them as two
separate, independently-versioned bundles:

| | P1102 (our target) | P1005 |
|---|---|---|
| PPD | `*PCFileName: hp1100.ppd`, FileVersion 6.9, Apple PPD `hp1100.ppd.gz` | `HP LaserJet P1005.gz`, `NickName "… 1.6.1"` |
| filters | `p1100_1560_1600series/hpRaster.bundle/Contents/MacOS/{rastertozjs,usbink}` | `hplaserjetzjs.bundle/Contents/MacOS/{rasterToHPZJS,commandToHPZJS}` |
| bundle id / version | `com.hp.HPM1100_1560_1600Raster` 6.0 | `com.hp.ipg.mac.driver.print` 1.6.1 |
| device firmware blob in bundle | **none found** | `HP1005.acl`, `HP1006.acl`, `HP1505.acl`, `hp1020.acl` |
| commands | `ReportLevels` | `ReportLevels ReportStatus Clean` |

So: the P1005 needs its own investigation; nothing discovered here should be assumed
to transfer to it beyond "same protocol family".
