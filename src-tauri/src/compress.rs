//! Allègement des vidéos et des sons importés : bien moins lourds, à qualité
//! presque identique, sans rien changer au temps (la lanterne reste calée).
//!
//! - **Image** (Mac) : HEVC par la puce vidéo (VideoToolbox, par AVFoundation),
//!   en qualité constante, images clés toutes les 5 s comme YouTube, 30 images
//!   par seconde au plus (`video_plan`). Mesuré avec VMAF sur des vidéos
//!   d'apprenants en H.264 de YouTube : 92 à 95, pour 45 à 75 % de la taille.
//!   Un résultat qui ne gagne pas au moins 20 % est abandonné (l'original reste).
//! - **Son** (Mac) : AAC, en mono quand les deux voies disent la même chose (la
//!   plupart des voix enregistrées : MP3 de LingQ, podcasts), seulement la voie
//!   qui parle quand l'autre se tait, en stéréo sinon (`mix_of`, `audio_bitrate`).
//! - **Windows** : rien n'est réencodé ; les vidéos en ligne arrivent déjà en AV1
//!   ou en VP9 quand le moteur de la fenêtre les lit (`media::video_format`).

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Réglage `media_quality` : ce que Lumen fait des vidéos et des sons importés.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// rien n'est réencodé
    Original,
    /// presque sans perte visible (par défaut)
    Balanced,
    /// 720p, sons plus légers : pour un Mac à l'étroit
    Compact,
}

impl Level {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("original") => Level::Original,
            Some("compact") => Level::Compact,
            _ => Level::Balanced,
        }
    }

    pub fn of(conn: &rusqlite::Connection) -> Self {
        Self::parse(crate::db::setting(conn, "media_quality").as_deref())
    }

    /// Côté court maximal de l'image (une vidéo en hauteur compte par sa largeur).
    pub fn max_side(self) -> u32 {
        if self == Level::Compact {
            720
        } else {
            1080
        }
    }
}

// ---------- décisions (sans dépendre du système) ----------

/// Ce que devient l'image d'une vidéo.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoPlan {
    /// qualité constante de VideoToolbox (0 à 1)
    pub quality: f64,
    pub width: u32,
    pub height: u32,
    /// 50 ou 60 images/s : une sur deux suffit
    pub halve_fps: bool,
}

/// Plan pour une image `codec` (code à quatre lettres : avc1, hvc1…) de
/// `w`×`h` à `fps` images/s et `bitrate` bit/s ; `None` : rien à gagner.
///
/// La qualité suit le débit de la source par pixel : YouTube donne peu de débit
/// aux images simples (visage filmé à la webcam, dessin animé), que l'encodeur
/// d'Apple lisse vite ; elles demandent une qualité plus haute (mesuré : à
/// qualité égale, VMAF 85 pour une webcam à 0,025 bit par pixel contre 95 pour
/// une promenade filmée à 0,07).
pub fn video_plan(level: Level, codec: &str, w: u32, h: u32, fps: f64, bitrate: f64) -> Option<VideoPlan> {
    if level == Level::Original || w == 0 || h == 0 {
        return None;
    }
    let short = w.min(h);
    let scale = if short > level.max_side() { level.max_side() as f64 / short as f64 } else { 1.0 };
    let even = |x: f64| (((x / 2.0).round() as u32) * 2).max(2);
    let (width, height) = (even(w as f64 * scale), even(h as f64 * scale));
    let halve_fps = fps > 45.0;
    let fps = if fps > 1.0 { fps } else { 30.0 };
    let bpp = if bitrate > 0.0 { bitrate / (w as f64 * h as f64 * fps) } else { 0.1 };
    // déjà dans un format moderne et à petit débit : rien à gagner sans perte
    let modern = matches!(codec, "hvc1" | "hev1" | "av01" | "vp09");
    if scale == 1.0 && !halve_fps && modern && bpp < 0.1 {
        return None;
    }
    // 0,02 bit par pixel ou moins : 0,62 ; 0,06 ou plus : 0,54 ; entre les deux, en ligne droite
    // en 720p (compacte), l'encodeur donne moins à chaque image : un cran au-dessus
    // (mesuré, vu en 720p : VMAF 81 à 87 à 0,04 en dessous, 84 à 89 à égalité)
    let t = ((bpp - 0.02) / 0.04).clamp(0.0, 1.0);
    let quality = 0.62 - 0.08 * t + if level == Level::Compact { 0.03 } else { 0.0 };
    Some(VideoPlan { quality, width, height, halve_fps })
}

/// Voies gardées d'un son stéréo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mix {
    Stereo,
    /// les deux voies disent la même chose : leur moyenne
    Mono,
    /// une seule voie parle (l'autre est muette) : elle seule, sans perdre de volume
    Left,
    Right,
}

impl Mix {
    pub fn channels(self) -> u32 {
        if self == Mix::Stereo {
            2
        } else {
            1
        }
    }
}

/// Décide des voies d'après l'énergie de chacune (`l`, `r` : somme des carrés)
/// et celle de leur demi-différence (`side`).
pub fn mix_of(l: f64, r: f64, side: f64) -> Mix {
    let (lo, hi) = (l.min(r), l.max(r));
    if hi <= 0.0 {
        return Mix::Mono;
    }
    // une voie 30 dB sous l'autre : muette
    if lo < hi * 1e-3 {
        return if l > r { Mix::Left } else { Mix::Right };
    }
    // différence 40 dB sous les voies : le même son des deux côtés
    if side < (l + r) / 2.0 * 1e-4 {
        Mix::Mono
    } else {
        Mix::Stereo
    }
}

/// Débit de l'AAC (bit/s). Apple AAC : à 64 kbit/s en mono comme à 96 en stéréo,
/// la voix reste intacte.
pub fn audio_bitrate(level: Level, channels: u32) -> u32 {
    match (level, channels) {
        (Level::Compact, 1) => 48_000,
        (Level::Compact, _) => 80_000,
        (_, 1) => 64_000,
        _ => 96_000,
    }
}

/// Réencoder un son n'en vaut la peine qu'au-delà de 20 % de gagné.
pub fn audio_worth(source_bps: f64, target_bps: u32) -> bool {
    source_bps > target_bps as f64 * 1.25
}

/// Un résultat est gardé s'il pèse au plus 80 % de l'original.
pub fn kept(before: u64, after: u64) -> bool {
    after > 0 && (after as f64) <= before as f64 * 0.8
}

/// Formats que les outils de macOS savent lire (les autres restent tels quels).
pub fn readable(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    ["mp4", "m4v", "mov", "m4a", "mp3", "aac", "wav", "aif", "aiff", "caf", "flac"].contains(&ext.as_str())
}

/// Fichier allégé, à côté de l'original : `X.video.mp4` → `X.video.light.mp4`.
fn light_path(src: &Path, ext: &str) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("media");
    src.with_file_name(format!("{stem}.light.{ext}"))
}

/// Fichier en cours d'écriture (supprimé s'il reste après un arrêt brutal, `sweep`).
fn temp_path(dst: &Path) -> PathBuf {
    let name = dst.file_name().and_then(|s| s.to_str()).unwrap_or("media").replace(".light.", ".light-tmp.");
    dst.with_file_name(name)
}

/// Liste des originaux à supprimer au prochain lancement (`delete_later`).
fn pending_list(media_dir: &Path) -> PathBuf {
    media_dir.join(".allegement-a-supprimer")
}

/// Original remplacé alors qu'une leçon ouverte le lit peut-être encore : le
/// lecteur le redemande par morceaux, il ne disparaît qu'au prochain lancement.
pub fn delete_later(media_dir: &Path, file: &Path) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(pending_list(media_dir)) {
        let _ = writeln!(f, "{}", file.display());
    }
}

/// Au lancement : supprime les fichiers d'un allègement interrompu, et les
/// originaux mis de côté par `delete_later` qu'aucune leçon ne cite (`used`).
pub fn sweep(media_dir: &Path, used: impl Fn(&str) -> bool) {
    if let Ok(rd) = std::fs::read_dir(media_dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().contains(".light-tmp.") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let list = pending_list(media_dir);
    if let Ok(text) = std::fs::read_to_string(&list) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            // seulement dans la médiathèque, et plus cité par aucune leçon
            if Path::new(line).starts_with(media_dir) && !used(line) {
                let _ = std::fs::remove_file(line);
            }
        }
        let _ = std::fs::remove_file(list);
    }
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Fichiers qui commencent par le nom de `p` : lui-même et le brouillon (« X.sb-… »)
/// où macOS écrit la vidéo jusqu'à la fin, pour mettre l'index en tête.
fn with_drafts(p: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (p.parent(), p.file_name().and_then(|n| n.to_str())) else { return vec![] };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.flatten().map(|e| e.path()).filter(|q| q.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(name))).collect()
}

