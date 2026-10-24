#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/target/native"
BUNDLE_ID="com.fourquadrants.gpui.native-helper"
if [[ "$#" -eq 1 && "$1" == --notification-test ]]; then
  # A fresh identity and output directory avoid the installed app's permissions,
  # pending requests and helper executable. Never reuse an old test identity.
  BUNDLE_ID="$BUNDLE_ID.test.$(uuidgen)"
  OUT="$ROOT/target/notification-tests/$BUNDLE_ID"
elif [[ "$#" -ne 0 ]]; then
  printf '%s\n' 'Usage: bash scripts/build-native.sh [--notification-test]' >&2
  exit 1
fi
APP="$OUT/FourQuadrantsHelper.app"
ARCH="$(uname -m)"

mkdir -p "$OUT" "$APP/Contents/MacOS" "$APP/Contents/Resources" "$OUT/module-cache"
xcrun swiftc -parse-as-library -swift-version 6 -O \
  -target "$ARCH-apple-macosx14.0" \
  -module-name FourQuadrantsNative \
  -module-cache-path "$OUT/module-cache" \
  -framework Foundation -framework AppKit -framework SwiftData \
  -framework UserNotifications -framework CryptoKit -framework CoreImage -framework ImageIO -lsqlite3 \
  "$ROOT/native/LegacyModels.swift" \
  "$ROOT/native/StoreExport.swift" \
  "$ROOT/native/RichEditor.swift" \
  "$ROOT/native/NoteMarkdown.swift" \
  "$ROOT/native/Notifications.swift" \
  "$ROOT/native/InputParser.swift" \
  "$ROOT/native/BackgroundImage.swift" \
  "$ROOT/native/SelfTest.swift" \
  "$ROOT/native/HelperMain.swift" \
  -o "$OUT/four-quadrants-helper"

cp "$ROOT/native/Info.plist" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $BUNDLE_ID" "$APP/Contents/Info.plist"
if [[ "$#" -eq 1 ]]; then
  /usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName Four Quadrants Notification Test' "$APP/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c 'Set :CFBundleName Four Quadrants Notification Test' "$APP/Contents/Info.plist"
fi
cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp "$OUT/four-quadrants-helper" "$APP/Contents/MacOS/four-quadrants-helper"
codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"

# Register the bundle so notifications have a stable, discoverable app identity.
# --notification-test registers only its fresh, independent identity.
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "$LSREGISTER" ]]; then
  "$LSREGISTER" -f "$APP" || true
fi
printf '%s\n' "Built $OUT/four-quadrants-helper" "Notification bundle: $APP"
