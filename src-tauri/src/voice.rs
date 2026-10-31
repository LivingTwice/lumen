//! Voix naturelle pour prononcer un mot ou une expression : Supertonic 3
//! (modèle neuronal, 31 langues dont les six de Lumen), exécuté par l'outil
//! officiel de sherpa-onnx.
//!
//! Le moteur (~20 Mo, `sherpa.rs`, partagé avec Parakeet) et le modèle (~130 Mo)
//! se téléchargent à part, comme les modèles d'IA. Rien ne tourne en permanence :
//! l'outil est lancé à la demande puis s'arrête. Chaque prononciation est gardée
//! en cache (`media/voice`), le même mot revient donc instantanément.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
#[cfg(not(target_os = "ios"))]
use std::time::Duration;

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::models::{self, ModelInfo};

/// Les langues de Lumen portent le même code chez Supertonic.
use crate::text::VOICE_LANGS;

/// Locuteur du modèle : 0 à 4 = voix féminines F1 à F5, 5 à 9 = masculines M1
/// à M5 (ordre de voice.bin). « f » et « m » (premiers réglages) valent F1 et
/// M1. Jusqu'à 186 : le modèle vits chinois a 187 locuteurs (Supertonic, lui,
/// n'en attend que dix : un numéro trop grand y échoue à la synthèse).
fn speaker(voice: &str) -> u32 {
    match voice {
        "m" => 5,
        v => v.parse::<u32>().ok().filter(|n| *n <= 186).unwrap_or(0),
    }
}


/// Dossier des prononciations en cache (dans `media/`, lisible par l'interface).
fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("media").join("voice")
}

/// Fichier de cache d'une prononciation (même texte, même voix : même fichier).
pub fn cached_path(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(format!("{}\u{1}{}\u{1}{lang}\u{1}{text}", m.id, speaker(voice)));
    cache_dir(data_dir).join(format!("{}.wav", &hex::encode(h.finalize())[..20]))
}

/// iPhone et iPad : la bibliothèque sherpa-onnx dans l'app (sherpa_ios.rs), même résultat.
#[cfg(target_os = "ios")]
async fn run_engine(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str, threads: u32, out: &Path) -> Result<()> {
    let model = models::path_of(data_dir, m);
    let (lang, text, sid) = (lang.to_string(), text.to_string(), speaker(voice));
    let (rate, samples) = tokio::task::spawn_blocking(move || crate::sherpa_ios::tts(&model, &lang, &text, sid, threads.min(4))).await??;
    let pcm: Vec<i16> = samples.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
    tokio::fs::write(out, write_wav(rate, &pcm)).await?;
    Ok(())
}

/// Linux : le moteur téléchargé attend l'éditeur de liens du système
/// (/lib64/ld-linux…), qui n'existe pas sous NixOS. On le lance avec celui de
/// Lumen et les bibliothèques déjà chargées par l'app (la libc, libstdc++…) :
/// le moteur trouve ainsi tout ce qu'il lui faut, sur toute distribution.
/// (Retourne None si l'ELF de Lumen ne se laisse pas lire : lancement direct.)
#[cfg(target_os = "linux")]
fn engine_command(data_dir: &Path) -> Option<tokio::process::Command> {
    let exe = std::fs::read(std::fs::read_link("/proc/self/exe").ok()?).ok()?;
    let interp = elf_interpreter(&exe)?;
    // les bibliothèques du moteur : la libc et ses sœurs viennent du dossier de
    // l'éditeur de liens lui-même (une seule libc, celle qui l'accompagne —
    // /proc/self/maps peut contenir une seconde libc chargée en privé par une
    // bibliothèque de l'app, qui mélangée à l'autre écrase la pile du moteur) ;
    // libstdc++ et libgcc_s, de la première chargée par l'app.
    let mut dirs: Vec<String> = vec![
        crate::sherpa::dir(data_dir).join("lib").display().to_string(),
        Path::new(&interp).parent().map(|d| d.display().to_string())?,
    ];
    let mut have_cxx = false;
    let mut have_gcc = false;
    for line in std::fs::read_to_string("/proc/self/maps").ok()?.lines() {
        let Some((_, path)) = line.rsplit_once(' ') else { continue };
        let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) else { continue };
        let Some(dir) = Path::new(path).parent().map(|d| d.display().to_string()) else { continue };
        if !have_cxx && name.starts_with("libstdc++.so") {
            have_cxx = true;
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        } else if !have_gcc && name.starts_with("libgcc_s.so") {
            have_gcc = true;
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        if have_cxx && have_gcc {
            break;
        }
    }
    let mut cmd = crate::proc::tokio_command(&interp);
    cmd.arg("--library-path").arg(dirs.join(":")).arg(crate::sherpa::bin(data_dir, crate::sherpa::Tool::Tts));
    Some(cmd)
}

