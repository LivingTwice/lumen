# Environnement complet pour développer Lumen sous Linux :
# npm run dev (navigateur), npm run app:dev (vraie app), scripts/*.mjs.
{ lib
, rustc
, cargo
, rust-analyzer
, nodejs
, pkg-config
, cmake
, llvmPackages
, glib
, gsettings-desktop-schemas
, gtk3
, webkitgtk_4_1
, libsoup_3
, libxkbcommon
, wayland
, gst_all_1
, yt-dlp
, ffmpeg
}:

llvmPackages.stdenv.mkDerivation {
  name = "lumen-dev";

  # nativeBuildInputs : les .pc se trouvent par pkg-config, les outils dans le PATH
  nativeBuildInputs = [
    rustc
    cargo
    rust-analyzer
    nodejs
    pkg-config
    cmake
    glib
    gsettings-desktop-schemas
    gtk3
    webkitgtk_4_1
    libsoup_3
    libxkbcommon
    wayland
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
    gst_all_1.gst-plugins-ugly
    gst_all_1.gst-libav
    yt-dlp
    ffmpeg
  ];

  # bindgen (llama.cpp, whisper.cpp) lit les en-têtes avec libclang ; ceux de
  # la libc doivent être donnés (sinon stdio.h manque dans le bac à sable)
  LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
  BINDGEN_EXTRA_CLANG_ARGS = "-I${lib.getDev llvmPackages.stdenv.cc.libc}/include";

  # dialogues GTK, schémas de réglages, et vidéos qui se lisent dans la fenêtre
  # (le contournement du rendu DMABUF de WebKitGTK vit dans lib.rs)
  shellHook = ''
    export XDG_DATA_DIRS="${gsettings-desktop-schemas}/share/gsettings-schemas/${gsettings-desktop-schemas.name}:${gtk3}/share/gsettings-schemas/${gtk3.name}:$XDG_DATA_DIRS"
    export GST_PLUGIN_SYSTEM_PATH_1_0="${lib.makeSearchPath "lib/gstreamer-1.0" [
      gst_all_1.gst-plugins-base
      gst_all_1.gst-plugins-good
      gst_all_1.gst-plugins-bad
      gst_all_1.gst-plugins-ugly
      gst_all_1.gst-libav
    ]}"
  '';
}
