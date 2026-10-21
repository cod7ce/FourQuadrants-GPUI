#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-debug}"
if [[ "$#" -gt 1 || ( "$PROFILE" != debug && "$PROFILE" != release ) ]]; then
  printf '%s\n' 'Usage: bash scripts/bundle.sh [debug|release]' >&2
  exit 1
fi

# Cargo is the source of truth. Refuse a bundle whose update versions disagree.
python3 -I - "$ROOT/Cargo.toml" "$ROOT/native/AppInfo.plist" <<'PY'
import plistlib
import re
import sys

manifest = open(sys.argv[1], encoding="utf-8").read()
package = re.search(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)", manifest)
version = re.search(r'^version\s*=\s*"(0\.\d+\.\d+)"\s*$', package[1], re.M) if package else None
if not version:
    raise SystemExit("Cargo.toml must declare a 0.x.x package version.")
with open(sys.argv[2], "rb") as source:
    info = plistlib.load(source)
for key in ("CFBundleShortVersionString", "CFBundleVersion"):
    if info.get(key) != version[1]:
        raise SystemExit(f"{key} must match Cargo.toml version {version[1]}.")
PY

SPARKLE_SDK="$(bash "$ROOT/scripts/fetch-sparkle.sh")"
export SPARKLE_SDK
export MACOSX_DEPLOYMENT_TARGET=14.0
bash "$ROOT/scripts/build-native.sh"
if [[ "$PROFILE" == release ]]; then
  cargo build --locked --release --manifest-path "$ROOT/Cargo.toml"
else
  cargo build --locked --manifest-path "$ROOT/Cargo.toml"
fi

mkdir -p "$ROOT/dist"
STAGING="$(mktemp -d "$ROOT/dist/.bundle.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
APP="$STAGING/FourQuadrants GPUI.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$ROOT/target/$PROFILE/four-quadrants" "$APP/Contents/MacOS/four-quadrants"
cp "$ROOT/target/native/four-quadrants-helper" "$APP/Contents/MacOS/four-quadrants-helper"
ditto "$ROOT/target/native/FourQuadrantsHelper.app" "$APP/Contents/MacOS/FourQuadrantsHelper.app"
# ditto retains the framework's Versions/Current and resource symlinks.
ditto "$SPARKLE_SDK/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"
cp "$ROOT/native/AppInfo.plist" "$APP/Contents/Info.plist"
cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE.txt"
cp "$ROOT/assets/icons/HUGEICONS-LICENSE.txt" "$APP/Contents/Resources/HUGEICONS-LICENSE.txt"
cp "$ROOT/assets/icons/LUCIDE-LICENSE.txt" "$APP/Contents/Resources/LUCIDE-LICENSE.txt"
cp "$SPARKLE_SDK/LICENSE" "$APP/Contents/Resources/SPARKLE-LICENSE.txt"

ARCH="$(xcrun lipo -archs "$APP/Contents/MacOS/four-quadrants")"
if [[ "$ARCH" != arm64 && "$ARCH" != x86_64 ]]; then
  printf '%s\n' "Expected a single-architecture executable; found: $ARCH" >&2
  exit 1
fi
SIGNING_ARGS=(--force --sign "${CODE_SIGN_IDENTITY:--}")
if [[ "${CODE_SIGN_IDENTITY:--}" != - ]]; then
  SIGNING_ARGS+=(--options runtime --timestamp)
fi
sign_code() {
  # Retain Sparkle XPC sandbox entitlements when replacing its upstream signature.
  if codesign --display "$1" >/dev/null 2>&1; then
    codesign "${SIGNING_ARGS[@]}" --preserve-metadata=entitlements "$1"
  else
    codesign "${SIGNING_ARGS[@]}" "$1"
  fi
}

# Thin copied dependencies to the executable's architecture before signing.
# Sign all Mach-O leaves, then nested bundles from deepest to outermost.
while IFS= read -r -d '' FILE; do
  case "$(/usr/bin/file -b "$FILE")" in
    *Mach-O*)
      ARCHES="$(xcrun lipo -archs "$FILE")"
      if [[ " $ARCHES " != *" $ARCH "* ]]; then
        printf '%s\n' "Missing $ARCH architecture: $FILE" >&2
        exit 1
      fi
      if [[ "$ARCHES" != "$ARCH" ]]; then
        xcrun lipo "$FILE" -thin "$ARCH" -output "$FILE.thin"
        chmod "$(stat -f '%Lp' "$FILE")" "$FILE.thin"
        mv -f "$FILE.thin" "$FILE"
      fi
      sign_code "$FILE"
      ;;
  esac
done < <(find "$APP" -type f -print0)
while IFS= read -r -d '' BUNDLE; do
  sign_code "$BUNDLE"
done < <(find "$APP" -depth -type d \( -name '*.app' -o -name '*.xpc' -o -name '*.framework' \) -print0)
codesign --verify --deep --strict "$APP"

DESTINATION="$ROOT/dist/FourQuadrants GPUI.app"
rm -rf "$DESTINATION"
mv "$APP" "$DESTINATION"
printf '%s\n' "Built: $DESTINATION ($ARCH, signing identity: ${CODE_SIGN_IDENTITY:--})"
