#!/bin/sh
# Regression checks for installer guardrails. No CUPS mutation or system writes.
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d -t p1100-script-guards)
trap 'rm -rf "$TMP"' EXIT INT TERM
QUEUE=HP_LaserJet_P1100_guard_$$
mkdir "$TMP/bin"
printf '%s\n' '#!/bin/sh' 'exit 1' >"$TMP/bin/lpstat"
printf '%s\n' '#!/bin/sh' 'echo lpadmin-called >&2' 'exit 99' >"$TMP/bin/lpadmin"
chmod +x "$TMP/bin/lpstat" "$TMP/bin/lpadmin"
export PATH="$TMP/bin:$PATH"

mkdir "$TMP/unowned"
echo keep >"$TMP/unowned/user-file"
if "$REPO/scripts/uninstall.sh" --dest "$TMP/unowned" >/dev/null 2>&1; then
    echo 'uninstall accepted an unowned directory' >&2; exit 1
fi
[ -f "$TMP/unowned/user-file" ]

mkdir "$TMP/unowned-dir"
mkdir "$TMP/unowned-dir/nested"
if "$REPO/scripts/uninstall.sh" --dest "$TMP/unowned-dir" >/dev/null 2>&1; then
    echo 'uninstall accepted a directory containing an unowned subdirectory' >&2; exit 1
fi

if "$REPO/scripts/uninstall.sh" --queue HP_LaserJet_Professional_P1102 --dest "$TMP/missing" >/dev/null 2>&1; then
    echo 'uninstall accepted the source queue' >&2; exit 1
fi

if "$REPO/scripts/uninstall.sh" --queue "$QUEUE" --dry-run --dest /private/etc/cups >/dev/null 2>&1; then
    echo 'uninstall accepted canonical protected path' >&2; exit 1
fi

mkdir "$TMP/owned"
echo marker >"$TMP/owned/.p1100-installed"
echo keep >"$TMP/owned/user-file"
"$REPO/scripts/uninstall.sh" --dry-run --dest "$TMP/owned" >/dev/null
"$REPO/scripts/uninstall.sh" --dest "$TMP/owned" >/dev/null
[ -f "$TMP/owned/user-file" ]

if "$REPO/scripts/uninstall.sh" --dry-run --dest "$TMP/owned/.." >/dev/null 2>&1; then
    echo 'uninstall accepted a path containing ..' >&2; exit 1
fi

mkdir "$TMP/install-nonempty"
mkdir "$TMP/install-nonempty/nested"
if "$REPO/scripts/install.sh" --dry-run --no-queue --dest "$TMP/install-nonempty" >/dev/null 2>&1; then
    echo 'install accepted a nonempty unowned directory' >&2; exit 1
fi
ln -s "$TMP/unowned" "$TMP/install-symlink"
if "$REPO/scripts/install.sh" --dry-run --no-queue --dest "$TMP/install-symlink" >/dev/null 2>&1; then
    echo 'install accepted a symlink destination' >&2; exit 1
fi

if ! "$REPO/scripts/rehearse-install.sh" >/dev/null 2>&1; then
    echo 'offline install rehearsal failed' >&2; exit 1
fi

echo 'script guard checks passed'