/// Poids écrit jusqu'ici, brouillon compris.
fn written(p: &Path) -> u64 {
    with_drafts(p).iter().map(|q| size(q)).sum()
}

/// Supprime un fichier en cours d'écriture et son brouillon.
fn remove_drafts(p: &Path) {
    for q in with_drafts(p) {
        let _ = std::fs::remove_file(q);
    }
}

/// Durée inscrite en tête d'un MP4 (boîte `mvhd`), et s'il est fragmenté (`mvex`).
/// Les fichiers DASH de YouTube y portent leur durée totale, que macOS ajoute à
/// celle des fragments : il les croit deux fois plus longs (`true_duration`).
pub fn mp4_header(path: &Path) -> Option<(f64, bool)> {
    use std::io::Read;
    let mut head = vec![0u8; 256 * 1024];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let be32 = |b: &[u8], i: usize| -> Option<u32> { b.get(i..i + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]])) };
    let be64 = |b: &[u8], i: usize| -> Option<u64> { b.get(i..i + 8).map(|x| u64::from_be_bytes(x.try_into().unwrap())) };
    // boîtes de premier niveau jusqu'à `moov`
    let mut pos = 0usize;
    let moov = loop {
        let size = be32(&head, pos)? as usize;
        let kind = head.get(pos + 4..pos + 8)?;
        if kind == b"moov" {
            break head.get(pos + 8..(pos + size).min(head.len()))?;
        }
        if size < 8 {
            return None;
        }
        pos += size;
    };
    let (mut duration, mut fragmented) = (None, false);
    let mut i = 0usize;
    while i + 8 <= moov.len() {
        let size = be32(moov, i)? as usize;
        match moov.get(i + 4..i + 8)? {
            b"mvhd" => {
                let body = &moov[i + 8..];
                duration = if body.first()? == &1 {
                    Some(be64(body, 20)? as f64 / be32(body, 16)?.max(1) as f64)
                } else {
                    Some(be32(body, 16)? as f64 / be32(body, 12)?.max(1) as f64)
                };
            }
            b"mvex" => fragmented = true,
            _ => {}
        }
        if size < 8 {
            break;
        }
        i += size;
    }
    Some((duration?, fragmented))
}

/// Vraie durée d'un fichier, d'après celle que donne macOS (`seen`) : un MP4
/// fragmenté dont l'en-tête porte déjà toute la durée est compté deux fois.
pub fn true_duration(path: &Path, seen: f64) -> f64 {
    match mp4_header(path) {
        Some((d, true)) if d > 0.0 && (seen - 2.0 * d).abs() < seen * 0.02 => d,
        _ => seen,
    }
}

// ---------- allègement ----------

/// Une vidéo à la fois (la puce vidéo ne court pas deux lièvres), un son à la
/// fois à côté : un import LingQ n'attend pas derrière une longue vidéo.
static VIDEO_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static AUDIO_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Résultat d'un allègement : les nouveaux fichiers (l'original n'est pas supprimé ici).
#[derive(Clone, Debug, Default)]
pub struct Lighter {
    pub video: Option<PathBuf>,
    pub audio: Option<PathBuf>,
    pub before: u64,
    pub after: u64,
}

/// Allège l'image d'une vidéo dont le son est à part (YouTube, sites vidéo).
/// `None` : rien à gagner, ou impossible (l'original reste).
pub async fn video(src: &Path, level: Level, cancel: &AtomicBool, progress: impl FnMut(f64) + Send) -> Result<Option<Lighter>> {
    run(src, level, Part::Video, cancel, progress).await
}

/// Allège un son.
pub async fn audio(src: &Path, level: Level, cancel: &AtomicBool) -> Result<Option<Lighter>> {
    run(src, level, Part::Audio, cancel, |_| {}).await
}

/// Vidéo qui porte aussi son son (fichier du Mac, lien direct) : image et son
/// séparés, chacun allégé, comme pour YouTube (le son seul part dans la
/// sauvegarde, la vidéo seulement si on l'a choisi).
pub async fn split(src: &Path, level: Level, cancel: &AtomicBool, progress: impl FnMut(f64) + Send) -> Result<Option<Lighter>> {
    run(src, level, Part::Both, cancel, progress).await
}

/// Comme `video`, `audio` ou `split`, sans jamais échouer : le fichier à garder
/// (l'original s'il n'y a rien à gagner), l'original supprimé s'il est remplacé.
pub async fn video_or_keep(src: PathBuf, level: Level) -> PathBuf {
    let cancel = AtomicBool::new(false);
    match video(&src, level, &cancel, |_| {}).await {
        Ok(Some(l)) => {
            let _ = std::fs::remove_file(&src);
            l.video.unwrap_or(src)
        }
        Ok(None) => src,
        Err(e) => {
            eprintln!("allègement de {} : {e}", src.display());
            src
        }
    }
}

