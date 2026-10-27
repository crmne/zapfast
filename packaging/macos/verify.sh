#!/bin/bash
# Verify the app that users receive inside a signed, notarized release DMG.
set -euo pipefail

dmg="$1"
temporary="$(mktemp -d)"
mount="$temporary/mount"
mkdir "$mount"
cleanup() {
    hdiutil detach "$mount" >/dev/null 2>&1 || true
    rm -f "$temporary/entitlements.plist"
    rmdir "$mount" "$temporary" 2>/dev/null || true
}
trap cleanup EXIT

xcrun stapler validate "$dmg"
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount" >/dev/null
app="$mount/ZapFast.app"
codesign --verify --strict --deep "$app"
spctl --assess --type execute --verbose=2 "$app"
lipo "$app/Contents/MacOS/zapfast" -verify_arch x86_64 arm64
codesign --display --entitlements - --xml "$app" > "$temporary/entitlements.plist"

python3 - "$app/Contents/Info.plist" "$temporary/entitlements.plist" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as source:
    info = plistlib.load(source)
with open(sys.argv[2], "rb") as source:
    entitlements = plistlib.load(source)
if not info.get("NSMicrophoneUsageDescription", "").strip():
    sys.exit("The release app is missing its microphone permission description")
if entitlements.get("com.apple.security.device.audio-input") is not True:
    sys.exit("The signed release app is missing its audio-input entitlement")
if not info.get("NSCameraUsageDescription", "").strip():
    sys.exit("The release app is missing its camera permission description")
if entitlements.get("com.apple.security.device.camera") is not True:
    sys.exit("The signed release app is missing its camera entitlement")
print("Verified microphone and camera permission metadata in the signed universal app")
PY
