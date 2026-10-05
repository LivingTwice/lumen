// Compile Lumen pour Windows depuis ce Mac : l'installateur (NSIS) et, si la clé
// des mises à jour est là, sa signature.
//   node scripts/build-windows.mjs           l'installateur
//   node scripts/build-windows.mjs --check   vérifie seulement que tout compile pour Windows
//   node scripts/build-windows.mjs --tests   compile aussi les tests pour Windows (sans les lancer)
// Résultat : src-tauri/target/windows/x86_64-pc-windows-msvc/release/bundle/nsis/Lumen_<version>_x64-setup.exe
//
// Outils, installés au besoin la première fois : rustup et sa cible Windows (le Rust
// de Homebrew ne compile que pour le Mac), cargo-xwin (il télécharge une fois les
// bibliothèques de Windows et de Visual C++ de Microsoft), LLVM et lld (compilateur
// et éditeur de liens à la manière de Microsoft), NSIS (l'installateur).
// Un dossier de compilation à part (target/windows) : la compilation pour le Mac n'est
// jamais à refaire après celle-ci.
import { execSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const TARGET = "x86_64-pc-windows-msvc";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = join(root, "src-tauri");
const targetDir = join(tauriDir, "target", "windows");
const cargoBin = join(homedir(), ".cargo", "bin");
const brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].find((p) => existsSync(p));
const prefix = brew ? execSync(`"${brew}" --prefix`).toString().trim() : "/opt/homebrew";

const env = {
  ...process.env,
  PATH: [cargoBin, join(prefix, "opt/llvm/bin"), join(prefix, "opt/lld/bin"), join(prefix, "bin"), process.env.PATH].join(":"),
  LIBCLANG_PATH: join(prefix, "opt/llvm/lib"),
  CARGO_TARGET_DIR: targetDir,
};

const step = (s) => console.log(`\n› ${s}`);
const fail = (s) => {
  console.error(`\n✗ ${s}`);
  process.exit(1);
};
const run = (cmd, cwd = root) => execSync(cmd, { cwd, stdio: "inherit", env });
const has = (cmd) => spawnSync("sh", ["-c", `command -v ${cmd}`], { env, stdio: "ignore" }).status === 0;

step("Outils de compilation pour Windows");
if (!existsSync(join(cargoBin, "rustup"))) {
  console.log("  Installation de rustup (pour compiler aussi pour Windows)…");
  run("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | RUSTUP_INIT_SKIP_PATH_CHECK=yes sh -s -- -y --profile minimal --no-modify-path");
}
run(`rustup target add ${TARGET} >/dev/null 2>&1 || rustup target add ${TARGET}`);
if (!has("cargo-xwin")) {
  console.log("  Installation de cargo-xwin…");
  run("cargo install cargo-xwin --locked");
}
const missing = [
  ["llvm", existsSync(join(prefix, "opt/llvm/bin/clang-cl"))],
  ["lld", existsSync(join(prefix, "opt/lld/bin/lld-link"))],
  ["nsis", has("makensis")],
].filter(([, ok]) => !ok);
if (missing.length) {
  if (!brew) fail("Homebrew est introuvable : il faut LLVM, lld et NSIS pour compiler la version Windows.");
  console.log(`  Installation de ${missing.map(([n]) => n).join(", ")}…`);
  run(`"${brew}" install ${missing.map(([n]) => n).join(" ")}`);
}
console.log("  ok");

// whisper-rs-sys regarde le système qui compile (macOS) au lieu de la cible : il réclame
// la bibliothèque ggml-blas des Mac (une bibliothèque vide en tient lieu, rien ne l'appelle
// sous Windows) et oublie celle du registre de Windows (advapi32), qu'on ajoute. Sur un
// PC, il fait ce qu'il faut.
const crossLib = join(targetDir, "cross-lib");
mkdirSync(crossLib, { recursive: true });
writeFileSync(join(crossLib, "ggml-blas.lib"), "!<arch>\n");
if (/\s/.test(crossLib)) fail(`le chemin du projet ne doit pas contenir d'espace : ${crossLib}`);
env.RUSTFLAGS = `-L native=${crossLib} -l advapi32`;

if (process.argv.includes("--check") || process.argv.includes("--tests")) {
  step("Vérification de la compilation pour Windows");
  // Tauri veut le composant de transcription : un fichier vide suffit pour vérifier
  const placeholder = join(tauriDir, "binaries", `lumen-whisper-${TARGET}.exe`);
  mkdirSync(join(tauriDir, "binaries"), { recursive: true });
  if (!existsSync(placeholder)) writeFileSync(placeholder, "");
  run(`cargo xwin check --workspace --target ${TARGET}`, tauriDir);
  if (process.argv.includes("--tests")) run(`cargo xwin test --no-run --release --lib --target ${TARGET} -p lumen`, tauriDir);
  console.log("\n✓ Lumen compile pour Windows");
  process.exit(0);
}

step("Composant de transcription (lumen-whisper.exe)");
run(`cargo xwin build -p lumen-whisper --release --target ${TARGET}`, tauriDir);
const sidecar = join(targetDir, TARGET, "release", "lumen-whisper.exe");
if (!existsSync(sidecar)) fail(`introuvable : ${sidecar}`);
mkdirSync(join(tauriDir, "binaries"), { recursive: true });
copyFileSync(sidecar, join(tauriDir, "binaries", `lumen-whisper-${TARGET}.exe`));
console.log("  ok");

step("Lumen pour Windows (une vingtaine de minutes la première fois)");
// la clé des mises à jour signe l'installateur ; sans elle, pas de signature (essai)
const key = join(homedir(), ".tauri", "lumen-updater.key");
if (!env.TAURI_SIGNING_PRIVATE_KEY && existsSync(key)) {
  env.TAURI_SIGNING_PRIVATE_KEY = key;
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "";
}
const unsigned = env.TAURI_SIGNING_PRIVATE_KEY ? "" : ` --config '{"bundle":{"createUpdaterArtifacts":false}}'`;
run(`npx tauri build --runner cargo-xwin --target ${TARGET} --bundles nsis${unsigned}`);

const version = JSON.parse(readFileSync(join(tauriDir, "tauri.conf.json"), "utf8")).version;
const setup = join(targetDir, TARGET, "release", "bundle", "nsis", `Lumen_${version}_x64-setup.exe`);
if (!existsSync(setup)) fail(`installateur introuvable : ${setup}`);
console.log(`\n✓ ${setup}${existsSync(`${setup}.sig`) ? "\n✓ signature de mise à jour" : ""}`);
