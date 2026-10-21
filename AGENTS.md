# Project conventions

This is a macOS-only Rust / GPUI application. The original SwiftUI repository is ../FourQuadrants; do not modify its source or the user's old data as part of this project.

- Keep versions in 0.x.x. Cargo.toml is the source of truth; native/AppInfo.plist must match when changing versions.
- GPUI is pinned to 0.2.2 and gpui-component/assets to 0.5.1. Newer GPUI Kit versions use gpui-pre and are not type-compatible; upgrade as a coordinated change.
- Main UI and domain operations are Rust. Keep RTFD/AppKit, SwiftData reads, Foundation regex and notifications in native/.
- All durable mutations go through Workspace::change and Store::save. Keep user-visible errors, atomic writes, backups, concurrent-writer checks, tree validation and idempotent imports.
- Preserve rich text bytes and attachments. A cancelled native editor must not clear existing content. Store native saved results in Recovery before trying to update the main document.
- Use --data target/... for UI tests. Alternate stores must not schedule/cancel the user's system notifications.
- Commands: cargo fmt --check; cargo test --locked; cargo clippy --locked --all-targets; bash scripts/build-native.sh; target/native/four-quadrants-helper self-test.
- Run: bash scripts/build-native.sh && cargo run --locked. Package: bash scripts/bundle.sh [debug|release].
- Screenshots and build artifacts belong in target/ or dist/, both ignored. Do not commit local data or screenshots of unrelated windows.
