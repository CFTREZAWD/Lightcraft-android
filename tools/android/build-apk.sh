#!/usr/bin/env bash
# Build LightCraft for Android: target/android/lightcraft-<version>-arm64.apk
#
# Needs: Rust with the aarch64-linux-android target, the Android NDK (r27+), the SDK's
# build-tools and an android.jar (platform 34+), a JDK (javac, keytool).
#
#   ANDROID_NDK_HOME=/path/to/ndk ANDROID_HOME=/path/to/sdk tools/android/build-apk.sh [--debug]
#
# Signed with LIGHTCRAFT_KEYSTORE (+ LIGHTCRAFT_KEYSTORE_PASSWORD, LIGHTCRAFT_KEY_ALIAS) when set, else with a
# local debug key (target/android/debug.keystore, made on first use).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="$ROOT/apps/lightcraft-android"
OUT="$ROOT/target/android"
PROFILE=release
[[ "${1:-}" == "--debug" ]] && PROFILE=dev

: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK}"
: "${ANDROID_HOME:?set ANDROID_HOME to the SDK}"
API=28
BT="$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)"
PLATFORM="$(ls -d "$ANDROID_HOME"/platforms/android-* | sort -V | tail -1)"
TC="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
[[ -d "$TC" ]] || TC="$(ls -d "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin | head -1)"

export CC_aarch64_linux_android="$TC/aarch64-linux-android$API-clang"
export AR_aarch64_linux_android="$TC/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TC/aarch64-linux-android$API-clang"
# 16 KB pages (required by Google Play for Android 15+ devices)
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')"
IFS=. read -r MA MI PA <<<"$VERSION"
CODE=$((MA * 1000000 + MI * 1000 + PA))

echo "== native library ($PROFILE)"
cargo build --manifest-path "$ROOT/Cargo.toml" -p lightcraft-android --target aarch64-linux-android --profile "$PROFILE"
DIR=$([[ $PROFILE == dev ]] && echo debug || echo release)
SO="$ROOT/target/aarch64-linux-android/$DIR/liblightcraft_android.so"

STAGE="$OUT/stage"
rm -rf "$STAGE" && mkdir -p "$STAGE"/{classes,dex,res,apk/lib/arm64-v8a}

echo "== activity (Java)"
javac -source 8 -target 8 -Xlint:-options -bootclasspath "$PLATFORM/android.jar" -classpath "$PLATFORM/android.jar" \
    -d "$STAGE/classes" $(find "$APP/android/java" -name '*.java')
"$BT/d8" --min-api $API --lib "$PLATFORM/android.jar" --output "$STAGE/dex" $(find "$STAGE/classes" -name '*.class')

echo "== resources and manifest"
"$BT/aapt2" compile --dir "$APP/android/res" -o "$STAGE/res/res.zip"
"$BT/aapt2" link -o "$STAGE/base.apk" -I "$PLATFORM/android.jar" \
    --manifest "$APP/android/AndroidManifest.xml" --version-code "$CODE" --version-name "$VERSION" \
    --min-sdk-version $API "$STAGE/res/res.zip"

echo "== apk"
cp "$STAGE/dex/classes.dex" "$STAGE/apk/"
cp "$SO" "$STAGE/apk/lib/arm64-v8a/"
"$TC/llvm-strip" --strip-debug "$STAGE/apk/lib/arm64-v8a/liblightcraft_android.so"
cp "$STAGE/base.apk" "$STAGE/unaligned.apk"
(cd "$STAGE/apk" && zip -q -r "$STAGE/unaligned.apk" classes.dex lib)
APK="$OUT/lightcraft-$VERSION-arm64.apk"
"$BT/zipalign" -f -P 16 4 "$STAGE/unaligned.apk" "$STAGE/aligned.apk"

if [[ -n "${LIGHTCRAFT_KEYSTORE:-}" ]]; then
    # your own key (LIGHTCRAFT_KEYSTORE, LIGHTCRAFT_KEYSTORE_PASSWORD, LIGHTCRAFT_KEY_ALIAS): updates install over each other
    "$BT/apksigner" sign --ks "$LIGHTCRAFT_KEYSTORE" --ks-pass env:LIGHTCRAFT_KEYSTORE_PASSWORD \
        --ks-key-alias "${LIGHTCRAFT_KEY_ALIAS:?set LIGHTCRAFT_KEY_ALIAS}" --out "$APK" "$STAGE/aligned.apk"
else
    KS="$OUT/debug.keystore"
    if [[ ! -f "$KS" ]]; then
        keytool -genkeypair -keystore "$KS" -storepass android -keypass android -alias debug \
            -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=LightCraft debug" >/dev/null
    fi
    "$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android --out "$APK" "$STAGE/aligned.apk"
fi
"$BT/apksigner" verify "$APK"
echo "== $APK ($(du -h "$APK" | cut -f1))"
