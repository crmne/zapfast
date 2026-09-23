#!/bin/bash
# Build ZapFast.app from a GUI binary on macOS.
#
#   packaging/macos/bundle.sh <binary> <output.app> <version>
#
# Set CODESIGN_IDENTITY to use a Developer ID. Otherwise, use the ad-hoc
# signature required on arm64.
#
# Generate the .icns from the committed 1024px PNG with macOS iconutil.
# Info.plist is next to this script.
set -euo pipefail

binary="$1"
app="$2"
version="$3"
numeric_version="${version%%-*}"
here="$(cd "$(dirname "$0")" && pwd)"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$binary" "$app/Contents/MacOS/zapfast"
chmod 755 "$app/Contents/MacOS/zapfast"
sed "s/__VERSION__/$numeric_version/g" "$here/Info.plist" > "$app/Contents/Info.plist"

iconset="$(mktemp -d)/zapfast.iconset"
mkdir -p "$iconset"
# iconutil reads these base sizes and optional @2x versions. It ignores 64x64.
for size in 16 32 128 256 512; do
    sips -z $size $size "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/zapfast.icns"

# Attach the microphone entitlement before native-packages preserves it when
# replacing the ad-hoc signature with a hardened Developer ID signature.
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --options runtime \
        --entitlements "$here/entitlements.plist" --sign "$CODESIGN_IDENTITY" "$app"
else
    codesign --force --entitlements "$here/entitlements.plist" --sign - "$app"
fi
codesign --verify --strict "$app"

echo "$app"