/// Son allégé, ou l'original (supprimé s'il est remplacé).
pub async fn audio_or_keep(src: PathBuf, level: Level) -> PathBuf {
    let cancel = AtomicBool::new(false);
    match audio(&src, level, &cancel).await {
        Ok(Some(l)) => {
            let _ = std::fs::remove_file(&src);
            l.audio.unwrap_or(src)
        }
        Ok(None) => src,
        Err(e) => {
            eprintln!("allègement de {} : {e}", src.display());
            src
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Video,
    Audio,
    Both,
}

async fn run(src: &Path, level: Level, part: Part, cancel: &AtomicBool, mut progress: impl FnMut(f64) + Send) -> Result<Option<Lighter>> {
    if level == Level::Original || !readable(src) || !src.exists() {
        return Ok(None);
    }
    let _gate = if part == Part::Audio { AUDIO_GATE.lock().await } else { VIDEO_GATE.lock().await };
    let src = src.to_path_buf();
    // le travail se fait hors du moteur asynchrone ; l'avancement et l'arrêt passent par des canaux
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    // import abandonné (tâche interrompue) : l'encodage s'arrête aussi, sans laisser de fichier
    struct StopOnDrop(std::sync::Arc<AtomicBool>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _guard = StopOnDrop(stop.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let worker = {
        let stop = stop.clone();
        tokio::task::spawn_blocking(move || lighten(&src, level, part, &stop, &mut |p| {
            let _ = tx.send(p);
        }))
    };
    tokio::pin!(worker);
    loop {
        tokio::select! {
            res = &mut worker => return res.map_err(|e| anyhow!("{e}"))?,
            Some(p) = rx.recv() => progress(p),
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                if cancel.load(Ordering::Relaxed) {
                    stop.store(true, Ordering::Relaxed);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn lighten(src: &Path, level: Level, part: Part, cancel: &AtomicBool, progress: &mut dyn FnMut(f64)) -> Result<Option<Lighter>> {
    let probe = mac::probe(src)?;
    let before = size(src);
    let mut out = Lighter { before, ..Default::default() };
    let cleanup = |l: &Lighter| {
        for p in [&l.video, &l.audio].into_iter().flatten() {
            let _ = std::fs::remove_file(p);
        }
    };

    // l'image
    let mut video_bytes = 0u64;
    if part != Part::Audio {
        let Some(v) = &probe.video else { return Ok(None) };
        // HDR (iPhone…) : l'encodage en 8 bits en ternirait les couleurs, on n'y touche pas
        let plan = if v.hdr { None } else { video_plan(level, &v.codec, v.width, v.height, v.fps, v.bitrate) };
        match plan {
            Some(plan) => {
                let dst = light_path(src, "mp4");
                let tmp = temp_path(&dst);
                // le son ne pèse presque rien à côté : l'image fait toute la barre d'avancement
                let res = mac::encode_video(src, &tmp, &plan, probe.duration, v.bitrate, cancel, progress);
                match res {
                    Ok(true) => {
                        std::fs::rename(&tmp, &dst)?;
                        remove_drafts(&tmp);
                        video_bytes = size(&dst);
                        out.video = Some(dst);
                    }
                    Ok(false) => {
                        // pas assez gagné : on garde l'image d'origine
                        remove_drafts(&tmp);
                    }
                    Err(e) => {
                        remove_drafts(&tmp);
                        return Err(e);
                    }
                }
            }
            None => {}
        }
        // vidéo seule (son à part) : on s'arrête là
        if part == Part::Video {
            return Ok(match out.video {
                Some(_) if kept(before, video_bytes) => {
                    out.after = video_bytes;
                    Some(out)
                }
                _ => {
                    cleanup(&out);
                    None
                }
            });
        }
    }

    // le son
    let Some(a) = &probe.audio else {
        cleanup(&out);
        return Ok(None);
    };
    // plus de deux voies (film en 5.1) : on n'y touche pas
    if a.channels > 2 {
        cleanup(&out);
        return Ok(None);
    }
    let mix = if a.channels == 2 { mac::analyze_audio(src, probe.duration)? } else { Mix::Mono };
    let target = audio_bitrate(level, mix.channels());
    let dst = light_path(src, "m4a");
    let tmp = temp_path(&dst);
    // une vidéo séparée garde toujours son son à part, même s'il n'y a rien à gagner dessus
    if part == Part::Audio && !audio_worth(a.bitrate, target) {
        return Ok(None);
    }
    if let Err(e) = mac::encode_audio(src, &tmp, mix, target, cancel) {
        let _ = std::fs::remove_file(&tmp);
        cleanup(&out);
        return Err(e);
    }
    std::fs::rename(&tmp, &dst)?;
    out.audio = Some(dst);
    match part {
        Part::Audio => {
            out.after = size(out.audio.as_ref().unwrap());
            if kept(before, out.after) {
                Ok(Some(out))
            } else {
                cleanup(&out);
                Ok(None)
            }
        }
        _ => {
            // image d'origine gardée telle quelle : on ne sépare pas pour si peu
            let Some(_) = out.video else {
                cleanup(&out);
                return Ok(None);
            };
            out.after = video_bytes + size(out.audio.as_ref().unwrap());
            if kept(before, out.after) {
                Ok(Some(out))
            } else {
                cleanup(&out);
                Ok(None)
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn lighten(_src: &Path, _level: Level, _part: Part, _cancel: &AtomicBool, _progress: &mut dyn FnMut(f64)) -> Result<Option<Lighter>> {
    Ok(None)
}

// ---------- ce qui peut encore être allégé ----------

/// Rôle d'un fichier dans ses leçons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// le son de la leçon
    Sound,
    /// l'image seule (le son est à part)
    Video,
    /// une vidéo qui porte aussi le son de la leçon (fichier du Mac, lien direct)
    Both,
}

/// Ce qu'un fichier déjà importé deviendrait.
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub path: String,
    pub video: bool,
    pub bytes: u64,
    /// estimation du poids après allègement
    pub estimate: u64,
}

/// Le fichier mérite-t-il d'être allégé, et combien pèserait-il ? (Mac seulement)
#[cfg(target_os = "macos")]
pub fn candidate(path: &Path, level: Level, role: Role) -> Option<Candidate> {
    if level == Level::Original || !readable(path) || path.to_string_lossy().contains(".light.") {
        return None;
    }
    let bytes = size(path);
    let probe = mac::probe(path).ok()?;
    let secs = probe.duration.max(1.0);
    let mut estimate = 0f64;
    if role != Role::Sound {
        let v = probe.video.as_ref().filter(|v| !v.hdr)?;
        let plan = video_plan(level, &v.codec, v.width, v.height, v.fps, v.bitrate)?;
        // mesuré : environ 60 % du débit H.264 de YouTube à cette qualité (moins en 720p)
        let pixels = plan.width as f64 * plan.height as f64 / (v.width as f64 * v.height as f64);
        let fps = if plan.halve_fps { 0.6 } else { 1.0 };
        estimate += v.bitrate * 0.6 * pixels.max(0.3) * fps * secs / 8.0;
    }
    if let Some(a) = probe.audio.as_ref().filter(|_| role != Role::Video) {
        // estimation prudente : stéréo, sauf son déjà en mono
        let target = audio_bitrate(level, a.channels.min(2));
        estimate += (a.bitrate.min(target as f64 * if audio_worth(a.bitrate, target) { 1.0 } else { 10.0 })) * secs / 8.0;
    }
    let estimate = estimate as u64;
    kept(bytes, estimate).then(|| Candidate { path: path.to_string_lossy().into_owned(), video: role != Role::Sound, bytes, estimate })
}

#[cfg(not(target_os = "macos"))]
pub fn candidate(_path: &Path, _level: Level, _role: Role) -> Option<Candidate> {
    None
}

// ---------- leçons déjà importées ----------

/// Un fichier d'une leçon : chemin, rôle, titre et adresse d'origine de la leçon.
pub struct LessonFile {
    pub path: String,
    pub role: Role,
    pub title: String,
    pub source: String,
}

/// Fichiers des leçons, chacun une fois.
pub fn lesson_files(conn: &rusqlite::Connection) -> Result<Vec<LessonFile>> {
    let rows = crate::db::media_files(conn)?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (_, title, audio, video, source) in rows {
        // une vidéo du Mac ou d'un lien direct sert aussi de son
        let both = video.is_some() && video == audio;
        if let Some(v) = &video {
            if seen.insert(v.clone()) {
                let role = if both { Role::Both } else { Role::Video };
                out.push(LessonFile { path: v.clone(), role, title: title.clone(), source: source.clone() });
            }
        }
        if let Some(a) = audio.filter(|_| !both) {
            if seen.insert(a.clone()) {
                out.push(LessonFile { path: a, role: Role::Sound, title: title.clone(), source: source.clone() });
            }
        }
    }
    Ok(out)
}

/// Pour retélécharger une vidéo en ligne dans un format plus léger que la
/// fenêtre lit (VP9, AV1) : sans aucune perte, souvent plus léger que le
/// réencodage (mesuré : promenade filmée de YouTube, 53 % en VP9 contre plus
/// de 80 % réencodée).
pub struct Refetch {
    pub data_dir: PathBuf,
    pub codecs: Vec<String>,
    pub browser: Option<String>,
}

/// Retélécharge l'image si YouTube (ou un autre site) en propose une version
/// lisible ici qui pèse au plus 80 % du fichier actuel. `None` : pas mieux, hors
/// ligne, ou pas un site vidéo.
async fn refetch_lighter(rf: &Refetch, url: &str, current: &Path, level: Level, progress: impl FnMut(f64) + Send) -> Option<Lighter> {
    let mut progress = progress;
    if rf.codecs.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let ytdlp = crate::tools::find_ytdlp(&rf.data_dir)?;
    let before = size(current);
    let (codec, announced) =
        crate::media::yt_video_choice(&rf.data_dir, &ytdlp, url, rf.browser.as_deref(), &rf.codecs, level.max_side()).await.ok()?;
    if codec.starts_with("avc1") || announced == 0 || !kept(before, announced) {
        return None;
    }
    let stem = crate::media::new_stem();
    let p = crate::media::yt_video(&rf.data_dir, &ytdlp, url, &stem, rf.browser.as_deref(), &rf.codecs, level.max_side(), &mut progress)
        .await
        .ok()?;
    let after = size(&p);
    // le fichier attendu, entier
    if !kept(before, after) || (after as f64) < announced as f64 * 0.9 {
        let _ = std::fs::remove_file(&p);
        return None;
    }
    Some(Lighter { video: Some(p), audio: None, before, after })
}

/// Avancement de l'allègement des leçons déjà importées.
#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
    pub title: String,
    /// avancement du fichier en cours (0 à 100)
    pub value: f64,
    /// octets gagnés jusqu'ici
    pub saved: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub done: u32,
    pub failed: u32,
    pub saved: u64,
    pub cancelled: bool,
}

/// Allège les vidéos et les sons des leçons déjà importées, l'un après l'autre.
/// Chaque fichier allégé remplace l'original dans les leçons qui le citent,
/// puis l'original est supprimé (au prochain lancement s'il appartient à la
/// dernière leçon ouverte, que le lecteur lit peut-être encore).
pub async fn lighten_lessons(
    db: &parking_lot::Mutex<rusqlite::Connection>,
    media_dir: &Path,
    level: Level,
    refetch: Option<&Refetch>,
    cancel: &AtomicBool,
    on_progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Report> {
    let files = lesson_files(&db.lock())?;
    // d'abord ce qui peut s'alléger, estimé sans rien toucher
    let todo: Vec<LessonFile> =
        tokio::task::spawn_blocking(move || files.into_iter().filter(|f| candidate(Path::new(&f.path), level, f.role).is_some()).collect())
            .await?;
    let total = todo.len() as u32;
    let mut report = Report::default();
    for (i, LessonFile { path, role, title, source }) in todo.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let src = PathBuf::from(&path);
        let saved = report.saved;
        let send = |value: f64| on_progress(Progress { done: i as u32, total, title: title.clone(), value, saved });
        send(0.0);
        // une image en ligne : d'abord un format plus léger chez la source, sans rien perdre
        let fetched = match (role, refetch) {
            (Role::Video, Some(rf)) => refetch_lighter(rf, &source, &src, level, send).await,
            _ => None,
        };
        let res = match (fetched, role) {
            (Some(l), _) => Ok(Some(l)),
            (None, Role::Sound) => audio(&src, level, cancel).await,
            // une vidéo qui porte le son de la leçon est séparée en image et son
            (None, Role::Both) => split(&src, level, cancel, send).await,
            (None, Role::Video) => video(&src, level, cancel, send).await,
        };
        match res {
            Ok(Some(l)) => {
                let (a, v) = (l.audio.as_ref().map(|p| p.display().to_string()), l.video.as_ref().map(|p| p.display().to_string()));
                let replaced = {
                    let c = db.lock();
                    crate::db::media_replace(&c, &path, a.as_deref(), v.as_deref()).map(|_| {
                        // la leçon ouverte (ou la dernière) lit peut-être encore l'original
                        let open = crate::db::setting(&c, "last_lesson").and_then(|v| v.parse::<i64>().ok());
                        open.is_some_and(|id| [a.as_deref(), v.as_deref()].into_iter().flatten().any(|n| crate::db::media_used(&c, n, Some(id))))
                    })
                };
                match replaced {
                    Ok(reading) => {
                        if reading {
                            delete_later(media_dir, &src);
                        } else {
                            let _ = std::fs::remove_file(&src);
                        }
                        report.saved += l.before.saturating_sub(l.after);
                        report.done += 1;
                    }
                    Err(_) => {
                        for p in [l.audio, l.video].into_iter().flatten() {
                            let _ = std::fs::remove_file(p);
                        }
                        report.failed += 1;
                    }
                }
            }
            Ok(None) => {}
            Err(e) if e.to_string() == "annulé" => break,
            Err(e) => {
                eprintln!("allègement de {path} : {e}");
                report.failed += 1;
            }
        }
        on_progress(Progress { done: i as u32 + 1, total, title: title.clone(), value: 0.0, saved: report.saved });
    }
    report.cancelled = cancel.load(Ordering::Relaxed);
    Ok(report)
}

// ---------- macOS : AVFoundation et AudioToolbox ----------

#[cfg(target_os = "macos")]
#[allow(deprecated)]
mod mac {
    use super::{Mix, VideoPlan};
    use anyhow::{anyhow, bail, Result};
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::AnyObject;
    use objc2_audio_toolbox::{
        kAudioCodecBitRateControlMode_VariableConstrained, kAudioCodecPropertyBitRateControlMode, kAudioConverterCodecQuality,
        kAudioConverterEncodeBitRate, kAudioConverterQuality_Max, kAudioFileM4AType, kExtAudioFileProperty_AudioConverter,
        kExtAudioFileProperty_ClientDataFormat, kExtAudioFileProperty_ConverterConfig, AudioConverterRef, AudioConverterSetProperty,
        AudioFileFlags, ExtAudioFileCreateWithURL, ExtAudioFileDispose, ExtAudioFileGetProperty, ExtAudioFileRef, ExtAudioFileSetProperty,
        ExtAudioFileWrite,
    };
    use objc2_av_foundation::{
        AVAsset, AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack, AVAssetWriter, AVAssetWriterInput,
        AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeAudio, AVMediaTypeVideo, AVURLAsset, AVVideoAllowFrameReorderingKey,
        AVVideoCodecKey, AVVideoCodecTypeHEVC, AVVideoCompressionPropertiesKey, AVVideoExpectedSourceFrameRateKey, AVVideoHeightKey,
        AVVideoMaxKeyFrameIntervalDurationKey, AVVideoQualityKey, AVVideoScalingModeKey, AVVideoScalingModeResizeAspect, AVVideoWidthKey,
    };
    use objc2_core_audio_types::{
        kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked, kAudioFormatLinearPCM, kAudioFormatMPEG4AAC, AudioBuffer, AudioBufferList,
        AudioStreamBasicDescription,
    };
    use objc2_core_foundation::{CFString, CFURL};
    use objc2_core_media::{kCMTimeZero, CMAudioFormatDescriptionGetStreamBasicDescription, CMFormatDescription, CMSampleBuffer, CMTime, CMTimeRange};
    use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString, NSURL};
    use std::ffi::c_void;
    use std::path::Path;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub struct Track {
        pub codec: String,
        /// image HDR (HLG, PQ)
        pub hdr: bool,
        pub width: u32,
        pub height: u32,
        pub fps: f64,
        /// bit/s
        pub bitrate: f64,
        pub channels: u32,
    }

    pub struct Probe {
        pub duration: f64,
        pub video: Option<Track>,
        pub audio: Option<Track>,
    }

    fn url(p: &Path) -> Retained<NSURL> {
        NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
    }

    fn ns_err(e: Option<Retained<NSError>>) -> anyhow::Error {
        match e {
            Some(e) => anyhow!("{}", e.localizedDescription()),
            None => anyhow!("AVFoundation"),
        }
    }

    type Dict = NSDictionary<NSString, AnyObject>;

    fn dict(pairs: &[(&NSString, &AnyObject)]) -> Retained<Dict> {
        let keys: Vec<&NSString> = pairs.iter().map(|p| p.0).collect();
        let values: Vec<&AnyObject> = pairs.iter().map(|p| p.1).collect();
        NSDictionary::from_slices(&keys, &values)
    }

    /// Clés d'AVFoundation, toujours définies sur les macOS pris en charge.
    fn key(k: Option<&'static NSString>) -> &'static NSString {
        k.expect("constante d'AVFoundation")
    }

    fn fourcc(code: u32) -> String {
        code.to_be_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { ' ' }).collect::<String>().trim().to_string()
    }

    unsafe fn first_track(asset: &AVAsset, kind: Option<&'static NSString>) -> Option<Retained<AVAssetTrack>> {
        asset.tracksWithMediaType(key(kind)).firstObject()
    }

    /// Premier descripteur de format d'une piste (un objet CoreMedia dans un NSArray).
    unsafe fn format_of(track: &AVAssetTrack) -> Option<Retained<AnyObject>> {
        track.formatDescriptions().firstObject()
    }

    unsafe fn as_format(obj: &AnyObject) -> &CMFormatDescription {
        &*(obj as *const AnyObject as *const CMFormatDescription)
    }

    /// Image HDR : fonction de transfert HLG ou PQ (UIT-R BT.2100, SMPTE ST 2084).
    unsafe fn is_hdr(fd: &CMFormatDescription) -> bool {
        let key = CFString::from_str("CVImageBufferTransferFunction");
        fd.extension(&key)
            .and_then(|v| v.downcast::<CFString>().ok())
            .is_some_and(|v| {
                let v = v.to_string();
                v.contains("2100") || v.contains("2084")
            })
    }

    /// Débit réel d'une piste (bit/s), d'après le poids de ses données :
    /// `estimatedDataRate` se trompe de moitié sur les fichiers fragmentés de YouTube.
    unsafe fn bitrate(track: &AVAssetTrack, duration: f64) -> f64 {
        let bytes = track.totalSampleDataLength();
        if bytes > 0 && duration > 0.0 {
            bytes as f64 * 8.0 / duration
        } else {
            track.estimatedDataRate() as f64
        }
    }

    pub fn probe(path: &Path) -> Result<Probe> {
        autoreleasepool(|_| unsafe {
            let asset = AVURLAsset::URLAssetWithURL_options(&url(path), None);
            let duration = super::true_duration(path, asset.duration().seconds());
            if !duration.is_finite() || duration <= 0.0 {
                bail!("durée illisible");
            }
            let video = first_track(&asset, AVMediaTypeVideo).map(|t| {
                let s = t.naturalSize();
                let f = format_of(&t);
                let codec = f.as_ref().map(|f| fourcc(as_format(f).media_sub_type())).unwrap_or_default();
                Track {
                    codec,
                    hdr: f.as_ref().is_some_and(|f| is_hdr(as_format(f))),
                    width: s.width.abs().round() as u32,
                    height: s.height.abs().round() as u32,
                    fps: t.nominalFrameRate() as f64,
                    bitrate: bitrate(&t, duration),
                    channels: 0,
                }
            });
            let audio = first_track(&asset, AVMediaTypeAudio).map(|t| {
                let f = format_of(&t);
                let (codec, channels) = f
                    .map(|f| {
                        let f = as_format(&f);
                        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(f);
                        (fourcc(f.media_sub_type()), if asbd.is_null() { 2 } else { (*asbd).mChannelsPerFrame })
                    })
                    .unwrap_or_default();
                Track { codec, hdr: false, width: 0, height: 0, fps: 0.0, bitrate: bitrate(&t, duration), channels }
            });
            Ok(Probe { duration, video, audio })
        })
    }

    /// Attend la fin de l'écriture.
    unsafe fn finish(writer: &AVAssetWriter) -> Result<()> {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let block = block2::RcBlock::new(move || {
            let _ = tx.send(());
        });
        writer.finishWritingWithCompletionHandler(&block);
        rx.recv_timeout(std::time::Duration::from_secs(900)).map_err(|_| anyhow!("écriture interminable"))?;
        if writer.status() != AVAssetWriterStatus::Completed {
            return Err(ns_err(writer.error()));
        }
        Ok(())
    }

    /// Réencode l'image en HEVC. `Ok(false)` : abandonné en route parce que le
    /// fichier ne serait pas allégé (il pèse déjà autant que l'original au même point).
    pub fn encode_video(
        src: &Path,
        dst: &Path,
        plan: &VideoPlan,
        duration: f64,
        source_bps: f64,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(f64),
    ) -> Result<bool> {
        let _ = std::fs::remove_file(dst);
        unsafe {
            let asset = AVURLAsset::URLAssetWithURL_options(&url(src), None);
            let track = first_track(&asset, AVMediaTypeVideo).ok_or_else(|| anyhow!("pas d'image"))?;
            let fps = track.nominalFrameRate() as f64;
            let reader = AVAssetReader::assetReaderWithAsset_error(&asset).map_err(|e| ns_err(Some(e)))?;
            // image décodée par la puce, en YUV 4:2:0 (« 420v »), ce que l'encodeur attend
            let pix = NSNumber::new_u32(u32::from_be_bytes(*b"420v"));
            let out_settings = dict(&[(&NSString::from_str("PixelFormatType"), &pix)]);
            let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&out_settings));
            output.setAlwaysCopiesSampleData(false);
            if !reader.canAddOutput(&output) {
                bail!("lecture de l'image impossible");
            }
            reader.addOutput(&output);

            let writer = AVAssetWriter::assetWriterWithURL_fileType_error(&url(dst), key(AVFileTypeMPEG4)).map_err(|e| ns_err(Some(e)))?;
            // l'index en tête du fichier : la lecture commence tout de suite
            writer.setShouldOptimizeForNetworkUse(true);
            let fps_out = if plan.halve_fps { fps / 2.0 } else { fps.max(1.0) };
            let (q, gop, reorder, rate) =
                (NSNumber::new_f64(plan.quality), NSNumber::new_f64(5.0), NSNumber::new_bool(true), NSNumber::new_f64(fps_out));
            let props = dict(&[
                (key(AVVideoQualityKey), &q),
                (key(AVVideoMaxKeyFrameIntervalDurationKey), &gop),
                (key(AVVideoAllowFrameReorderingKey), &reorder),
                (key(AVVideoExpectedSourceFrameRateKey), &rate),
            ]);
            let (w, h) = (NSNumber::new_u32(plan.width), NSNumber::new_u32(plan.height));
            let settings = dict(&[
                (key(AVVideoCodecKey), key(AVVideoCodecTypeHEVC)),
                (key(AVVideoWidthKey), &w),
                (key(AVVideoHeightKey), &h),
                (key(AVVideoScalingModeKey), key(AVVideoScalingModeResizeAspect)),
                (key(AVVideoCompressionPropertiesKey), &props),
            ]);
            // des réglages refusés lèvent une exception Objective-C : on la rattrape
            let input = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(key(AVMediaTypeVideo), Some(&settings))
            }))
            .map_err(|e| anyhow!("encodeur HEVC refusé : {:?}", e.map(|e| e.to_string())))?;
            input.setExpectsMediaDataInRealTime(false);
            input.setTransform(track.preferredTransform());
            if !writer.canAddInput(&input) {
                bail!("encodeur HEVC indisponible");
            }
            writer.addInput(&input);
            if !reader.startReading() {
                return Err(ns_err(reader.error()));
            }
            if !writer.startWriting() {
                reader.cancelReading();
                return Err(ns_err(writer.error()));
            }
            // même origine que la source : l'image reste calée sur le son
            writer.startSessionAtSourceTime(kCMTimeZero);

            let stop = |reader: &AVAssetReader, writer: &AVAssetWriter| {
                reader.cancelReading();
                writer.cancelWriting();
            };
            // 50 ou 60 images/s : on garde une image sur deux
            let min_gap = if plan.halve_fps && fps > 0.0 { 1.5 / fps } else { 0.0 };
            let mut last = f64::NEG_INFINITY;
            let mut shown = -1i64;
            let mut since_check = 0u32;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    stop(&reader, &writer);
                    bail!("annulé");
                }
                let step: Result<Option<f64>> = autoreleasepool(|_| {
                    let Some(sample) = output.copyNextSampleBuffer() else { return Ok(None) };
                    let t = sample.presentation_time_stamp().seconds();
                    if min_gap > 0.0 && t - last < min_gap {
                        return Ok(Some(t));
                    }
                    last = t;
                    while !input.isReadyForMoreMediaData() {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(Some(t));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    if !input.appendSampleBuffer(&sample) {
                        return Err(ns_err(writer.error()));
                    }
                    Ok(Some(t))
                });
                let t = match step {
                    Ok(Some(t)) => t,
                    Ok(None) => break,
                    Err(e) => {
                        stop(&reader, &writer);
                        return Err(e);
                    }
                };
                let pct = (t / duration * 100.0).clamp(0.0, 100.0);
                if pct as i64 != shown {
                    shown = pct as i64;
                    progress(pct);
                }
                // passé le tiers de la vidéo (2 min au moins), un fichier qui pèse déjà autant
                // que l'original au même point ne vaut pas la peine. Prudent : le débit de la
                // source varie (début d'une promenade filmée : 80 %, fin à 72 % au total)
                since_check += 1;
                if since_check >= 250 && t > 120.0_f64.max(duration / 3.0) && source_bps > 0.0 {
                    since_check = 0;
                    if super::written(dst) as f64 > source_bps * t / 8.0 {
                        stop(&reader, &writer);
                        return Ok(false);
                    }
                }
            }
            if reader.status() == AVAssetReaderStatus::Failed {
                stop(&reader, &writer);
                return Err(ns_err(reader.error()));
            }
            input.markAsFinished();
            finish(&writer)?;
            Ok(true)
        }
    }

    /// Lecteur du son en PCM flottant entrelacé, voies d'origine (mono ou stéréo).
    unsafe fn pcm_reader(src: &Path, range: Option<(f64, f64)>) -> Result<(Retained<AVAssetReader>, Retained<AVAssetReaderTrackOutput>)> {
        let asset = AVURLAsset::URLAssetWithURL_options(&url(src), None);
        let track = first_track(&asset, AVMediaTypeAudio).ok_or_else(|| anyhow!("pas de son"))?;
        let reader = AVAssetReader::assetReaderWithAsset_error(&asset).map_err(|e| ns_err(Some(e)))?;
        if let Some((start, len)) = range {
            reader.setTimeRange(CMTimeRange { start: CMTime::with_seconds(start, 48_000), duration: CMTime::with_seconds(len, 48_000) });
        }
        // valeurs des clés d'AVFAudio (AVFormatIDKey…), qui portent leur propre nom
        let (id, bits, yes, no) =
            (NSNumber::new_u32(kAudioFormatLinearPCM), NSNumber::new_u32(32), NSNumber::new_bool(true), NSNumber::new_bool(false));
        let settings = dict(&[
            (&NSString::from_str("AVFormatIDKey"), &id),
            (&NSString::from_str("AVLinearPCMBitDepthKey"), &bits),
            (&NSString::from_str("AVLinearPCMIsFloatKey"), &yes),
            (&NSString::from_str("AVLinearPCMIsNonInterleaved"), &no),
            (&NSString::from_str("AVLinearPCMIsBigEndianKey"), &no),
        ]);
        let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
        output.setAlwaysCopiesSampleData(false);
        if !reader.canAddOutput(&output) {
            bail!("lecture du son impossible");
        }
        reader.addOutput(&output);
        if !reader.startReading() {
            return Err(ns_err(reader.error()));
        }
        Ok((reader, output))
    }

    /// Échantillons d'un morceau de son, avec leur fréquence et leur nombre de voies.
    unsafe fn samples(sample: &CMSampleBuffer, buf: &mut Vec<f32>) -> Option<(f64, u32)> {
        let fd = sample.format_description()?;
        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(&fd);
        if asbd.is_null() {
            return None;
        }
        let (rate, ch) = ((*asbd).mSampleRate, (*asbd).mChannelsPerFrame);
        let block = sample.data_buffer()?;
        let len = block.data_length();
        buf.clear();
        buf.resize(len / 4, 0.0);
        let status = block.copy_data_bytes(0, len, NonNull::new(buf.as_mut_ptr() as *mut c_void)?);
        (status == 0).then_some((rate, ch))
    }

    /// Écoute cinq passages de 10 s répartis dans le son (tout, s'il est court)
    /// et décide des voies à garder.
    pub fn analyze_audio(src: &Path, duration: f64) -> Result<Mix> {
        let windows: Vec<Option<(f64, f64)>> = if duration < 60.0 {
            vec![None]
        } else {
            [0.1, 0.3, 0.5, 0.7, 0.9].iter().map(|f| Some((duration * f, 10.0))).collect()
        };
        let (mut l, mut r, mut side) = (0f64, 0f64, 0f64);
        let mut buf = Vec::new();
        for w in windows {
            unsafe {
                let (reader, output) = pcm_reader(src, w)?;
                while let Some(sample) = output.copyNextSampleBuffer() {
                    let Some((_, ch)) = samples(&sample, &mut buf) else { continue };
                    if ch != 2 {
                        return Ok(Mix::Mono);
                    }
                    for f in buf.chunks_exact(2) {
                        let (a, b) = (f[0] as f64, f[1] as f64);
                        l += a * a;
                        r += b * b;
                        side += (a - b) * (a - b) / 4.0;
                    }
                }
                if reader.status() == AVAssetReaderStatus::Failed {
                    return Err(ns_err(reader.error()));
                }
            }
        }
        Ok(super::mix_of(l, r, side))
    }

    /// Réencode le son en AAC (m4a), avec les voies choisies. Le fichier garde
    /// l'indication du délai de l'encodeur : il commence au même instant que
    /// l'original.
    pub fn encode_audio(src: &Path, dst: &Path, mix: Mix, bitrate: u32, cancel: &AtomicBool) -> Result<()> {
        let _ = std::fs::remove_file(dst);
        unsafe {
            let (reader, output) = pcm_reader(src, None)?;
            let mut buf: Vec<f32> = Vec::new();
            let mut mixed: Vec<f32> = Vec::new();
            let mut file: ExtAudioFileRef = std::ptr::null_mut();
            let ch_out = mix.channels();
            let check = |status: i32, what: &str| -> Result<()> {
                if status != 0 {
                    bail!("{what} ({status})");
                }
                Ok(())
            };
            let res: Result<()> = (|| {
                while let Some(sample) = output.copyNextSampleBuffer() {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("annulé");
                    }
                    let Some((rate, ch)) = samples(&sample, &mut buf) else { continue };
                    if file.is_null() {
                        file = create_aac(dst, rate, ch_out, bitrate)?;
                    }
                    let src_ch = ch.max(1) as usize;
                    mixed.clear();
                    for f in buf.chunks_exact(src_ch) {
                        match (mix, src_ch) {
                            (_, 1) => mixed.push(f[0]),
                            (Mix::Stereo, _) => mixed.extend_from_slice(&f[..2]),
                            (Mix::Mono, _) => mixed.push((f[0] + f[1]) * 0.5),
                            (Mix::Left, _) => mixed.push(f[0]),
                            (Mix::Right, _) => mixed.push(f[1]),
                        }
                    }
                    // mono lu alors qu'on attendait de la stéréo : la même voie des deux côtés
                    if mix == Mix::Stereo && src_ch == 1 {
                        let mono = std::mem::take(&mut mixed);
                        mixed = mono.iter().flat_map(|&x| [x, x]).collect();
                    }
                    let frames = (mixed.len() / ch_out as usize) as u32;
                    if frames == 0 {
                        continue;
                    }
                    let mut list = AudioBufferList {
                        mNumberBuffers: 1,
                        mBuffers: [AudioBuffer { mNumberChannels: ch_out, mDataByteSize: (mixed.len() * 4) as u32, mData: mixed.as_mut_ptr() as *mut c_void }],
                    };
                    check(ExtAudioFileWrite(file, frames, NonNull::from(&mut list)), "écriture du son")?;
                }
                if reader.status() == AVAssetReaderStatus::Failed {
                    return Err(ns_err(reader.error()));
                }
                if file.is_null() {
                    bail!("son vide");
                }
                Ok(())
            })();
            if !file.is_null() {
                // referme le fichier : écrit la fin du son et le délai de l'encodeur
                let closed = ExtAudioFileDispose(file);
                if res.is_ok() {
                    check(closed, "fermeture du son")?;
                }
            }
            if res.is_err() {
                reader.cancelReading();
            }
            res
        }
    }

    /// Ouvre un fichier m4a AAC qui reçoit du PCM flottant entrelacé.
    unsafe fn create_aac(dst: &Path, rate: f64, ch: u32, bitrate: u32) -> Result<ExtAudioFileRef> {
        let cf = CFURL::from_file_path(dst).ok_or_else(|| anyhow!("chemin illisible"))?;
        // AAC jusqu'à 48 kHz : au-delà, le convertisseur rééchantillonne
        let out_rate = if rate > 48_000.0 { 48_000.0 } else { rate };
        let mut aac = AudioStreamBasicDescription {
            mSampleRate: out_rate,
            mFormatID: kAudioFormatMPEG4AAC,
            mFormatFlags: 0,
            mBytesPerPacket: 0,
            mFramesPerPacket: 1024,
            mBytesPerFrame: 0,
            mChannelsPerFrame: ch,
            mBitsPerChannel: 0,
            mReserved: 0,
        };
        let mut file: ExtAudioFileRef = std::ptr::null_mut();
        let status =
            ExtAudioFileCreateWithURL(&cf, kAudioFileM4AType, NonNull::from(&mut aac), std::ptr::null(), AudioFileFlags::EraseFile.0, NonNull::from(&mut file));
        if status != 0 || file.is_null() {
            bail!("création du son ({status})");
        }
        let fail = |file: ExtAudioFileRef, what: &str, status: i32| -> anyhow::Error {
            ExtAudioFileDispose(file);
            anyhow!("{what} ({status})")
        };
        let client = AudioStreamBasicDescription {
            mSampleRate: rate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mBytesPerPacket: 4 * ch,
            mFramesPerPacket: 1,
            mBytesPerFrame: 4 * ch,
            mChannelsPerFrame: ch,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        let status = ExtAudioFileSetProperty(
            file,
            kExtAudioFileProperty_ClientDataFormat,
            std::mem::size_of::<AudioStreamBasicDescription>() as u32,
            NonNull::from(&client).cast(),
        );
        if status != 0 {
            return Err(fail(file, "format du son", status));
        }
        let mut conv: AudioConverterRef = std::ptr::null_mut();
        let mut len = std::mem::size_of::<AudioConverterRef>() as u32;
        let status = ExtAudioFileGetProperty(file, kExtAudioFileProperty_AudioConverter, NonNull::from(&mut len), NonNull::from(&mut conv).cast());
        if status != 0 || conv.is_null() {
            return Err(fail(file, "encodeur AAC", status));
        }
        // débit variable contenu, meilleure qualité de l'encodeur (comme afconvert -s 2 -q 127)
        let set = |id: u32, value: u32| AudioConverterSetProperty(conv, id, 4, NonNull::from(&value).cast());
        let _ = set(kAudioCodecPropertyBitRateControlMode, kAudioCodecBitRateControlMode_VariableConstrained);
        let _ = set(kAudioConverterCodecQuality, kAudioConverterQuality_Max);
        // un son à basse fréquence (8 kHz) n'accepte pas tous les débits : on descend
        let status = [bitrate, bitrate * 3 / 4, bitrate / 2, bitrate / 3]
            .into_iter()
            .map(|b| set(kAudioConverterEncodeBitRate, b))
            .find(|&s| s == 0)
            .unwrap_or(-1);
        if status != 0 {
            return Err(fail(file, "débit du son", status));
        }
        let config: *const c_void = std::ptr::null();
        let status = ExtAudioFileSetProperty(
            file,
            kExtAudioFileProperty_ConverterConfig,
            std::mem::size_of::<*const c_void>() as u32,
            NonNull::from(&config).cast(),
        );
        if status != 0 {
            return Err(fail(file, "réglage de l'encodeur", status));
        }
        Ok(file)
    }

    /// Son décodé, mono, pour les tests (comparer l'original et l'allégé).
    #[cfg(test)]
    pub fn decode_mono(src: &Path) -> Result<(Vec<f32>, f64)> {
        let mut out = Vec::new();
        let mut buf = Vec::new();
        let mut rate = 0.0;
        unsafe {
            let (_reader, output) = pcm_reader(src, None)?;
            while let Some(sample) = output.copyNextSampleBuffer() {
                let Some((r, ch)) = samples(&sample, &mut buf) else { continue };
                rate = r;
                let ch = ch.max(1) as usize;
                out.extend(buf.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
            }
        }
        Ok((out, rate))
    }
}

#[cfg(all(test, target_os = "macos"))]
use mac::probe;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_follows_source_detail() {
        // YouTube 1080p H.264, promenade filmée (0,072 bit par pixel) : qualité la plus basse
        let walk = video_plan(Level::Balanced, "avc1", 1920, 1080, 25.0, 3_756_000.0).unwrap();
        assert!((walk.quality - 0.54).abs() < 1e-9);
        assert_eq!((walk.width, walk.height, walk.halve_fps), (1920, 1080, false));
        // visage à la webcam (0,025) : plus haute
        let cam = video_plan(Level::Balanced, "avc1", 1920, 1080, 29.97, 1_577_000.0).unwrap();
        assert!(cam.quality > 0.6 && cam.quality <= 0.62);
        // 60 images/s : une sur deux
        assert!(video_plan(Level::Balanced, "avc1", 1920, 1080, 59.94, 4_000_000.0).unwrap().halve_fps);
        // rien en « originale »
        assert!(video_plan(Level::Original, "avc1", 1920, 1080, 25.0, 3_756_000.0).is_none());
    }

    #[test]
    fn plan_scales_by_the_short_side() {
        // 4K → 1080p, vidéo en hauteur → 1080 de large
        let p = video_plan(Level::Balanced, "hvc1", 3840, 2160, 30.0, 20_000_000.0).unwrap();
        assert_eq!((p.width, p.height), (1920, 1080));
        let p = video_plan(Level::Balanced, "avc1", 1080, 1920, 30.0, 8_000_000.0).unwrap();
        assert_eq!((p.width, p.height), (1080, 1920));
        // compacte : 720p, dimensions paires
        let p = video_plan(Level::Compact, "avc1", 1920, 1080, 25.0, 3_000_000.0).unwrap();
        assert_eq!((p.width, p.height), (1280, 720));
        let p = video_plan(Level::Compact, "avc1", 1280, 960, 25.0, 1_700_000.0).unwrap();
        assert_eq!((p.width, p.height), (960, 720));
        // déjà en HEVC à petit débit, à la bonne taille : on n'y touche pas
        assert!(video_plan(Level::Balanced, "hvc1", 1920, 1080, 30.0, 3_000_000.0).is_none());
        // HEVC d'un iPhone à gros débit : si
        assert!(video_plan(Level::Balanced, "hvc1", 1920, 1080, 30.0, 16_000_000.0).is_some());
    }

    #[test]
    fn channels_are_kept_only_when_they_differ() {
        // même voix des deux côtés
        assert_eq!(mix_of(100.0, 100.0, 0.0), Mix::Mono);
        assert_eq!(mix_of(100.0, 99.0, 0.001), Mix::Mono);
        // une voie muette : l'autre seule
        assert_eq!(mix_of(100.0, 0.0, 25.0), Mix::Left);
        assert_eq!(mix_of(0.01, 50.0, 12.0), Mix::Right);
        // vraie stéréo (ambiance d'une interview dans la rue : différence 20 dB sous les voies)
        assert_eq!(mix_of(100.0, 100.0, 1.0), Mix::Stereo);
        // silence complet
        assert_eq!(mix_of(0.0, 0.0, 0.0), Mix::Mono);
    }

    #[test]
    fn audio_only_when_worth_it() {
        // MP3 de LingQ 128 kbit/s, voix en mono : 64 kbit/s
        assert!(audio_worth(128_000.0, audio_bitrate(Level::Balanced, 1)));
        // AAC de YouTube 128 kbit/s en stéréo : 96 kbit/s, 25 % de gagné
        assert!(audio_worth(129_000.0, audio_bitrate(Level::Balanced, 2)));
        // voix naturelle de Lumen, déjà en AAC 64 kbit/s mono
        assert!(!audio_worth(64_000.0, audio_bitrate(Level::Balanced, 1)));
        assert!(kept(100, 80) && !kept(100, 81) && !kept(100, 0));
    }

    #[test]
    fn youtube_fragmented_duration_is_not_doubled() {
        // ftyp, puis moov : mvhd (version 0, 30 000 par seconde, 117,02 s) et mvex
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&30_000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&3_510_507u32.to_be_bytes());
        let mut moov_body = Vec::new();
        moov_body.extend_from_slice(&((8 + mvhd.len()) as u32).to_be_bytes());
        moov_body.extend_from_slice(b"mvhd");
        moov_body.extend_from_slice(&mvhd);
        moov_body.extend_from_slice(&8u32.to_be_bytes());
        moov_body.extend_from_slice(b"mvex");
        let mut file = Vec::new();
        file.extend_from_slice(&16u32.to_be_bytes());
        file.extend_from_slice(b"ftypdash\0\0\0\0");
        file.extend_from_slice(&((8 + moov_body.len()) as u32).to_be_bytes());
        file.extend_from_slice(b"moov");
        file.extend_from_slice(&moov_body);
        let p = std::env::temp_dir().join(format!("lumen-mp4-head-{}.mp4", std::process::id()));
        std::fs::write(&p, &file).unwrap();
        let (d, frag) = mp4_header(&p).unwrap();
        assert!((d - 117.0169).abs() < 0.01 && frag);
        // macOS compte 234 s : on retrouve 117 ; une durée cohérente reste telle quelle
        assert!((true_duration(&p, 234.03) - 117.017).abs() < 0.01);
        assert_eq!(true_duration(&p, 117.02), 117.02);
        let _ = std::fs::remove_file(&p);
        assert!(mp4_header(Path::new("/nulle/part.mp4")).is_none());
    }

    #[test]
    fn light_files_sit_next_to_the_original() {
        let p = Path::new("/m/20261006-143028074.video.mp4");
        assert_eq!(light_path(p, "mp4"), Path::new("/m/20261006-143028074.video.light.mp4"));
        assert_eq!(temp_path(&light_path(p, "mp4")), Path::new("/m/20261006-143028074.video.light-tmp.mp4"));
        assert!(readable(Path::new("a.MP3")) && !readable(Path::new("a.webm")) && !readable(Path::new("a.mkv")));
    }

    /// Allège toutes les leçons d'une copie des vraies données, comme le bouton
    /// « Alléger » (rien n'est touché dans les données d'origine) :
    /// `LUMEN_LIGHTEN_DATA="$HOME/Library/Application Support/app.lumen.reader" cargo test --lib lighten_live -- --ignored --nocapture`
    /// Vérifie que chaque leçon cite un fichier qui existe, de même durée.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore]
    async fn lighten_live() {
        let Ok(data) = std::env::var("LUMEN_LIGHTEN_DATA") else { return };
        let level = Level::parse(std::env::var("LUMEN_TEST_QUALITY").ok().as_deref());
        let data = PathBuf::from(data);
        let tmp = std::env::temp_dir().join(format!("lumen-lighten-{}", std::process::id()));
        let media = tmp.join("media");
        std::fs::create_dir_all(&media).unwrap();
        // copie cohérente de la base, chemins ramenés vers la copie des médias
        let db_copy = tmp.join("lumen.db");
        {
            let src = rusqlite::Connection::open_with_flags(data.join("lumen.db"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            src.execute("VACUUM INTO ?1", [db_copy.to_string_lossy()]).unwrap();
        }
        let conn = rusqlite::Connection::open(&db_copy).unwrap();
        let old = crate::media::media_dir(&data).to_string_lossy().into_owned();
        let new = media.to_string_lossy().into_owned();
        conn.execute("UPDATE lessons SET media_path=replace(media_path, ?1, ?2), video_path=replace(video_path, ?1, ?2)", [&old, &new]).unwrap();
        let mut before = std::collections::HashMap::new();
        for LessonFile { path, .. } in lesson_files(&conn).unwrap() {
            let from = Path::new(&old).join(Path::new(&path).file_name().unwrap());
            if std::fs::copy(&from, &path).is_ok() {
                before.insert(path.clone(), probe(Path::new(&path)).map(|p| p.duration).unwrap_or(0.0));
            }
        }
        let total_before: u64 = before.keys().map(|p| size(Path::new(p))).sum();
        let db = parking_lot::Mutex::new(conn);
        let cancel = AtomicBool::new(false);
        let t0 = std::time::Instant::now();
        let shown = std::sync::Mutex::new(u32::MAX);
        // vidéos en ligne : retéléchargées en VP9 si c'est plus léger (comme sur ce Mac)
        let refetch = Refetch { data_dir: tmp.clone(), codecs: vec!["vp9".into()], browser: None };
        let report = lighten_lessons(&db, &media, level, Some(&refetch), &cancel, &|p: Progress| {
            let mut last = shown.lock().unwrap();
            if *last != p.done {
                *last = p.done;
                println!("  {}/{} {} ({:.0} Mo gagnés)", p.done, p.total, p.title, p.saved as f64 / 1e6);
            }
        })
        .await
        .unwrap();
        let secs = t0.elapsed().as_secs_f64();
        // chaque leçon cite un fichier présent, de même durée que l'original
        let conn = db.lock();
        let rows = crate::db::media_files(&conn).unwrap();
        let mut after_files = std::collections::HashSet::new();
        for (id, title, a, v, _) in rows {
            for p in [a, v].into_iter().flatten() {
                assert!(Path::new(&p).exists(), "leçon {id} « {title} » : {p} absent");
                after_files.insert(p);
            }
        }
        let total_after: u64 = after_files.iter().map(|p| size(Path::new(p))).sum();
        println!(
            "{:?} : {} fichiers allégés, {} échecs, {:.0} → {:.0} Mo ({:.0} %), {:.0} s",
            level,
            report.done,
            report.failed,
            total_before as f64 / 1e6,
            total_after as f64 / 1e6,
            total_after as f64 / total_before.max(1) as f64 * 100.0,
            secs
        );
        for (id, title, _, v, _) in crate::db::media_files(&conn).unwrap() {
            if let Some(v) = v {
                println!("  vidéo {id} « {title} » : {} ({:.0} Mo)", Path::new(&v).file_name().unwrap().to_string_lossy(), size(Path::new(&v)) as f64 / 1e6);
            }
        }
        for p in after_files.iter().filter(|p| p.contains(".light.")) {
            let d = probe(Path::new(p)).unwrap().duration;
            let stem = Path::new(p).file_name().unwrap().to_string_lossy().split(".light.").next().unwrap().to_string();
            if let Some((_, d0)) = before.iter().find(|(k, _)| Path::new(k).file_name().unwrap().to_string_lossy().starts_with(&stem)) {
                assert!((d - d0).abs() < 0.2, "{p} : durée {d0} → {d}");
            }
        }
        assert_eq!(report.failed, 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Allège de vrais fichiers et vérifie que rien ne bouge dans le temps :
    /// `LUMEN_TEST_VIDEO=film.mp4 LUMEN_TEST_AUDIO=son.mp3 cargo test --release --lib compress_live -- --ignored --nocapture`
    /// (`LUMEN_TEST_QUALITY=compact` pour l'autre réglage). Les fichiers allégés
    /// restent dans un dossier jetable, affiché.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore]
    async fn compress_live() {
        let level = Level::parse(std::env::var("LUMEN_TEST_QUALITY").ok().as_deref());
        let dir = std::env::temp_dir().join(format!("lumen-compress-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = |var: &str| -> Vec<PathBuf> {
            std::env::var(var)
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    let src = Path::new(s.trim());
                    let dst = dir.join(src.file_name().unwrap());
                    std::fs::copy(src, &dst).unwrap();
                    dst
                })
                .collect()
        };
        let cancel = AtomicBool::new(false);
        for v in copy("LUMEN_TEST_VIDEO") {
            let p = probe(&v).unwrap();
            let t0 = std::time::Instant::now();
            let with_sound = p.audio.is_some() && std::env::var("LUMEN_TEST_SPLIT").is_ok();
            let res = if with_sound { split(&v, level, &cancel, |_| {}).await } else { video(&v, level, &cancel, |_| {}).await };
            let secs = t0.elapsed().as_secs_f64();
            let vi = p.video.as_ref().unwrap();
            match res.unwrap() {
                Some(l) => {
                    let out = probe(l.video.as_ref().unwrap()).unwrap();
                    let ov = out.video.as_ref().unwrap();
                    println!(
                        "{} : {} {}x{} {:.0} kbit/s → {} {}x{} {:.0} kbit/s, {:.0} % ({:.1} → {:.1} Mo), {:.1} s pour {:.0} s ({:.1}× le temps réel)",
                        v.file_name().unwrap().to_string_lossy(),
                        vi.codec, vi.width, vi.height, vi.bitrate / 1000.0,
                        ov.codec, ov.width, ov.height, ov.bitrate / 1000.0,
                        l.after as f64 / l.before as f64 * 100.0,
                        l.before as f64 / 1e6, l.after as f64 / 1e6,
                        secs, p.duration, p.duration / secs
                    );
                    assert_eq!(ov.codec, "hvc1");
                    // même durée, au dixième de seconde
                    assert!((out.duration - p.duration).abs() < 0.15, "durée {} → {}", p.duration, out.duration);
                    if let Some(a) = &l.audio {
                        println!("  son à part : {}", a.display());
                    }
                }
                None => println!("{} : rien à gagner ({:.1} s)", v.file_name().unwrap().to_string_lossy(), secs),
            }
        }
        for a in copy("LUMEN_TEST_AUDIO") {
            let p = probe(&a).unwrap();
            let ai = p.audio.as_ref().unwrap();
            let t0 = std::time::Instant::now();
            let mix = if ai.channels == 2 { mac::analyze_audio(&a, p.duration).unwrap() } else { Mix::Mono };
            let res = audio(&a, level, &cancel).await.unwrap();
            let secs = t0.elapsed().as_secs_f64();
            let Some(l) = res else {
                println!("{} : {} {:.0} kbit/s, {:?}, rien à gagner", a.file_name().unwrap().to_string_lossy(), ai.codec, ai.bitrate / 1000.0, mix);
                continue;
            };
            let out = l.audio.as_ref().unwrap();
            let po = probe(out).unwrap();
            // décalage entre l'original et l'allégé : corrélation sur 20 s au milieu
            let (x, rate) = mac::decode_mono(&a).unwrap();
            let (y, rate2) = mac::decode_mono(out).unwrap();
            let lag = if (rate - rate2).abs() < 1.0 {
                let mid = x.len() / 2;
                let n = (20.0 * rate) as usize;
                let max = (0.2 * rate) as i64;
                let xs = &x[mid.min(x.len().saturating_sub(n))..][..n.min(x.len())];
                let (mut best, mut best_lag) = (f64::MIN, 0i64);
                let start = mid.min(x.len().saturating_sub(n)) as i64;
                for lag in -max..=max {
                    let s = start + lag;
                    if s < 0 || (s as usize + xs.len()) > y.len() {
                        continue;
                    }
                    let ys = &y[s as usize..][..xs.len()];
                    let c: f64 = xs.iter().zip(ys).map(|(a, b)| (*a as f64) * (*b as f64)).sum();
                    if c > best {
                        best = c;
                        best_lag = lag;
                    }
                }
                best_lag as f64 / rate * 1000.0
            } else {
                f64::NAN
            };
            println!(
                "{} : {} {:.0} kbit/s {} voies → {:?} AAC {:.0} kbit/s, {:.0} % ({:.1} → {:.1} Mo), durée {:.2} → {:.2} s, décalage {:.1} ms, {:.1} s",
                a.file_name().unwrap().to_string_lossy(),
                ai.codec, ai.bitrate / 1000.0, ai.channels, mix,
                po.audio.as_ref().unwrap().bitrate / 1000.0,
                l.after as f64 / l.before as f64 * 100.0,
                l.before as f64 / 1e6, l.after as f64 / 1e6,
                p.duration, po.duration, lag, secs
            );
            assert!((po.duration - p.duration).abs() < 0.1);
            assert!(lag.is_nan() || lag.abs() < 2.0, "décalage de {lag} ms");
        }
        println!("fichiers : {}", dir.display());
    }
}
