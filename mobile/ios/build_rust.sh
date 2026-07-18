#!/usr/bin/env bash
#
# Build the client-sdk crate for iOS and package it as an .xcframework, plus the
# generated Swift bindings. Run from anywhere:
#
#     ./mobile/ios/build_rust.sh
#
# Outputs (git-ignored):
#   mobile/ios/Generated/client_sdk.swift   — the Swift API
#   mobile/ios/ClientSdk.xcframework         — device + simulator static libs
#
# Requires: rustup iOS targets (aarch64-apple-ios, aarch64-apple-ios-sim) and Xcode.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

CRATE="client-sdk"
LIB_NAME="libclient_sdk.a"
GEN_DIR="$SCRIPT_DIR/Generated"
XCF="$SCRIPT_DIR/ClientSdk.xcframework"
BUILD_MODE="release"

echo "> Building $CRATE for iOS device + simulator ($BUILD_MODE)..."
cargo build -p "$CRATE" --$BUILD_MODE --target aarch64-apple-ios
cargo build -p "$CRATE" --$BUILD_MODE --target aarch64-apple-ios-sim

DEVICE_LIB="target/aarch64-apple-ios/$BUILD_MODE/$LIB_NAME"
SIM_LIB="target/aarch64-apple-ios-sim/$BUILD_MODE/$LIB_NAME"

echo "> Generating Swift bindings..."
rm -rf "$GEN_DIR" && mkdir -p "$GEN_DIR"
cargo run -p "$CRATE" --bin uniffi-bindgen -- \
    generate --library "$SIM_LIB" --language swift --out-dir "$GEN_DIR"

# uniffi emits: client_sdk.swift, client_sdkFFI.h, client_sdkFFI.modulemap.
# An .xcframework wants a headers dir containing `module.modulemap`.
HEADERS="$GEN_DIR/headers"
rm -rf "$HEADERS" && mkdir -p "$HEADERS"
cp "$GEN_DIR"/*FFI.h "$HEADERS/"
cp "$GEN_DIR"/*FFI.modulemap "$HEADERS/module.modulemap"

echo "> Assembling ${XCF} ..."
rm -rf "$XCF"
xcodebuild -create-xcframework \
    -library "$DEVICE_LIB" -headers "$HEADERS" \
    -library "$SIM_LIB" -headers "$HEADERS" \
    -output "$XCF"

echo "Done."
echo "  Swift API : $GEN_DIR/client_sdk.swift"
echo "  Framework : $XCF"
