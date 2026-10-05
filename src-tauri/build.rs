fn main() {
    // Windows : Media Foundation (le son des leçons compressé en AAC, `win::aac_m4a`)
    // manque aux éditions « N » de Windows sans le Media Feature Pack. Chargé seulement
    // au moment où il sert : Lumen démarre partout, et garde alors le son en WAV.
    let target = |k: &str| std::env::var(k).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        for dll in ["mfplat.dll", "mfreadwrite.dll"] {
            println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
        }
        println!("cargo:rustc-link-lib=delayimp");
    }
    tauri_build::build()
}
