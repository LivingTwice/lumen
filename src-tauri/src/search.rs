//! Chercher : des vidéos, des podcasts, des chansons et des articles en ligne,
//! dans la langue étudiée, sans passer par un navigateur. Chaque plateforme a sa
//! façon de chercher (yt-dlp pour YouTube, les API publiques de Dailymotion,
//! d'Apple Podcasts, de Deezer, de LRCLIB et de Wikipédia) ; les résultats ont
//! tous la même forme (`Hit`). Rien n'est téléchargé : l'apprenant regarde ou
//! écoute d'abord (`stream`), puis en fait une leçon par l'import habituel.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{stream, StreamExt};
use parking_lot::Mutex;
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

use crate::i18n::t;
use crate::lyrics::{self, Lyrics};
use crate::media::{self, ImportEvent};
use crate::{discover, langid, link};

/// Résultats par page (YouTube, Dailymotion, Deezer).
const PER_PAGE: usize = 20;

/// Un résultat, quelle que soit la plateforme.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Hit {
    /// identifiant unique : plateforme et identifiant chez elle
    pub id: String,
    /// "youtube", "dailymotion", "podcast", "music" ou "wiki"
    pub platform: String,
    /// "video", "audio", "show" (une émission), "song" ou "text"
    pub kind: String,
    pub title: String,
    /// ce qu'on regarde et importe : vidéo, fichier son, flux de l'émission, article
    pub url: String,
    /// page d'origine, à ouvrir dans le navigateur
    pub page: String,
    pub image: String,
    /// chaîne, émission, artiste ou encyclopédie
    pub author: String,
    pub summary: String,
    /// secondes, 0 si inconnue
    pub duration: f64,
    /// secondes depuis 1970, 0 si inconnue
    pub published: i64,
    /// vues (vidéos), épisodes (émissions)
    pub count: i64,
    /// niveaux annoncés (1 = A1 … 5 = C1), 0 si inconnus
    pub lo: u8,
    pub hi: u8,
    /// dans la langue étudiée (vrai), dans une autre (faux), on ne sait pas (null)
    pub in_lang: Option<bool>,
    /// langue reconnue quand ce n'est pas celle étudiée
    pub other_lang: String,
    /// chanson : album, extrait de 30 s, paroles
    pub album: String,
    pub sample: String,
    pub lyrics: Option<Lyrics>,
    /// article : nombre de mots (approché)
    pub words: i64,
}

#[derive(Serialize, Debug, Default)]
pub struct SearchPage {
    pub hits: Vec<Hit>,
    /// il y a d'autres résultats (page suivante)
    pub more: bool,
}

/// Cherche `query` sur une plateforme, dans la langue `lang`. `page` : 0, 1, 2…
/// `filter` : durée des vidéos ("short", "medium", "long", sinon toutes).
pub async fn search(
    data_dir: &Path,
    lang: &str,
    platform: &str,
    query: &str,
    page: usize,
    filter: &str,
    on_event: &mut (dyn FnMut(ImportEvent) + Send),
) -> Result<SearchPage> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(SearchPage::default());
    }
    match platform {
        "youtube" => youtube(data_dir, lang, q, page, filter, on_event).await,
        "dailymotion" => dailymotion(lang, q, page, filter).await,
        "podcast" => podcasts(lang, q).await,
        "music" => music(lang, q, page).await,
        "wiki" => wiki(lang, q, page).await,
        _ => Err(anyhow!(t("Plateforme inconnue.", "Unknown platform."))),
    }
}

