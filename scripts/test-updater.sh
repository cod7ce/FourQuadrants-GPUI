#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/target/updater-tests"
mkdir -p "$OUT"
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror \
  "$ROOT/native/UpdaterTests.m" -framework AppKit -framework Foundation \
  -o "$OUT/updater-tests"
# Every mode has its own process and no mode loads Sparkle. The AppKit mode
# starts a windowless test app and terminates only that test process.
for mode in synthetic disabled unavailable disable-queued appkit; do
  "$OUT/updater-tests" "$mode"
done
