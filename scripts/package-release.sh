#!/bin/bash
# Never trace a private key supplied in the environment, even under bash -x.
set +x
set +a
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$#" -lt 1 || "$#" -gt 2 || ( "$#" == 2 && "$2" != --skip-build ) ]]; then
  printf '%s\n' 'Usage: bash scripts/package-release.sh BASE_URL [--skip-build]' >&2
  printf '%s\n' 'BASE_URL is the future HTTPS directory for the archive. This script does not upload.' >&2
  exit 1
fi
BASE_URL="$1"
# Capture and unexport in one compound command, including on macOS Bash 3.2.
{ unset PRIVATE_KEY; PRIVATE_KEY="${SPARKLE_PRIVATE_KEY:-}"; unset SPARKLE_PRIVATE_KEY; }

python3 -I - "$BASE_URL" <<'PY'
import sys
from urllib.parse import urlsplit

value = sys.argv[1]
url = urlsplit(value)
if (url.scheme != "https" or not url.hostname or url.username or url.password
        or url.query or url.fragment or any(c.isspace() or ord(c) < 32 for c in value)):
    raise SystemExit("BASE_URL must be an HTTPS directory URL without credentials, query, or fragment.")
PY

validate_metadata() {
  python3 -I - "$ROOT/Cargo.toml" "$ROOT/native/AppInfo.plist" "$@" <<'PY'
import base64
import plistlib
import re
import sys

manifest = open(sys.argv[1], encoding="utf-8").read()
package = re.search(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)", manifest)
version = re.search(r'^version\s*=\s*"(0\.\d+\.\d+)"\s*$', package[1], re.M) if package else None
if not version:
    raise SystemExit("Cargo.toml must declare a 0.x.x package version.")
source_info = None
for path in sys.argv[2:]:
    with open(path, "rb") as source:
        info = plistlib.load(source)
    for key in ("CFBundleShortVersionString", "CFBundleVersion"):
        if info.get(key) != version[1]:
            raise SystemExit(f"{path}: {key} must match Cargo.toml version {version[1]}.")
    try:
        public_key = base64.b64decode(info.get("SUPublicEDKey", ""), validate=True)
    except (ValueError, TypeError):
        public_key = b""
    if len(public_key) != 32:
        raise SystemExit(f"{path}: SUPublicEDKey must contain a valid Ed25519 public key.")
    if not info.get("LSMinimumSystemVersion"):
        raise SystemExit(f"{path}: LSMinimumSystemVersion is required.")
    if source_info is None:
        source_info = info
    else:
        for key in ("SUPublicEDKey", "LSMinimumSystemVersion", "SUFeedURL", "CFBundleIdentifier"):
            if info.get(key) != source_info.get(key):
                raise SystemExit(f"{path}: {key} differs from native/AppInfo.plist; rebuild the bundle.")
print(version[1])
PY
}

# Detect missing update setup before starting the native/Rust release build.
VERSION="$(validate_metadata)"
OUTPUT="$ROOT/dist/releases/$VERSION"
if [[ -e "$OUTPUT" ]]; then
  printf '%s\n' "Release output already exists: $OUTPUT" 'Move it aside before creating another archive for the same version.' >&2
  exit 1
fi
if [[ "${2:-}" != --skip-build ]]; then
  bash "$ROOT/scripts/bundle.sh" release
fi
APP="$ROOT/dist/FourQuadrants GPUI.app"
if [[ ! -d "$APP" ]]; then
  printf '%s\n' "Missing bundle: $APP" >&2
  exit 1
fi
validate_metadata "$APP/Contents/Info.plist" >/dev/null

# The appcast declares arm64 hardware only. Check every embedded Mach-O, not
# just the main executable, so an accidental Intel or universal build fails.
while IFS= read -r -d '' FILE; do
  case "$(/usr/bin/file -b "$FILE")" in
    *Mach-O*)
      if [[ "$(xcrun lipo -archs "$FILE")" != arm64 ]]; then
        printf '%s\n' "Release packaging requires arm64-only code: $FILE" >&2
        exit 1
      fi
      ;;
  esac
done < <(find "$APP" -type f -print0)
if [[ ! -d "$APP/Contents/Frameworks/Sparkle.framework" || ! -x "$APP/Contents/MacOS/four-quadrants" ]]; then
  printf '%s\n' 'The bundle is missing Sparkle.framework or its executable.' >&2
  exit 1