fn str_of(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Texte court et lisible (sans balises ni sauts de ligne), coupé sur un mot.
fn short(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{}…", cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut).trim_end_matches(|c: char| !c.is_alphanumeric()))
}

/// Langue reconnue d'un texte, par rapport à la langue étudiée.
fn lang_of(text: &str, lang: &str) -> (Option<bool>, String) {
    match langid::guess(text) {
        Some(g) => {
            let ok = langid::matches(text, lang).unwrap_or(g == lang);
            (Some(ok), if ok { String::new() } else { g.to_string() })
        }
        None => (None, String::new()),
    }
}

/// La langue étudiée d'abord, puis ce qu'on ne sait pas, puis les autres langues
/// (écartées s'il reste assez de résultats).
fn by_language(mut hits: Vec<Hit>, keep_others_below: usize) -> Vec<Hit> {
    let rank = |h: &Hit| match h.in_lang {
        Some(true) => 0,
        None => 1,
        Some(false) => 2,
    };
    hits.sort_by_key(rank);
    let good = hits.iter().filter(|h| h.in_lang != Some(false)).count();
    if good >= keep_others_below {
        hits.retain(|h| h.in_lang != Some(false));
    }
    hits
}

// ---------- YouTube ----------

async fn ytdlp(data_dir: &Path, on_event: &mut (dyn FnMut(ImportEvent) + Send)) -> Result<std::path::PathBuf> {
    if let Some(p) = crate::tools::find_ytdlp(data_dir) {
        return Ok(p);
    }
    on_event(ImportEvent::Stage { stage: "tools".into() });
    crate::tools::ensure_youtube_tools(data_dir, |p| on_event(ImportEvent::Progress { value: p })).await
}

/// La plus grande miniature proposée, sinon celle qui existe toujours.
fn yt_thumb(e: &Value, id: &str) -> String {
    e.get("thumbnails")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .filter(|t| t.get("url").and_then(Value::as_str).is_some_and(|u| u.starts_with("https://")))
                .max_by_key(|t| t.get("width").and_then(Value::as_i64).unwrap_or(0))
        })
        .and_then(|t| t.get("url").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
}

/// Résultats d'une recherche YouTube tels que yt-dlp les liste.
pub(crate) fn youtube_hits(v: &Value, lang: &str, shorts: bool) -> Vec<Hit> {
    let Some(entries) = v.get("entries").and_then(Value::as_array) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|e| {
            let id = str_of(e, "id");
            let title = str_of(e, "title");
            let url = str_of(e, "url");
            if id.len() != 11 || title.is_empty() || (url.contains("/shorts/") && !shorts) {
                return None;
            }
            if matches!(e.get("live_status").and_then(Value::as_str), Some("is_live" | "is_upcoming" | "post_live")) {
                return None;
            }
            let duration = num(e, "duration");
            // un direct n'a pas de durée
            if duration <= 0.0 {
                return None;
            }
            let summary = str_of(e, "description");
            let (in_lang, other_lang) = lang_of(&format!("{title}. {summary}"), lang);
            let (lo, hi) = discover::title_levels(&title).unwrap_or((0, 0));
            let watch = format!("https://www.youtube.com/watch?v={id}");
            Some(Hit {
                id: format!("yt:{id}"),
                platform: "youtube".into(),
                kind: "video".into(),
                image: yt_thumb(e, &id),
                title,
                url: watch.clone(),
                page: watch,
                author: ["channel", "uploader"].iter().map(|k| str_of(e, k)).find(|s| !s.is_empty()).unwrap_or_default(),
                summary: short(&summary, 220),
                duration,
                published: e.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
                count: e.get("view_count").and_then(Value::as_i64).unwrap_or(0),
                lo,
                hi,
                in_lang,
                other_lang,
                ..Default::default()
            })
        })
        .collect()
}

async fn youtube(data_dir: &Path, lang: &str, q: &str, page: usize, filter: &str, on_event: &mut (dyn FnMut(ImportEvent) + Send)) -> Result<SearchPage> {
    let bin = ytdlp(data_dir, on_event).await?;
    on_event(ImportEvent::Stage { stage: "search".into() });
    // filtres de YouTube : vidéos seulement, et leur durée
    let sp = match filter {
        "short" => "EgQQARgB",
        "medium" => "EgQQARgD",
        "long" => "EgQQARgC",
        _ => "EgIQAQ==",
    };
    let url = Url::parse_with_params("https://www.youtube.com/results", &[("search_query", q), ("sp", sp)])?;
    let (from, to) = (page * PER_PAGE + 1, (page + 1) * PER_PAGE);
    let v = tokio::time::timeout(Duration::from_secs(45), media::yt_flat(data_dir, &bin, url.as_str(), from, to, Some(lang)))
        .await
        .map_err(|_| anyhow!(t("YouTube ne répond pas.", "YouTube isn't responding.")))??;
    let listed = v.get("entries").and_then(Value::as_array).map_or(0, Vec::len);
    let hits = youtube_hits(&v, lang, filter == "short");
    Ok(SearchPage { hits, more: listed >= PER_PAGE })
}

// ---------- Dailymotion ----------

