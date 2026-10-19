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
    whisper_inapp(&target);
    // iPhone et iPad : remplaçants des fonctions du pont Swift pour la bibliothèque
    // dynamique que Cargo lie aussi (l'app garde les vraies, ios/stubs.c)
    if target("CARGO_CFG_TARGET_OS") == "ios" {
        println!("cargo:rerun-if-changed=ios/stubs.c");
        cc::Build::new().file("ios/stubs.c").compile("lumen_ios_stubs");
    }
    tauri_build::build()
}

/// Whisper dans l'app (src/whisper.rs) : sur iPhone et iPad, où le programme à part
/// `lumen-whisper` est interdit ; sur Mac, seulement pour l'essayer
/// (`LUMEN_WHISPER_INAPP=1`). whisper.cpp est compilé avec les en-têtes de la ggml de
/// llama.cpp et se lie à elle : une seule ggml dans le programme.
fn whisper_inapp(target: &dyn Fn(&str) -> String) {
    println!("cargo::rustc-check-cfg=cfg(whisper_inapp)");
    println!("cargo:rerun-if-env-changed=LUMEN_WHISPER_INAPP");
    println!("cargo:rerun-if-changed=whisper");
    let os = target("CARGO_CFG_TARGET_OS");
    let wanted = os == "ios" || (os == "macos" && std::env::var_os("LUMEN_WHISPER_INAPP").is_some());
    if !wanted {
        return;
    }
    let cmake = std::env::var("DEP_LLAMA_GGML_CMAKE_DIR").expect("en-têtes de ggml introuvables (llama-cpp-sys-2)");
    let include = std::path::Path::new(&cmake).join("..").join("..").join("include");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include("whisper")
        .include(include)
        .define("WHISPER_VERSION", "\"1.9.4\"")
        .define("NDEBUG", None)
        .opt_level(3)
        .warnings(false)
        .file("whisper/whisper.cpp")
        .file("whisper/shim.cpp")
        .compile("lumen_whisper_inapp");
    println!("cargo:rustc-cfg=whisper_inapp");
}
