//! Qui parle quand : les interlocuteurs d'une conversation (« diarisation »).
//!
//! Deux petits modèles, lancés par l'outil `sherpa-onnx-offline-speaker-diarization`
//! du moteur partagé avec la voix et Parakeet (`sherpa.rs`) : Pyannote 3.0 (MIT)
//! repère les tours de parole, TitaNet Small (NVIDIA, CC BY 4.0) reconnaît les voix,
//! puis les tours d'une même voix sont regroupés. Rien ne dépend de la langue.
//!
//! Lumen s'en sert pour la mise en page : chaque mot minuté par Whisper revient à
//! une voix, et chaque réplique a son paragraphe (`media::airy_turns`), même quand
//! deux personnes se coupent la parole au milieu d'une phrase. Mesuré sur des
//! podcasts à deux voix (Easy Italian, 25 min) : les deux animateurs retrouvés, une
//! minute de calcul sur M2 Pro ; un récit lu : une seule voix.

use std::path::PathBuf;

#[cfg(not(target_os = "ios"))]
use anyhow::anyhow;
use anyhow::Result;

use crate::media::{byte_positions, sentence_stop};
use crate::text;

/// Un tour de parole : de `t0` à `t1` (secondes), la voix `who`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    pub t0: f64,
    pub t1: f64,
    pub who: u32,
}

/// Les fichiers du modèle, et le dossier de données (où se trouve le moteur).
pub struct Model {
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    pub data_dir: PathBuf,
    /// reconnaît les voix (TitaNet)
    pub voices: PathBuf,
    /// repère les tours de parole (Pyannote)
    pub turns: PathBuf,
}

/// Distance en deçà de laquelle deux tours sont de la même voix. La valeur
/// d'origine de l'outil (0,5) trouvait 70 voix dans un podcast à deux ; à 0,9,
/// les deux animateurs, et quelques voix d'un instant (générique, rires).
const THRESHOLD: f32 = 0.9;

/// Pas de l'analyse, en part de sa fenêtre de 10 s. Au quart : deux fois plus
/// rapide qu'au dixième (58 s contre 180 pour 25 min), mêmes voix ; à la moitié,
/// une voix se dédouble.
const SHIFT: f32 = 0.25;

/// Fils de calcul : 6 au plus, comme Parakeet.
fn threads() -> i32 {
    crate::ai::threads().min(6)
}

/// Une écoute en cours. Sur Mac et Windows, l'outil tourne déjà, à côté du modèle
/// qui écrit le texte ; l'abandonner l'arrête.
pub struct Listening {
    #[cfg(not(target_os = "ios"))]
    job: tokio::task::JoinHandle<Result<Vec<Turn>>>,
    #[cfg(not(target_os = "ios"))]
    progress: tokio::sync::mpsc::UnboundedReceiver<f64>,
    #[cfg(not(target_os = "ios"))]
    wav: PathBuf,
    #[cfg(target_os = "ios")]
    model: (PathBuf, PathBuf),
}

#[cfg(not(target_os = "ios"))]
impl Drop for Listening {
    fn drop(&mut self) {
        self.job.abort();
        let _ = std::fs::remove_file(&self.wav);
    }
}