async fn dailymotion(lang: &str, q: &str, page: usize, filter: &str) -> Result<SearchPage> {
    let c = link::client()?;
    let page_s = (page + 1).to_string();
    let limit = PER_PAGE.to_string();
    let mut params: Vec<(&str, &str)> = vec![
        ("search", q),
        ("languages", lang),
        ("fields", "id,title,duration,thumbnail_720_url,owner.screenname,created_time,views_total,url,description,language"),
        ("limit", limit.as_str()),
        ("page", page_s.as_str()),
        ("sort", "relevance"),
        ("flags", "no_live"),
    ];
    match filter {
        "short" => params.push(("shorter_than", "4")),
        "medium" => params.extend([("longer_than", "4"), ("shorter_than", "20")]),
        "long" => params.push(("longer_than", "20")),
        _ => params.push(("longer_than", "1")),
    }
    let url = Url::parse_with_params("https://api.dailymotion.com/videos", &params)?;
    let v = link::get_json(&c, url.as_str()).await?;
    let hits = v
        .get("list")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let title = str_of(e, "title");
                    let url = str_of(e, "url");
                    if title.is_empty() || !url.starts_with("http") {
                        return None;
                    }
                    let summary = link::decode_entities(&str_of(e, "description"));
                    let declared = str_of(e, "language");
                    let (in_lang, other_lang) = if declared == lang { (Some(true), String::new()) } else { lang_of(&format!("{title}. {summary}"), lang) };
                    Some(Hit {
                        id: format!("dm:{}", str_of(e, "id")),
                        platform: "dailymotion".into(),
                        kind: "video".into(),
                        title,
                        page: url.clone(),
                        url,
                        image: str_of(e, "thumbnail_720_url"),
                        author: str_of(e, "owner.screenname"),
                        summary: short(&summary, 220),
                        duration: num(e, "duration"),
                        published: e.get("created_time").and_then(Value::as_i64).unwrap_or(0),
                        count: e.get("views_total").and_then(Value::as_i64).unwrap_or(0),
                        in_lang,
                        other_lang,
                        ..Default::default()
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(SearchPage { hits, more: v.get("has_more").and_then(Value::as_bool).unwrap_or(false) })
}

// ---------- podcasts (annuaire d'Apple) ----------

/// Pays de l'annuaire d'Apple où chercher les podcasts d'une langue.
pub fn country(lang: &str) -> &'static str {
    match lang {
        "en" => "us",
        "es" => "es",
        "fr" => "fr",
        "de" => "de",
        "it" => "it",
        "pt" => "br",
        "ru" => "ru",
        "nl" => "nl",
        "sv" => "se",
        "da" => "dk",
        "fi" => "fi",
        "et" => "ee",
        "lv" => "lv",
        "lt" => "lt",
        "pl" => "pl",
        "cs" => "cz",
        "sk" => "sk",
        "sl" => "si",
        "hr" => "hr",
        "hu" => "hu",
        "ro" => "ro",
        "bg" => "bg",
        "uk" => "ua",
        "el" => "gr",
        "tr" => "tr",
        "ar" => "eg",
        "hi" => "in",
        "id" => "id",
        "vi" => "vn",
        "ko" => "kr",
        "ja" => "jp",
        _ => "us",
    }
}

fn itunes(params: &[(&str, &str)]) -> Result<Url> {
    Ok(Url::parse_with_params("https://itunes.apple.com/search", params)?)
}

fn episode_hit(r: &Value, lang: &str) -> Option<Hit> {
    let url = str_of(r, "episodeUrl");
    let title = str_of(r, "trackName");
    if !url.starts_with("http") || title.is_empty() {
        return None;
    }
    let summary = str_of(r, "description");
    let (in_lang, other_lang) = lang_of(&format!("{title}. {summary}"), lang);
    let ext = str_of(r, "episodeFileExtension").to_lowercase();
    let image = ["artworkUrl600", "artworkUrl160", "artworkUrl100"].iter().map(|k| str_of(r, k)).find(|s| !s.is_empty()).unwrap_or_default();
    Some(Hit {
        id: format!("ap:{}", str_of(r, "trackId")),
        platform: "podcast".into(),
        kind: if matches!(ext.as_str(), "mp4" | "m4v" | "mov") { "video" } else { "audio" }.into(),
        title,
        url,
        page: str_of(r, "trackViewUrl"),
        image,
        author: str_of(r, "collectionName"),
        summary: short(&summary, 240),
        duration: num(r, "trackTimeMillis") / 1000.0,
        published: chrono::DateTime::parse_from_rfc3339(&str_of(r, "releaseDate")).map_or(0, |d| d.timestamp()),
        in_lang,
        other_lang,
        ..Default::default()
    })
}

fn show_hit(r: &Value) -> Option<Hit> {
    let feed = str_of(r, "feedUrl");
    let title = str_of(r, "collectionName");
    if !feed.starts_with("http") || title.is_empty() {
        return None;
    }
    Some(Hit {
        id: format!("as:{}", str_of(r, "collectionId")),
        platform: "podcast".into(),
        kind: "show".into(),
        title,
        url: feed,
        page: str_of(r, "collectionViewUrl"),
        image: ["artworkUrl600", "artworkUrl100"].iter().map(|k| str_of(r, k)).find(|s| !s.is_empty()).unwrap_or_default(),
        author: str_of(r, "artistName"),
        summary: str_of(r, "primaryGenreName"),
        count: r.get("trackCount").and_then(Value::as_i64).unwrap_or(0),
        published: chrono::DateTime::parse_from_rfc3339(&str_of(r, "releaseDate")).map_or(0, |d| d.timestamp()),
        ..Default::default()
    })
}

async fn podcasts(lang: &str, q: &str) -> Result<SearchPage> {
    let c = link::client()?;
    let cc = country(lang);
    let episodes = itunes(&[("term", q), ("media", "podcast"), ("entity", "podcastEpisode"), ("limit", "40"), ("country", cc)])?;
    let shows = itunes(&[("term", q), ("media", "podcast"), ("entity", "podcast"), ("limit", "10"), ("country", cc)])?;
    let (e, s) = tokio::join!(link::get_json(&c, episodes.as_str()), link::get_json(&c, shows.as_str()));
    let rows = |v: &Result<Value>| v.as_ref().ok().and_then(|v| v.get("results").and_then(Value::as_array).cloned()).unwrap_or_default();
    if e.is_err() && s.is_err() {
        return Err(e.err().unwrap());
    }
    let mut hits: Vec<Hit> = rows(&s).iter().filter_map(show_hit).take(8).collect();
    let eps: Vec<Hit> = rows(&e).iter().filter_map(|r| episode_hit(r, lang)).collect();
    hits.extend(by_language(eps, 8));
    Ok(SearchPage { hits, more: false })
}

// ---------- chansons (Deezer pour trouver, LRCLIB pour les paroles) ----------

async fn music(lang: &str, q: &str, page: usize) -> Result<SearchPage> {
    let c = link::client()?;
    let index = (page * PER_PAGE).to_string();
    let limit = PER_PAGE.to_string();
    let url = Url::parse_with_params("https://api.deezer.com/search", &[("q", q), ("limit", limit.as_str()), ("index", index.as_str())])?;
    let v = link::get_json(&c, url.as_str()).await?;
    let tracks: Vec<Value> = v.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
    let more = v.get("next").is_some_and(|n| n.is_string());
    let lc = lyrics::client()?;
    let found: Vec<Hit> = stream::iter(tracks)
        .map(|tr| {
            let lc = lc.clone();
            async move {
                let artist = tr.get("artist").map(|a| str_of(a, "name")).unwrap_or_default();
                let title = str_of(&tr, "title");
                let album = tr.get("album").map(|a| str_of(a, "title")).unwrap_or_default();
                let duration = num(&tr, "duration");
                let l = tokio::time::timeout(Duration::from_secs(12), lyrics::find(&lc, &artist, &title, &album, duration)).await.ok().flatten();
                let (in_lang, other_lang) = match &l {
                    Some(l) if !l.is_empty() => lang_of(&l.text(), lang),
                    _ => (None, String::new()),
                };
                let words = l.as_ref().map_or(0, |l| lyrics::word_count(l, lang) as i64);
                Hit {
                    id: format!("dz:{}", str_of(&tr, "id")),
                    platform: "music".into(),
                    kind: "song".into(),
                    title,
                    url: String::new(),
                    page: str_of(&tr, "link"),
                    image: tr.get("album").map(|a| str_of(a, "cover_xl")).unwrap_or_default(),
                    author: artist,
                    summary: String::new(),
                    album,
                    sample: str_of(&tr, "preview"),
                    duration,
                    count: tr.get("rank").and_then(Value::as_i64).unwrap_or(0),
                    in_lang,
                    other_lang,
                    lyrics: l.filter(|l| !l.is_empty()),
                    words,
                    ..Default::default()
                }
            }
        })
        .buffered(6)
        .collect()
        .await;
    // des paroles dans la langue étudiée d'abord ; une chanson sans paroles en dernier
    let mut hits = found;
    hits.sort_by_key(|h| {
        let has = h.lyrics.is_some();
        match (has, h.in_lang) {
            (true, Some(true)) => 0,
            (true, None) => 1,
            (true, Some(false)) => 3,
            (false, _) => 2,
        }
    });
    Ok(SearchPage { hits, more })
}

/// La chanson sur YouTube Music (version de l'album, dont le minutage suit celui
/// des paroles), sinon la première vidéo trouvée sur YouTube.
pub async fn song_url(data_dir: &Path, artist: &str, title: &str) -> Result<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let key = format!("{}\u{1f}{}", artist.to_lowercase(), title.to_lowercase());
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(u) = cache.lock().get(&key) {
        return Ok(u.clone());
    }
    let mut quiet = |_e: ImportEvent| {};
    let bin = ytdlp(data_dir, &mut quiet).await?;
    let q = format!("{artist} {title}");
    let music = Url::parse_with_params("https://music.youtube.com/search", &[("q", q.as_str())])?;
    let mut tries = vec![format!("{}#songs", music), format!("ytsearch3:{artist} - {title}")];
    let mut found = None;
    for u in tries.drain(..) {
        let Ok(Ok(v)) = tokio::time::timeout(Duration::from_secs(40), media::yt_flat(data_dir, &bin, &u, 1, 3, None)).await else { continue };
        let first = v.get("entries").and_then(Value::as_array).and_then(|a| {
            a.iter().map(|e| str_of(e, "id")).find(|id| id.len() == 11)
        });
        if let Some(id) = first {
            found = Some(format!("https://www.youtube.com/watch?v={id}"));
            break;
        }
    }
    let url = found.ok_or_else(|| anyhow!(t("Lumen ne trouve pas cette chanson sur YouTube.", "Lumen can't find this song on YouTube.")))?;
    cache.lock().insert(key, url.clone());
    Ok(url)
}

