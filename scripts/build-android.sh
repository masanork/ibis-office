#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Build a NativeActivity + JNI bridge APK for Android (arm64-v8a).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${ANDROID_HOME:-}" ]]; then
  if [[ -d "$HOME/Library/Android/sdk" ]]; then
    ANDROID_HOME="$HOME/Library/Android/sdk"
  elif [[ -d "/usr/local/lib/android/sdk" ]]; then
    ANDROID_HOME="/usr/local/lib/android/sdk"
  elif [[ -d "$HOME/Android/Sdk" ]]; then
    ANDROID_HOME="$HOME/Android/Sdk"
  else
    echo "ANDROID_HOME is not set and default SDK paths were not found." >&2
    exit 1
  fi
fi
export ANDROID_HOME

BUILD_TOOLS=$(ls -d "$ANDROID_HOME/build-tools/"* | sort -V | tail -n 1)
PLATFORM_DIR=$(ls -d "$ANDROID_HOME/platforms/android-"* | sort -V | tail -n 1)
PLATFORM_JAR="$PLATFORM_DIR/android.jar"
NDK_DIR=$(ls -d "$ANDROID_HOME/ndk/"* | sort -V | tail -n 1)
STRIP=$(find "$NDK_DIR/toolchains/llvm/prebuilt" -name llvm-strip | head -n 1)

if [[ -z "$STRIP" || ! -x "$STRIP" ]]; then
  echo "llvm-strip not found under $NDK_DIR" >&2
  exit 1
fi

PROFILE_FLAG=()
TARGET_PROFILE=debug
if [[ "${1:-}" == "--release" ]]; then
  PROFILE_FLAG=(--release)
  TARGET_PROFILE=release
fi

echo "1. Building libibis_office_app.so (arm64-v8a, $TARGET_PROFILE)..."
cargo ndk -t arm64-v8a build --locked -p ibis-office-app --no-default-features --lib ${PROFILE_FLAG[@]+"${PROFILE_FLAG[@]}"}

SO_PATH="target/aarch64-linux-android/$TARGET_PROFILE/libibis_office_app.so"
OUT_DIR="target/android-apk"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/lib/arm64-v8a" "$OUT_DIR/classes" "$OUT_DIR/dex" "$OUT_DIR/res_flat"

echo "2. Compiling Java activity -> classes.dex..."
javac --release 11 -cp "$PLATFORM_JAR" -d "$OUT_DIR/classes" \
  apps/android/src/dev/sorane/ibisoffice/IbisActivity.java
"$BUILD_TOOLS/d8" --min-api 26 --lib "$PLATFORM_JAR" --output "$OUT_DIR/dex" \
  "$OUT_DIR/classes/dev/sorane/ibisoffice/"*.class

echo "3. Stripping shared library..."
"$STRIP" --strip-unneeded -o "$OUT_DIR/lib/arm64-v8a/libibis_office_app.so" "$SO_PATH"

echo "4. Packaging APK with aapt2..."
"$BUILD_TOOLS/aapt2" compile --dir apps/android/res -o "$OUT_DIR/res_flat"
"$BUILD_TOOLS/aapt2" link \
  --manifest apps/android/AndroidManifest.xml \
  -I "$PLATFORM_JAR" \
  "$OUT_DIR/res_flat"/*.flat \
  -o "$OUT_DIR/unaligned.apk"

cp "$OUT_DIR/dex/classes.dex" "$OUT_DIR/classes.dex"
(cd "$OUT_DIR" && zip -q -r unaligned.apk lib classes.dex)

"$BUILD_TOOLS/zipalign" -f 4 "$OUT_DIR/unaligned.apk" "$OUT_DIR/aligned.apk"

KEYSTORE="${ANDROID_KEYSTORE_PATH:-target/android-debug.keystore}"
if [[ ! -f "$KEYSTORE" ]]; then
  keytool -genkeypair -v -keystore "$KEYSTORE" \
    -alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" \
    -storepass "${ANDROID_KEYSTORE_PASS:-android}" \
    -keypass "${ANDROID_KEY_PASS:-android}" \
    -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi

APK_NAME="ibis-office-debug.apk"
if [[ "$TARGET_PROFILE" == "release" ]]; then
  APK_NAME="ibis-office-android-arm64.apk"
fi

"$BUILD_TOOLS/apksigner" sign \
  --ks "$KEYSTORE" \
  --ks-pass "pass:${ANDROID_KEYSTORE_PASS:-android}" \
  --key-pass "pass:${ANDROID_KEY_PASS:-android}" \
  --out "$OUT_DIR/$APK_NAME" \
  "$OUT_DIR/aligned.apk"

"$BUILD_TOOLS/apksigner" verify "$OUT_DIR/$APK_NAME"

ls -lh "$OUT_DIR/$APK_NAME"
echo "Built: $OUT_DIR/$APK_NAME"