/// Commence à écouter qui parle dans un son (mono, 16 kHz).
#[cfg(not(target_os = "ios"))]
pub fn start(model: &Model, pcm: &[f32]) -> Result<Listening> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let wav = std::env::temp_dir().join(format!("lumen-voix-{}.wav", crate::media::new_stem()));
    let samples: Vec<i16> = pcm.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
    std::fs::write(&wav, crate::voice::write_wav(crate::asr::RATE as u32, &samples))?;
    let secs = pcm.len() as u64 / crate::asr::RATE as u64;
    let mut cmd = crate::sherpa::command(&model.data_dir, crate::sherpa::Tool::Speakers);
    cmd.arg(format!("--segmentation.pyannote-model={}", model.turns.display()))
        .arg(format!("--segmentation.pyannote-window-shift-ratio={SHIFT}"))
        .arg(format!("--segmentation.num-threads={}", threads()))
        .arg(format!("--embedding.model={}", model.voices.display()))
        .arg(format!("--embedding.num-threads={}", threads()))
        .arg(format!("--clustering.cluster-threshold={THRESHOLD}"))
        .arg("--print-args=false")
        .arg(&wav)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let (tx, progress) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let job = tokio::spawn(async move {
        let mut child = cmd.spawn()?;
        let (mut stdout, stderr) = (child.stdout.take().expect("sortie"), child.stderr.take().expect("erreurs"));
        // les deux sorties sont lues ensemble : l'outil annonce son avancement d'un côté,
        // et rend les tours de parole de l'autre
        let out = async {
            let mut s = String::new();
            stdout.read_to_string(&mut s).await.map(|_| s)
        };
        let err = async {
            let mut lines = BufReader::new(stderr).lines();
            let mut last = String::new();
            while let Ok(Some(l)) = lines.next_line().await {
                match l.strip_prefix("progress ").and_then(|p| p.trim_end_matches('%').parse::<f64>().ok()) {
                    Some(p) => drop(tx.send(p)),
                    None if !l.trim().is_empty() => last = l,
                    None => {}
                }
            }
            last
        };
        // un quart d'heure de son s'écoute en moins d'une minute : aussi long que le son, c'est un blocage
        let limit = std::time::Duration::from_secs(secs + 120);
        let (out, last) = tokio::time::timeout(limit, async { tokio::join!(out, err) })
            .await
            .map_err(|_| anyhow!(crate::i18n::t("la reconnaissance des interlocuteurs a mis trop de temps", "speaker recognition took too long")))?;
        let status = child.wait().await?;
        if !status.success() {
            eprintln!("interlocuteurs : {last}");
            return Err(anyhow!(crate::i18n::t("les interlocuteurs n'ont pas pu être reconnus", "the speakers couldn't be recognized")));
        }
        Ok(parse(&out?))
    });
    Ok(Listening { job, progress, wav })
}

/// iPhone et iPad : la mémoire ne permet pas d'écouter pendant qu'un autre modèle
/// écrit ; tout se fait dans `finish`.
#[cfg(target_os = "ios")]
pub fn start(model: &Model, _pcm: &[f32]) -> Result<Listening> {
    Ok(Listening { model: (model.voices.clone(), model.turns.clone()) })
}

impl Listening {
    /// Les tours de parole, dans l'ordre. `on_progress` : de 0 à 100.
    #[cfg(not(target_os = "ios"))]
    pub async fn finish(mut self, _pcm: &[f32], mut on_progress: impl FnMut(f64)) -> Result<Vec<Turn>> {
        loop {
            tokio::select! {
                Some(p) = self.progress.recv() => on_progress(p),
                r = &mut self.job => return r?,
            }
        }
    }

    /// iPhone et iPad : par la bibliothèque sherpa-onnx, dans l'app (sherpa_ios.rs).
    #[cfg(target_os = "ios")]
    pub async fn finish(self, pcm: &[f32], mut on_progress: impl FnMut(f64)) -> Result<Vec<Turn>> {
        on_progress(0.0);
        let turns = tokio::task::block_in_place(|| crate::sherpa_ios::speakers(&self.model.0, &self.model.1, pcm, THRESHOLD, SHIFT, threads()))?;
        on_progress(100.0);
        Ok(turns)
    }
}

/// Une ligne par tour : « 0.318 -- 6.865 speaker_00 » ; le reste est ignoré.
#[cfg_attr(target_os = "ios", allow(dead_code))]
fn parse(stdout: &str) -> Vec<Turn> {
    let mut out: Vec<Turn> = stdout
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let (t0, sep, t1, who) = (p.next()?.parse().ok()?, p.next()?, p.next()?.parse().ok()?, p.next()?);
            (sep == "--").then_some(Turn { t0, t1, who: who.strip_prefix("speaker_")?.parse().ok()? })
        })
        .collect();
    out.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    out
}

