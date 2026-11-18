//! Stage 7: on Android, notebook-core's voice module links the
//! bundled VOICEVOX core library (MIT, voicevox_core 0.17.1). The
//! .so is vendored under android/third_party/voicevox and also
//! packaged into the APK's lib dir by build-apk.sh; this only gives
//! the linker the search path for the cross build.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        let manifest = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
        );
        let dir = manifest.join("../../android/third_party/voicevox");
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}
