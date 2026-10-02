// Compile le composant de transcription (lumen-whisper) et le place là où
// Tauri l'attend : src-tauri/binaries/lumen-whisper-<triplet cible>.
import { execSync } from "node:child_process";
import { copyFileSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = join(root, "src-tauri");
const triple = execSync("rustc -vV").toString().match(/host: (\S+)/)[1];
const profile = process.argv.includes("--debug") ? "debug" : "release";
const ext = process.platform === "win32" ? ".exe" : "";

console.log(`› Compilation de lumen-whisper (${triple}, ${profile})…`);
execSync(`cargo build -p lumen-whisper ${profile === "release" ? "--release" : ""}`, {
  cwd: tauriDir,
  stdio: "inherit",
});
const built = join(tauriDir, "target", profile, `lumen-whisper${ext}`);
if (!existsSync(built)) throw new Error(`binaire introuvable : ${built}`);
mkdirSync(join(tauriDir, "binaries"), { recursive: true });
const dest = join(tauriDir, "binaries", `lumen-whisper-${triple}${ext}`);
copyFileSync(built, dest);
console.log(`✓ ${dest}`);