// ---------- articles (Wikipédia, Vikidia) ----------

/// Encyclopédies où chercher, de la plus simple à la plus riche : (adresse, nom, niveaux).
fn encyclopedias(lang: &str) -> Vec<(String, &'static str, u8, u8)> {
    let mut out = Vec::new();
    // Vikidia : l'encyclopédie des 8-13 ans, des phrases plus simples
    if ["fr", "es", "it", "en", "de", "ru", "pt", "ca", "el"].contains(&lang) {
        out.push((format!("https://{lang}.vikidia.org"), "Vikidia", discover::B1, discover::B2));
    }
    if lang == "en" {
        out.push(("https://simple.wikipedia.org".into(), "Simple English Wikipedia", discover::A2, discover::B1));
    }
    out.push((format!("https://{lang}.wikipedia.org"), "Wikipedia", discover::C1, discover::C1));
    out
}

async fn wiki_site(c: &reqwest::Client, base: &str, name: &str, lo: u8, hi: u8, q: &str, page: usize) -> Result<(Vec<Hit>, bool)> {
    let per = 10usize;
    let offset = (page * per).to_string();
    let limit = per.to_string();
    let url = Url::parse_with_params(
        &format!("{base}/w/api.php"),
        &[
            ("action", "query"),
            ("generator", "search"),
            ("gsrsearch", q),
            ("gsrlimit", limit.as_str()),
            ("gsroffset", offset.as_str()),
            ("gsrnamespace", "0"),
            ("prop", "pageimages|extracts|info"),
            ("inprop", "url"),
            ("exintro", "1"),
            ("explaintext", "1"),
            ("exchars", "300"),
            ("exlimit", "max"),
            ("piprop", "thumbnail"),
            ("pithumbsize", "640"),
            ("format", "json"),
            ("formatversion", "2"),
        ],
    )?;
    let v = link::get_json(c, url.as_str()).await?;
    let more = v.get("continue").is_some();
    let mut pages: Vec<Value> = v.pointer("/query/pages").and_then(Value::as_array).cloned().unwrap_or_default();
    pages.sort_by_key(|p| p.get("index").and_then(Value::as_i64).unwrap_or(0));
    let hits = pages
        .iter()
        .filter_map(|p| {
            let title = str_of(p, "title");
            let url = str_of(p, "fullurl");
            if title.is_empty() || !url.starts_with("http") {
                return None;
            }
            // une page d'homonymie n'est pas un article
            let extract = str_of(p, "extract");
            if extract.is_empty() {
                return None;
            }
            Some(Hit {
                id: format!("wk:{}:{}", name, str_of(p, "pageid")),
                platform: "wiki".into(),
                kind: "text".into(),
                title,
                page: url.clone(),
                url,
                image: p.pointer("/thumbnail/source").and_then(Value::as_str).unwrap_or("").to_string(),
                author: name.to_string(),
                summary: short(&extract, 260),
                lo,
                hi,
                in_lang: Some(true),
                // le texte source (wikicode) compte environ huit caractères par mot
                words: p.get("length").and_then(Value::as_i64).unwrap_or(0) / 8,
                ..Default::default()
            })
        })
        .collect();
    Ok((hits, more))
}

