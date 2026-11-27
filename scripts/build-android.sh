#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Build a NativeActivity + JNI bridge APK (and optionally a Play-ready AAB) for Android (arm64-v8a).
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE_FLAG=()
TARGET_PROFILE=debug
BUILD_AAB=0
REQUIRE_RELEASE_KEYSTORE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release)
      PROFILE_FLAG=(--release)
      TARGET_PROFILE=release
      shift
      ;;
    --aab)
      BUILD_AAB=1
      shift
      ;;
    --require-release-keystore)
      REQUIRE_RELEASE_KEYSTORE=1
      shift
      ;;
    *)
      echo "Unknown option: $1" >&2
      echo "Usage: $0 [--release] [--aab] [--require-release-keystore]" >&2
      exit 2
      ;;
  esac
done

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
ANDROID_COMPILE_SDK="${ANDROID_COMPILE_SDK:-36}"
if [[ -d "$ANDROID_HOME/platforms/android-${ANDROID_COMPILE_SDK}" ]]; then
  PLATFORM_DIR="$ANDROID_HOME/platforms/android-${ANDROID_COMPILE_SDK}"
else
  PLATFORM_DIR=$(ls -d "$ANDROID_HOME/platforms/android-"* | sort -V | tail -n 1)
fi
PLATFORM_JAR="$PLATFORM_DIR/android.jar"
NDK_DIR=$(ls -d "$ANDROID_HOME/ndk/"* | sort -V | tail -n 1)
STRIP=$(find "$NDK_DIR/toolchains/llvm/prebuilt" -name llvm-strip | head -n 1)
READELF=$(find "$NDK_DIR/toolchains/llvm/prebuilt" -name llvm-readelf | head -n 1)

if [[ -z "$STRIP" || ! -x "$STRIP" ]]; then
  echo "llvm-strip not found under $NDK_DIR" >&2
  exit 1
fi

ANDROID_VERSION_NAME="${ANDROID_VERSION_NAME:-$(python3 -c 'import sys; sys.path.insert(0, "scripts"); from docs_build import version_for; print(version_for("Android"))')}"
ANDROID_VERSION_CODE="${ANDROID_VERSION_CODE:-$(python3 -c 'import sys; sys.path.insert(0, "scripts"); from docs_build import build_number; n = build_number(); print(n if n else 1)')}"

if ! [[ "$ANDROID_VERSION_CODE" =~ ^[1-9][0-9]*$ ]]; then
  echo "ANDROID_VERSION_CODE must be a positive integer (got: $ANDROID_VERSION_CODE)" >&2
  exit 1
fi

echo "Building Android arm64 ($TARGET_PROFILE) versionName=$ANDROID_VERSION_NAME versionCode=$ANDROID_VERSION_CODE"

echo "1. Building libibis_office_app.so (arm64-v8a, $TARGET_PROFILE)..."
export IBIS_THIRD_PARTY_NOTICES="$PWD/target/third-party-android.md"
python3 scripts/gen-web-licenses.py android
ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"
if [[ "$TARGET_PROFILE" == "release" ]]; then
  export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-z}"
  export CARGO_PROFILE_RELEASE_LTO="${CARGO_PROFILE_RELEASE_LTO:-fat}"
  export CARGO_PROFILE_RELEASE_CODEGEN_UNITS="${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-1}"
  export CARGO_PROFILE_RELEASE_INCREMENTAL="${CARGO_PROFILE_RELEASE_INCREMENTAL:-false}"
  export CARGO_PROFILE_RELEASE_PANIC="${CARGO_PROFILE_RELEASE_PANIC:-abort}"
  export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-symbols}"
  ANDROID_RUSTFLAGS="$ANDROID_RUSTFLAGS -C link-arg=-Wl,--gc-sections -C link-arg=-Wl,--icf=all"
fi
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="$ANDROID_RUSTFLAGS"
cargo ndk -t arm64-v8a build --locked -p ibis-office-app --no-default-features --lib ${PROFILE_FLAG[@]+"${PROFILE_FLAG[@]}"}

SO_PATH="target/aarch64-linux-android/$TARGET_PROFILE/libibis_office_app.so"
OUT_DIR="target/android-apk"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/lib/arm64-v8a" "$OUT_DIR/classes" "$OUT_DIR/dex" "$OUT_DIR/res_flat"

echo "2. Compiling Java activity -> classes.dex..."
javac --release 11 -cp "$PLATFORM_JAR" -d "$OUT_DIR/classes" \
  apps/android/src/dev/sorane/docs/IbisActivity.java
