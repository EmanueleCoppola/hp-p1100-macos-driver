# Installing and removing

## What gets installed, and where

Four files, all under a directory we own, plus one new CUPS queue:

    /Library/Printers/p1102/p1102-filter    our ARM64 CUPS filter
    /Library/Printers/p1102/p1102.ppd       generated PPD pointing at that filter
    /Library/Printers/p1102/p1102-400.tones, p1102-600.tones
                                            the two tone tables the p1102Grey option needs,
                                            read out of the installed HP driver
    /Library/Printers/p1102/p1102.icns      the picture the printer row shows
    CUPS queue: HP_LaserJet_P1102_native    new queue, inherits the existing device URI

Nothing outside /Library/Printers/p1102 is written. Specifically:

- /Library/Printers/hp/** - HP's filters, frameworks, PPDs and utility - is only ever
  *read*. The source PPD is opened read-only to generate ours.
- /Library/Printers/PPDs/Contents/Resources/** - Apple's PPD set - is only read.
- /usr/libexec/cups/** and /etc/cups/** are not written by these scripts. lpadmin
  maintains /etc/cups/printers.conf and /etc/cups/ppd/<queue>.ppd itself, as it does for
  any queue.
- SIP is not disabled, and no kernel extension, DriverKit driver, system extension or
  launch daemon is installed. This is a userspace filter plus a queue definition.

## Why a separate queue instead of editing the HP one

The existing HP_LaserJet_Professional_P1102 queue still points at HP's rastertozjs, which
cannot run on this machine at all (x86_64, with no Rosetta and no x86_64 runtime). Leaving it
untouched means the change is reversible, both queues can coexist, and if HP's driver is
ever fixed there is nothing to undo. When the new queue is proven, delete the old one by
hand if you want to.

## How CUPS finds the filter

A PPD's *cupsFilter line names the program by absolute path:

    *cupsFilter: "application/vnd.cups-raster 100 /Library/Printers/p1102/p1102-filter"

When a job reaches the raster step, cupsd runs that program as

    p1102-filter job-id user title copies options file

feeding application/vnd.cups-raster on stdin and sending whatever the program writes to
stdout on to the usb backend. There is no registration step and no registry: the path in
the PPD is the whole discovery mechanism. That is also why HP's drivers install into
/Library/Printers/... rather than into Apple's filter directory.

## Where the PPD comes from

HP's PPD already describes this printer correctly - media sizes, resolutions, duplex,
imageable areas, all of it - and there is no reason to re-author that data. So
scripts/make-ppd.py reads the installed PPD and changes only the driver wiring:

| change | why |
|---|---|
| the application/vnd.cups-raster filter line is repointed at our filter | HP's is x86_64 |
| the application/vnd.cups-command filter line is dropped | it points at HP's usbink, which we have no replacement for yet, so CUPS must not try to run it |
| *cupsCommands is dropped | no command filter to serve ReportLevels |
| *APPrinterUtilityPath is dropped | that is HP's utility app |
| *APPrinterIconPath is repointed at our icns | so the row shows a picture we ship, rather than one of HP's read out of HP's directory |
| *NickName gains a " (native ARM64)" suffix, and the localised NickName translation keys follow it | so the two queues are distinguishable in the UI, without breaking localisation |
| *OpenUI *p1102Grey is added, with a label in every language the PPD declares | the two-bit grey screening is ours rather than HP's, and a PPD that declares languages has to translate its own options - CUPS's conformance test fails every one that is missing |

Everything else is byte-for-byte the same file. You can see the exact diff before
installing anything - the script only reads, and writes a new file:

    PYTHON=/opt/homebrew/bin/python3.13
    $PYTHON scripts/make-ppd.py \
        /etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd \
        /Library/Printers/p1102/p1102-filter /tmp/p1102.ppd \
        --icon /Library/Printers/p1102/p1102.icns     # as install.sh calls it
    diff /etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd /tmp/p1102.ppd

Apple's /usr/bin/python3 is a shim that refuses to run until the Xcode licence is
accepted, hence the PYTHON= above. install.sh warns about that rather than failing
obscurely.

Two things about the generated file are worth knowing before trusting it.

**Its first line has to stay `*PPD-Adobe: "4.3"`.** That is why the banner make-ppd.py writes
and the option it adds go *after* that line rather than on top of it. A PPD whose first line is
a comment is one CUPS cannot open, and it fails quietly rather than loudly: cupsfilter falls
back to its built-in conversion rules and produces a raster anyway - 100 dpi, one bit per
pixel, 119,500 bytes for the test page, where HP's own PPD produces the printer's 600 dpi grey
at 31,361,800 bytes. The generated PPD was in exactly that state until it was measured.
`scripts/ppd-compare.py` is the check: it drives cupsfilter with the PPD install.sh would
write, generated from either PPD it can start from, for six option combinations that each move
the raster in a different way, and requires the result to be byte-identical to the source
PPD's for the same document and options.

**CUPS's own conformance test is on this Mac.** `/usr/bin/cupstestppd` - an earlier version of
this document said macOS ships no such thing, which is how the file above stayed malformed
without anyone noticing, since nothing else in the toolchain reads a PPD's structure. install.sh
runs it before the queue is created, strictly. The requirement that every option carry a label
in every language the PPD declares comes from there.

### The printer icon

`*APPrinterIconPath` is the one line a driver can use to put a picture next to its queue. macOS
shows that file wherever it lists the printer - the row in System Settings, the printer list in
the print dialog - and falls back to a generic glyph when the key is absent, unreadable, or
names nothing. That is measured here rather than taken on trust: the row for HP's own queue
matches `/Library/Printers/hp/Icons/hp1100.icns` to a mean absolute luminance error of 8/255
over the icon box, while the stock pictures on this machine land between 44 and 98 away.

CUPS's own notion of a printer icon is a different thing, and worth not confusing with it. The
`printer-icons` attribute answers for every queue on this machine with the same 4609 bytes, byte
for byte: `/icons/<queue>.png` is one built-in picture for all of them, so nothing a driver writes
reaches the CUPS web interface that way. The PPD key is the macOS path, and the only one.

The icon we install is drawn by `scripts/make-icns.py`, which is code rather than a borrowed
picture: it carries this repository's licence, and it is redrawn at each size the icns container
needs instead of being resampled down from one large bitmap. `--from FILE` builds the container
from a PNG of your own instead, and the installer takes `--icon FILE` to install a different one:

    PYTHON=/opt/homebrew/bin/python3.13
    $PYTHON scripts/make-icns.py                    # rewrite assets/p1102.icns
    $PYTHON scripts/make-icns.py --from my-art.png  # from a picture of your own

`install.sh` refuses an icon that is not an icns macOS can read back, by round-tripping the file
through `iconutil` before anything is written. The failure it catches is silent by nature: a
container the print system cannot read is a row showing a generic glyph, not an error message.

The conventional home for these files is `/Library/Printers/Icons/`, where Apple's own
`ipp2ppd` puts a UUID-named icns for every AirPrint queue it creates. Ours goes next to the
filter instead, so that everything this installer adds stays one directory to delete; the key
holds an absolute path and does not care which directory it names.


## Installing

    ./scripts/build-native.sh      # a plain 'cargo build --release' also works if the
                                   # Xcode licence is accepted; see docs/testing.md
    sudo ./scripts/install.sh

Review the plan first, which needs no privileges and prints every command it would run:

    ./scripts/install.sh --dry-run

The dry run also generates the PPD into a temporary file and checks it, so it tells you
something about the artifact rather than only about the commands. Three things are checked,
and the dry run is not a formality: `scripts/verify-all.sh` passes `PYTHON` in for exactly
this reason, because without it install.sh picks Apple's gated `/usr/bin/python3`, prints a
note that PPD generation would fail, and skips these checks while still reporting ok.

- The file begins with `*PPD-Adobe: "4.3"`, which is what makes it a PPD at all.
- It differs from the source in the listed ways and in no others. That is the property this
  document claims about the file, and it is the one that catches a generator that mangled the
  device data rather than the driver wiring.
- `/usr/bin/cupstestppd`, CUPS's own conformance test, reports no failures. It does ship with
  macOS; an earlier version of this document said it did not. In a dry run its two file checks
  are ignored with `-I filters -I profiles`, because nothing has been installed yet. A real
  install runs it strictly, where the filter's and the icon's permissions are part of what is
  checked.

A real install runs all three before creating the queue, so a PPD CUPS could not use never
reaches `lpadmin`.

Options:

    --queue NAME        queue to create              (default HP_LaserJet_P1102_native)
    --source-queue N    queue to inherit the device URI and PPD from
                                                     (default HP_LaserJet_Professional_P1102)
    --device URI        use this device URI instead of reading it from the source queue
    --force             replace an existing queue of the same name

The installer stops if the filter binary is missing, or if file(1) does not report it as
an ARM64 binary. It also refuses outright to install into any Apple or HP directory, as a
guard against a mistyped path.

## Uninstalling

    ./scripts/uninstall.sh --dry-run     # review
    sudo ./scripts/uninstall.sh

That deletes the queue and removes /Library/Printers/p1102. HP's queue, files and receipts
are left alone.

## Verifying an install

    # the installed filter must be native
    file /Library/Printers/p1102/p1102-filter     # expect Mach-O 64-bit executable arm64

    # the queue exists and points at our PPD
    lpstat -v HP_LaserJet_P1102_native
    grep cupsFilter /etc/cups/ppd/HP_LaserJet_P1102_native.ppd

    # and it points the icon at ours, which is installed beside it
    grep APPrinterIconPath /etc/cups/ppd/HP_LaserJet_P1102_native.ppd
    file /Library/Printers/p1102/p1102.icns     # expect "Mac OS X icon"

    # CUPS's own conformance test, against the file as it was installed
    /usr/bin/cupstestppd /etc/cups/ppd/HP_LaserJet_P1102_native.ppd     # expect "PASS"

    # print something and watch the job
    lp -d HP_LaserJet_P1102_native /path/to/file.pdf
    lpstat -o HP_LaserJet_P1102_native

    # CUPS writes filter stderr to the job log
    log show --predicate 'process == "cupsd"' --last 5m

The filter's own messages are prefixed the way CUPS expects - `ERROR:`, `INFO:`, `WARNING:` -
so a failure does not only reach the log: CUPS uses that first word to set the queue's state
message, which `lpstat -p` then shows. A job that stops for no visible reason therefore has
its reason in the queue's own state, not just in a log nobody opened.

The icon is the one thing here that can look wrong without being wrong. macOS may keep the
picture it read for a queue, so a row that still shows the previous icon after an install is a
cache rather than a bad file: restart the daemon and reopen the pane.

    sudo launchctl kickstart -k system/org.cups.cupsd

## Trying it without installing anything

The first hardware test does not need this installer, or an administrator. A complete job can
be produced without a compiler (`scripts/make-test-job.py`) and handed straight to Apple's own
`usb` backend, which is world-executable and takes its device URI from the environment:

```bash
PY=/opt/homebrew/bin/python3.13
$PY scripts/make-test-job.py --pattern square --page a4 -o /tmp/square.zjs --verify
DEV=$(lpstat -v HP_LaserJet_Professional_P1102 | sed -n 's/^device for [^:]*: //p')
DEVICE_URI="$DEV" /usr/libexec/cups/backend/usb 1 "$USER" bring-up 1 '' /tmp/square.zjs
```

That is the same transport the installed queue would use, so a page that prints settles the
protocol, the framing and the USB path in one go - and it leaves nothing behind to undo. What
it does not test is CUPS's chain from the PPD through the filter, which is what the raw queue
in docs/testing.md and then this installer are for.

## Undoing a bad install by hand

If a script fails halfway, this fully reverses it:

    lpadmin -x HP_LaserJet_P1102_native     # ignore an error if it does not exist
    sudo rm -rf /Library/Printers/p1102

## Before you install: test the printer without building anything

The filter compresses the page image with our own JBIG encoder, so an install is only
worth doing once the printer is known to accept our framing. That can be checked without
compiling anything, because a job can be produced by a Python tool and delivered through
Apple's own USB backend on a raw queue - no PPD, no filters, bytes to the printer verbatim:

    PY=/opt/homebrew/bin/python3.13
    $PY scripts/make-test-job.py --pattern square --page a4 -o /tmp/square.zjs --verify
    sudo lpadmin -p P1102_raw -E -v '<device URI from lpstat -v>' -m raw
    lp -d P1102_raw -o raw /tmp/square.zjs
    sudo lpadmin -x P1102_raw          # remove it again

That exercises the printer protocol, the framing and the USB path in isolation. Only once
a page prints is installing the filter worth it. See docs/testing.md for what each step
answers, including how to tell a printer-side rejection apart from an encoder problem.