async fn wiki(lang: &str, q: &str, page: usize) -> Result<SearchPage> {
    let c = lyrics::client()?;
    let sites = encyclopedias(lang);
    let results = futures_util::future::join_all(sites.iter().map(|(base, name, lo, hi)| wiki_site(&c, base, name, *lo, *hi, q, page))).await;
    let mut lists: Vec<Vec<Hit>> = Vec::new();
    let mut more = false;
    let mut error = None;
    for r in results {
        match r {
            Ok((h, m)) => {
                more |= m;
                lists.push(h);
            }
            Err(e) => error = Some(e),
        }
    }
    if lists.iter().all(Vec::is_empty) {
        if let Some(e) = error {
            return Err(e);
        }
    }
    // les encyclopédies en alternance, la plus simple d'abord
    let mut hits = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for l in &lists {
            if let Some(h) = l.get(i) {
                hits.push(h.clone());
            }
        }
    }
    Ok(SearchPage { hits, more })
}

/// Sections de fin d'article (références, liens) : le texte s'arrête avant.
const END_SECTIONS: &[&str] = &[
    "notes", "références", "voir aussi", "liens externes", "bibliographie", "articles connexes", "see also", "references", "external links",
    "further reading", "notes and references", "note", "bibliografia", "collegamenti esterni", "voci correlate", "einzelnachweise", "weblinks",
    "literatur", "siehe auch", "anmerkungen", "referencias", "enlaces externos", "véase también", "referências", "ligações externas",
    "ver também", "примечания", "литература", "ссылки", "см. также", "источники",
];