"$BUILD_TOOLS/d8" --min-api 26 --lib "$PLATFORM_JAR" --output "$OUT_DIR/dex" \
  "$OUT_DIR/classes/dev/sorane/docs/"*.class

echo "3. Stripping shared library and checking 16 KB ELF alignment..."
"$STRIP" --strip-unneeded -o "$OUT_DIR/lib/arm64-v8a/libibis_office_app.so" "$SO_PATH"
if [[ -n "${READELF:-}" && -x "$READELF" ]]; then
  python3 - "$READELF" "$OUT_DIR/lib/arm64-v8a/libibis_office_app.so" <<'PY'
import subprocess, sys
readelf, so_path = sys.argv[1], sys.argv[2]
out = subprocess.check_output([readelf, "-lW", so_path], text=True)
aligns = []
for line in out.splitlines():
    parts = line.split()
    if parts and parts[0] == "LOAD":
        aligns.append(int(parts[-1], 16))
if not aligns:
    sys.exit(f"No PT_LOAD segments found in {so_path}")
bad = [hex(a) for a in aligns if a < 0x4000]
if bad:
    sys.exit(f"16 KB page-size alignment check failed for {so_path}: PT_LOAD aligns={bad}")
print(f"Verified 16 KB ELF alignment ({len(aligns)} PT_LOAD segments, min align={hex(min(aligns))})")
PY
fi

KEYSTORE="${ANDROID_KEYSTORE_PATH:-target/android-debug.keystore}"
KEY_ALIAS="${ANDROID_KEY_ALIAS:-androiddebugkey}"
KEYSTORE_PASS="${ANDROID_KEYSTORE_PASS:-android}"
KEY_PASS="${ANDROID_KEY_PASS:-$KEYSTORE_PASS}"

if [[ "$REQUIRE_RELEASE_KEYSTORE" == "1" ]]; then
  if [[ -z "${ANDROID_KEYSTORE_PATH:-}" || ! -f "$ANDROID_KEYSTORE_PATH" ]]; then
    echo "--require-release-keystore was set, but ANDROID_KEYSTORE_PATH is missing or does not exist." >&2
    exit 1
  fi
  if [[ -z "${ANDROID_KEY_ALIAS:-}" || -z "${ANDROID_KEYSTORE_PASS:-}" ]]; then
    echo "--require-release-keystore requires ANDROID_KEY_ALIAS and ANDROID_KEYSTORE_PASS." >&2
    exit 1
  fi
else
  if [[ ! -f "$KEYSTORE" ]]; then
    keytool -genkeypair -v -keystore "$KEYSTORE" \
      -alias "$KEY_ALIAS" \
      -storepass "$KEYSTORE_PASS" \
      -keypass "$KEY_PASS" \
      -keyalg RSA -keysize 2048 -validity 10000 \
      -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
  fi
fi

