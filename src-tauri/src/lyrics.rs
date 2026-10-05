//! Chansons : paroles trouvées dans LRCLIB (base ouverte et gratuite de paroles,
//! souvent minutées ligne par ligne au format LRC), et leur mise en page en leçon.
//! Avec des paroles minutées, la lanterne suit la chanson sans transcription ; si
//! Whisper est installé, elle est recalée mot à mot sur la voix.

use std::time::Duration;

use anyhow::Result;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{link, text};

const API: &str = "https://lrclib.net/api";
/// LRCLIB demande qu'on se présente.
const UA: &str = "Lumen (https://github.com/LivingTwice/lumen)";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    /// paroles minutées (format LRC), vide si l'on n'a que le texte
    pub synced: String,
    pub plain: String,
    pub instrumental: bool,
}

impl Lyrics {
    pub fn is_empty(&self) -> bool {
        self.synced.trim().is_empty() && self.plain.trim().is_empty()
    }

    /// Le texte seul, ligne par ligne (pour reconnaître la langue, compter les mots).
    pub fn text(&self) -> String {
        if !self.plain.trim().is_empty() {
            return self.plain.clone();
        }
        parse_lrc(&self.synced).into_iter().map(|(_, l)| l).collect::<Vec<_>>().join("\n")
    }
}

pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(UA).connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build()?)
}

fn of(v: &Value) -> Lyrics {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Lyrics { synced: s("syncedLyrics"), plain: s("plainLyrics"), instrumental: v.get("instrumental").and_then(Value::as_bool).unwrap_or(false) }
}

/// Paroles d'une chanson : la correspondance exacte (titre, artiste, album, durée),
/// sinon la meilleure de la recherche (durée proche, paroles minutées de préférence).
pub async fn find(c: &reqwest::Client, artist: &str, title: &str, album: &str, duration: f64) -> Option<Lyrics> {
    let (artist, title) = (artist.trim(), title.trim());
    if artist.is_empty() || title.is_empty() {
        return None;
    }
    if !album.trim().is_empty() && duration > 0.0 {
        let d = (duration.round() as i64).to_string();
        let url = Url::parse_with_params(
            &format!("{API}/get"),
            &[("artist_name", artist), ("track_name", title), ("album_name", album.trim()), ("duration", d.as_str())],
        )
        .ok()?;
        if let Ok(r) = c.get(url).send().await {
            if r.status().is_success() {
                if let Ok(v) = json_of(r).await {
                    let l = of(&v);
                    if !l.is_empty() {
                        return Some(l);
                    }
                }
            }
        }
    }
    let url = Url::parse_with_params(&format!("{API}/search"), &[("track_name", title), ("artist_name", artist)]).ok()?;
    let list = json_of(c.get(url).send().await.ok()?).await.ok()?;
    let rows = list.as_array()?;
    let close = |v: &Value| {
        let d = v.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
        duration <= 0.0 || d <= 0.0 || (d - duration).abs() <= 6.0
    };
    let fits = |v: &Value| link::similarity(v.get("trackName").and_then(Value::as_str).unwrap_or(""), title) >= 0.5;
    let pick = rows
        .iter()
        .filter(|v| close(v) && fits(v))
        .max_by_key(|v| (!of(v).synced.is_empty(), !of(v).plain.is_empty()))
        .or_else(|| rows.iter().find(|v| fits(v) && !of(v).is_empty()))?;
    let l = of(pick);
    (!l.is_empty() || l.instrumental).then_some(l)
}

/// Lecture d'une réponse JSON sans la fonction `json` de reqwest (non compilée).
async fn json_of(r: reqwest::Response) -> Result<Value> {
    Ok(serde_json::from_str(&r.text().await?)?)
}