/// Écart, en mots, où chercher la ponctuation qui porte un changement de voix : les
/// instants des mots et ceux des tours de parole s'accordent à un mot ou deux près.
const SNAP: usize = 2;

/// Au plus cette longueur, une réplique doit être une phrase entière (« Sì. »,
/// « Che cosa? ») ; sinon c'est un mot mal attribué au bord d'un tour.
const SHORT: usize = 2;

/// Silence entre deux mots (secondes) qui vaut une fin de phrase : la parole a été
/// laissée (un lecteur qui reprend son souffle à une virgule s'arrête moins longtemps).
const PAUSE: f64 = 1.5;

/// Deux voix qui se relaient au moins tant de fois se jugent sur leurs changements.
const SEEN: usize = 6;

/// En dessous de cette part de changements tombant à une fin de phrase (à un mot
/// près), deux « voix » n'en sont qu'une, que l'outil a comptée deux fois : on ne
/// change pas de voix au milieu d'une phrase une fois sur deux. Mesuré sur le texte
/// d'un podcast à deux voix (Parakeet) : 77 à 88 % entre les deux animateurs, 25 à
/// 33 % entre les deux moitiés du même.
const SAME: f64 = 0.55;

/// Au-dessus (et relayées au moins vingt fois), deux voix sont assez sûres pour qu'une
/// parole coupée au milieu d'une phrase, sans fin de phrase ni silence, ouvre une réplique.
const SURE: f64 = 0.75;

