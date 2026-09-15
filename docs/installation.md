# Installing and removing

## What gets installed, and where

Two files, both under a directory we own, plus one new CUPS queue:

    /Library/Printers/p1102/p1102-filter    our ARM64 CUPS filter
    /Library/Printers/p1102/p1102.ppd       generated PPD pointing at that filter
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
| *NickName gains a " (native ARM64)" suffix, and the localised NickName translation keys follow it | so the two queues are distinguishable in the UI, without breaking localisation |

Everything else is byte-for-byte the same file. You can see the exact diff before
installing anything - the script only reads, and writes a new file:

    PYTHON=/opt/homebrew/bin/python3.13
    $PYTHON scripts/make-ppd.py \
        /etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd \
        /Library/Printers/p1102/p1102-filter /tmp/p1102.ppd
    diff /etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd /tmp/p1102.ppd

Apple's /usr/bin/python3 is a shim that refuses to run until the Xcode licence is
accepted, hence the PYTHON= above. install.sh warns about that rather than failing
obscurely.

## Installing

    ./scripts/build-native.sh      # a plain 'cargo build --release' also works if the
                                   # Xcode licence is accepted; see docs/testing.md
    sudo ./scripts/install.sh

Review the plan first, which needs no privileges and prints every command it would run:

    ./scripts/install.sh --dry-run

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

    # print something and watch the job
    lp -d HP_LaserJet_P1102_native /path/to/file.pdf
    lpstat -o HP_LaserJet_P1102_native

    # CUPS writes filter stderr to the job log
    log show --predicate 'process == "cupsd"' --last 5m

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