/// Lit l'éditeur de liens d'un exécutable ELF64 (segment PT_INTERP) : c'est
/// lui qui charge le binaire et ses bibliothèques. Sous Linux sans paquet
/// (NixOS), le moteur téléchargé attend celui du système, absent : Lumen lui
/// prête le sien.
#[cfg(target_os = "linux")]
fn elf_interpreter(exe: &[u8]) -> Option<String> {
    if exe.first()? != &0x7f || exe.get(1)? != &b'E' {
        return None;
    }
    let u16at = |i: usize| -> Option<u16> {
        let b = exe.get(i..i + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    };
    let u64at = |i: usize| -> Option<u64> {
        let b = exe.get(i..i + 8)?;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    };
    let phoff = u64at(0x20)? as usize;
    let phsize = u16at(0x36)? as usize;
    let phnum = u16at(0x38)? as usize;
    for i in 0..phnum {
        let p = phoff + i * phsize;
        let kind = u32::from_le_bytes([*exe.get(p)?, *exe.get(p + 1)?, *exe.get(p + 2)?, *exe.get(p + 3)?]);
        if kind == 3 {
            let off = u64at(p + 8)? as usize;
            let end = exe[off..].iter().position(|b| *b == 0)? + off;
            return std::str::from_utf8(&exe[off..end]).ok().map(String::from);
        }
    }
    None
}
/// Lance le moteur une fois : `text` prononcé dans `out` (WAV PCM 16 bits mono).
#[cfg(not(target_os = "ios"))]
async fn run_engine(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str, threads: u32, out: &Path) -> Result<()> {
    let model = models::path_of(data_dir, m);
    let f = |name: &str| model.join(name);
    #[cfg(target_os = "linux")]
    let mut cmd = engine_command(data_dir).unwrap_or_else(|| crate::proc::tokio_command(crate::sherpa::bin(data_dir, crate::sherpa::Tool::Tts)));
    #[cfg(not(target_os = "linux"))]
    let mut cmd = crate::proc::tokio_command(crate::sherpa::bin(data_dir, crate::sherpa::Tool::Tts));
    // Supertonic dit les 31 langues ; le chinois a son modèle vits (fanchen-C)
    // : autres drapeaux. Les fst réécrivent dates et nombres, les homonymes
    // se lisent par contexte.
    let supertonic = |cmd: &mut tokio::process::Command| {
        cmd.arg(format!("--supertonic-duration-predictor={}", f("duration_predictor.int8.onnx").display()))
            .arg(format!("--supertonic-text-encoder={}", f("text_encoder.int8.onnx").display()))
            .arg(format!("--supertonic-vector-estimator={}", f("vector_estimator.int8.onnx").display()))
            .arg(format!("--supertonic-vocoder={}", f("vocoder.int8.onnx").display()))
            .arg(format!("--supertonic-tts-json={}", f("tts.json").display()))
            .arg(format!("--supertonic-unicode-indexer={}", f("unicode_indexer.bin").display()))
            .arg(format!("--supertonic-voice-style={}", f("voice.bin").display()))
            .arg(format!("--lang={lang}"))
            .arg(format!("--sid={}", speaker(voice)))
            // 8 étapes : la meilleure qualité (5 n'irait que 0,2 s plus vite)
            .arg("--num-steps=8")
            .arg(format!("--num-threads={threads}"))
            .arg(format!("--output-filename={}", out.display()))
            .arg(text)
            .kill_on_drop(true);
    };
    #[cfg(target_os = "linux")]
    if m.id == crate::models::ZH_VOICE.id {
        cmd.arg(format!("--vits-model={}", f("vits-zh-hf-fanchen-C.onnx").display()))
            .arg(format!("--vits-lexicon={}", f("lexicon.txt").display()))
            .arg(format!("--vits-tokens={}", f("tokens.txt").display()))
            .arg(format!("--tts-rule-fsts={},{}", f("date.fst").display(), f("number.fst").display()))
            .arg(format!("--tts-rule-fars={}", f("new_heteronym.fst").display()))
            .arg(format!("--sid={}", speaker(voice)))
            .arg(format!("--num-threads={threads}"))
            .arg(format!("--output-filename={}", out.display()))
            .arg(text)
            .kill_on_drop(true);
    } else {
        supertonic(&mut cmd);
    }
    #[cfg(not(target_os = "linux"))]
    supertonic(&mut cmd);
    // LUMEN_DEBUG_VOICE=1 : l'appel exact sur la console, pour le rejouer à la main
    // et comparer
    if std::env::var_os("LUMEN_DEBUG_VOICE").is_some() {
        eprintln!("lumen voix : {:?}", cmd.as_std());
    }
    let res = tokio::time::timeout(Duration::from_secs(120), cmd.output())
        .await
        .map_err(|_| anyhow!(crate::i18n::t("la voix a mis trop de temps à répondre", "the voice took too long to answer")))?
        .map_err(|e| anyhow!(crate::tr!("le moteur de voix n'a pas pu être lancé : {}", "the voice engine couldn't be started: {}", e)))?;
    if !res.status.success() || !out.exists() {
        // la raison sur la console : ce moteur vit hors de l'app, ses erreurs ne se voient pas autrement
        let why: String = String::from_utf8_lossy(&res.stderr).trim().chars().take(300).collect();
        eprintln!("lumen : moteur de voix échoué : {why}");
        let _ = tokio::fs::remove_file(out).await;
        return Err(anyhow!(crate::i18n::t("La voix n'a pas pu prononcer ce texte.", "The voice couldn't pronounce this text.")));
    }
    Ok(())
}

/// Prononce `text` et renvoie le chemin du fichier WAV (déjà en cache, ou créé).
pub async fn say(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str) -> Result<PathBuf> {
    if !model_speaks(m, lang) {
        return Err(anyhow!(crate::i18n::t("Cette langue n'a pas encore de voix naturelle.", "This language has no natural voice yet.")));
    }
    let dest = cached_path(data_dir, m, lang, text, voice);
    if dest.exists() {
        return Ok(dest);
    }
    tokio::fs::create_dir_all(cache_dir(data_dir)).await?;
    let tmp = dest.with_extension("tmp.wav");
    run_engine(data_dir, m, lang, text, voice, 4, &tmp).await?;
    let raw = tokio::fs::read(&tmp).await?;
    let _ = tokio::fs::remove_file(&tmp).await;
    let clean = polish_wav(&raw).unwrap_or(raw);
    tokio::fs::write(&dest, clean).await?;
    Ok(dest)
}

/// La voix naturelle parle cette langue (miroir de `NO_NATURAL_VOICE` côté interface).
pub fn speaks(lang: &str) -> bool {
    VOICE_LANGS.contains(&lang)
}

/// Ce modèle de voix installé dit-il cette langue ? Supertonic dit les 31
/// langues de `VOICE_LANGS` ; sous Linux, le modèle vits chinois dit le chinois.
pub fn model_speaks(m: &ModelInfo, lang: &str) -> bool {
    #[cfg(target_os = "linux")]
    if m.id == crate::models::ZH_VOICE.id {
        return lang == "zh";
    }
    #[cfg(not(target_os = "linux"))]
    let _ = m;
    VOICE_LANGS.contains(&lang)
}

/// Prononce `text` et renvoie le WAV : la voix naturelle si elle parle la
/// langue et est installée (`m`), sinon la voix du système de Linux, espeak-ng
/// (le WebView de Linux n'en a pas : WebKitGTK embarque flite, anglais seul).
pub async fn say_any(data_dir: &Path, m: Option<&ModelInfo>, lang: &str, text: &str, voice: &str) -> Result<PathBuf> {
    // LUMEN_DEBUG_VOICE=1 : la voix choisie sur la console (naturelle ou espeak-ng)
    if std::env::var_os("LUMEN_DEBUG_VOICE").is_some() {
        eprintln!("lumen voix : {} pour « {text} »", match m.filter(|m| model_speaks(m, lang)) {
            Some(m) => format!("voix naturelle ({})", m.id),
            None => "espeak-ng".to_string(),
        });
    }
    if let Some(m) = m.filter(|m| model_speaks(m, lang)) {
        return say(data_dir, m, lang, text, voice).await;
    }
    #[cfg(target_os = "linux")]
    return espeak_say(data_dir, lang, text, voice).await;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (lang, text, voice);
        match m {
            None => Err(anyhow!(crate::i18n::t("NO_VOICE: aucune voix naturelle n'est installée", "NO_VOICE: no natural voice is installed"))),
            _ => Err(anyhow!(crate::i18n::t("Cette langue n'a pas encore de voix naturelle.", "This language has no natural voice yet."))),
        }
    }
}

