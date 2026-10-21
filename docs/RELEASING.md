# Local macOS release packaging

Local commands build and verify files under `dist/` without uploading them. The GitHub Actions workflow publishes version tags after validation. Release archives are Apple Silicon (`arm64`) only. Run local commands from this repository on an Apple Silicon Mac with Xcode command-line tools, Rust, Python 3, and `curl` available.

`Cargo.toml` is the version source of truth. Both `CFBundleShortVersionString` and `CFBundleVersion` in `native/AppInfo.plist` must equal its `0.x.x` version, currently `0.19.2`. Packaging rejects mismatches.

## Fetch the pinned Sparkle SDK

```sh
bash scripts/fetch-sparkle.sh
```

This downloads Sparkle **2.10.0** from the official release:

```text
https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-2.10.0.tar.xz
SHA256: c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
```

The fetch checks that digest before extraction, checks archive paths, and uses a new empty directory for every download and extraction attempt. Scripts run outside the downloaded directories, and Python reads use isolated mode (`-I`). The extracted SDK remains under `target/sparkle/sdk.*`; `target/sparkle/sdk-path` records its absolute path for native builds. A successful cached fetch returns that path. Remove the pathfile to request a fresh verified download. No downloaded SDK executables run as part of fetching.

## Configure the local update key

Sparkle update signatures and macOS code signing serve different purposes. The update signature proves that the archive was signed by the private key corresponding to the app's `SUPublicEDKey`. It does not replace Developer ID signing or notarization.

The repository's `scripts/update-key.swift` uses Apple's CryptoKit and Security frameworks. Its persistent signing key uses the login Keychain service `com.fourquadrants.gpui.sparkle` and account `ed25519`. The following one-time command creates that record only when absent, or prints the existing public key:

```sh
xcrun swift scripts/update-key.swift public
```

Creating the Keychain record is an explicit local setup action and may require approval. Keep the private key in Keychain; do not add it to the repository. Place only the printed **public** key in `native/AppInfo.plist` as the string `SUPublicEDKey`, then rebuild the bundle. Do not replace an established key merely to resolve a packaging error: previously installed versions trust the public key they already contain. For GitHub Actions, add the base64 Ed25519 seed as the repository Actions secret `SPARKLE_PRIVATE_KEY`; the workflow exposes it only to the setup check and signing step, and passes it to the signing helper over standard input. The initial 0.19.0 release was signed locally; the Actions secret must be configured before a later new version can build in CI. Never print it or store it in a file.

`SUFeedURL` in the same plist is the HTTPS URL where the app will look for an appcast. Configure the intended URL before distributing an update-enabled build. Its location may differ from the archive directory passed below; for example, the stable URL `https://github.com/cod7ce/FourQuadrants-GPUI/releases/latest/download/appcast.xml` can describe a versioned archive under a tag release. The workflow publishes `appcast.xml`, the ZIP, and `SHA256SUMS` as GitHub Release assets. Local scripts do not upload to either location.

A local bundle can be built without `SUPublicEDKey`; release packaging refuses to sign an update until a valid key is configured. The package script verifies its archive signature against the public key in the actual bundled plist and fails on a mismatch.

## Build and sign a local bundle

```sh
bash scripts/bundle.sh release
```

For a development bundle, use `bash scripts/bundle.sh debug`. The result is `dist/FourQuadrants GPUI.app`.

The bundle sets `MACOSX_DEPLOYMENT_TARGET=14.0` for Rust and Objective-C compilation; the Swift helper uses the same minimum. It embeds `Sparkle.framework` with `ditto`, preserving its version and resource symlinks. It thins the copied framework and all other Mach-O files to the main executable's architecture, signs executable files first, then nested `.app`, `.xpc`, and `.framework` bundles from the inside out. The final bundle passes `codesign --verify --deep --strict` before replacing the previous local bundle.

Code signing defaults to **ad hoc** (`-`). An available local identity can be selected with `CODE_SIGN_IDENTITY`; that mode adds hardened runtime and a signing timestamp. No Developer ID identity is currently configured, and these scripts do not notarize or staple. An ad hoc archive is suitable for local development, but does not establish Developer ID trust for downloaded applications and may be rejected by Gatekeeper. Successful local signature checks do not make it a notarized public release.

