#!/bin/sh
# Stage 6a local build: assemble the APK by hand (no Gradle).
# Rehearsed 2026-11-17 in the local toolchain sandbox.
# Toolchain locations default to the Memoria rehearsal set under
# ~/workspace/.rust; override with env vars on another machine.
set -e

RUST_HOME="${RUST_HOME:-$HOME/workspace/.rust}"
export RUSTUP_HOME="$RUST_HOME/rustup"
export CARGO_HOME="$RUST_HOME/cargo"
export ANDROID_NDK_HOME="$RUST_HOME/android-ndk-r27c"
export JAVA_HOME="$RUST_HOME/jdk-17.0.21+9"
export PATH="$CARGO_HOME/bin:$JAVA_HOME/bin:$PATH"

SDK="$RUST_HOME/android-sdk"
BT="$SDK/build-tools/36.0.0"
AJAR="$SDK/platforms/android-36/android.jar"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
W="${OUT_DIR:-/tmp/nbapk}"
rm -rf "$W"
mkdir -p "$W/classes" "$W/dex" "$W/lib/arm64-v8a"

# llama.cpp (notebook-infer, since 6d) builds C++ via cmake and
# generates bindings via bindgen during the cross build: bindgen
# runs on the host and needs the NDK's libclang, and cmake needs
# the NDK toolchain file to target Android instead of the host.
export LIBCLANG_PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/musl/lib"
export BINDGEN_EXTRA_CLANG_ARGS="-I$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/lib/clang/18/include"
export ANDROID_NDK="$ANDROID_NDK_HOME"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
export CMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake"
export ANDROID_ABI="arm64-v8a"
export ANDROID_PLATFORM="android-26"

# 1. Rust .so for arm64 (cargo-ndk 4.x: platform flag is -P).
cd "$ROOT"
cargo ndk -t arm64-v8a -P 26 build --release -p notebook-app --offline

# 2. Java vessel -> dex.
cd "$ROOT/android/app/src/main"
javac -encoding UTF-8 -cp "$AJAR" -d "$W/classes" $(find java -name '*.java')
"$BT/d8" --lib "$AJAR" --output "$W/dex" $(find "$W/classes" -name '*.class')

# 3. Package, align, sign (debug key).
"$BT/aapt2" link -o "$W/app-unsigned.apk" -I "$AJAR" --manifest AndroidManifest.xml
cp "$ROOT/target/aarch64-linux-android/release/libnotebook_app.so" "$W/lib/arm64-v8a/"
# llama.cpp links the NDK's shared C++ runtime; the app library
# NEEDs libc++_shared.so, so it ships alongside (from the NDK
# sysroot). Without it the library fails to load on device.
cp "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "$W/lib/arm64-v8a/"
# ONNX Runtime (arm64, from the official 1.23.2 AAR) sits next to
# the app library; the Rust side dlopens it by path at init.
RURI_ASSETS="${RURI_ASSETS:-$HOME/workspace/ruri-assets}"
cp "$RURI_ASSETS/libonnxruntime-arm64.so" "$W/lib/arm64-v8a/libonnxruntime.so"
# The ruri model files ride as assets; the vessel extracts them to
# filesDir on first launch.
mkdir -p "$W/assets/ruri"
cp "$RURI_ASSETS/model_int8.onnx" "$RURI_ASSETS/tokenizer.json" "$W/assets/ruri/"
# Stage 7 (audio overview): the VOICEVOX engine is bundled — the
# core library (MIT) is linked by the Rust side and the VOICEVOX
# ONNX Runtime is dlopened at synthesis time. Voice models (VVM)
# are NOT bundled; the app downloads them per-voice at runtime.
VV_DIR="$ROOT/android/third_party/voicevox"
cp "$VV_DIR/libvoicevox_core.so" "$VV_DIR/libvoicevox_onnxruntime.so" "$W/lib/arm64-v8a/"
# The Open JTalk dictionary (1.11) rides as assets; the vessel
# extracts it to filesDir before the first synthesis.
mkdir -p "$W/assets/open_jtalk_dic"
cp "$ROOT/android/third_party/open_jtalk_dic/"* "$W/assets/open_jtalk_dic/"
cd "$W"
cp dex/classes.dex .
zip -q app-unsigned.apk classes.dex
zip -q -0 app-unsigned.apk lib/arm64-v8a/libnotebook_app.so lib/arm64-v8a/libonnxruntime.so lib/arm64-v8a/libc++_shared.so lib/arm64-v8a/libvoicevox_core.so lib/arm64-v8a/libvoicevox_onnxruntime.so
zip -q -9 app-unsigned.apk assets/ruri/model_int8.onnx assets/ruri/tokenizer.json assets/open_jtalk_dic/*
"$BT/zipalign" -f 4 app-unsigned.apk app-aligned.apk
# The debug key must be STABLE across builds, or Android refuses to
# install a new build over the previous one. It therefore lives
# outside the wiped work dir. (The first two 6a builds each used a
# fresh key because the keystore sat inside $W — fixed 2026-11-17.)
KEYSTORE="${KEYSTORE:-$HOME/.android/debug.keystore}"
mkdir -p "$(dirname "$KEYSTORE")"
if [ ! -f "$KEYSTORE" ]; then
  keytool -genkeypair -keystore "$KEYSTORE" -storepass android \
    -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 \
    -validity 10000 -dname 'CN=Android Debug,O=Android,C=US'
fi
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android \
  --key-pass pass:android --out "$W/MemoriaNotebook-7.apk" app-aligned.apk
"$BT/apksigner" verify "$W/MemoriaNotebook-7.apk"
echo "APK: $W/MemoriaNotebook-7.apk"