/// La voix du système de Linux : espeak-ng, repéré par la variable
/// `LUMEN_ESPEAK` (posée par le paquet nix), sinon dans le PATH. Sans lui,
/// l'erreur laisse la parole au WebView.
#[cfg(target_os = "linux")]
fn espeak_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LUMEN_ESPEAK") {
        return (p.as_os_str() != "").then(|| PathBuf::from(p));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("espeak-ng")).find(|p| p.is_file())
}

/// Voix et variante d'espeak-ng pour un réglage de Lumen : la même locutrice
/// que la voix naturelle (0 à 4 = féminines, 5 à 9 = masculines, « f » et « m »
/// en premières). Le mandarin s'appelle « cmn » chez espeak-ng.
#[cfg(target_os = "linux")]
fn espeak_voice(lang: &str, voice: &str) -> String {
    let n = speaker(voice);
    // espeak-ng n'a que quelques variantes : les grands numéros (les 187
    // locuteurs du vits chinois) s'y replient sur la dernière de chaque genre
    let variant = if n >= 5 { format!("m{}", (n - 4).min(5)) } else { format!("f{}", (n + 1).min(5)) };
    let lang = if lang == "zh" { "cmn" } else { lang };
    format!("{lang}+{variant}")
}

/// Cache d'une prononciation d'espeak-ng, à part de celui de la voix naturelle
/// (une voix téléchargée ensuite doit se faire entendre).
#[cfg(target_os = "linux")]
pub(crate) fn espeak_path(data_dir: &Path, lang: &str, text: &str, voice: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(format!("espeak\u{1}{lang}\u{1}{}\u{1}{text}", speaker(voice)));
    cache_dir(data_dir).join(format!("{}.wav", &hex::encode(h.finalize())[..20]))
}

/// Dit `text` par espeak-ng, WAV mis en cache comme la voix naturelle (mêmes
/// silences taillés, même nivellement du volume).
#[cfg(target_os = "linux")]
async fn espeak_say(data_dir: &Path, lang: &str, text: &str, voice: &str) -> Result<PathBuf> {
    let dest = espeak_path(data_dir, lang, text, voice);
    if dest.exists() {
        return Ok(dest);
    }
    let bin = espeak_bin().ok_or_else(|| anyhow!(crate::i18n::t("espeak-ng est introuvable", "espeak-ng cannot be found")))?;
    tokio::fs::create_dir_all(cache_dir(data_dir)).await?;
    let tmp = dest.with_extension("tmp.wav");
    let mut cmd = crate::proc::tokio_command(&bin);
    cmd.arg("-v").arg(espeak_voice(lang, voice))
        .arg("-w").arg(&tmp)
        .arg(text);
    // LUMEN_DEBUG_VOICE=1 : l'appel exact, comme pour le moteur de la voix naturelle
    if std::env::var_os("LUMEN_DEBUG_VOICE").is_some() {
        eprintln!("lumen voix : {:?}", cmd.as_std());
    }
    let out = cmd.output().await?;
    let raw = tokio::fs::read(&tmp).await?;
    let _ = tokio::fs::remove_file(&tmp).await;
    if !out.status.success() || raw.is_empty() {
        return Err(anyhow!(crate::i18n::t("espeak-ng n'a pas pu dire ce mot", "espeak-ng couldn't speak this word")));
    }
    let clean = polish_wav(&raw).unwrap_or(raw);
    tokio::fs::write(&dest, clean).await?;
    Ok(dest)
}

