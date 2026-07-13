#!/usr/bin/env sh
set -eu

pnpm tauri build --target universal-apple-darwin --bundles app

APP_PATH="src-tauri/target/universal-apple-darwin/release/bundle/macos/Cortana.app"
OUTPUT_PATH="src-tauri/target/universal-apple-darwin/release/bundle/macos/Cortana.pkg"

test -d "$APP_PATH"
pkgbuild --component "$APP_PATH" --install-location /Applications "$OUTPUT_PATH"
printf 'Created %s\n' "$OUTPUT_PATH"
