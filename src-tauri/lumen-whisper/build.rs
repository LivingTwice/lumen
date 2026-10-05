// Windows : le runtime de Visual C++ intégré au programme, comme Tauri le fait
// pour l'app (staticVCRuntime), sinon un PC sans « Visual C++ Redistributable »
// ne pourrait pas transcrire. La bibliothèque C de Windows (ucrt), présente sur
// tout Windows 10 et 11, reste partagée. Rien à faire sur Mac.
// Repris de tauri-build (static_vcruntime.rs), lui-même tiré de
// https://github.com/ChrisDenton/static_vcruntime

use std::{env, fs, io::Write, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    // le msvcrt.lib que Rust demande toujours, remplacé par une bibliothèque presque vide
    let machine: &[u8] = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => &[0x64, 0x86],
        Ok("x86") => &[0x4C, 0x01],
        _ => return,
    };
    let bytes: &[u8] = &[
        1, 0, 94, 3, 96, 98, 60, 0, 0, 0, 1, 0, 0, 0, 0, 0, 132, 1, 46, 100, 114, 101, 99, 116, 118, 101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 60, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 16, 0, 46, 100, 114, 101, 99, 116, 118, 101, 0, 0, 0, 0, 1, 0, 0, 0, 3, 0, 4, 0, 0, 0,
    ];
    let out_dir = env::var("OUT_DIR").unwrap();
    let path = Path::new(&out_dir).join("msvcrt.lib");
    if let Ok(mut f) = fs::OpenOptions::new().write(true).create_new(true).open(path) {
        f.write_all(machine).unwrap();
        f.write_all(bytes).unwrap();
    }
    println!("cargo:rustc-link-search=native={out_dir}");
    // les runtimes qui entreraient en conflit, puis ceux qu'on veut
    for lib in ["libvcruntimed.lib", "vcruntime.lib", "vcruntimed.lib", "libcmtd.lib", "msvcrt.lib", "msvcrtd.lib", "libucrt.lib", "libucrtd.lib"] {
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:{lib}");
    }
    for lib in ["libcmt.lib", "libvcruntime.lib", "ucrt.lib"] {
        println!("cargo:rustc-link-arg=/DEFAULTLIB:{lib}");
    }
}