/// Échantillons d'un WAV PCM 16 bits mono, avec leur fréquence.
pub(crate) fn read_pcm(wav: &[u8]) -> Option<(u32, Vec<i16>)> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut i = 12;
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let len = u32::from_le_bytes(wav[i + 4..i + 8].try_into().ok()?) as usize;
        let body = &wav[i + 8..(i + 8 + len).min(wav.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = Some(body);
        }
        i += 8 + len + (len & 1);
    }
    if channels != 1 || bits != 16 || rate == 0 {
        return None;
    }
    Some((rate, data?.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()))
}

pub(crate) fn write_wav(rate: u32, s: &[i16]) -> Vec<u8> {
    let bytes = (s.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for v in s {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Début et fin de la parole (au-dessus de 2 % de la crête), en échantillons.
pub(crate) fn speech_bounds(s: &[i16]) -> Option<(usize, usize, i32)> {
    let peak = s.iter().map(|v| (*v as i32).abs()).max().unwrap_or(0);
    if peak < 200 {
        return None;
    }
    let thr = (peak / 50).max(60);
    let first = s.iter().position(|v| (*v as i32).abs() > thr)?;
    let last = s.iter().rposition(|v| (*v as i32).abs() > thr)?;
    Some((first, last, peak))
}

/// Prépare le son pour une écoute immédiate : retire les silences du début et
/// de la fin (le modèle en ajoute près d'une demi-seconde de chaque côté), égalise
/// le volume et adoucit les bords pour éviter un clic. WAV PCM 16 bits mono.
pub fn polish_wav(wav: &[u8]) -> Option<Vec<u8>> {
    let (rate, s) = read_pcm(wav)?;
    let (first, last, peak) = speech_bounds(&s)?;
    let ms = |x: u32| (rate as usize * x as usize) / 1000;
    let a = first.saturating_sub(ms(30));
    let b = (last + ms(160)).min(s.len());
    let mut s = s[a..b].to_vec();
    let gain = (0.85 * 32767.0 / peak as f32).min(2.5);
    let fade = ms(8).max(1).min(s.len() / 2);
    let n = s.len();
    for (k, v) in s.iter_mut().enumerate() {
        let edge = k.min(n - 1 - k);
        let ramp = if edge < fade { edge as f32 / fade as f32 } else { 1.0 };
        *v = (*v as f32 * gain * ramp).round().clamp(-32768.0, 32767.0) as i16;
    }
    Some(write_wav(rate, &s))
}

// ---------- audio d'une leçon entière ----------

/// Au-delà, une phrase est coupée (aux virgules si possible) : le modèle
/// garde un débit naturel et la lanterne reste précise.
const CHUNK_MAX_WORDS: usize = 40;
/// Phrases prononcées en même temps (chacune lance le moteur).
const PARALLEL: usize = 3;

/// Morceau de texte prononcé d'un seul tenant.
struct Chunk {
    /// texte exact (pour les positions des mots)
    raw: String,
    /// position du début dans le texte de la leçon (unités UTF-16)
    base: usize,
    /// silence après le morceau, en secondes
    pause: f64,
}

fn chunks_of(text: &str, lang: &str) -> Vec<Chunk> {
    let tokens = crate::text::tokenize(text, lang);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (a, b) in crate::text::sentences(&tokens) {
        // coupe les phrases trop longues, de préférence après une virgule
        let mut start = a;
        let mut words = 0usize;
        let mut last_comma: Option<usize> = None;
        for i in a..b {
            if tokens[i].w {
                words += 1;
            } else if tokens[i].t.contains([',', ';', ':', '،', '、', '，']) {
                last_comma = Some(i + 1);
            }
            if words >= CHUNK_MAX_WORDS && i + 1 < b {
                let cut = last_comma.filter(|c| *c > start + 3).unwrap_or(i + 1);
                ranges.push((start, cut));
                start = cut;
                words = tokens[start..=i].iter().filter(|t| t.w).count();
                last_comma = None;
            }
        }
        if tokens[start..b].iter().any(|t| t.w) {
            ranges.push((start, b));
        }
    }
    let mut out = Vec::with_capacity(ranges.len());
    for (k, (a, b)) in ranges.iter().enumerate() {
        let raw: String = tokens[*a..*b].iter().map(|t| t.t.as_str()).collect();
        // pause : plus longue entre deux paragraphes, brève au milieu d'une phrase coupée
        let next_gap = ranges.get(k + 1).map(|(na, _)| tokens[*b..*na].iter().chain(tokens[*na..].iter().take_while(|t| !t.w)).any(|t| t.t.contains('\n')));
        let ends_sentence = tokens[*a..*b].iter().rev().find(|t| !t.t.trim().is_empty()).map(|t| !t.w && crate::text::ends_sentence(&t.t)).unwrap_or(false);
        let pause = match next_gap {
            Some(true) => 0.75,
            Some(false) if ends_sentence => 0.32,
            Some(false) => 0.12,
            None => 0.0,
        };
        out.push(Chunk { raw, base: tokens[*a].s, pause });
    }
    out
}

pub struct LessonAudio {
    pub path: PathBuf,
    /// horodatages par mot, au format de Lumen
    pub timings: Vec<[f64; 4]>,
    pub duration: f64,
}

/// Lit toute une leçon avec la voix naturelle : chaque phrase est prononcée à
/// part (trois à la fois), les morceaux sont assemblés avec de courtes pauses,
/// et la position de chaque phrase dans l'audio donne celle de ses mots.
/// L'audio est compressé en AAC (m4a) dans `media/`.
pub async fn lesson_audio(
    data_dir: &Path,
    m: &'static ModelInfo,
    lang: &str,
    text: &str,
    voice: &str,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64),
) -> Result<LessonAudio> {
    use futures_util::StreamExt;
    use std::sync::atomic::Ordering;
    if !model_speaks(m, lang) {
        return Err(anyhow!(crate::i18n::t("Cette langue n'a pas encore de voix naturelle.", "This language has no natural voice yet.")));
    }
    let chunks = chunks_of(text, lang);
    if chunks.is_empty() {
        return Err(anyhow!(crate::i18n::t("Cette leçon n'a pas de texte à lire.", "This lesson has no text to read.")));
    }
    let media = crate::media::media_dir(data_dir);
    let work = media.join(format!(".voice-{}", crate::media::new_stem()));
    tokio::fs::create_dir_all(&work).await?;
    let total = chunks.len();
    // chaque tâche a ses propres copies : elles tournent en parallèle
    let jobs: Vec<_> = chunks
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let out = work.join(format!("{k}.wav"));
            let spoken = c.raw.split_whitespace().collect::<Vec<_>>().join(" ");
            let (cancel, dir, lang, voice) = (cancel.clone(), data_dir.to_path_buf(), lang.to_string(), voice.to_string());
            async move {
                if cancel.load(Ordering::Relaxed) {
                    return Err(anyhow!("annulé"));
                }
                run_engine(&dir, m, &lang, &spoken, &voice, 2, &out).await?;
                let wav = tokio::fs::read(&out).await?;
                let _ = tokio::fs::remove_file(&out).await;
                read_pcm(&wav).ok_or_else(|| anyhow!(crate::i18n::t("son illisible", "unreadable sound")))
            }
        })
        .collect();
    let mut stream = futures_util::stream::iter(jobs).buffered(PARALLEL);
    let mut rate = 0u32;
    let mut all: Vec<i16> = Vec::new();
    let mut timings: Vec<[f64; 4]> = Vec::new();
    let mut done = 0usize;
    let result: Result<()> = async {
        while let Some(res) = stream.next().await {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("annulé"));
            }
            let (r, s) = res?;
            rate = r;
            let c = &chunks[done];
            let ms = |x: f64| (r as f64 * x / 1000.0) as usize;
            if let Some((first, last, _)) = speech_bounds(&s) {
                let a = first.saturating_sub(ms(20.0));
                let b = (last + ms(80.0)).min(s.len());
                let t0 = all.len() as f64 / r as f64 + (first - a) as f64 / r as f64;
                all.extend_from_slice(&s[a..b]);
                let t1 = all.len() as f64 / r as f64 - (b - last) as f64 / r as f64;
                crate::lingq::spread_words(&c.raw, lang, c.base, t0, t1, &mut timings);
            }
            all.extend(std::iter::repeat_n(0i16, (r as f64 * c.pause) as usize));
            done += 1;
            on_progress(done as f64 / total as f64 * 100.0);
        }
        Ok(())
    }
    .await;
    drop(stream);
    if let Err(e) = result {
        let _ = tokio::fs::remove_dir_all(&work).await;
        return Err(e);
    }
    // volume égalisé sur l'ensemble (les phrases gardent leurs nuances entre elles)
    level_volume(&mut all);
    let duration = all.len() as f64 / rate.max(1) as f64;
    let path = save_m4a(&work, &media, "voice", rate, &all).await;
    drop(all);
    let _ = tokio::fs::remove_dir_all(&work).await;
    Ok(LessonAudio { path: path?, timings, duration })
}