echo "4. Packaging APK with aapt2..."
"$BUILD_TOOLS/aapt2" compile --dir apps/android/res -o "$OUT_DIR/res_flat"
"$BUILD_TOOLS/aapt2" link \
  --manifest apps/android/AndroidManifest.xml \
  --version-code "$ANDROID_VERSION_CODE" \
  --version-name "$ANDROID_VERSION_NAME" \
  --replace-version \
  -I "$PLATFORM_JAR" \
  "$OUT_DIR/res_flat"/*.flat \
  -o "$OUT_DIR/unaligned.apk"

cp "$OUT_DIR/dex/classes.dex" "$OUT_DIR/classes.dex"
mkdir -p "$OUT_DIR/assets/licenses"
cp "$IBIS_THIRD_PARTY_NOTICES" "$OUT_DIR/assets/licenses/third-party-licenses.md"
cp crates/ibis-office-app/fonts/OFL.txt "$OUT_DIR/assets/licenses/IBM-Plex-OFL.txt"
(cd "$OUT_DIR" && zip -q -r unaligned.apk lib classes.dex assets/licenses)

"$BUILD_TOOLS/zipalign" -f -P 16 4 "$OUT_DIR/unaligned.apk" "$OUT_DIR/aligned.apk"

APK_NAME="ibis-office-debug.apk"
if [[ "$TARGET_PROFILE" == "release" ]]; then
  APK_NAME="ibis-office-android-arm64.apk"
fi

"$BUILD_TOOLS/apksigner" sign \
  --ks "$KEYSTORE" \
  --ks-key-alias "$KEY_ALIAS" \
  --ks-pass "pass:$KEYSTORE_PASS" \
  --key-pass "pass:$KEY_PASS" \
  --out "$OUT_DIR/$APK_NAME" \
  "$OUT_DIR/aligned.apk"

"$BUILD_TOOLS/apksigner" verify "$OUT_DIR/$APK_NAME"

ls -lh "$OUT_DIR/$APK_NAME"
echo "Built: $OUT_DIR/$APK_NAME"

if [[ "$BUILD_AAB" == "1" ]]; then
  echo "5. Packaging Android App Bundle (AAB)..."
  BUNDLETOOL_CMD=()
  if command -v bundletool >/dev/null 2>&1; then
    BUNDLETOOL_CMD=(bundletool)
  elif [[ -n "${BUNDLETOOL_JAR:-}" && -f "$BUNDLETOOL_JAR" ]]; then
    BUNDLETOOL_CMD=(java -jar "$BUNDLETOOL_JAR")
  else
    echo "bundletool not found (install bundletool or set BUNDLETOOL_JAR)." >&2
    exit 1
  fi

  "$BUILD_TOOLS/aapt2" link \
    --proto-format \
    --manifest apps/android/AndroidManifest.xml \
    --version-code "$ANDROID_VERSION_CODE" \
    --version-name "$ANDROID_VERSION_NAME" \
    --replace-version \
    -I "$PLATFORM_JAR" \
    "$OUT_DIR/res_flat"/*.flat \
    -o "$OUT_DIR/base_proto.apk"

  BASE_DIR="$OUT_DIR/aab_base"
  rm -rf "$BASE_DIR" "$OUT_DIR/base.zip"
  mkdir -p "$BASE_DIR/manifest" "$BASE_DIR/dex" "$BASE_DIR/lib/arm64-v8a" "$BASE_DIR/assets/licenses"

  unzip -q -o "$OUT_DIR/base_proto.apk" -d "$OUT_DIR/base_proto_unzipped"
  cp "$OUT_DIR/base_proto_unzipped/AndroidManifest.xml" "$BASE_DIR/manifest/AndroidManifest.xml"
  cp "$OUT_DIR/base_proto_unzipped/resources.pb" "$BASE_DIR/resources.pb"
  if [[ -d "$OUT_DIR/base_proto_unzipped/res" ]]; then
    cp -R "$OUT_DIR/base_proto_unzipped/res" "$BASE_DIR/res"
  fi
  cp "$OUT_DIR/dex/classes.dex" "$BASE_DIR/dex/classes.dex"
  cp "$OUT_DIR/lib/arm64-v8a/libibis_office_app.so" "$BASE_DIR/lib/arm64-v8a/libibis_office_app.so"
  cp "$IBIS_THIRD_PARTY_NOTICES" "$BASE_DIR/assets/licenses/third-party-licenses.md"
  cp crates/ibis-office-app/fonts/OFL.txt "$BASE_DIR/assets/licenses/IBM-Plex-OFL.txt"

  (cd "$BASE_DIR" && zip -q -r ../base.zip .)

  AAB_NAME="ibis-office-debug.aab"
  if [[ "$TARGET_PROFILE" == "release" ]]; then
    AAB_NAME="ibis-office-android-arm64.aab"
  fi

  "${BUNDLETOOL_CMD[@]}" build-bundle \
    --modules="$OUT_DIR/base.zip" \
    --output="$OUT_DIR/$AAB_NAME"

  jarsigner -digestalg SHA-256 \
    -keystore "$KEYSTORE" \
    -storepass "$KEYSTORE_PASS" \
    -keypass "$KEY_PASS" \
    "$OUT_DIR/$AAB_NAME" \
    "$KEY_ALIAS" >/dev/null

  jarsigner -verify "$OUT_DIR/$AAB_NAME" >/dev/null
  "${BUNDLETOOL_CMD[@]}" validate --bundle="$OUT_DIR/$AAB_NAME"
  "${BUNDLETOOL_CMD[@]}" dump manifest --bundle="$OUT_DIR/$AAB_NAME" | grep -q "android:versionCode=\"${ANDROID_VERSION_CODE}\"" || {
    echo "AAB manifest did not contain expected android:versionCode=${ANDROID_VERSION_CODE}" >&2
    exit 1
  }

  ls -lh "$OUT_DIR/$AAB_NAME"
  echo "Built: $OUT_DIR/$AAB_NAME"
fi
