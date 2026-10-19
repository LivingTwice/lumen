//! Texte des transcriptions par Parakeet TDT 0.6B v3 (NVIDIA, CC BY 4.0), lancé
//! par l'outil `sherpa-onnx-offline` du moteur partagé avec la voix (`sherpa.rs`).
//!
//! Comme Qwen3-ASR, il écrit le texte ; Whisper garde le minutage des mots. Ses
//! propres temps, par pas de 80 ms, sont moins fins que ceux de Whisper recalés
//! (70 ms d'écart médian, plus de 200 ms pour un mot sur dix).
//!
//! Le modèle est une compression maison des poids d'origine (`tools/build_parakeet.py` :
//! 8 bits par blocs de 32) : aussi juste que l'original, aussi rapide que l'int8
//! publié par sherpa-onnx, qui fait un tiers de fautes en plus et tronque des mots
//! (« ingle » pour « inglese »).

use std::path::Path;
#[cfg(any(not(target_os = "ios"), test))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_os = "ios"))]
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::asr::split_on_silence;
#[cfg(any(not(target_os = "ios"), test))]
use crate::asr::RATE;
use crate::sherpa::{self, Tool};

/// Langues où Parakeet écrit le texte : celles des 24 qu'il connaît où il fait au
/// moins aussi bien que Whisper seul (mots faux, FLEURS, 15 min par langue, octobre
/// 2026 : ukrainien 7,6 % contre 28,9 %, croate 13,7 contre 23,1, lituanien 24,5
/// contre 38,4, hongrois 15,9 contre 24,0 ; égalité en allemand, polonais, slovène,
/// bulgare). Pas le suédois, le danois, le finnois, le roumain ni le letton, où
/// Whisper fait nettement mieux, ni le grec : son vocabulaire n'a pas de « ς »
/// final (« επίση » pour « επίσης »).
pub fn knows(lang: &str) -> bool {
    matches!(lang, "en" | "it" | "de" | "pt" | "ru" | "es" | "fr" | "nl" | "pl" | "cs" | "hu" | "et" | "lt" | "sk" | "sl" | "hr" | "bg" | "uk")
}

/// Langues où Parakeet passe avant Qwen3-ASR quand les deux sont installés :
/// l'italien (14 leçons LingQ : 2,7 % de mots faux contre 3,2 %) et le néerlandais
/// (FLEURS : 7,3 contre 8,5). Ailleurs, Qwen3-ASR fait aussi bien ou mieux.
pub fn before_qwen(lang: &str) -> bool {
    matches!(lang, "it" | "nl")
}

/// Durée visée d'un morceau, coupé dans le silence le plus proche. Plus court
/// que pour Qwen3-ASR : sur 30 s, Parakeet lâche parfois toute une phrase après
/// un changement de voix (FLEURS : jusqu'à 38 mots d'affilée ; 10 au plus sur 15 s,
/// sans rien perdre sur les leçons italiennes).
const CHUNK_SECS: f64 = 15.0;

/// Morceaux par lancement de l'outil (deux minutes de son) : il les décode
/// ensemble, et la mémoire grandit avec leur nombre (environ 2,4 Go ici).
#[cfg(not(target_os = "ios"))]
const GROUP: usize = 8;

/// Fils de calcul : 6 au plus, rien n'allait plus vite au-delà (M2 Pro).
fn threads() -> i32 {
    crate::ai::threads().min(6)
}

/// Les fichiers du modèle, dans son dossier.
#[cfg(any(not(target_os = "ios"), test))]
fn files(model: &Path) -> [PathBuf; 4] {
    ["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"].map(|f| model.join(f))
}

