#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK_PATH="$(xcrun --sdk macosx --show-sdk-path)"
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"

echo "Type-checking QRCodeScannerView.swift with Xcode $(xcodebuild -version | head -n 1), macOS SDK ${SDK_VERSION}"
xcrun swiftc \
  -typecheck \
  -parse-as-library \
  -sdk "$SDK_PATH" \
  -target arm64-apple-macosx13.0 \
  "$ROOT_DIR/macos/Sources/QRCodeScannerView.swift"
