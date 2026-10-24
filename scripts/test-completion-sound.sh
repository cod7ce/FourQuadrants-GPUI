#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/target/completion-sound-tests"
mkdir -p "$OUT"
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror \
  "$ROOT/native/CompletionSoundTests.m" \
  -framework AVFAudio -framework AudioToolbox -framework Foundation \
  -o "$OUT/completion-sound-tests"
"$OUT/completion-sound-tests" "$ROOT/assets/audio/task-complete.aiff"