/// Écrit le texte d'un son (mono, 16 kHz, f32). `on_progress` : de 0 à 100.
pub async fn transcribe(data_dir: &Path, model: &Path, pcm: &[f32], cancel: &AtomicBool, mut on_progress: impl FnMut(f64)) -> Result<String> {
    if !sherpa::ready(data_dir, Tool::Asr) {
        return Err(anyhow!(crate::i18n::t("le moteur de Parakeet est absent, réinstallez le modèle", "Parakeet's engine is missing, reinstall the model")));
    }
    let tmp = std::env::temp_dir().join(format!("lumen-parakeet-{}", crate::media::new_stem()));
    std::fs::create_dir_all(&tmp)?;
    let res = run(data_dir, model, pcm, &tmp, cancel, &mut on_progress).await;
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// iPhone et iPad : la bibliothèque sherpa-onnx dans l'app (sherpa_ios.rs), mêmes morceaux.
#[cfg(target_os = "ios")]
async fn run(_data_dir: &Path, model: &Path, pcm: &[f32], _tmp: &Path, cancel: &AtomicBool, on_progress: &mut impl FnMut(f64)) -> Result<String> {
    let pieces = split_on_silence(pcm, CHUNK_SECS);
    let engine = tokio::task::block_in_place(|| crate::sherpa_ios::Parakeet::load(model, threads()))?;
    // la mémoire d'un iPhone : deux morceaux à la fois (environ 1,4 Go avec le modèle),
    // quatre s'il en reste beaucoup (iPad)
    let group = if crate::ios::available_memory() > 4 << 30 { 4 } else { 2 };
    let mut texts: Vec<String> = Vec::new();
    for group in pieces.chunks(group) {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("annulé"));
        }
        let slices: Vec<&[f32]> = group.iter().map(|&(a, b)| &pcm[a..b]).collect();
        texts.extend(tokio::task::block_in_place(|| engine.decode(&slices)));
        let done = group.last().map(|p| p.1).unwrap_or(pcm.len());
        on_progress(done as f64 / pcm.len().max(1) as f64 * 100.0);
    }
    Ok(texts.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "))
}

#[cfg(not(target_os = "ios"))]
async fn run(data_dir: &Path, model: &Path, pcm: &[f32], tmp: &Path, cancel: &AtomicBool, on_progress: &mut impl FnMut(f64)) -> Result<String> {
    let pieces = split_on_silence(pcm, CHUNK_SECS);
    let mut texts: Vec<String> = Vec::new();
    for (g, group) in pieces.chunks(GROUP).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("annulé"));
        }
        let mut wavs = Vec::new();
        for (k, &(a, b)) in group.iter().enumerate() {
            let path = tmp.join(format!("{g:04}-{k}.wav"));
            let samples: Vec<i16> = pcm[a..b].iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
            std::fs::write(&path, crate::voice::write_wav(RATE as u32, &samples))?;
            wavs.push(path);
        }
        texts.extend(decode(data_dir, model, &wavs).await?);
        let done = group.last().map(|p| p.1).unwrap_or(pcm.len());
        on_progress(done as f64 / pcm.len().max(1) as f64 * 100.0);
        for w in wavs {
            let _ = std::fs::remove_file(w);
        }
    }
    Ok(texts.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "))
}

/// Un lancement de l'outil : le texte de chaque fichier, dans l'ordre.
#[cfg(not(target_os = "ios"))]
async fn decode(data_dir: &Path, model: &Path, wavs: &[PathBuf]) -> Result<Vec<String>> {
    let [encoder, decoder, joiner, tokens] = files(model);
    let mut cmd = crate::proc::tokio_command(sherpa::bin(data_dir, Tool::Asr));
    cmd.arg(format!("--encoder={}", encoder.display()))
        .arg(format!("--decoder={}", decoder.display()))
        .arg(format!("--joiner={}", joiner.display()))
        .arg(format!("--tokens={}", tokens.display()))
        .arg("--model-type=nemo_transducer")
        .arg(format!("--num-threads={}", threads()))
        .arg("--print-args=false")
        .args(wavs)
        .kill_on_drop(true);
    // deux minutes de son se décodent en quelques secondes : une demi-minute par morceau, c'est un blocage
    let limit = Duration::from_secs(30 * wavs.len() as u64 + 60);
    let out = tokio::time::timeout(limit, cmd.output())
        .await
        .map_err(|_| anyhow!(crate::i18n::t("Parakeet a mis trop de temps à répondre", "Parakeet took too long to answer")))??;
    let texts = parse(&String::from_utf8_lossy(&out.stdout));
    if !out.status.success() || texts.len() != wavs.len() {
        let err = String::from_utf8_lossy(&out.stderr);
        eprintln!("Parakeet : {}", err.lines().rev().take(5).collect::<Vec<_>>().join(" | "));
        return Err(anyhow!(crate::i18n::t("Parakeet n'a pas pu transcrire ce son", "Parakeet couldn't transcribe this audio")));
    }
    Ok(texts)
}

