#!/bin/sh
# Run every check this repository has, in one command, and print what each one said.
#
# Why: the checks are spread across a build script, two test runners, several verifiers and a dry
# run, so running them by hand means remembering which ones exist. This runs them all, needs no
# privileges, changes nothing outside the repository and one scratch directory, and exits non-zero
# if any of them fails - so it is the one thing to run before trusting the tree, and the one thing
# to paste into a report.
#
# It takes a minute or so: the reference comparison writes and compares twenty-eight CUPS rasters.
#
# Usage:
#   scripts/verify-all.sh            every check
#   scripts/verify-all.sh --quick    everything except the reference comparison and the sweep
#
# What it deliberately does not do: install anything, create a queue, use sudo, or talk to a
# printer. Those need the hardware or an administrator and are described in docs/testing.md.
set -u

REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO"

PY=${PYTHON:-}
if [ -z "$PY" ]; then
    for candidate in /opt/homebrew/bin/python3.13 /opt/homebrew/bin/python3.10 python3; do
        if "$candidate" -c 'pass' >/dev/null 2>&1; then PY=$candidate; break; fi
    done
fi
if [ -z "$PY" ] || ! "$PY" -c 'pass' >/dev/null 2>&1; then
    echo 'error: no working Python; set PYTHON=/path/to/python3' >&2
    echo "       (Apple's /usr/bin/python3 is behind the Xcode licence gate)" >&2
    exit 2
fi

QUICK=0
[ "${1:-}" = "--quick" ] && QUICK=1

TMP=$(mktemp -d -t p1102-verify) || exit 2
trap 'rm -rf "$TMP"' EXIT INT TERM

failed=0
run() {
    label=$1
    shift
    if "$@" >"$TMP/out" 2>"$TMP/err"; then
        printf '  %-40s ok\n' "$label"
    else
        printf '  %-40s FAILED\n' "$label"
        tail -5 "$TMP/err" | sed 's/^/      /'
        failed=$((failed + 1))
    fi
}

echo 'P1102 verification'
echo
echo 'build and tests:'
run 'native build' ./scripts/build-native.sh
run 'tests, native, whole suite' ./scripts/test-native.sh
run 'tests, wasm, the same suite' ./scripts/test-wasm.sh
run 'clippy' cargo clippy --offline --workspace --all-targets --quiet
run 'rustfmt' cargo fmt --check
echo
echo 'what the machine can see:'
run 'native ARM64 check, with HP control' ./scripts/verify-native.sh --include-hp
run 'pre-flight self-test' "$PY" scripts/preflight-usb.py --selftest
run 'HP gray tone tables, read and checked' "$PY" scripts/hp-tone-table.py
run 'tone table check, self-test' "$PY" scripts/hp-tone-table.py --selftest
echo
echo 'correctness against the references:'
if [ "$QUICK" = 1 ]; then
    echo '  (skipped: the comparison and the sweep, as --quick asks)'
else
    run 'whole job vs the reference driver' "$PY" scripts/reference-compare.py
run 'JBIG encoder vs the reference' "$PY" scripts/jbig-sweep.py
run 'two-bit path vs the reference' "$PY" scripts/two-bit-compare.py
fi
echo
echo 'the CUPS boundary, and the shape of the install:'
run 'rehearsal, JBIG path' "$PY" scripts/rehearse-job.py
run 'rehearsal, uncompressed path' "$PY" scripts/rehearse-job.py --raw
run 'rehearsal, with PPD options' "$PY" scripts/rehearse-job.py --options 'Duplex=DuplexNoTumble InputSlot=Manual EconoMode=True pjlDensity=5 pjlJamRecovery=True'
run 'peak memory does not grow with the job' "$PY" scripts/memory-check.py
# PYTHON is passed on: without it install.sh would fall back to Apple's gated /usr/bin/python3,
# skip the PPD it is supposed to generate and check, and this line would report ok for a check
# that never ran.
run 'PPD the installer would write' env PYTHON="$PY" ./scripts/install.sh --dry-run
run 'PPD the installer writes, through CUPS' "$PY" scripts/ppd-compare.py
run 'uninstall, dry run' ./scripts/uninstall.sh --dry-run
echo
echo 'the scripts themselves:'
# Every script has to parse, including the ones nothing above calls.
py_ok=1; "$PY" -m py_compile scripts/*.py >/dev/null 2>&1 || py_ok=0
sh_ok=1; for f in scripts/*.sh; do sh -n "$f" || sh_ok=0; done
node_ok=1; for f in scripts/*.mjs; do node --check "$f" >/dev/null 2>&1 || node_ok=0; done
for pair in "python:$py_ok" "shell:$sh_ok" "node:$node_ok"; do
    case $pair in
        *:1) printf '  %-40s ok\n' "${pair%%:*} scripts parse" ;;
        *)   printf '  %-40s FAILED\n' "${pair%%:*} scripts parse"; failed=$((failed + 1)) ;;
    esac
done
echo
if [ "$failed" = 0 ]; then
    echo 'everything passed.'
    echo 'not covered here, and needing the printer or an administrator: docs/differences.md item'
    echo 'by item, and the two install steps in docs/testing.md.'
    exit 0
fi
echo "$failed check(s) failed"
exit 1
