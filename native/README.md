# P1100 Utility

A compact, standalone SwiftUI macOS window for P1100 print information pages, Quiet Mode, and EcoSMART settings. The window follows the system Light/Dark appearance automatically.

## Build the app bundle

Requires macOS 13 or later and Xcode command-line tools.

```sh
./native/build-app.sh
```

The script builds the release executable and packages it as:

```text
target/P1100 Utility.app
```

Open it in Finder, or run:

```sh
open "target/P1100 Utility.app"
```

For a command-line debug build, run `cd native && swift build`.

## Device integration status

The utility reads the current EcoSMART timers over USB and submits changes as raw CUPS jobs to the named queue. Readback has been verified on the P1102; the write path has not yet been verified on a physical printer. CUPS accepting a job does not confirm that the printer applied it; if several settings are selected, an error may occur after some have already been queued. If readback fails, the values remain unknown and Apply is disabled. Quiet Mode is disabled: HP's utility explicitly disables it for the P1102 USB product ID.

Information pages use commands recovered from HP's utility; they have not yet been tested on the physical printer. Cleaning asks for confirmation before submitting a job. On the tested macOS installation, the PPD exposes the app under Options & Supplies → Utility → Open Printer Utility.
