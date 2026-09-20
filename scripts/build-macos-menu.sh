#!/bin/bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --locked --release --bin ccsw
swift build --package-path macos -c release
SWIFT_BIN="$(swift build --package-path macos -c release --show-bin-path)"
APP="$REPO_ROOT/target/macos/CCSW Menu.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources"
cp "$SWIFT_BIN/CCSWMenu" "$APP/Contents/MacOS/CCSWMenu"
cp target/release/ccsw "$APP/Contents/Helpers/ccsw"
cp macos/Info.plist "$APP/Contents/Info.plist"
swift scripts/macos-icon.swift "$REPO_ROOT/target/macos/Menu.iconset"
iconutil -c icns "$REPO_ROOT/target/macos/Menu.iconset" -o "$APP/Contents/Resources/Menu.icns"
chmod 755 "$APP/Contents/MacOS/CCSWMenu" "$APP/Contents/Helpers/ccsw"
codesign --force --sign - "$APP/Contents/Helpers/ccsw"
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"
printf '\nBuilt: %s\n' "$APP"