/// Le texte d'un article de Wikipédia ou de Vikidia (titre et paragraphes),
/// sans les références ni les tableaux : plus propre que la page lue telle quelle.
pub async fn wiki_article(url: &Url) -> Option<(String, Vec<String>)> {
    let host = url.host_str()?.to_string();
    if !(host.ends_with(".wikipedia.org") || host.ends_with(".vikidia.org")) {
        return None;
    }
    let title = url.path().strip_prefix("/wiki/")?;
    let title = percent(title).replace('_', " ");
    let api = Url::parse_with_params(
        &format!("https://{host}/w/api.php"),
        &[
            ("action", "query"),
            ("prop", "extracts"),
            ("explaintext", "1"),
            ("exsectionformat", "plain"),
            ("redirects", "1"),
            ("titles", title.as_str()),
            ("format", "json"),
            ("formatversion", "2"),
        ],
    )
    .ok()?;
    let c = lyrics::client().ok()?;
    let v = link::get_json(&c, api.as_str()).await.ok()?;
    let page = v.pointer("/query/pages/0")?;
    let name = str_of(page, "title");
    let text = str_of(page, "extract");
    let mut paras = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if END_SECTIONS.contains(&line.to_lowercase().as_str()) {
            break;
        }
        paras.push(line.to_string());
    }
    (!paras.is_empty()).then_some((name, paras))
}

/// « Caf%C3%A9 » → « Café ».
fn percent(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------- lecture directe (aperçu) ----------

/// De quoi regarder ou écouter une vidéo avant d'en faire une leçon.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Stream {
    /// image (et son si `audio` est vide) ; vide pour un son seul
    pub video: String,
    /// son à part, que la vidéo muette suit
    pub audio: String,
    pub width: i64,
    pub height: i64,
    /// langue parlée annoncée par le site (YouTube), vide si inconnue
    pub language: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub duration: f64,
    pub published: i64,
    pub count: i64,
}