/// Une ligne JSON par fichier (`{"text": …, "tokens": …}`) ; le reste est ignoré.
#[cfg_attr(target_os = "ios", allow(dead_code))]
fn parse(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v["text"].as_str().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tool_output() {
        let out = "{\"lang\": \"\", \"emotion\": \"\", \"event\": \"\", \"text\": \"Alles hat ein Ende,  nur die Wurst hat zwei.\", \"timestamps\": [0.00, 0.24], \"tokens\":[\" Al\", \"les\"], \"words\": []}\n\
                   num threads: 4\n\
                   {\"lang\": \"\", \"text\": \"\", \"tokens\": []}\n\
                   {\"lang\": \"\", \"text\": \"Ciao a tutti.\"}\n";
        assert_eq!(parse(out), vec!["Alles hat ein Ende, nur die Wurst hat zwei.", "", "Ciao a tutti."]);
        assert!(parse("Done!\n----\n").is_empty());
    }

    #[test]
    fn languages() {
        assert!(knows("it") && knows("uk") && knows("hr"));
        // celles que Parakeet ne connaît pas, ou où Whisper fait mieux, restent à Qwen3-ASR et à Whisper
        for l in ["tr", "ar", "hi", "id", "vi", "ko", "ja", "el", "fi", "sv"] {
            assert!(!knows(l) && !before_qwen(l), "{l}");
        }
        // Parakeet ne passe avant Qwen3-ASR que là où il le connaît
        for l in crate::text::LANGS {
            assert!(!before_qwen(l) || knows(l), "{l}");
        }
    }

    /// Les Réglages et la barre latérale disent qui écrit (lib/writers.ts) : mêmes listes qu'ici.
    #[test]
    fn interface_mirror() {
        let ts = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/writers.ts")).unwrap();
        let set = |name: &str| -> Vec<String> {
            let start = ts.find(&format!("const {name} = new Set([")).unwrap_or_else(|| panic!("{name}"));
            let body = &ts[start..];
            body[body.find('[').unwrap() + 1..body.find(']').unwrap()].split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect()
        };
        let (qwen, parakeet, first) = (set("QWEN"), set("PARAKEET"), set("PARAKEET_FIRST"));
        for l in crate::text::LANGS {
            let has = |v: &[String]| v.iter().any(|x| x == l);
            assert_eq!(has(&qwen), crate::asr::writes(l), "Qwen3-ASR, {l}");
            assert_eq!(has(&parakeet), knows(l), "Parakeet, {l}");
            assert_eq!(has(&first), before_qwen(l), "Parakeet d'abord, {l}");
        }
    }
}

/// Test réel : LUMEN_PARAKEET_DIR=dossier du modèle (encoder.onnx…) LUMEN_TEST_AUDIO=a.wav,b.wav
/// (WAV f32 16 kHz mono : afconvert -f WAVE -d LEF32@16000 -c 1) [LUMEN_PARAKEET_OUT=dossier]
/// cargo test --lib parakeet_live -- --ignored --nocapture
/// Le moteur se télécharge dans un dossier jetable.
#[cfg(test)]
mod live {
    use super::*;
    use std::sync::Arc;

