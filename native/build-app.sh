#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
BUILD_DIR="$ROOT/target/native-build"
APP="$ROOT/target/P1100 Utility.app"
STAGE="$ROOT/target/.P1100 Utility.app.tmp"

swift build \
  --disable-sandbox \
  -debug-info-format none \
  --package-path "$ROOT/native" \
  --configuration release \
  --build-path "$BUILD_DIR"

sh "$ROOT/native/inquiry/build.sh"

rm -rf "$STAGE"
mkdir -p "$STAGE/Contents/MacOS"
cp "$BUILD_DIR/release/P1100Utility" "$STAGE/Contents/MacOS/P1100Utility"
cp "$ROOT/target/inquiry/p1100-inquire" "$STAGE/Contents/MacOS/p1100-inquire"
cat > "$STAGE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>P1100Utility</string>
    <key>CFBundleIdentifier</key><string>org.p1100drivers.utility</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>P1100 Utility</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST
chmod +x "$STAGE/Contents/MacOS/P1100Utility"
chmod +x "$STAGE/Contents/MacOS/p1100-inquire"
rm -rf "$APP"
mv "$STAGE" "$APP"
printf 'Built %s\n' "$APP"
