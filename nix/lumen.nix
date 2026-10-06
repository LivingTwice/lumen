# Lumen sous Linux : interface (npm), cœur Rust (Tauri, llama.cpp, whisper.cpp),
# composant de transcription à côté du binaire, dictionnaires livrés, et
# l'enveloppe qui met à sa portée GStreamer (vidéo), yt-dlp et ffmpeg.
{ lib
, rustPlatform
, buildNpmPackage
, nodejs
, stdenv
, pkg-config
, cmake
, clang
, llvmPackages
, makeWrapper
, wrapGAppsHook3
, vulkan-headers
, vulkan-loader
, spirv-headers
, shaderc
, glib
, gsettings-desktop-schemas
, gtk3
, webkitgtk_4_1
, libsoup_3
, libxkbcommon
, libglvnd
, wayland
, gst_all_1
, yt-dlp
, ffmpeg
, src
}:

let
  version = (lib.importJSON ../src-tauri/tauri.conf.json).version;

  # ---------- interface (dist embarqué par Tauri) ----------
  frontend = buildNpmPackage {
    pname = "lumen-frontend";
    inherit version;
    nodejs = nodejs;
    src = lib.cleanSourceWith {
      inherit src;
      filter = path: type:
        let rel = lib.removePrefix (toString src + "/") (toString path);
        in
        type == "directory" || builtins.elem rel [
          "package.json"
          "package-lock.json"
          "index.html"
          "vite.config.ts"
          "tsconfig.json"
        ] || lib.hasPrefix "src/" rel;
    };
    npmDepsHash = "sha256-pPvISSoa0xiuDU5k+jEG6qvPkZ7Zmqg6GWmEQU4nyjM=";
    buildPhase = ''
      runHook preBuild
      npm run build
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };
in
rustPlatform.buildRustPackage {
  pname = "lumen";
  inherit version;

  # le cœur Rust avec son dossier de ressources (dictionnaires), sans le reste
  src = lib.cleanSourceWith {
    inherit src;
    filter = path: type:
      let rel = lib.removePrefix (toString src + "/") (toString path);
      in
      type == "directory" || lib.hasPrefix "src-tauri/" rel;
  };
  sourceRoot = "source/src-tauri";
  cargoLock.lockFile = ../src-tauri/Cargo.lock;
  doCheck = false;
  # Sans la caractéristique custom-protocol de Tauri (celle que « tauri build »
  # active), l'app cherche son interface sur le serveur de développement
  # (localhost:1420) au lieu de l'embarquer
  buildFeatures = [ "tauri/custom-protocol" ];

  # les bibliothèques de Tauri sous Linux (WebKitGTK), le compilateur de
  # shaders de Vulkan, les moteurs de construction
  nativeBuildInputs = [
    pkg-config
    cmake
    llvmPackages.libclang
    makeWrapper
    wrapGAppsHook3
    shaderc
  ];
  buildInputs = [
    # Vulkan : la bibliothèque est liée à l'app (les ICD du pilote viennent du
    vulkan-headers
    vulkan-loader
    spirv-headers
    glib
    gsettings-desktop-schemas
    gtk3
    webkitgtk_4_1
    libsoup_3
    libxkbcommon
    libglvnd
    wayland
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
    gst_all_1.gst-plugins-ugly
    gst_all_1.gst-libav
  ];

  # bindgen (llama.cpp, whisper.cpp) lit les en-têtes avec le libclang de nix,
  # pour toutes les constructions, y compris celle du composant de transcription.
  # Les en-têtes de la libc ne sont pas dans ses chemins par défaut : sans ceci,
  # bindgen cherche stdio.h dans le système (réfusé dans le bac à sable de Nix).
  env.LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
  env.BINDGEN_EXTRA_CLANG_ARGS = "-I${lib.getDev stdenv.cc.libc}/include";

  # lumen-whisper d'abord : le script de construction de Tauri veut le trouver,
  # portant le triplet du système, dans binaries/. Les scripts de construction
  # de llama.cpp et whisper.cpp lancent leur propre CMake : les paquets de
  # Vulkan ne s'y trouvent que par ce chemin.
  preBuild = ''
    export CMAKE_PREFIX_PATH="${lib.concatStringsSep ":" [ vulkan-headers vulkan-loader spirv-headers ]}"
    export TAURI_CONFIG='{"build":{"beforeBuildCommand":"","frontendDist":"${frontend}"},"bundle":{"createUpdaterArtifacts":false}}'
    cargo build -p lumen-whisper --release --offline
    mkdir -p binaries
    cp target/release/lumen-whisper binaries/lumen-whisper-${stdenv.hostPlatform.config}
  '';

  # la phase de construction par défaut (cargoBuildHook) compile tout l'espace
  # de travail : lumen-whisper (déjà prêt) et lumen

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin $out/share/lumen
    cp target/${stdenv.hostPlatform.config}/release/lumen target/${stdenv.hostPlatform.config}/release/lumen-whisper $out/bin/
    cp -r resources/dicts $out/share/lumen/dicts
    runHook postInstall
  '';

  # GStreamer pour la vidéo et le son des leçons (H.264, AAC), yt-dlp et ffmpeg
  # pour YouTube et l'audio des leçons, les ressources là où Lumen les attend.
  # (Le contournement du rendu DMABUF de WebKitGTK vit dans lib.rs, pour tous
  # les paquets : nix, deb, appimage, développement.)
  postFixup = ''
    wrapProgram $out/bin/lumen \
      --set LUMEN_RESOURCES $out/share/lumen \
      --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${lib.makeSearchPath "lib/gstreamer-1.0" [
        gst_all_1.gst-plugins-base
        gst_all_1.gst-plugins-good
        gst_all_1.gst-plugins-bad
        gst_all_1.gst-plugins-ugly
        gst_all_1.gst-libav
      ]}" \
      --prefix PATH : ${lib.makeBinPath [ yt-dlp ffmpeg ]}
  '';

  meta = {
    description = "Apprendre les langues en lisant et en écoutant, avec une IA qui tourne sur place";
    homepage = "https://github.com/LivingTwice/lumen";
    license = lib.licenses.gpl3Plus;
    mainProgram = "lumen";
    platforms = [ "x86_64-linux" ];
  };
}