/// Change la fréquence d'un son (interpolation linéaire, assez fine pour la voix) :
/// l'encodeur AAC de Windows n'accepte que 44,1 et 48 kHz.
#[cfg(any(windows, test))]
pub(crate) fn resample(s: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || from == 0 || to == 0 || s.is_empty() {
        return s.to_vec();
    }
    let n = (s.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    let last = s.len() - 1;
    (0..n)
        .map(|i| {
            let x = i as f64 * step;
            let k = (x as usize).min(last);
            let f = x - k as f64;
            let (a, b) = (s[k] as f64, s[(k + 1).min(last)] as f64);
            (a + (b - a) * f).round().clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

/// Crête ramenée à 85 % du maximum (gain borné : un son presque muet reste discret).
pub(crate) fn level_volume(all: &mut [i16]) {
    if let Some(peak) = all.iter().map(|v| (*v as i32).abs()).max().filter(|p| *p > 0) {
        let gain = (0.85 * 32767.0 / peak as f32).min(2.5);
        for v in all.iter_mut() {
            *v = (*v as f32 * gain).round().clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// Enregistre un son assemblé dans `media/<horodatage>.<tag>.m4a`, compressé en
/// AAC (environ 0,5 Mo par minute au lieu de 5 Mo en WAV) ; en WAV si la compression
/// échoue (`afconvert` sur Mac, l'encodeur de Windows sur PC, AudioToolbox sur iPhone, `ffmpeg` sous Linux).
/// `work` : dossier de travail, où passe le WAV intermédiaire.
pub(crate) async fn save_m4a(work: &Path, media: &Path, tag: &str, rate: u32, samples: &[i16]) -> Result<PathBuf> {
    let wav_path = work.join(format!("{tag}.wav"));
    tokio::fs::write(&wav_path, write_wav(rate, samples)).await?;
    let stem = crate::media::new_stem();
    let m4a = media.join(format!("{stem}.{tag}.m4a"));
    #[cfg(target_os = "macos")]
    let converted = tokio::process::Command::new("afconvert")
        .args(["-f", "m4af", "-d", "aac", "-b", "64000"])
        .arg(&wav_path)
        .arg(&m4a)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    #[cfg(windows)]
    let converted = {
        let (out, pcm) = (m4a.clone(), samples.to_vec());
        tokio::task::spawn_blocking(move || crate::win::aac_m4a(rate, &pcm, &out)).await.map(|r| r.is_ok()).unwrap_or(false)
    };
    // iPhone et iPad : l'encodeur AAC d'AudioToolbox, appelé ici (compress.rs)
    #[cfg(target_os = "ios")]
    let converted = {
        let (out, pcm) = (m4a.clone(), samples.to_vec());
        tokio::task::spawn_blocking(move || crate::compress::write_aac(&out, rate, &pcm, 64_000)).await.map(|r| r.is_ok()).unwrap_or(false)
    };
    // Linux : ffmpeg s'il est installé (le paquet nix le met au chaud)
    #[cfg(target_os = "linux")]
    let converted = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-nostdin", "-loglevel", "error", "-i"])
        .arg(&wav_path)
        .args(["-c:a", "aac", "-b:a", "96000"])
        .arg(&m4a)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if converted && m4a.exists() {
        let _ = tokio::fs::remove_file(&wav_path).await;
        return Ok(m4a);
    }
    let wav = media.join(format!("{stem}.{tag}.wav"));
    tokio::fs::rename(&wav_path, &wav).await?;
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let bytes = (samples.len() * 2) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&bytes.to_le_bytes());
        for v in samples {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    #[test]
    fn polish_trims_silence_and_levels_volume() {
        let rate = 1000;
        // 400 ms de silence, 200 ms de son, 500 ms de silence
        let mut s = vec![0i16; 400];
        s.extend((0..200).map(|k| if k % 2 == 0 { 12000 } else { -12000 }));
        s.extend(vec![0i16; 500]);
        let out = polish_wav(&wav(&s, rate)).expect("wav valide");
        let n = (out.len() - 44) / 2;
        // 30 ms avant, 200 ms de son, 160 ms après
        assert_eq!(n, 30 + 200 + 159);
        let peak = out[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        // crête ramenée à 85 % du maximum
        assert!((27000..=28500).contains(&peak), "volume égalisé : {peak}");
    }

    #[test]
    fn chunks_follow_sentences_and_paragraphs() {
        let c = chunks_of("One two. Three four?\n\nFive six", "en");
        let raw: Vec<&str> = c.iter().map(|c| c.raw.trim()).collect();
        assert_eq!(raw, ["One two.", "Three four?", "Five six"]);
        assert_eq!(c[0].pause, 0.32);
        assert_eq!(c[1].pause, 0.75);
        assert_eq!(c[2].pause, 0.0);
        // positions UTF-16 du début de chaque morceau
        assert_eq!(c[0].base, 0);
        assert!(c[2].raw.trim_start().starts_with("Five"));
        // phrase très longue : coupée en morceaux d'au plus 40 mots
        let long = (0..95).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ") + ".";
        let parts = chunks_of(&long, "en");
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.raw.split_whitespace().count() <= 41));
    }

    #[test]
    fn resample_keeps_duration_and_shape() {
        // 24 kHz (voix de Gemini) vers 48 kHz : deux fois plus d'échantillons, même allure
        let s: Vec<i16> = (0..2400).map(|k| ((k as f64 / 24.0).sin() * 10000.0) as i16).collect();
        let r = resample(&s, 24_000, 48_000);
        assert_eq!(r.len(), 4800);
        assert_eq!(r[0], s[0]);
        assert_eq!(r[200], s[100]);
        assert!((r[201] as i32 - (s[100] as i32 + s[101] as i32) / 2).abs() <= 1);
        assert_eq!(resample(&s, 48_000, 48_000), s);
    }

    #[test]
    fn polish_leaves_unknown_formats_alone() {
        assert!(polish_wav(b"pas un wav").is_none());
        assert!(polish_wav(&wav(&[0; 100], 1000)).is_none());
    }

    /// Un ELF64 minimal avec un segment PT_INTERP : l'en-tête (64 octets),
    /// un programme d'en-tête (56 octets) de type 3, et le chemin.
    #[cfg(target_os = "linux")]
    fn elf_with_interpreter(path: &str) -> Vec<u8> {
        let mut e = vec![0u8; 64];
        e[0] = 0x7f;
        e[1] = b'E';
        e[2] = b'L';
        e[3] = b'F';
        e[4] = 2; // ELF64
        e[5] = 1; // little endian
        e[0x20..0x28].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        e[0x36..0x38].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        e[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let mut p = vec![0u8; 56];
        p[0..4].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        let path_off = 64 + 56;
        p[8..16].copy_from_slice(&(path_off as u64).to_le_bytes()); // p_offset
        p[32..40].copy_from_slice(&(path.len() as u64 + 1).to_le_bytes()); // p_filesz
        let mut out = e;
        out.extend(p);
        out.extend_from_slice(path.as_bytes());
        out.push(0);
        out
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn elf_interpreter_is_found() {
        let elf = elf_with_interpreter("/nix/store/…-glibc/lib/ld-linux-x86-64.so.2");
        assert_eq!(elf_interpreter(&elf).as_deref(), Some("/nix/store/…-glibc/lib/ld-linux-x86-64.so.2"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn elf_interpreter_rejects_non_elf() {
        assert!(elf_interpreter(b"pas un elf").is_none());
        assert!(elf_interpreter(b"\x7fEL\x00").is_none()); // magie incomplète
        assert!(elf_interpreter(&[]).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn espeak_voices_follow_the_speaker() {
        // la même locutrice que Supertonic : 0 à 4 féminines, 5 à 9 masculines
        assert_eq!(espeak_voice("zh", "0"), "cmn+f1");
        assert_eq!(espeak_voice("zh", "f"), "cmn+f1");
        assert_eq!(espeak_voice("zh", "m"), "cmn+m1");
        assert_eq!(espeak_voice("zh", "7"), "cmn+m3");
        assert_eq!(espeak_voice("it", "3"), "it+f4");
        // un réglage inconnu retombe sur la première voix féminine
        assert_eq!(espeak_voice("fr", "non"), "fr+f1");
        // les grands numéros (187 locuteurs du vits chinois) se replient
        assert_eq!(espeak_voice("zh", "150"), "cmn+m5");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn chinese_speakers_have_their_own_cache() {
        // avant, les numéros au-delà de 9 se confondaient dans le cache
        let a = cached_path(Path::new("/tmp"), &crate::models::ZH_VOICE, "zh", "你", "10");
        let b = cached_path(Path::new("/tmp"), &crate::models::ZH_VOICE, "zh", "你", "150");
        assert_ne!(a, b);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn espeak_cache_is_its_own() {
        // même texte, même voix : même fichier ; la voix change le fichier
        let a = espeak_path(Path::new("/tmp"), "zh", "你好", "0");
        assert_eq!(espeak_path(Path::new("/tmp"), "zh", "你好", "0"), a);
        assert_ne!(espeak_path(Path::new("/tmp"), "zh", "你好", "5"), a);
        assert!(a.to_string_lossy().contains("media/voice"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn chinese_voice_model_speaks_chinese_only() {
        // Supertonic dit les 31 langues, le modèle vits dit le chinois seul
        let zh = &crate::models::ZH_VOICE;
        let super3 = crate::models::CATALOG.iter().find(|m| m.id == "supertonic-3").unwrap();
        assert!(model_speaks(super3, "fr"));
        assert!(!model_speaks(super3, "zh"));
        assert!(model_speaks(zh, "zh"));
        assert!(!model_speaks(zh, "it"));
        // il se trouve comme les autres modèles (téléchargement, suppression)
        assert_eq!(crate::models::find("vits-zh-fanchen-c").map(|m| m.id), Some("vits-zh-fanchen-c"));
    }
}

/// Test réel (télécharge le moteur dans un dossier jetable) :
/// `LUMEN_VOICE_MODEL=/chemin/sherpa-onnx-supertonic-3-tts-int8-2026-05-11 cargo test --lib voice_live -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;
    use std::time::Instant;

    // liens symboliques du Mac : essai réservé au Mac
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn voice_live() {
        let model_src = PathBuf::from(std::env::var("LUMEN_VOICE_MODEL").expect("LUMEN_VOICE_MODEL"));
        let data = std::env::temp_dir().join("lumen-voice-live");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(data.join("models")).unwrap();
        let m = models::CATALOG.iter().find(|m| m.kind == "tts").unwrap();
        std::os::unix::fs::symlink(&model_src, models::path_of(&data, m)).unwrap();

        let t = Instant::now();
        crate::sherpa::ensure(&data, crate::sherpa::Tool::Tts, Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap();
        println!("moteur installé en {:.1} s", t.elapsed().as_secs_f32());
        assert!(models::installed(&data, m));

        for (lang, text) in [("en", "thoroughly"), ("it", "sfortunatamente"), ("de", "Schifffahrt"), ("pt", "saudade"), ("ru", "счастье"), ("es", "hasta luego")] {
            let t = Instant::now();
            let p = say(&data, m, lang, text, "f").await.unwrap();
            let first = t.elapsed().as_secs_f32();
            let t = Instant::now();
            say(&data, m, lang, text, "f").await.unwrap();
            let again = t.elapsed().as_secs_f32();
            let len = std::fs::metadata(&p).unwrap().len();
            let secs = (len - 44) as f32 / 2.0 / 44100.0;
            println!("{lang} « {text} » : {first:.2} s, puis {:.0} ms en cache, son de {secs:.2} s", again * 1000.0);
            assert!(secs > 0.2 && secs < 3.0);
            if let Ok(out) = std::env::var("LUMEN_VOICE_OUT") {
                std::fs::copy(&p, PathBuf::from(out).join(format!("{lang}.wav"))).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Audio d'une leçon entière, puis recalage par Whisper si `LUMEN_ASR_MODEL` est donné :
    /// `LUMEN_VOICE_MODEL=… LUMEN_ASR_MODEL=…/ggml-large-v3-turbo-q5_0.bin cargo test --lib lesson_audio_live -- --ignored --nocapture`
    // liens symboliques du Mac : essai réservé au Mac
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn lesson_audio_live() {
        let model_src = PathBuf::from(std::env::var("LUMEN_VOICE_MODEL").expect("LUMEN_VOICE_MODEL"));
        let data = std::env::temp_dir().join("lumen-voice-lesson");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(data.join("models")).unwrap();
        let m = models::CATALOG.iter().find(|m| m.kind == "tts").unwrap();
        std::os::unix::fs::symlink(&model_src, models::path_of(&data, m)).unwrap();
        crate::sherpa::ensure(&data, crate::sherpa::Tool::Tts, Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap();
        let text = "Ogni mattina Marta saliva le scale strette del vecchio faro. Dall'alto, il mare sembrava infinito e tranquillo. Le piaceva ascoltare il vento mentre il sole sorgeva lentamente dietro le nuvole.\n\nUn giorno trovò una lettera nascosta tra due pietre. La carta era umida, ma le parole si potevano ancora leggere: «Se stai leggendo queste righe, non sei sola».";
        let t = Instant::now();
        let mut steps = 0;
        let a = lesson_audio(&data, m, "it", text, "3", Arc::new(AtomicBool::new(false)), |_| steps += 1).await.unwrap();
        let words = crate::text::tokenize(text, "it").iter().filter(|t| t.w).count();
        println!("audio de {:.1} s créé en {:.1} s ({} morceaux), {} mots minutés sur {words}, fichier {}", a.duration, t.elapsed().as_secs_f32(), steps, a.timings.len(), a.path.display());
        assert_eq!(a.timings.len(), words);
        assert!(a.timings.windows(2).all(|w| w[1][2] >= w[0][2]), "horodatages dans l'ordre");
        assert!(a.path.extension().unwrap() == "m4a");
        if let Ok(asr) = std::env::var("LUMEN_ASR_MODEL") {
            let heard = crate::media::transcribe(Path::new(&asr), &a.path, "it", None, |_| {}).await.unwrap();
            let (aligned, found) = crate::media::align_timings(text, "it", &heard);
            println!("Whisper retrouve {:.0} % des mots", found * 100.0);
            let precise: Vec<[f64; 4]> = serde_json::from_str(&aligned).unwrap();
            let gap: f64 = a.timings.iter().zip(&precise).map(|(x, y)| (x[2] - y[2]).abs()).sum::<f64>() / words as f64;
            println!("écart moyen entre la répartition et Whisper : {:.0} ms", gap * 1000.0);
            assert!(found > 0.8);
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Téléchargement complet comme dans l'app (moteur + modèle, ~150 Mo) :
    /// `cargo test --lib voice_download_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn voice_download_live() {
        let data = std::env::temp_dir().join("lumen-voice-download");
        let _ = std::fs::remove_dir_all(&data);
        let m = models::CATALOG.iter().find(|m| m.kind == "tts").unwrap();
        let t = Instant::now();
        let mut last = 0u64;
        models::download(&data, m, Arc::new(AtomicBool::new(false)), |e| {
            if let models::DownloadEvent::Progress { received, total, .. } = e {
                assert!(received >= last && received <= total + 1024, "progression {received}/{total}");
                last = received;
            }
        })
        .await
        .unwrap();
        println!("installé en {:.1} s ({last} octets annoncés sur {})", t.elapsed().as_secs_f32(), m.size);
        assert!(models::installed(&data, m));
        assert!(!models::part_of(&data, m).exists());
        let p = say(&data, m, "it", "buongiorno", "m").await.unwrap();
        assert!(std::fs::metadata(p).unwrap().len() > 10_000);
        let _ = std::fs::remove_dir_all(&data);
    }

    // Linux : espeak-ng dit les langues sans voix naturelle (chinois, ou avant
    // le téléchargement). Repéré par LUMEN_ESPEAK ou le PATH.
    // `LUMEN_ESPEAK=/chemin/espeak-ng cargo test --lib espeak_live -- --ignored --nocapture`
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore]
    async fn espeak_live() {
        if espeak_bin().is_none() {
            println!("espeak-ng introuvable : test sauté (LUMEN_ESPEAK=/chemin/espeak-ng)");
            return;
        }
        let dir = std::env::temp_dir().join(format!("lumen-espeak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // un mot chinois, une voix féminine puis masculine : deux fichiers distincts
        let a = espeak_say(&dir, "zh", "你好", "0").await.unwrap();
        let b = espeak_say(&dir, "zh", "你好", "5").await.unwrap();
        assert_ne!(a, b);
        let wav = std::fs::read(&a).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        let (rate, samples) = read_pcm(&wav).unwrap();
        assert!(rate > 0 && samples.len() > 2000, "{} échantillons à {rate} Hz", samples.len());
        println!("你好 : {:.2} s en chinois", samples.len() as f32 / rate as f32);
        // sans voix naturelle installée, say_any tombe sur espeak-ng
        let c = say_any(&dir, None, "it", "buongiorno", "0").await.unwrap();
        assert!(c.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Linux : la voix naturelle du chinois (vits, fanchen-C) dit les mots et
    // les leçons. `LUMEN_ZH_VOICE=/chemin/vits-zh-hf-fanchen-C cargo test
    // --lib voice_zh_live -- --ignored --nocapture` (sans la variable, le
    // modèle se télécharge, 114 Mo ; le moteur vient avec).
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore]
    async fn voice_zh_live() {
        let m = &crate::models::ZH_VOICE;
        let data = std::env::temp_dir().join("lumen-voice-zh-live");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(data.join("models")).unwrap();
        if let Ok(src) = std::env::var("LUMEN_ZH_VOICE") {
            std::os::unix::fs::symlink(src, models::path_of(&data, m)).unwrap();
        } else {
            models::download(&data, m, Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap();
        }
        crate::sherpa::ensure(&data, crate::sherpa::Tool::Tts, Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap();
        assert!(models::installed(&data, m));
        let t = Instant::now();
        let p = say(&data, m, "zh", "我们是学生，每天早上学习中文。", "0").await.unwrap();
        println!("premier mot en {:.1} s", t.elapsed().as_secs_f32());
        let wav = std::fs::read(&p).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        let (rate, samples) = read_pcm(&wav).unwrap();
        assert!(samples.len() > 20_000, "{} échantillons à {rate} Hz", samples.len());
        // une autre voix : un autre fichier, une autre locutrice
        let q = say(&data, m, "zh", "我们是学生，每天早上学习中文。", "5").await.unwrap();
        assert_ne!(p, q);
        let _ = std::fs::remove_dir_all(&data);
    }
}