/// Lignes d'un texte LRC, dans l'ordre du temps : « [01:02.50] paroles ». Une
/// ligne peut porter plusieurs instants (refrain répété) ; les lignes vides
/// marquent les pauses. `[offset:+300]` décale tout (en millisecondes).
pub fn parse_lrc(s: &str) -> Vec<(f64, String)> {
    let mut offset = 0.0;
    let mut out: Vec<(f64, String)> = Vec::new();
    for line in s.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let tag = &inner[..end];
            if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
            } else if let Some((m, sec)) = tag.split_once(':') {
                if let (Ok(m), Ok(sec)) = (m.trim().parse::<f64>(), sec.trim().replace(',', ".").parse::<f64>()) {
                    times.push(m * 60.0 + sec);
                }
            }
            rest = inner[end + 1..].trim_start();
        }
        for t in times {
            out.push((t, rest.trim().to_string()));
        }
    }
    for x in &mut out {
        x.0 = (x.0 - offset).max(0.0);
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// La chanson en leçon : une ligne des paroles par paragraphe (une ligne vide
/// entre les couplets), et, si les paroles sont minutées, les horodatages des
/// mots, répartis dans le temps de chaque ligne.
pub fn layout(lyrics: &Lyrics, lang: &str, duration: f64) -> (String, Option<String>) {
    let lines = parse_lrc(&lyrics.synced);
    if lines.iter().any(|(_, l)| !l.is_empty()) {
        let mut text = String::new();
        let mut len16 = 0usize;
        let mut timings: Vec<[f64; 4]> = Vec::new();
        let mut gap = false;
        for (i, (t0, line)) in lines.iter().enumerate() {
            if line.is_empty() {
                gap = true;
                continue;
            }
            if !text.is_empty() {
                let sep = if gap { "\n\n" } else { "\n" };
                text.push_str(sep);
                len16 += sep.len();
            }
            gap = false;
            // la ligne dure jusqu'à la suivante (pause comprise), dix secondes au plus
            let next = lines.get(i + 1).map(|x| x.0).unwrap_or(if duration > *t0 { duration } else { t0 + 6.0 });
            let t1 = (next - 0.12).min(t0 + 10.0).max(t0 + 0.3);
            crate::lingq::spread_words(line, lang, len16, round2(*t0), round2(t1), &mut timings);
            text.push_str(line);
            len16 += line.encode_utf16().count();
        }
        return (text, Some(serde_json::to_string(&timings).unwrap_or_else(|_| "[]".into())));
    }
    // paroles sans minutage : les couplets tels quels
    let mut text = String::new();
    let mut gap = false;
    for line in lyrics.plain.lines().map(str::trim) {
        if line.is_empty() {
            gap = true;
            continue;
        }
        if !text.is_empty() {
            text.push_str(if gap { "\n\n" } else { "\n" });
        }
        gap = false;
        text.push_str(line);
    }
    (text, None)
}

/// Mots des paroles (pour savoir si elles sont assez longues pour une leçon).
pub fn word_count(lyrics: &Lyrics, lang: &str) -> usize {
    text::tokenize(&lyrics.text(), lang).iter().filter(|t| t.w).count()
}

/// Ce que les titres de clips ajoutent autour du nom de la chanson.
const NOISE: &[&str] = &[
    "official", "video", "audio", "lyric", "visualizer", "visualiser", "clip", "videoclip", "videoclipe", "musical", "music", "vidéo", "officiel",
    "oficial", "ufficiale", "offizielles", "prod", "remaster", "hd", "4k", "mv", "m/v", "testo", "paroles", "letra", "lyrics", "karaoke",
];

/// Artiste et titre d'un clip YouTube : « Artiste - Titre (Official Video) ».
/// `channel` : la chaîne, à défaut d'artiste dans le titre.
pub fn split_title(video_title: &str, channel: &str) -> (String, String) {
    // parenthèses et crochets de décor retirés, les autres gardés (« (Remix) »)
    let mut clean = String::new();
    let mut rest = video_title;
    while let Some(i) = rest.find(['(', '[', '【']) {
        clean.push_str(&rest[..i]);
        let close = match rest[i..].chars().next() {
            Some('(') => ')',
            Some('[') => ']',
            _ => '】',
        };
        let Some(j) = rest[i..].find(close) else {
            rest = &rest[i..];
            break;
        };
        let inner = &rest[i..i + j + close.len_utf8()];
        let low = inner.to_lowercase();
        if !NOISE.iter().any(|n| low.contains(n)) && !low.contains("feat") && !low.contains("ft.") {
            clean.push_str(inner);
        }
        rest = &rest[i + j + close.len_utf8()..];
    }
    clean.push_str(rest);
    let clean: String = clean.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace() || ",.'’&!?-–—|:()".contains(*c)).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut_feat = |s: &str| {
        let low = s.to_lowercase();
        let i = [" feat.", " feat ", " ft.", " ft ", " featuring "].iter().filter_map(|p| low.find(p)).min();
        i.map_or(s, |i| &s[..i]).trim().to_string()
    };
    for sep in [" - ", " – ", " — ", " | ", " ｜ "] {
        if let Some((a, b)) = clean.split_once(sep) {
            let b = b.split(" | ").next().unwrap_or(b);
            return (cut_feat(a), cut_feat(b));
        }
    }
    let artist = channel.trim_end_matches(" - Topic").trim_end_matches("VEVO").trim_end_matches(" OFFICIAL").trim_end_matches(" Official").trim();
    (artist.to_string(), cut_feat(&clean))
}

#[cfg(test)]
mod tests {
    use super::*;

    // paroles inventées pour les tests (jamais de vraies paroles ici)
    const LRC: &str = "[ar:Lumen]\n[00:01.00] La luce del mattino\n[00:04.50] Sopra il mare calmo\n[00:08.00]\n[00:10.00][00:20.00] Canta piano, canta\n[00:14.00] Il giorno nuovo";

