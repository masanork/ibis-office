#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Build the iOS app bundle (IbisOffice.app) for simulator or device.
#
#   scripts/build-ios.sh            debug build for iOS simulator (aarch64-apple-ios-sim)
#   scripts/build-ios.sh --release  release build for iOS simulator
#   scripts/build-ios.sh --device   release build for physical iOS device (aarch64-apple-ios)
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=aarch64-apple-ios-sim
SDK=iphonesimulator
PROFILE=debug
CARGO_FLAGS=()
MIN_VERSION_FLAG="-mios-simulator-version-min=16.0"

for arg in "$@"; do
  case "$arg" in
    --release)
      PROFILE=release
      CARGO_FLAGS=(--release)
      ;;
    --device)
      TARGET=aarch64-apple-ios
      SDK=iphoneos
      PROFILE=release
      CARGO_FLAGS=(--release)
      MIN_VERSION_FLAG="-miphoneos-version-min=16.0"
      ;;
  esac
done

echo "1. Building libibis_office_app.a ($TARGET, $PROFILE)..."
IPHONEOS_DEPLOYMENT_TARGET=16.0 cargo rustc --locked -p ibis-office-app --no-default-features --lib --crate-type staticlib --target "$TARGET" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}

LIB_PATH="target/$TARGET/$PROFILE/libibis_office_app.a"
APP_DIR="target/ios-app/IbisOffice.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR"

SDK_PATH=$(xcrun --sdk "$SDK" --show-sdk-path)

echo "2. Compiling Objective-C UIKit host and linking IbisOffice.app..."
clang -fobjc-arc -arch arm64 -isysroot "$SDK_PATH" "$MIN_VERSION_FLAG" \
  apps/ios/main.m "$LIB_PATH" \
  -framework UIKit \
  -framework Foundation \
  -framework Metal \
  -framework QuartzCore \
  -framework CoreGraphics \
  -framework CoreText \
  -framework UniformTypeIdentifiers \
  -framework Security \
  -framework SystemConfiguration \
  -lc++ -lz -liconv \
  -o "$APP_DIR/IbisOffice"

cp apps/ios/Info.plist "$APP_DIR/Info.plist"
PARTIAL_PLIST=$(mktemp)
xcrun actool apps/ios/Assets.xcassets \
  --compile "$APP_DIR" \
  --platform "$SDK" \
  --minimum-deployment-target 16.0 \
  --app-icon AppIcon \
  --output-partial-info-plist "$PARTIAL_PLIST" >/dev/null
/usr/libexec/PlistBuddy -c "Merge $PARTIAL_PLIST" "$APP_DIR/Info.plist"
rm -f "$PARTIAL_PLIST"
codesign --force --sign - "$APP_DIR"

echo "Built: $APP_DIR"
