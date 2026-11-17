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
cd "$W"
cp dex/classes.dex .
zip -q app-unsigned.apk classes.dex
zip -q -0 app-unsigned.apk lib/arm64-v8a/libnotebook_app.so
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
  --key-pass pass:android --out "$W/MemoriaNotebook-6a.apk" app-aligned.apk
"$BT/apksigner" verify "$W/MemoriaNotebook-6a.apk"
echo "APK: $W/MemoriaNotebook-6a.apk"