## Create the archive and appcast locally

Supply the intended HTTPS **archive directory** as `BASE_URL`. It is used to construct the enclosure URL, not contacted or uploaded to:

```sh
bash scripts/package-release.sh 'https://downloads.example.com/fourquadrants/0.19.2'
```

To package a bundle you have already built:

```sh
bash scripts/package-release.sh 'https://downloads.example.com/fourquadrants/0.19.2' --skip-build
```

`--skip-build` still checks the source and bundled plist versions, exact public key and feed agreement, minimum macOS version, bundle signature, Sparkle framework presence, and every Mach-O architecture. Use it only with the intended release bundle. A missing source public key is rejected before the build starts.

The script creates these files under `dist/releases/0.19.2/`:

- `FourQuadrants-GPUI-0.19.2-arm64.zip`, a `ditto` archive containing the complete `.app` bundle.
- `appcast.xml`, with a signed enclosure, the archive length, version strings, minimum macOS version, and an `arm64` hardware requirement.
- `SHA256SUMS`, containing the archive's SHA256 digest.

The appcast enclosure is signed; the XML feed itself is not separately signed. The package script invokes the repository's CryptoKit helper to sign and independently verify the exact archive bytes against the bundled `SUPublicEDKey`. It does not run downloaded Sparkle executables. Existing version output is never overwritten: move an earlier output directory aside before rebuilding that version.

An optional `SPARKLE_PRIVATE_KEY` environment variable supplies a base64-encoded 32-byte Ed25519 private seed to the local packaging script. It disables tracing, removes the variable from build subprocesses, passes its value to the signing helper over standard input, and clears its temporary copy after signing. Provision that value outside source control; do not place it in shell examples, repository files, or logs. The corresponding public key must already be in the app's plist.

## Sparkle's optional signing tools

The pinned SDK also includes upstream signing tools; local and CI packaging use the repository's CryptoKit helper instead. Sparkle 2.10.0 advertises these options:

- `generate_keys --account com.fourquadrants.gpui.sparkle -p` looks up an existing public key; omitting `-p` creates or reuses a key in Sparkle's own Keychain format.
- `sign_update --account com.fourquadrants.gpui.sparkle -p ARCHIVE` signs using that Sparkle-managed key.
- `sign_update --ed-key-file - -p ARCHIVE` reads a private key from standard input.
- `sign_update --verify ARCHIVE SIGNATURE` verifies using its selected signing key; it has no option to take the app's public key directly.

The repository's CryptoKit helper uses its own Keychain record, so the upstream tool's account option does not automatically access it. Do not create a second signing identity as a substitute for the configured app key. Never execute tools from an unverified SDK download.

## Publish later versions

Update `Cargo.toml`, the root package version in `Cargo.lock`, and both app version fields in `native/AppInfo.plist` together. Keep the existing public key and feed. Commit and push the changes, then create and push a matching `v0.x.x` tag. `.github/workflows/release.yml` validates the tag, runs Rust and native tests on an arm64 macOS runner, builds and signs the package, then publishes the three release assets. A missing signing secret fails before the build. Actions are pinned to commit hashes and only the publish job has repository write access.

The workflow can also be dispatched with an existing tag. If that version is already published, it downloads the three assets, checks their hash, metadata and Ed25519 signature, then skips building and uploading. It never overwrites an existing release. A checksum check alone does not replace signature verification.

```sh
gh workflow run release.yml --repo cod7ce/FourQuadrants-GPUI --ref main -f tag=v0.19.2
```

## Local checks

```sh
bash -n scripts/fetch-sparkle.sh scripts/bundle.sh scripts/package-release.sh
bash scripts/test-updater.sh
swift scripts/update-key.swift self-test
codesign --verify --deep --strict 'dist/FourQuadrants GPUI.app'
(cd dist/releases/0.19.2 && shasum -a 256 -c SHA256SUMS)
```

Use a `--data target/...` store for any app launch used to validate packaging. Do not test against the original SwiftUI source or its data. Build products, SDKs, archives, checksums, and appcasts stay in ignored `target/` or `dist/` directories.