fn stream_of(v: &Value) -> Stream {
    let mut s = Stream {
        title: str_of(v, "title"),
        description: short(&str_of(v, "description"), 1200),
        author: ["channel", "uploader", "creator"].iter().map(|k| str_of(v, k)).find(|x| !x.is_empty()).unwrap_or_default(),
        duration: num(v, "duration"),
        published: v.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
        count: v.get("view_count").and_then(Value::as_i64).unwrap_or(0),
        language: str_of(v, "language").split(['-', '_']).next().unwrap_or("").to_lowercase(),
        ..Default::default()
    };
    let formats: Vec<&Value> = match v.get("requested_formats").and_then(Value::as_array) {
        Some(a) => a.iter().collect(),
        None => vec![v],
    };
    for f in formats {
        let url = str_of(f, "url");
        if url.is_empty() {
            continue;
        }
        // un flux qui porte aussi le son se suffit à lui-même ; sans codec annoncé, c'est une vidéo
        let sound_only = f.get("vcodec").and_then(Value::as_str) == Some("none");
        if !sound_only && s.video.is_empty() {
            s.video = url;
            s.width = f.get("width").and_then(Value::as_i64).unwrap_or(0);
            s.height = f.get("height").and_then(Value::as_i64).unwrap_or(0);
        } else if sound_only && s.audio.is_empty() {
            s.audio = url;
        }
    }
    // un flux unique sans indication de codec : on le confie à l'élément vidéo
    if s.video.is_empty() && s.audio.is_empty() {
        s.video = str_of(v, "url");
    }
    s
}

