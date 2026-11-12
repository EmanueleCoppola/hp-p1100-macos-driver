# Read-only USB settings inquiry

Standalone macOS command-line helper. No UI, Swift package, Cargo manifest,
installer, existing transport, or settings writer changes are required.

Build and run the hardware-free tests from the repository root:

```sh
sh native/inquiry/build.sh
```

The script builds `target/inquiry/p1100-inquire` for the host architecture and
runs two test executables with AddressSanitizer and UndefinedBehaviorSanitizer.
It does not query hardware. Requires the macOS SDK, C compiler and system CUPS.

## Integration interface

Invoke on a background worker, supplying the associated CUPS queue name:

```sh
target/inquiry/p1100-inquire --queue HP_LaserJet_P1100_native
```

Success is exit 0 and one JSON object on stdout:

```json
{"serial":"000000000Q80E182PR1a","QUIETMODE":"OFF","POWERSAVETIME":5,"INACTIVEOFFTIME":240}
```

Timers are integers in minutes; zero is returned as zero. Quiet mode is `ON` or
`OFF`. Values are the current `INQUIRE` readback, not factory/persistent defaults.
Only emit results after all three responses and successful interface close.
On failure stdout is empty; stderr describes the error and exit status is:

| Exit | Meaning |
| --- | --- |
| 2 | Invalid CLI arguments |
| 3 | No matching attached device / disconnected |
| 4 | Busy interface, non-idle queue, or any outstanding local CUPS job |
| 5 | USB write/read timeout |
| 6 | Unsupported inquiry (`?` or `NA` reply) |
| 7 | CUPS, USB, endpoint, response, permission, or close error |
| 8 | Missing, invalid, changed, or ambiguous queue/device identity |

The Swift app bundles this helper, decodes JSON only on exit 0, and shows unknown
values on failure. A manual refresh retries busy/timeout errors. It does not
substitute a CUPS raw job, cached defaults, or a settings write on failure, and
does not poll or retry automatically.

## Device and transport policy

- Connect explicitly to the local CUPS Unix socket, ignoring `CUPS_SERVER` and
  remote client defaults. Read the named queue's configured `device-uri`.
- Require an HP P110-family USB product and a nonempty, strictly decoded serial.
  Reject duplicate serials, unknown URI routing options, control bytes and
  truncated identities. There is no arbitrary URI/serial override.
- Enumerate all interfaces before selecting exactly one match for product,
  serial, HP vendor `03f0`, class/subclass/protocol `7/1/2`, alternate setting 0.
  Never select the vendor EWS interface `ff/2/16`.
- Require idle queue state and no non-completed jobs across **all** local queues
  and users, including aliases. CUPS failures fail closed. Recheck the queue
  identity and jobs after exclusive open and before each inquiry.
- Use plain `USBInterfaceOpen`, which fails on exclusive ownership by another
  task. Validate class/vendor again and require exactly one bulk IN and OUT.
  No seize, detach, device open, configuration/alternate changes, reset,
  stall clearing, pipe abort, pre-read/drain, or fallback transport.
- Send only these fixed requests, one at a time, with the exact framing already
  used by `native/backend/SettingsBackend.swift`:

  ```text
  <ESC>%-12345X@PJL INQUIRE QUIETMODE<LF><ESC>%-12345X
  <ESC>%-12345X@PJL INQUIRE POWERSAVETIME<LF><ESC>%-12345X
  <ESC>%-12345X@PJL INQUIRE INACTIVEOFFTIME<LF><ESC>%-12345X
  ```

  There is no caller-supplied PJL, `SET`, `DEFAULT`, `JOB`, `ENTER LANGUAGE`,
  page command, outbound form feed, or print submission. Read-only here means
  no settings mutation; USB OUT transfers necessarily carry the inquiries.
- Each write has a 1-second timeout. Each complete response has an absolute
  1-second budget across fragments and a 4095-byte maximum. Require the exact
  requested response header, one valid value and the PJL response form feed.
  Reject unsolicited/stale status, extra frames and malformed replies.
  Never retry a failed/possibly partial write or send recovery bytes.
- Close only an interface successfully opened by this process and release all
  handles, including every failure path. Retain exclusive access for the full
  transaction.

The implementation follows the existing Rust usbink legacy transport, using
the SDK's typed C IOUSBLib/CFUUID declarations rather than another manually
maintained Rust vtable. This adds no third-party dependencies. Apple's
[IOUSBLib header](https://github.com/apple-oss-distributions/IOUSBFamily/blob/main/IOUSBFamily/Headers/IOUSBLib.h)
documents exclusive-open and timeout behavior.

## Verification on 2026-11-12

`sh native/inquiry/build.sh`: **exit 0**. Both test executables passed with
`-Wall -Wextra -Werror` and AddressSanitizer/UndefinedBehaviorSanitizer enabled;
no sanitizer findings.

- Protocol/policy tests: strict serial/URI validation; wrong serial/product/vendor
  and EWS rejection; valid/unsupported/malformed/truncated/NUL/oversized replies;
  fragmentation at every chunk size 1–39; guard failure before and after open,
  including a queue change between inquiries; busy/no-device/timeout/I/O errors
  at open/write/read; absolute deadlines including late completed responses;
  zero/invalid read counts; close failure and ownership; exact inquiry-only bytes
  and no retries. No-device or busy-open failures make zero writes.
- Native SDK tests use fake vtables: exclusive-open refusal, absent device,
  wrong class/vendor, failed endpoint properties, missing/duplicate/non-bulk
  endpoints, correct close ownership, status mapping and JSON escaping. Their
  printed USB error codes are intentional injected failures, not hardware I/O.
- `xcrun clang --analyze -std=c11 -Wall -Wextra -Werror native/inquiry/main.c
  -o /tmp/p1100-inquiry-analysis.plist`: **exit 0**, no findings.
- `sh -n native/inquiry/build.sh`: **exit 0**.
- Built executable: **Mach-O 64-bit arm64**.
- Sandbox-only execution without CUPS socket access: **exit 7**, no stdout,
  fails before USB selection/open.
- One authorized read-only hardware execution against
  `HP_LaserJet_P1100_native`: **exit 0**, JSON exactly as above. Selected serial
  `000000000Q80E182PR1a`; Quiet Mode OFF, sleep 5 minutes, auto-off 240 minutes.
  `lpstat -W not-completed -o` before and after: **exit 0**, empty output.
  No page or settings command was sent.

## Remaining limits

- Busy/disconnect/timeout behavior is tested by injection, not by interrupting a
  real job or unplugging hardware. Successful readback is verified on this P1102
  only. Other firmware may use unsupported response formats and will fail closed.
- A new CUPS job can arrive after the last guard. Exclusive ownership prevents
  interleaved USB transfers; a newly scheduled backend may briefly wait/retry.
  The helper never pauses, cancels, holds, or changes a queue. Existing jobs
  that CUPS has already marked completed are not visible to the scheduler guard;
  it cannot prove the printer has mechanically finished every earlier sheet.
- PJL replies contain no transaction identifier. Strict framing rejects unrelated
  replies but cannot distinguish an old, otherwise identical inquiry response.
- CUPS connect timeout is 1.5 seconds and socket inactivity timeout is 1.5 seconds;
  these are not a hard wall-clock limit for the entire invocation. USB transfer
  timeouts are bounded separately. Run off the UI thread.
- Only printable ASCII identities are supported. Unknown USB URI query options
  are rejected intentionally. Any unrelated local queued job also blocks.
- Querying may wake a sleeping printer. No wake/reset command is sent explicitly.
- The helper performs no periodic monitoring, setting mutation, or information-page printing.
