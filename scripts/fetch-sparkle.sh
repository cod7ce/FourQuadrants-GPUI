#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="2.10.0"
SHA256="c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c"
URL="https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-$VERSION.tar.xz"
CACHE="$ROOT/target/sparkle"
PATHFILE="$CACHE/sdk-path"
mkdir -p "$CACHE"

# A successful fetch publishes its path only after verification and extraction.
# Each attempt gets separate, initially empty download and extraction directories.
if [[ -f "$PATHFILE" ]]; then
  SDK="$(<"$PATHFILE")"
  if [[ "$SDK" == "$CACHE"/sdk.* && -f "$SDK/.verified-sha256" ]] &&
    [[ "$(<"$SDK/.verified-sha256")" == "$SHA256" ]] &&
    [[ -d "$SDK/Sparkle.framework" && -x "$SDK/bin/sign_update" && -x "$SDK/bin/generate_keys" ]]; then
    printf '%s\n' "$SDK"
    exit 0
  fi
fi

DOWNLOAD="$(mktemp -d "$CACHE/download.XXXXXX")"
SDK="$(mktemp -d "$CACHE/sdk.XXXXXX")"
PENDING_PATHFILE=""
COMPLETE=false
cleanup() {
  rm -rf "$DOWNLOAD"
  if [[ "$COMPLETE" != true ]]; then
    rm -rf "$SDK"
  fi
  if [[ -n "$PENDING_PATHFILE" ]]; then
    rm -f "$PENDING_PATHFILE"
  fi
}
trap cleanup EXIT
ARCHIVE="$DOWNLOAD/Sparkle-$VERSION.tar.xz"
curl --fail --location --silent --show-error --retry 3 \
  --proto '=https' --proto-redir '=https' --output "$ARCHIVE" "$URL"
ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE")"
ACTUAL_SHA256="${ACTUAL_SHA256%% *}"
if [[ "$ACTUAL_SHA256" != "$SHA256" ]]; then
  printf '%s\n' "Sparkle $VERSION archive SHA256 mismatch; refusing to extract." >&2
  exit 1
fi

# Python is isolated from the downloaded files and never runs from their directory.
python3 -I - "$ARCHIVE" <<'PY'
import pathlib
import sys
import tarfile

with tarfile.open(sys.argv[1], "r:xz") as archive:
    for member in archive:
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"Unsafe archive path: {member.name!r}")
        if member.isdev() or member.isfifo():
            raise SystemExit(f"Unsupported archive entry: {member.name!r}")
        if member.issym() or member.islnk():
            target = pathlib.PurePosixPath(member.linkname)
            if target.is_absolute():
                raise SystemExit(f"Unsafe archive link: {member.name!r}")
            parts = list(path.parent.parts) if member.issym() else []
            for part in target.parts:
                if part == "..":
                    if not parts:
                        raise SystemExit(f"Archive link escapes SDK: {member.name!r}")
                    parts.pop()
                elif part != ".":
                    parts.append(part)
PY
/usr/bin/tar -xf "$ARCHIVE" -C "$SDK"
if [[ ! -d "$SDK/Sparkle.framework" || ! -x "$SDK/bin/sign_update" || ! -x "$SDK/bin/generate_keys" ]]; then
  printf '%s\n' 'Verified Sparkle archive is missing its framework or signing tools.' >&2
  exit 1
fi
printf '%s\n' "$SHA256" > "$SDK/.verified-sha256"
PENDING_PATHFILE="$(mktemp "$CACHE/sdk-path.XXXXXX")"
printf '%s\n' "$SDK" > "$PENDING_PATHFILE"
mv -f "$PENDING_PATHFILE" "$PATHFILE"
COMPLETE=true
printf '%s\n' "$SDK"