/// Adresses de lecture d'une vidéo ou d'un son en ligne (gardées 40 minutes :
/// rouvrir l'aperçu est immédiat).
pub async fn stream(data_dir: &Path, url: &str, audio_only: bool, browser: Option<&str>) -> Result<Stream> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Stream)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{audio_only}:{url}");
    if let Some((at, s)) = cache.lock().get(&key) {
        if at.elapsed() < Duration::from_secs(40 * 60) {
            return Ok(s.clone());
        }
    }
    let mut quiet = |_e: ImportEvent| {};
    let bin = ytdlp(data_dir, &mut quiet).await?;
    // trois préparations à la fois au plus (le survol des cartes en lance d'avance)
    static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let _slot = GATE.get_or_init(|| tokio::sync::Semaphore::new(3)).acquire().await?;
    if let Some((at, s)) = cache.lock().get(&key) {
        if at.elapsed() < Duration::from_secs(40 * 60) {
            return Ok(s.clone());
        }
    }
    let v = tokio::time::timeout(Duration::from_secs(60), media::yt_stream(data_dir, &bin, url, audio_only, browser))
        .await
        .map_err(|_| anyhow!(t("Le site ne répond pas.", "The site isn't responding.")))??;
    let s = stream_of(&v);
    if s.video.is_empty() && s.audio.is_empty() {
        return Err(anyhow!(t("Lumen ne trouve rien à lire à cette adresse.", "Lumen can't find anything to play at this address.")));
    }
    let mut map = cache.lock();
    map.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(40 * 60));
    map.insert(key, (Instant::now(), s.clone()));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_results() {
        let v: Value = serde_json::from_str(
            r#"{"entries":[
                {"id":"PyADfuVljko","title":"Fettuccine Alfredo, la ricetta della nonna: come si fa davvero","duration":576,"channel":"Gambero Rosso",
                 "view_count":3685887,"url":"https://www.youtube.com/watch?v=PyADfuVljko","description":"Guarda altre ricette di Giorgione e scopri come si preparano"},
                {"id":"shortshort1","title":"Un Short","duration":42,"url":"https://www.youtube.com/shorts/shortshort1"},
                {"id":"livelivelv1","title":"En direct","duration":null},
                {"id":"x2yz3abcdeF","title":"Easy Italian for Beginners (A2)","duration":991}
            ]}"#,
        )
        .unwrap();
        let h = youtube_hits(&v, "it", false);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].id, "yt:PyADfuVljko");
        assert_eq!(h[0].author, "Gambero Rosso");
        assert_eq!(h[0].count, 3685887);
        assert_eq!(h[0].in_lang, Some(true));
        assert_eq!(h[0].image, "https://i.ytimg.com/vi/PyADfuVljko/hqdefault.jpg");
        assert_eq!((h[1].lo, h[1].hi), (discover::A2, discover::A2));
        assert_eq!(youtube_hits(&v, "it", true).len(), 3);
    }

    #[test]
    fn streams() {
        let v: Value = serde_json::from_str(
            r#"{"title":"T","language":"it","duration":316,"requested_formats":[
                {"url":"https://v.example/video","vcodec":"avc1.4d401f","acodec":"none","width":1280,"height":720},
                {"url":"https://v.example/audio","vcodec":"none","acodec":"mp4a.40.2"}]}"#,
        )
        .unwrap();
        let s = stream_of(&v);
        assert_eq!((s.video.as_str(), s.audio.as_str(), s.height), ("https://v.example/video", "https://v.example/audio", 720));
        assert_eq!(s.language, "it");
        // Dailymotion : un seul flux HLS qui porte tout
        let v: Value = serde_json::from_str(r#"{"url":"https://dm.example/x.m3u8","vcodec":"avc1","acodec":"mp4a","language":"fr-FR"}"#).unwrap();
        let s = stream_of(&v);
        assert_eq!((s.video.as_str(), s.audio.as_str(), s.language.as_str()), ("https://dm.example/x.m3u8", "", "fr"));
        // un son seul
        let v: Value = serde_json::from_str(r#"{"url":"https://a.example/x.m4a","vcodec":"none","acodec":"mp4a"}"#).unwrap();
        assert_eq!(stream_of(&v).audio, "https://a.example/x.m4a");
    }

    #[test]
    fn languages_first() {
        let mk = |id: &str, l: Option<bool>| Hit { id: id.into(), in_lang: l, ..Default::default() };
        let h = by_language(vec![mk("a", Some(false)), mk("b", None), mk("c", Some(true))], 8);
        assert_eq!(h.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
        let h = by_language(vec![mk("a", Some(false)), mk("b", None), mk("c", Some(true))], 2);
        assert_eq!(h.len(), 2);
        assert_eq!(percent("Caf%C3%A9_au_lait"), "Café_au_lait");
    }

    /// Recherches réelles sur chaque plateforme (titres et nombres seulement) :
    /// `LUMEN_SEARCH="it:cucina" cargo test --lib search_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn search_live() {
        let spec = std::env::var("LUMEN_SEARCH").unwrap_or_else(|_| "it:cucina italiana".into());
        let (lang, q) = spec.split_once(':').unwrap();
        let dir = std::env::temp_dir().join("lumen-search-live");
        std::fs::create_dir_all(&dir).unwrap();
        for p in ["youtube", "dailymotion", "podcast", "music", "wiki"] {
            let started = Instant::now();
            match search(&dir, lang, p, q, 0, "", &mut |_| {}).await {
                Ok(r) => {
                    let ok = r.hits.iter().filter(|h| h.in_lang == Some(true)).count();
                    let lyr = r.hits.iter().filter(|h| h.lyrics.is_some()).count();
                    println!("{p:<12} {:>2} résultats ({ok} dans la langue, {lyr} avec paroles) en {:.1} s", r.hits.len(), started.elapsed().as_secs_f64());
                    for h in r.hits.iter().take(4) {
                        println!("   · [{}] {} — {}", h.kind, h.title.chars().take(60).collect::<String>(), h.author);
                    }
                }
                Err(e) => println!("{p:<12} erreur : {e}"),
            }
        }
        let first = search(&dir, lang, "youtube", q, 0, "", &mut |_| {}).await.unwrap();
        let started = Instant::now();
        let s = stream(&dir, &first.hits[0].url, false, None).await.unwrap();
        println!("aperçu : image {} · son {} · {}p · langue {:?} ({:.1} s)", !s.video.is_empty(), !s.audio.is_empty(), s.height, s.language, started.elapsed().as_secs_f64());
        // WebKit ne lit pas l'image DASH de YouTube : elle doit venir en HLS
        assert!(s.video.contains("m3u8") || s.video.contains("/hls_"), "image de l'aperçu hors HLS : {}", &s.video[..s.video.len().min(80)]);
        let started = Instant::now();
        let u = song_url(&dir, "Laura Pausini", "La solitudine").await.unwrap();
        println!("chanson sur YouTube : {u} ({:.1} s)", started.elapsed().as_secs_f64());
        let a = wiki_article(&Url::parse(&format!("https://{lang}.wikipedia.org/wiki/Roma")).unwrap()).await.unwrap();
        println!("article : {} · {} paragraphes", a.0, a.1.len());
    }
}

#[cfg(test)]
mod music_check {
    /// Chansons d'un artiste : combien ont des paroles, minutées, dans la langue (aucun texte affiché).
    #[tokio::test]
    #[ignore]
    async fn music_live() {
        for (lang, q) in [("it", "Laura Pausini"), ("fr", "Stromae"), ("es", "Rosalía"), ("de", "Mark Forster")] {
            let r = super::music(lang, q, 0).await.unwrap();
            let lyr = r.hits.iter().filter(|h| h.lyrics.is_some()).count();
            let synced = r.hits.iter().filter(|h| h.lyrics.as_ref().is_some_and(|l| !l.synced.is_empty())).count();
            let ok = r.hits.iter().filter(|h| h.in_lang == Some(true)).count();
            println!("{lang} {q:<14} {:>2} chansons · {lyr} avec paroles · {synced} minutées · {ok} dans la langue", r.hits.len());
        }
    }
}
