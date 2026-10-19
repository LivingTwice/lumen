//! Whisper dans l'app : sur iPhone et iPad, où iOS interdit le programme à part
//! `lumen-whisper`. whisper.cpp 1.9.4 (dossier `whisper/`, licence MIT) y est
//! compilé avec les en-têtes de la ggml de llama.cpp et lié à elle : une seule ggml
//! dans le programme, donc aucun conflit de symboles (build.rs). Mêmes réglages et
//! même minutage des mots que `lumen-whisper` (fichier partagé `timing.rs`).
//!
//! Sur Mac, ce chemin s'essaie en compilant avec `LUMEN_WHISPER_INAPP=1`
//! (`whisper_inapp_live`) ; l'app Mac garde son programme à part.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};

#[path = "../lumen-whisper/src/timing.rs"]
mod timing;
pub use timing::Word;
use timing::{assemble, dtw_name, refine, Envelope, Tok, TARGET_RATE};

#[repr(C)]
struct LwToken {
    id: i32,
    t0: i64,
    t1: i64,
    t_dtw: i64,
    text: *const c_char,
}

extern "C" {
    fn lw_init(model: *const c_char, use_gpu: c_int, preset: *const c_char) -> *mut c_void;
    fn lw_full(
        ctx: *mut c_void,
        pcm: *const f32,
        n: c_int,
        lang: *const c_char,
        threads: c_int,
        beam: c_int,
        progress: extern "C" fn(c_int, *mut c_void),
        progress_user: *mut c_void,
        abort: extern "C" fn(*mut c_void) -> bool,
        abort_user: *mut c_void,
    ) -> c_int;
    fn lw_n_segments(ctx: *mut c_void) -> c_int;
    fn lw_n_tokens(ctx: *mut c_void, seg: c_int) -> c_int;
    fn lw_eot(ctx: *mut c_void) -> c_int;
    fn lw_token(ctx: *mut c_void, seg: c_int, i: c_int, out: *mut LwToken);
    fn lw_free(ctx: *mut c_void);
}

/// Le modèle chargé, libéré à la fin de la transcription.
struct Ctx(*mut c_void);

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY : contexte rendu par lw_init, libéré une seule fois
        unsafe { lw_free(self.0) }
    }
}

extern "C" fn on_progress(p: c_int, user: *mut c_void) {
    // SAFETY : `user` pointe sur la fermeture de `transcribe`, vivante pendant lw_full
    let f = unsafe { &mut *(user as *mut &mut dyn FnMut(f64)) };
    f(p as f64);
}

extern "C" fn on_abort(user: *mut c_void) -> bool {
    // SAFETY : `user` pointe sur le drapeau d'arrêt, vivant pendant lw_full
    unsafe { &*(user as *const AtomicBool) }.load(Ordering::Relaxed)
}

/// Fils de calcul : les cœurs de performance de l'iPhone (sur la puce graphique,
/// Whisper n'en demande presque pas).
fn threads() -> c_int {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 6);
    // iPhone qui chauffe : moitié moins de cœurs
    (if crate::proc::hot() { (n / 2).max(2) } else { n }) as c_int
}

/// Transcrit `audio` (mono, 16 kHz) : les mots et leurs instants, calés sur la voix
/// comme le fait `lumen-whisper`. `gpu` : sur la puce graphique (Metal).
pub fn transcribe(model: &Path, audio: &[f32], lang: &str, gpu: bool, cancel: &AtomicBool, mut on_progress_cb: impl FnMut(f64)) -> Result<Vec<Word>> {
    if audio.is_empty() {
        return Err(anyhow!(crate::i18n::t("le fichier ne contient pas de son exploitable", "the file contains no usable sound")));
    }
    let path = CString::new(model.to_string_lossy().as_bytes())?;
    let name = model.to_string_lossy().to_string();
    let preset = dtw_name(&name);
    let preset_c = CString::new(preset.unwrap_or(""))?;
    // SAFETY : chaînes C valides pendant l'appel
    let raw = unsafe { lw_init(path.as_ptr(), gpu as c_int, preset_c.as_ptr()) };
    if raw.is_null() {
        return Err(anyhow!(crate::i18n::t("modèle Whisper illisible", "unreadable Whisper model")));
    }
    let ctx = Ctx(raw);
    let lang_c = CString::new(lang)?;
    let mut cb: &mut dyn FnMut(f64) = &mut on_progress_cb;
    // SAFETY : tampons et rappels valides pendant l'appel (bloquant)
    let status = unsafe {
        lw_full(
            ctx.0,
            audio.as_ptr(),
            audio.len() as c_int,
            lang_c.as_ptr(),
            threads(),
            3,
            on_progress,
            &mut cb as *mut &mut dyn FnMut(f64) as *mut c_void,
            on_abort,
            cancel as *const AtomicBool as *mut c_void,
        )
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(anyhow!("annulé"));
    }
    if status != 0 {
        return Err(anyhow!(crate::tr!("transcription échouée ({})", "transcription failed ({})", status)));
    }
    // SAFETY : lectures du résultat d'un contexte vivant
    let (eot, segments) = unsafe {
        let eot = lw_eot(ctx.0);
        let segments: Vec<Vec<Tok>> = (0..lw_n_segments(ctx.0))
            .map(|s| {
                (0..lw_n_tokens(ctx.0, s))
                    .map(|i| {
                        let mut t = LwToken { id: 0, t0: 0, t1: 0, t_dtw: -1, text: std::ptr::null() };
                        lw_token(ctx.0, s, i, &mut t);
                        let bytes = if t.text.is_null() { Vec::new() } else { CStr::from_ptr(t.text).to_bytes().to_vec() };
                        Tok { id: t.id, bytes, t0: t.t0, t_dtw: t.t_dtw }
                    })
                    .collect()
            })
            .collect();
        (eot, segments)
    };
    drop(ctx);
    let mut words = assemble(&segments, eot, preset.is_some());
    let duration = audio.len() as f64 / TARGET_RATE as f64;
    refine(&mut words, &Envelope::new(audio), duration);
    Ok(words)
}

#[cfg(test)]
mod tests {
    /// Whisper dans l'app, sur un vrai son (Mac, compilé avec `LUMEN_WHISPER_INAPP=1`) :
    /// `LUMEN_WHISPER_INAPP=1 LUMEN_ASR_MODEL=ggml-….bin LUMEN_TEST_AUDIO=son.mp3 LUMEN_TEST_LANG=it
    /// cargo test --release --lib whisper_inapp_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn whisper_inapp_live() {
        let model = std::env::var("LUMEN_ASR_MODEL").expect("LUMEN_ASR_MODEL");
        let audio = std::env::var("LUMEN_TEST_AUDIO").expect("LUMEN_TEST_AUDIO");
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "it".into());
        let started = std::time::Instant::now();
        let pcm = crate::compress::pcm_mono(std::path::Path::new(&audio), 16_000.0).unwrap();
        println!("décodé : {:.1} s de son en {:.1} s", pcm.len() as f64 / 16_000.0, started.elapsed().as_secs_f64());
        let started = std::time::Instant::now();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut last = 0.0;
        let words = super::transcribe(std::path::Path::new(&model), &pcm, &lang, true, &stop, |p| last = p).unwrap();
        println!("{} mots en {:.1} s (progression {last} %)", words.len(), started.elapsed().as_secs_f64());
        for w in words.iter().take(12) {
            println!("  {:>6.2} {:>6.2} {}", w.t0, w.t1, w.w);
        }
        assert!(words.len() > 10);
        assert!(words.windows(2).all(|p| p[0].t0 <= p[1].t0));
    }
}