    fn read_wav(path: &str) -> Vec<f32> {
        let b = std::fs::read(path).unwrap();
        let mut i = 12;
        while i + 8 <= b.len() {
            let id = &b[i..i + 4];
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if id == b"data" {
                return b[i + 8..(i + 8 + len).min(b.len())].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            }
            i += 8 + len + (len & 1);
        }
        panic!("pas de données dans {path}");
    }

    #[tokio::test]
    #[ignore]
    async fn parakeet_live() {
        let (Ok(model), Ok(files)) = (std::env::var("LUMEN_PARAKEET_DIR"), std::env::var("LUMEN_TEST_AUDIO")) else { return };
        let data = std::env::temp_dir().join("lumen-parakeet-live");
        std::fs::create_dir_all(&data).unwrap();
        let t = std::time::Instant::now();
        sherpa::ensure(&data, Tool::Asr, Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap();
        println!("moteur prêt en {:.1} s", t.elapsed().as_secs_f64());
        for f in files.split(',') {
            let pcm = read_wav(f);
            let t = std::time::Instant::now();
            let mut steps = 0;
            let text = transcribe(&data, Path::new(&model), &pcm, &AtomicBool::new(false), |_| steps += 1).await.unwrap();
            let secs = pcm.len() as f64 / RATE as f64;
            println!("\n=== {f} : {secs:.0} s d'audio, transcrit en {:.1} s ({steps} étapes)\n{text}", t.elapsed().as_secs_f64());
            if let Ok(out) = std::env::var("LUMEN_PARAKEET_OUT") {
                let name = Path::new(f).file_stem().unwrap().to_string_lossy().to_string();
                std::fs::write(Path::new(&out).join(format!("{name}.parakeet.txt")), &text).unwrap();
            }
            assert!(!text.is_empty());
        }
    }

    /// Téléchargement complet comme dans l'app (moteur et modèle, ≈ 1 Go), depuis la
    /// version publiée ou un serveur local (`python3 -m http.server 8765` dans le dossier
    /// de tools/build_parakeet.py) : [LUMEN_PARAKEET_URL=http://127.0.0.1:8765/parakeet-tdt-0.6b-v3-q8.tar]
    /// [LUMEN_TEST_AUDIO=a.wav] cargo test --lib parakeet_download_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn parakeet_download_live() {
        use crate::models;
        let data = std::env::temp_dir().join("lumen-parakeet-download");
        let _ = std::fs::remove_dir_all(&data);
        let mut m = models::find("parakeet-v3").unwrap().clone();
        if let Ok(url) = std::env::var("LUMEN_PARAKEET_URL") {
            m.url = Box::leak(url.into_boxed_str());
        }
        let t = std::time::Instant::now();
        let mut last = 0u64;
        models::download(&data, &m, Arc::new(AtomicBool::new(false)), |e| {
            if let models::DownloadEvent::Progress { received, total, .. } = e {
                assert!(received >= last && received <= total + 1024, "progression {received}/{total}");
                last = received;
            }
        })
        .await
        .unwrap();
        println!("installé en {:.1} s ({last} octets annoncés sur {})", t.elapsed().as_secs_f64(), m.size);
        assert!(models::installed(&data, &m));
        assert!(!models::part_of(&data, &m).exists());
        let model = models::path_of(&data, &m);
        assert!(files(&model).iter().all(|f| f.exists()));
        if let Ok(f) = std::env::var("LUMEN_TEST_AUDIO") {
            let pcm = read_wav(&f);
            let text = transcribe(&data, &model, &pcm, &AtomicBool::new(false), |_| {}).await.unwrap();
            println!("{} mots transcrits", text.split_whitespace().count());
            assert!(!text.is_empty());
        }
        let _ = std::fs::remove_dir_all(&data);
    }
}