/// Les mots qui ouvrent une réplique : leur position (unités UTF-16) dans `text`,
/// d'après ses horodatages [[début, fin, t0, t1], …] et les tours de parole. Chaque
/// mot revient à la voix qui parle le plus pendant qu'il est dit (à égalité, le
/// tour le plus long : un « mh » de l'autre ne coupe pas une phrase) ; un
/// changement de voix se cale sur la ponctuation la plus proche ; une réplique
/// d'un mot ou deux qui n'est pas une phrase entière rejoint sa voisine.
///
/// Le regroupement des voix par l'outil est fragile : selon le son, à un rien près,
/// une même personne compte pour deux. Le texte le trahit (`SAME`) : ces deux
/// « voix » sont réunies. Et un changement au milieu d'une phrase n'est gardé que
/// si les deux voix sont sûres (`SURE`). Dans le doute, pas de réplique : la mise en
/// page reste celle d'avant, une phrase par paragraphe. Une seule voix : rien.
pub fn openings(text: &str, lang: &str, timings: &str, turns: &[Turn]) -> Vec<usize> {
    use std::collections::HashMap;
    let times: Vec<[f64; 4]> = serde_json::from_str(timings).unwrap_or_default();
    let words: Vec<text::Token> = text::tokenize(text, lang).into_iter().filter(|t| t.w).collect();
    let n = words.len();
    if n < 2 || turns.is_empty() {
        return Vec::new();
    }
    let when: HashMap<usize, (f64, f64)> = times.iter().map(|t| (t[0] as usize, (t[2], t[3]))).collect();
    // 1. la voix de chaque mot
    let voice = |(t0, t1): (f64, f64)| {
        let t1 = t1.max(t0 + 0.01);
        turns
            .iter()
            .map(|t| ((t.t1.min(t1) - t.t0.max(t0)), t.t1 - t.t0, t.who))
            .filter(|x| x.0 > 0.0)
            .max_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal))
            .map(|x| x.2)
    };
    let heard: Vec<Option<u32>> = words.iter().map(|w| when.get(&w.s).copied().and_then(voice)).collect();
    // un mot dit dans un silence des tours, ou sans horodatage : la voix d'avant (au début, celle d'après)
    let Some(mut last) = heard.iter().flatten().next().copied() else { return Vec::new() };
    let mut who: Vec<u32> = heard
        .iter()
        .map(|w| {
            last = w.unwrap_or(last);
            last
        })
        .collect();
    // 2. ce qui sépare le mot k du suivant : fin de phrase (3), virgule (1), rien (0) ; et le silence entre eux
    let byte_at = byte_positions(text);
    let byte = |p: usize| byte_at[p.min(byte_at.len() - 1)];
    let mark = |k: usize| {
        let g = &text[byte(words[k].e)..byte(words[k + 1].s)];
        if g.contains('\n') || sentence_stop(g, &text[byte(words[k + 1].s)..byte(words[k + 1].e)]).is_some() {
            3.0
        } else if g.contains([',', ';', ':', '–', '—', '，', '、', '；', '：']) {
            1.0
        } else {
            0.0
        }
    };
    let pause = |k: usize| match (when.get(&words[k].s), when.get(&words[k + 1].s)) {
        (Some(a), Some(b)) => b.0 - a.1,
        _ => 0.0,
    };
    // un changement de voix « tombe bien » : à une fin de phrase, à un mot près
    let falls_well = |k: usize| (k.saturating_sub(1)..=(k + 1).min(n - 2)).any(|j| mark(j) >= 3.0);
    let pair = |a: u32, b: u32| (a.min(b), a.max(b));
    // changements entre chaque paire de voix : combien, et combien tombent bien
    let relays = |who: &[u32]| {
        let mut m: HashMap<(u32, u32), (usize, usize)> = HashMap::new();
        for k in 0..n - 1 {
            if who[k] != who[k + 1] {
                let e = m.entry(pair(who[k], who[k + 1])).or_default();
                e.0 += 1;
                e.1 += falls_well(k) as usize;
            }
        }
        m
    };
    // 3. deux voix qui se relaient au milieu des phrases n'en sont qu'une
    loop {
        let worst = relays(&who)
            .into_iter()
            .filter(|(_, (seen, well))| *seen >= SEEN && (*well as f64) < SAME * *seen as f64)
            .min_by(|a, b| (a.1 .1 * b.1 .0).cmp(&(b.1 .1 * a.1 .0)).then(a.0.cmp(&b.0)));
        let Some(((a, b), _)) = worst else { break };
        who.iter_mut().filter(|w| **w == b).for_each(|w| *w = a);
    }
    if who.iter().all(|w| *w == who[0]) {
        return Vec::new();
    }
    let sure: std::collections::HashSet<(u32, u32)> = relays(&who).into_iter().filter(|(_, (seen, well))| *seen >= 20 && *well as f64 >= SURE * *seen as f64).map(|(p, _)| p).collect();
    // 4. chaque changement de voix (après le mot k) se cale sur la ponctuation voisine ; au
    // milieu d'une phrase, il n'est gardé qu'entre deux voix sûres, ou à un long silence
    let changes: Vec<usize> = (0..n - 1).filter(|&k| who[k] != who[k + 1]).collect();
    let mut cuts: Vec<usize> = Vec::new();
    for (c, &k) in changes.iter().enumerate() {
        // sans dépasser le changement d'avant ni celui d'après
        let lo = cuts.last().map_or(0, |p| p + 1).max(k.saturating_sub(SNAP));
        let hi = changes.get(c + 1).map_or(n - 2, |next| next - 1).min(k + SNAP);
        let best = (lo..=hi.max(lo))
            .filter(|j| *j < n - 1)
            .map(|j| (mark(j) - 0.4 * (j as f64 - k as f64).abs(), j))
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then((b.1 as i64 - k as i64).abs().cmp(&(a.1 as i64 - k as i64).abs())));
        if let Some((_, j)) = best {
            let kept = mark(j) >= 3.0 || pause(j) >= PAUSE || sure.contains(&pair(who[k], who[k + 1]));
            if kept && cuts.last() != Some(&j) {
                cuts.push(j);
            }
        }
    }
    // 5. les répliques [a, b] et leur voix (celle de la plupart de leurs mots)
    let mut runs: Vec<(usize, usize, u32)> = Vec::new();
    let mut a = 0;
    for b in cuts.iter().copied().chain([n - 1]) {
        let mut count: Vec<(u32, usize)> = Vec::new();
        for w in &who[a..=b] {
            match count.iter_mut().find(|c| c.0 == *w) {
                Some(c) => c.1 += 1,
                None => count.push((*w, 1)),
            }
        }
        let voice = count.iter().max_by_key(|c| c.1).map_or(who[a], |c| c.0);
        runs.push((a, b, voice));
        a = b + 1;
    }
    // 6. une réplique d'un mot ou deux qui n'est pas une phrase entière rejoint sa voisine,
    // et deux répliques voisines de la même voix n'en font qu'une
    loop {
        let before = runs.len();
        let mut i = 0;
        while i + 1 < runs.len() {
            if runs[i].2 == runs[i + 1].2 {
                runs[i].1 = runs[i + 1].1;
                runs.remove(i + 1);
            } else {
                i += 1;
            }
        }
        let loose = (0..runs.len()).find(|&i| {
            let (a, b, _) = runs[i];
            b + 1 - a <= SHORT && runs.len() > 1 && ((a > 0 && mark(a - 1) < 3.0) || (b < n - 1 && mark(b) < 3.0))
        });
        if let Some(i) = loose {
            let (a, b, _) = runs[i];
            // du côté où rien ne la sépare de sa voisine
            if a > 0 && (mark(a - 1) < 3.0 || i + 1 == runs.len()) {
                runs[i - 1].1 = b;
            } else {
                runs[i + 1].0 = a;
            }
            runs.remove(i);
        }
        if runs.len() == before {
            break;
        }
    }
    runs.iter().skip(1).map(|r| words[r.0].s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Horodatages d'un texte : un mot toutes les 0,5 s.
    fn timed(text: &str) -> String {
        let t: Vec<[f64; 4]> = text::tokenize(text, "it").into_iter().filter(|w| w.w).enumerate().map(|(k, w)| [w.s as f64, w.e as f64, k as f64 * 0.5, k as f64 * 0.5 + 0.4]).collect();
        serde_json::to_string(&t).unwrap()
    }

    /// Les mots qui ouvrent une réplique.
    fn opens(text: &str, turns: &[(f64, f64, u32)]) -> Vec<String> {
        let turns: Vec<Turn> = turns.iter().map(|&(t0, t1, who)| Turn { t0, t1, who }).collect();
        let u: Vec<u16> = text.encode_utf16().collect();
        let words: Vec<text::Token> = text::tokenize(text, "it").into_iter().filter(|w| w.w).collect();
        openings(text, "it", &timed(text), &turns).into_iter().map(|s| String::from_utf16_lossy(&u[s..words.iter().find(|w| w.s == s).unwrap().e])).collect()
    }

    /// Répliques d'un vrai texte : LUMEN_TEST_TEXT=texte.txt LUMEN_TEST_WORDS=mots.json (sortie `done`
    /// de lumen-whisper, tableau des mots) LUMEN_TURNS=sortie de l'outil des voix [LUMEN_TEST_LANG=it]
    /// cargo test --lib openings_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn openings_live() {
        let (Ok(t), Ok(w), Ok(d)) = (std::env::var("LUMEN_TEST_TEXT"), std::env::var("LUMEN_TEST_WORDS"), std::env::var("LUMEN_TURNS")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "it".into());
        let text = std::fs::read_to_string(t).unwrap();
        let words: Vec<crate::media::WWord> = serde_json::from_str(&std::fs::read_to_string(w).unwrap()).unwrap();
        let turns = parse(&std::fs::read_to_string(d).unwrap());
        let (timings, found) = crate::media::align_timings(&text, &lang, &words);
        let open = openings(&text, &lang, &timings, &turns);
        let tm: Vec<[f64; 4]> = serde_json::from_str(&timings).unwrap();
        let u: Vec<u16> = text.encode_utf16().collect();
        println!("{} tours, {:.0} % des mots retrouvés, {} répliques", turns.len(), found * 100.0, open.len() + 1);
        for s in open.iter().take(std::env::var("LUMEN_SHOW").ok().and_then(|n| n.parse().ok()).unwrap_or(40)) {
            let t0 = tm.iter().find(|x| x[0] as usize == *s).map_or(-1.0, |x| x[2]);
            let (a, b) = (s.saturating_sub(40), (s + 40).min(u.len()));
            println!("{t0:8.1} s  …{} ⏎ {}", String::from_utf16_lossy(&u[a..*s]).replace('\n', " "), String::from_utf16_lossy(&u[*s..b]).replace('\n', " "));
        }
    }

    /// Installation complète comme dans l'app (moteur et les deux modèles, 67 Mo, dans un dossier
    /// jetable), puis écoute d'un vrai son : [LUMEN_TEST_AUDIO=son.wav (f32, 16 kHz, mono)]
    /// cargo test --lib speakers_download_live -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn speakers_download_live() {
        use crate::models;
        let data = std::env::temp_dir().join("lumen-speakers-download");
        let _ = std::fs::remove_dir_all(&data);
        let m = models::find("speakers-1").unwrap();
        let t = std::time::Instant::now();
        let mut last = 0u64;
        models::download(&data, m, Default::default(), |e| {
            if let models::DownloadEvent::Progress { received, total, .. } = e {
                assert!(received >= last && received <= total + 1024, "progression {received}/{total}");
                last = received;
            }
        })
        .await
        .unwrap();
        println!("installé en {:.1} s ({last} octets annoncés sur {})", t.elapsed().as_secs_f64(), m.size);
        assert!(models::installed(&data, m));
        let model = Model { data_dir: data.clone(), voices: models::path_of(&data, m), turns: models::companion_path(&data, m).unwrap() };
        let sizes = (std::fs::metadata(&model.voices).unwrap().len(), std::fs::metadata(&model.turns).unwrap().len());
        assert_eq!(sizes.0 + sizes.1 + crate::sherpa::SIZE, m.size, "{sizes:?}");
        if let Ok(f) = std::env::var("LUMEN_TEST_AUDIO") {
            let b = std::fs::read(f).unwrap();
            let at = b.windows(4).position(|w| w == b"data").unwrap() + 8;
            let pcm: Vec<f32> = b[at..].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            let t = std::time::Instant::now();
            let mut steps = 0;
            let turns = start(&model, &pcm).unwrap().finish(&pcm, |_| steps += 1).await.unwrap();
            let mut voices: Vec<u32> = turns.iter().map(|t| t.who).collect();
            voices.sort_unstable();
            voices.dedup();
            println!("{:.0} s de son écoutés en {:.1} s ({steps} étapes) : {} tours, {} voix", pcm.len() as f64 / 16_000.0, t.elapsed().as_secs_f64(), turns.len(), voices.len());
            assert!(!turns.is_empty() && steps > 0);
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn reads_the_tool_output() {
        let out = "Started\n\n7.017 -- 10.747 speaker_01\n0.318 -- 6.865 speaker_00\nDuration : 56.861 s\nElapsed seconds: 16.870 s\n";
        assert_eq!(parse(out), vec![Turn { t0: 0.318, t1: 6.865, who: 0 }, Turn { t0: 7.017, t1: 10.747, who: 1 }]);
        assert!(parse("progress 12.5%\n").is_empty());
    }

    #[test]
    fn each_reply_opens_where_its_sentence_does() {
        //          0     1     2     3      4      5    6      7     8     9     10   11
        let text = "Come stai oggi? Bene, grazie. Che cosa hai fatto ieri sera? Niente.";
        // les tours tombent un mot trop tard, puis un mot trop tôt : calés sur la ponctuation
        assert_eq!(opens(text, &[(0.0, 1.9, 0), (1.9, 2.4, 1), (2.4, 6.0, 0)]), ["Bene", "Che"]);
        assert_eq!(opens(text, &[(0.0, 1.4, 0), (1.4, 2.6, 1), (2.6, 5.4, 0), (5.4, 6.0, 1)]), ["Bene", "Che", "Niente"]);
        // une seule voix, ou aucun tour : rien
        assert!(opens(text, &[(0.0, 6.0, 0)]).is_empty());
        assert!(opens(text, &[]).is_empty());
    }

    /// Douze échanges de questions et de réponses (une voix chacune), un mot toutes les
    /// 0,5 s, puis `tail` : le texte et les tours de parole du dialogue.
    fn dialogue(tail: &str) -> (String, Vec<(f64, f64, u32)>) {
        let mut text = String::new();
        let mut turns = Vec::new();
        let mut at = 0.0;
        for i in 0..12 {
            for (line, who) in [(format!("Che cosa hai fatto il giorno {i}?"), 0), ("Niente di speciale, sono rimasto a casa.".to_string(), 1)] {
                let len = text::word_count(&line, "it") as f64 * 0.5;
                turns.push((at, at + len, who));
                at += len;
                text.push_str(&line);
                text.push(' ');
            }
        }
        text.push_str(tail);
        (text, turns)
    }

    #[test]
    fn interruptions_and_backchannels() {
        let (base, turns) = dialogue("");
        let end = turns.last().unwrap().1;
        let with = |tail: &str, more: &[(f64, f64, u32)]| {
            let (text, mut turns) = dialogue(tail);
            turns.extend(more.iter().map(|&(a, b, w)| (end + a, end + b, w)));
            opens(&text, &turns)
        };
        assert_eq!(opens(base.trim_end(), &turns).len(), 23);
        // la suite, dite par la première voix, ouvre une 24e réplique
        // la parole coupée au milieu d'une phrase, entre deux voix sûres : la réplique s'ouvre
        // là, sans ponctuation (7 mots, puis l'autre)
        let cut = "Non ha nulla a che fare con confermi che non ha nulla a che fare con l'Italia.";
        let opened = with(cut, &[(0.0, 3.45, 0), (3.45, 12.0, 1)]);
        assert_eq!((opened.len(), opened.last().unwrap().as_str()), (25, "confermi"));
        // une troisième voix, entendue là seulement : dans le doute, pas de réplique au milieu de la phrase
        assert_eq!(with(cut, &[(0.0, 3.45, 0), (3.45, 12.0, 7)]).len(), 24);
        // un « mh » de l'autre pendant la phrase (tour court, à cheval) ne la coupe pas
        let long = "Allora io penso che domani andremo tutti insieme al mare con la macchina.";
        assert_eq!(with(long, &[(0.0, 7.0, 0), (2.0, 2.4, 1)]).len(), 24);
        // un mot seul mal attribué au milieu d'une phrase rejoint ses voisins
        assert_eq!(with(long, &[(0.0, 2.9, 0), (2.9, 3.6, 1), (3.6, 7.0, 0)]).len(), 24);
        // une réponse d'un mot, phrase entière, garde sa voix, même entre deux voix peu entendues
        let text = "Andiamo al mare domani? Sì. Allora preparo la macchina stasera.";
        assert_eq!(opens(text, &[(0.0, 2.0, 0), (2.0, 2.5, 1), (2.5, 6.0, 0)]), ["Sì", "Allora"]);
    }

    #[test]
    fn one_voice_counted_twice_is_one() {
        // un récit lu d'une seule voix, que l'outil partage en deux « voix » qui changent toutes
        // les 3,5 s, le plus souvent au milieu des phrases : elles sont réunies, aucune réplique
        let text = (0..30).map(|i| format!("Il signor Rossi numero {i} va al mercato ogni mattina presto, compra il pane e torna a casa.")).collect::<Vec<_>>().join(" ");
        let secs = text::word_count(&text, "it") as f64 * 0.5;
        let turns: Vec<(f64, f64, u32)> = (0..(secs / 3.5) as usize + 1).map(|i| (i as f64 * 3.5, (i + 1) as f64 * 3.5, (i % 2) as u32 + 3)).collect();
        assert!(opens(&text, &turns).is_empty());
    }
}