fi
codesign --verify --deep --strict "$APP"

mkdir -p "$ROOT/dist/releases"
WORK="$(mktemp -d "$ROOT/dist/releases/.package.XXXXXX")"
trap 'unset PRIVATE_KEY; rm -rf "$WORK"' EXIT
ARCHIVE_NAME="FourQuadrants-GPUI-$VERSION-arm64.zip"
ARCHIVE="$WORK/$ARCHIVE_NAME"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ARCHIVE"

# This repository's CryptoKit utility signs the same Ed25519 archive bytes
# Sparkle verifies. Downloaded SDK executables are not run by this script.
mkdir -p "$ROOT/target/release-module-cache"
SIGN_TOOL=(xcrun swift -module-cache-path "$ROOT/target/release-module-cache" "$ROOT/scripts/update-key.swift")
if [[ -n "$PRIVATE_KEY" ]]; then
  SIGNATURE="$(printf '%s\n' "$PRIVATE_KEY" | "${SIGN_TOOL[@]}" sign "$ARCHIVE" --private-key-stdin)"
else
  SIGNATURE="$("${SIGN_TOOL[@]}" sign "$ARCHIVE")"
fi
unset PRIVATE_KEY
PUBLIC_KEY="$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$APP/Contents/Info.plist")"
"${SIGN_TOOL[@]}" verify "$ARCHIVE" "$SIGNATURE" "$PUBLIC_KEY"

python3 -I - "$APP/Contents/Info.plist" "$BASE_URL" "$ARCHIVE" "$SIGNATURE" "$WORK/appcast.xml" "$WORK/SHA256SUMS" <<'PY'
import datetime
import email.utils
import hashlib
import pathlib
import plistlib
import sys
from urllib.parse import quote
import xml.etree.ElementTree as ET

plist_path, base_url, archive_path, signature, appcast_path, checksums_path = sys.argv[1:]
with open(plist_path, "rb") as source:
    info = plistlib.load(source)
archive = pathlib.Path(archive_path)
feed_url = info.get("SUFeedURL") or base_url.rstrip("/") + "/appcast.xml"
sparkle = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", sparkle)
rss = ET.Element("rss", {"version": "2.0"})
channel = ET.SubElement(rss, "channel")
ET.SubElement(channel, "title").text = "FourQuadrants GPUI Updates"
ET.SubElement(channel, "link").text = feed_url
ET.SubElement(channel, "description").text = "FourQuadrants GPUI for Apple Silicon Macs"
ET.SubElement(channel, "language").text = "en"
item = ET.SubElement(channel, "item")
ET.SubElement(item, "title").text = f"FourQuadrants GPUI {info['CFBundleShortVersionString']}"
ET.SubElement(item, "pubDate").text = email.utils.format_datetime(datetime.datetime.now(datetime.timezone.utc))
ET.SubElement(item, f"{{{sparkle}}}version").text = info["CFBundleVersion"]
ET.SubElement(item, f"{{{sparkle}}}shortVersionString").text = info["CFBundleShortVersionString"]
ET.SubElement(item, f"{{{sparkle}}}minimumSystemVersion").text = info["LSMinimumSystemVersion"]
ET.SubElement(item, f"{{{sparkle}}}hardwareRequirements").text = "arm64"
ET.SubElement(item, "enclosure", {
    "url": base_url.rstrip("/") + "/" + quote(archive.name),
    "length": str(archive.stat().st_size),
    "type": "application/octet-stream",
    f"{{{sparkle}}}edSignature": signature,
    f"{{{sparkle}}}os": "macos",
})
ET.indent(rss, space="  ")
ET.ElementTree(rss).write(appcast_path, encoding="utf-8", xml_declaration=True)
digest = hashlib.sha256()
with archive.open("rb") as source:
    for block in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(block)
pathlib.Path(checksums_path).write_text(f"{digest.hexdigest()}  {archive.name}\n", encoding="utf-8")
PY

mv "$WORK" "$OUTPUT"
printf '%s\n' "Created local release files: $OUTPUT" "Archive: $ARCHIVE_NAME" 'Appcast: appcast.xml' 'Checksums: SHA256SUMS'
