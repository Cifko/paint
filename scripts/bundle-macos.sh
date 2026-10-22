#!/bin/sh
# Builds Paint.app (with its icon) from a release build. Run on macOS.
# Usage: scripts/bundle-macos.sh [target-triple]   e.g. aarch64-apple-darwin
set -eu
cd "$(dirname "$0")/.."

target=${1:-}
if [ -n "$target" ]; then
    cargo build --release --target "$target"
    bin="target/$target/release/paint"
else
    cargo build --release
    bin="target/release/paint"
fi
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

app=target/Paint.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/paint"

# The icon: an .iconset of the bundled PNGs, compiled with iconutil.
iconset=target/paint.iconset
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256; do
    cp "assets/icons/$size.png" "$iconset/icon_${size}x${size}.png"
done
cp assets/icons/32.png "$iconset/icon_16x16@2x.png"
cp assets/icons/64.png "$iconset/icon_32x32@2x.png"
cp assets/icons/256.png "$iconset/icon_128x128@2x.png"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/paint.icns"
rm -rf "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Paint</string>
    <key>CFBundleDisplayName</key><string>Paint</string>
    <key>CFBundleIdentifier</key><string>com.cifko.paint</string>
    <key>CFBundleExecutable</key><string>paint</string>
    <key>CFBundleIconFile</key><string>paint</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>10.13</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "Built $app"
