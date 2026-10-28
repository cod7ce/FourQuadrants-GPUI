#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/assets/app-icon.png"
mkdir -p "$ROOT/target"
WORK="$(mktemp -d "$ROOT/target/app-icons.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"

# Keep Finder, the notification helper and the embedded runtime artwork in sync.
for SIZE in 16 32 128 256 512; do
  sips -z "$SIZE" "$SIZE" "$SOURCE" --out "$ICONSET/icon_${SIZE}x${SIZE}.png" >/dev/null
  RETINA=$((SIZE * 2))
  sips -z "$RETINA" "$RETINA" "$SOURCE" --out "$ICONSET/icon_${SIZE}x${SIZE}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$WORK/AppIcon.icns"
mv "$WORK/AppIcon.icns" "$ROOT/assets/AppIcon.icns"
printf '%s\n' "Built $ROOT/assets/AppIcon.icns from $SOURCE"