    #[test]
    fn lrc_lines() {
        let l = parse_lrc(LRC);
        assert_eq!(l.len(), 6);
        assert_eq!(l[0], (1.0, "La luce del mattino".to_string()));
        assert_eq!(l[2], (8.0, String::new()));
        // la ligne répétée revient à son second instant
        assert_eq!(l[5], (20.0, "Canta piano, canta".to_string()));
        let shifted = parse_lrc("[offset:+500]\n[00:02.00] Ciao");
        assert_eq!(shifted[0].0, 1.5);
    }

    #[test]
    fn song_layout() {
        let lyrics = Lyrics { synced: LRC.into(), ..Default::default() };
        let (text, timings) = layout(&lyrics, "it", 30.0);
        assert_eq!(text, "La luce del mattino\nSopra il mare calmo\n\nCanta piano, canta\nIl giorno nuovo\nCanta piano, canta");
        let t: Vec<[f64; 4]> = serde_json::from_str(&timings.unwrap()).unwrap();
        // « La » commence avec la ligne ; « luce » au bon endroit du texte
        assert_eq!(t[0][2], 1.0);
        assert_eq!(&text[t[1][0] as usize..t[1][1] as usize], "luce");
        // « Sopra » : deuxième ligne, à 4,5 s
        let sopra = t.iter().find(|x| &text[x[0] as usize..x[1] as usize] == "Sopra").unwrap();
        assert_eq!(sopra[2], 4.5);
        // la dernière ligne dure jusqu'à la fin de la chanson, dix secondes au plus
        assert!(t.last().unwrap()[3] <= 30.0);
        let plain = Lyrics { plain: "Una riga\nUn'altra\n\nSecondo verso".into(), ..Default::default() };
        assert_eq!(layout(&plain, "it", 0.0), ("Una riga\nUn'altra\n\nSecondo verso".to_string(), None));
    }

    #[test]
    fn clip_titles() {
        assert_eq!(split_title("Lumi - LA STRADA feat. Aurora (Official Video)", "Lumi"), ("Lumi".into(), "LA STRADA".into()));
        assert_eq!(split_title("Aurora, Lumi - Il mare [Official Music Video] 🌊", "x"), ("Aurora, Lumi".into(), "Il mare".into()));
        assert_eq!(split_title("La notte (Remix)", "Lumi - Topic"), ("Lumi".into(), "La notte (Remix)".into()));
        assert_eq!(split_title("La notte", "LumiVEVO"), ("Lumi".into(), "La notte".into()));
    }

    /// Recherche réelle dans LRCLIB (n'affiche que des métadonnées, jamais les paroles) :
    /// `cargo test --lib lyrics_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn lyrics_live() {
        let c = client().unwrap();
        let l = find(&c, "Laura Pausini", "La solitudine", "", 237.0).await.expect("paroles trouvées");
        println!("minutées : {} · lignes : {} · langue : {:?}", !l.synced.is_empty(), parse_lrc(&l.synced).len(), crate::langid::guess(&l.text()));
        assert!(!l.is_empty());
    }

    /// Chaîne complète d'une leçon de chanson, comme `import_song` : paroles, version de
    /// l'album sur YouTube, son téléchargé, mise en page, et recalage par Whisper si
    /// `LUMEN_ASR_MODEL` est donné (copier d'abord le sidecar en `target/debug/deps/lumen-whisper`).
    /// N'affiche que des nombres : `cargo test --lib song_import_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn song_import_live() {
        let (artist, title, lang) = ("Laura Pausini", "La solitudine", "it");
        let dir = std::env::temp_dir().join(format!("lumen-song-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let l = find(&client().unwrap(), artist, title, "", 0.0).await.expect("paroles");
        let (text, lrc) = layout(&l, lang, 240.0);
        let lines = text.lines().filter(|x| !x.trim().is_empty()).count();
        println!("paroles : {lines} lignes, {} mots, minutées : {}", word_count(&l, lang), lrc.is_some());
        let started = std::time::Instant::now();
        let url = crate::search::song_url(&dir, artist, title).await.unwrap();
        let ytdlp = crate::tools::find_ytdlp(&dir).expect("yt-dlp");
        let (audio, _) = crate::media::yt_audio(&dir, &ytdlp, &url, "song", None, &mut |_| {}).await.unwrap();
        println!("son : {} Ko en {:.1} s", std::fs::metadata(&audio).unwrap().len() / 1024, started.elapsed().as_secs_f64());
        if let Ok(model) = std::env::var("LUMEN_ASR_MODEL") {
            let started = std::time::Instant::now();
            let words = crate::media::transcribe(std::path::Path::new(&model), &audio, lang, None, |_| {}).await.unwrap();
            let (_, found) = crate::media::align_timings(&text, lang, &words);
            println!("Whisper : {:.0} % des mots des paroles entendus ({:.1} s) → {}", found * 100.0, started.elapsed().as_secs_f64(), if found >= 0.35 { "lanterne mot à mot" } else { "lanterne ligne à ligne" });
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
