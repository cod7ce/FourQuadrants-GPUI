#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT/target/material"
xcrun clang -fobjc-arc -DFQ_MATERIAL_SELF_TEST "$ROOT/native/WindowMaterial.m" \
  -framework AppKit -o "$ROOT/target/material/self-test"
"$ROOT/target/material/self-test"
