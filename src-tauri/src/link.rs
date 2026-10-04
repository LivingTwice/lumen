//! Import universel d'un lien. Lumen regarde ce qu'il y a derrière une adresse
//! et propose ce qu'il peut en tirer :
//! - le texte d'un article (extrait ensuite par l'interface, avec Readability) ;
//! - le son ou la vidéo : par yt-dlp (YouTube et plus de mille sites), ou le
//!   fichier repéré dans la page (balises audio et vidéo, Open Graph, JSON-LD,
//!   lecteurs intégrés), là où yt-dlp ne sait pas faire (RFI…) ;
//! - les épisodes d'un podcast : flux RSS ou Atom, Apple Podcasts ;
//! - Spotify, dont les fichiers sont protégés contre la copie : Lumen retrouve
//!   le même épisode dans le flux public du podcast (annuaire d'Apple), et le
//!   même morceau sur YouTube.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use unicode_normalization::UnicodeNormalization;

use crate::i18n::t;
use crate::media::{self, ImportEvent};

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Éléments au plus dans une liste (podcast, playlist, chaîne).
const MAX_ITEMS: usize = 300;
/// Taille au plus d'une page ou d'un flux lu en entier.
const PAGE_MAX: usize = 15_000_000;

const AUDIO_EXT: &[&str] = &["mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "flac"];
const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mov", "webm"];

/// Sites confiés d'abord à yt-dlp (s'il échoue, la page est lue quand même).
const VIDEO_SITES: &[&str] = &[
    "youtube.com", "youtu.be", "youtube-nocookie.com", "vimeo.com", "dailymotion.com", "dai.ly", "tiktok.com", "twitch.tv",
    "ted.com", "arte.tv", "france.tv", "tv5monde.com", "rtbf.be", "rts.ch", "srf.ch", "rai.it", "raiplay.it", "rtve.es",
    "ardmediathek.de", "zdf.de", "orf.at", "bbc.co.uk", "bbc.com", "nhk.or.jp", "svtplay.se", "nrk.no", "dr.dk", "yle.fi",
    "npo.nl", "instagram.com", "facebook.com", "fb.watch", "x.com", "twitter.com", "rumble.com", "odysee.com",
    "bilibili.com", "nicovideo.jp", "vk.com", "ok.ru", "rutube.ru", "archive.org",
];
/// Sites de son seul : pas d'image à télécharger.
const AUDIO_SITES: &[&str] = &[
    "soundcloud.com", "bandcamp.com", "mixcloud.com", "audiomack.com", "ivoox.com", "spreaker.com", "radiofrance.fr",
    "raiplaysound.it",
];

/// Ce que Lumen a trouvé derrière un lien.
#[derive(Serialize, Clone, Debug, Default)]
pub struct LinkInfo {
    /// adresse finale (après redirections)
    pub url: String,
    pub title: String,
    /// site, émission, chaîne ou artiste
    pub site: String,
    pub image: String,
    /// page HTML, pour en extraire l'article (vide pour un flux ou un fichier)
    pub html: String,
    /// sons et vidéos à transcrire
    pub media: Vec<MediaItem>,
    /// liste (podcast, playlist, album) : l'utilisateur choisit ses éléments
    pub list: bool,
    /// d'où vient le son quand ce n'est pas du site donné : "rss" ou "youtube" (Spotify)
    pub via: String,
    /// à montrer si rien d'autre n'est trouvé (erreur de yt-dlp sur un site vidéo)
    pub note: String,
}

/// Un son ou une vidéo à importer.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MediaItem {
    /// fichier direct, ou adresse lue par yt-dlp (« ytsearch1: » pour un morceau)
    pub url: String,
    pub title: String,
    /// secondes, 0 si inconnue
    pub duration: f64,
    /// il y a une image à télécharger
    pub video: bool,
    /// fichier son ou vidéo téléchargé tel quel, sans yt-dlp
    pub direct: bool,
    /// couverture
    pub image: String,
    /// AAAA-MM-JJ
    pub date: String,
    /// page d'origine (source de la leçon)
    pub page: String,
    /// collection de la leçon : émission, playlist, album
    pub collection: String,
}

// ---------- adresses ----------

/// Adresse saisie par l'utilisateur, complétée et vérifiée.
pub fn normalize(input: &str) -> Result<Url> {
    let s = input.trim().trim_matches(|c| c == '<' || c == '>' || c == '"' || c == '\'');
    if let Some(rest) = s.strip_prefix("spotify:") {
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() >= 2 {
            return Ok(Url::parse(&format!("https://open.spotify.com/{}/{}", parts[0], parts[1]))?);
        }
    }
    // liens d'abonnement aux podcasts : feed://, pcast://, itpc://, podcast://
    let mut s = s.strip_prefix("feed:").filter(|r| r.starts_with("http")).unwrap_or(s).to_string();
    for p in ["feed://", "pcast://", "itpc://", "podcast://"] {
        if let Some(r) = s.strip_prefix(p) {
            s = format!("https://{r}");
        }
    }
    if !s.contains("://") {
        s = format!("https://{s}");
    }
    let invalid = || anyhow!(t("Cette adresse n'est pas valide.", "This address isn't valid."));
    let u = Url::parse(&s).map_err(|_| invalid())?;
    if !matches!(u.scheme(), "http" | "https") || !u.host_str().is_some_and(|h| h.contains('.')) {
        return Err(invalid());
    }
    Ok(u)
}

fn host(u: &Url) -> String {
    let h = u.host_str().unwrap_or("").to_lowercase();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

fn on_site(h: &str, sites: &[&str]) -> bool {
    sites.iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

pub fn is_youtube(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| on_site(&host(&u), &["youtube.com", "youtu.be", "youtube-nocookie.com"]))
}

/// Extension du fichier visé : `Some(true)` vidéo, `Some(false)` son.
pub(crate) fn ext_kind(u: &Url) -> Option<bool> {
    let last = u.path_segments()?.next_back()?.to_lowercase();
    let ext = last.rsplit_once('.')?.1;
    if AUDIO_EXT.contains(&ext) {
        Some(false)
    } else if VIDEO_EXT.contains(&ext) {
        Some(true)
    } else {
        None
    }
}

/// Radio en direct (Icecast, Shoutcast…) : un flux sans fin, pas un enregistrement.
fn live_host(u: &Url) -> bool {
    let h = host(u);
    ["icecast", ".ice.", "shoutcast", "streamtheworld", "radiojar", "liveradio"].iter().any(|k| h.contains(k))
        || h.starts_with("ice")
        || h.starts_with("stream")
        || h.starts_with("live")
}

fn live_response(resp: &reqwest::Response) -> bool {
    resp.headers().keys().any(|k| k.as_str().starts_with("icy-"))
}

fn live_error() -> anyhow::Error {
    anyhow!(t(
        "C'est une radio en direct : Lumen n'importe que des enregistrements (épisodes, vidéos, fichiers).",
        "This is a live radio: Lumen only imports recordings (episodes, videos, files)."
    ))
}

/// Réponse qui est un fichier son ou vidéo : `Some(true)` vidéo.
fn media_kind(u: &Url, ctype: &str) -> Option<bool> {
    if ctype.starts_with("audio/") {
        Some(false)
    } else if ctype.starts_with("video/") {
        Some(true)
    } else if ctype.is_empty() || ctype.contains("octet-stream") || ctype.contains("binary") {
        ext_kind(u)
    } else {
        None
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Titre lisible tiré du nom d'un fichier.
fn file_title(u: &Url) -> String {
    let last = u.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    let name = percent_decode(last);
    let stem = name.rsplit_once('.').map(|(a, _)| a).unwrap_or(&name);
    let clean: String = stem.chars().map(|c| if c == '_' || c == '-' { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        host(u)
    } else {
        clean
    }
}

// ---------- réseau ----------

pub(crate) fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(UA).connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(45)).build()?)
}

fn accept_language() -> &'static str {
    if crate::i18n::en() {
        "en,fr;q=0.8,*;q=0.5"
    } else {
        "fr,en;q=0.8,*;q=0.5"
    }
}

/// Texte d'une page : UTF-8, sinon Windows-1252 (vieux sites européens).
fn decode_bytes(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| cp1252(b)).collect(),
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”',
        '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&b) {
        HIGH[(b - 0x80) as usize]
    } else {
        b as char
    }
}

async fn body_text(resp: reqwest::Response) -> Result<String> {
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk?);
        if out.len() > PAGE_MAX {
            return Err(anyhow!(t("Page trop volumineuse.", "This page is too large.")));
        }
    }
    Ok(decode_bytes(out))
}

pub(crate) async fn get_text(c: &reqwest::Client, url: &str) -> Result<String> {
    let resp = c.get(url).header(ACCEPT_LANGUAGE, accept_language()).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(crate::tr!("La page a répondu {}.", "The page answered {}.", resp.status())));
    }
    body_text(resp).await
}

async fn get_json(c: &reqwest::Client, url: &str) -> Result<Value> {
    Ok(serde_json::from_str(&get_text(c, url).await?)?)
}

// ---------- analyse d'un lien ----------

/// Regarde ce qu'il y a derrière un lien. `on_event` : installation des
/// composants vidéo s'il le faut, puis analyse.
pub async fn probe(data_dir: &Path, input: &str, browser: Option<&str>, on_event: &mut (dyn FnMut(ImportEvent) + Send)) -> Result<LinkInfo> {
    let url = normalize(input)?;
    let h = host(&url);
    let c = client()?;
    on_event(ImportEvent::Stage { stage: "probe".into() });
    if on_site(&h, &["spotify.com", "spotify.link"]) || h == "spotify.app.link" {
        return spotify(&c, &url).await;
    }
    if on_site(&h, &["podcasts.apple.com", "itunes.apple.com"]) {
        if let Some(info) = apple(&c, &url).await? {
            return Ok(info);
        }
    }
    let known = on_site(&h, VIDEO_SITES) || on_site(&h, AUDIO_SITES);
    let mut note = String::new();
    if known {
        match ytdlp(data_dir, url.as_str(), browser, on_event).await {
            Ok(info) if !info.media.is_empty() => return Ok(info),
            Ok(_) => {}
            Err(e) => note = short_error(&e.to_string()),
        }
    }
    let fail = |e: String| if note.is_empty() { anyhow!(e) } else { anyhow!(note.clone()) };
    let resp = c
        .get(url.as_str())
        .header(ACCEPT_LANGUAGE, accept_language())
        .send()
        .await
        .map_err(|e| fail(crate::tr!("Page inaccessible : {e}", "Page unreachable: {e}")))?;
    if !resp.status().is_success() {
        return Err(fail(crate::tr!("La page a répondu {}.", "The page answered {}.", resp.status())));
    }
    let final_url = resp.url().clone();
    let ctype = resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();

    // le lien mène droit à un fichier son ou vidéo
    if let Some(video) = media_kind(&final_url, &ctype) {
        if live_response(&resp) {
            return Err(live_error());
        }
        drop(resp);
        let title = file_title(&final_url);
        let u = final_url.to_string();
        return Ok(LinkInfo {
            url: u.clone(),
            title: title.clone(),
            site: host(&final_url),
            media: vec![MediaItem { url: u.clone(), title, video, direct: true, page: u, ..Default::default() }],
            ..Default::default()
        });
    }

    let body = body_text(resp).await?;
    if is_feed(&ctype, &body) {
        return feed_info(&body, final_url.as_str())
            .ok_or_else(|| anyhow!(t("Ce flux ne contient aucun épisode à écouter.", "This feed has no episode to listen to.")));
    }

    let page = scan_html(&final_url, &body);
    let mut info = LinkInfo {
        url: final_url.to_string(),
        title: page.title.clone(),
        site: if page.site.is_empty() { host(&final_url) } else { page.site.clone() },
        image: page.image.clone(),
        ..Default::default()
    };
    let media: Vec<&(String, bool)> = page
        .media
        .iter()
        .filter(|(u, _)| note.is_empty() || Url::parse(u).is_ok_and(|x| ext_kind(&x).is_some()))
        .collect();
    let n = media.len();
    for (i, (u, video)) in media.into_iter().enumerate() {
        let title = if n == 1 {
            page.title.clone()
        } else {
            Url::parse(u).map(|x| file_title(&x)).unwrap_or_else(|_| format!("{} · {}", page.title, i + 1))
        };
        // une vidéo intégrée (YouTube, Vimeo…) passe par yt-dlp ; un fichier se télécharge tel quel
        let direct = Url::parse(u).is_ok_and(|x| ext_kind(&x).is_some());
        info.media.push(MediaItem { url: u.clone(), title, video: *video, direct, image: page.image.clone(), page: final_url.to_string(), ..Default::default() });
    }
    info.list = n > 1;

    // page d'une émission (pas d'un article) : son flux de podcast, s'il a des
    // épisodes, mieux renseigné (titres, dates, durées) que les fichiers de la page
    if (info.media.is_empty() || info.list) && page.kind != "article" {
        for feed in page.feeds.iter().take(2) {
            let Ok(xml) = get_text(&c, feed).await else { continue };
            if let Some(f) = feed_info(&xml, feed) {
                info.media = f.media;
                info.list = true;
                if info.image.is_empty() {
                    info.image = f.image;
                }
                if !f.title.is_empty() {
                    info.title = f.title;
                }
                break;
            }
        }
    }

    // dernier recours : yt-dlp, s'il est déjà là ou si la page se présente comme une vidéo
    if info.media.is_empty() && !known && (page.videoish || crate::tools::find_ytdlp(data_dir).is_some()) {
        if let Ok(found) = ytdlp(data_dir, final_url.as_str(), browser, on_event).await {
            info.media = found.media;
            info.list = found.list;
            if info.image.is_empty() {
                info.image = found.image;
            }
        }
    }
    info.note = note;
    info.html = body;
    Ok(info)
}

// ---------- yt-dlp ----------

fn str_of(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

fn thumb_of(v: &Value) -> String {
    let t = str_of(v, "thumbnail");
    if !t.is_empty() {
        return t;
    }
    v.get("thumbnails").and_then(Value::as_array).and_then(|a| a.last()).map(|x| str_of(x, "url")).unwrap_or_default()
}

/// Erreurs de yt-dlp dites simplement.
fn friendly_ytdlp_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("unsupported url") {
        t("Lumen ne trouve pas de vidéo à cette adresse.", "Lumen can't find a video at this address.").into()
    } else if l.contains("private") || l.contains("unavailable") || l.contains("not available") || l.contains("removed") || l.contains("terminated") {
        t(
            "Cette vidéo n'est pas disponible (privée, supprimée ou bloquée dans votre pays).",
            "This video isn't available (private, deleted or blocked in your country).",
        )
        .into()
    } else if l.contains("404") || l.contains("not found") {
        t("Cette page est introuvable : vérifiez le lien.", "This page can't be found: check the link.").into()
    } else {
        // « [site] identifiant: message (caused by …) » : le message seul
        let m = e.split_once("]").filter(|_| e.starts_with('[')).map(|(_, r)| r).unwrap_or(e);
        let m = m.split_once(": ").filter(|(id, _)| !id.contains(' ')).map(|(_, r)| r).unwrap_or(m);
        m.split(" (caused by").next().unwrap_or(m).trim().to_string()
    }
}

/// Message de yt-dlp sans sa partie technique entre parenthèses.
fn short_error(e: &str) -> String {
    let m = friendly_ytdlp_error(e);
    m.split(" ([").next().unwrap_or(&m).trim().to_string()
}

async fn ytdlp(data_dir: &Path, url: &str, browser: Option<&str>, on_event: &mut (dyn FnMut(ImportEvent) + Send)) -> Result<LinkInfo> {
    if crate::tools::find_ytdlp(data_dir).is_none() {
        on_event(ImportEvent::Stage { stage: "tools".into() });
    }
    let bin = crate::tools::ensure_youtube_tools(data_dir, |p| on_event(ImportEvent::Progress { value: p })).await?;
    on_event(ImportEvent::Stage { stage: "probe".into() });
    let audio_only = Url::parse(url).is_ok_and(|u| on_site(&host(&u), AUDIO_SITES));
    let slow = || anyhow!(t("Le site ne répond pas.", "The site isn't responding."));
    let v = tokio::time::timeout(Duration::from_secs(90), media::yt_info(data_dir, &bin, url, browser)).await.map_err(|_| slow())??;
    let info = from_ytdlp(&v, url, !audio_only);
    // chaîne YouTube : yt-dlp en donne les onglets (vidéos, shorts, directs) ; on ouvre celui des vidéos
    if info.media.is_empty() {
        if let Some(tab) = channel_tab(&v) {
            let v = tokio::time::timeout(Duration::from_secs(90), media::yt_info(data_dir, &bin, &tab, browser)).await.map_err(|_| slow())??;
            return Ok(from_ytdlp(&v, &tab, !audio_only));
        }
    }
    Ok(info)
}

fn ymd(s: &str) -> String {
    let s = s.trim();
    if s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()) {
        return format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..]);
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc2822(s) {
        return d.format("%Y-%m-%d").to_string();
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return d.format("%Y-%m-%d").to_string();
    }
    if s.len() >= 10 && s.is_char_boundary(10) && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        return s[..10].to_string();
    }
    String::new()
}

fn from_ytdlp(v: &Value, url: &str, video_site: bool) -> LinkInfo {
    let title = str_of(v, "title");
    let site = ["channel", "uploader", "playlist_uploader", "extractor_key"].iter().map(|k| str_of(v, k)).find(|s| !s.is_empty()).unwrap_or_default();
    let image = thumb_of(v);
    if let Some(entries) = v.get("entries").and_then(Value::as_array) {
        // chaîne YouTube : ses onglets (vidéos, directs, shorts), chacun avec ses vidéos ; on garde les vidéos
        let tabs: Vec<&Value> = entries.iter().filter(|e| e.get("entries").is_some_and(Value::is_array)).collect();
        if let Some(tab) = tabs.iter().find(|e| str_of(e, "webpage_url").ends_with("/videos")).or(tabs.first()) {
            let mut info = from_ytdlp(tab, url, video_site);
            for m in &mut info.media {
                m.collection = title.clone();
            }
            info.title = title;
            info.site = site;
            if info.image.is_empty() {
                info.image = image;
            }
            return info;
        }
        let media = entries.iter().filter_map(|e| ytdlp_entry(e, &title, video_site)).take(MAX_ITEMS).collect();
        return LinkInfo { url: url.into(), title, site, image, media, list: true, ..Default::default() };
    }
    let page = ["webpage_url", "original_url"].iter().map(|k| str_of(v, k)).find(|s| s.starts_with("http")).unwrap_or_else(|| url.to_string());
    let formats = v.get("formats").and_then(Value::as_array);
    let has_video = |f: &Value| matches!(f.get("vcodec").and_then(Value::as_str), Some(c) if c != "none");
    let video = if has_video(v) {
        true
    } else if let Some(fs) = formats {
        fs.iter().any(has_video)
    } else {
        video_site
    };
    let item = MediaItem {
        url: page.clone(),
        title: title.clone(),
        duration: num(v, "duration"),
        video,
        direct: false,
        image: image.clone(),
        date: ymd(&str_of(v, "upload_date")),
        page: page.clone(),
        collection: String::new(),
    };
    LinkInfo { url: page, title, site, image, media: vec![item], ..Default::default() }
}

fn ytdlp_entry(e: &Value, collection: &str, video_site: bool) -> Option<MediaItem> {
    let ie = str_of(e, "ie_key");
    if ie == "YoutubeTab" || str_of(e, "_type") == "playlist" {
        return None;
    }
    let mut u = str_of(e, "url");
    if !u.starts_with("http") {
        u = str_of(e, "webpage_url");
    }
    if !u.starts_with("http") {
        let id = str_of(e, "id");
        if ie == "Youtube" && !id.is_empty() {
            u = format!("https://www.youtube.com/watch?v={id}");
        } else {
            return None;
        }
    }
    Some(MediaItem {
        url: u.clone(),
        title: str_of(e, "title"),
        duration: num(e, "duration"),
        video: video_site,
        direct: false,
        image: thumb_of(e),
        date: ymd(&str_of(e, "upload_date")),
        page: u,
        collection: collection.into(),
    })
}

fn channel_tab(v: &Value) -> Option<String> {
    let tabs: Vec<String> = v
        .get("entries")?
        .as_array()?
        .iter()
        .filter(|e| str_of(e, "ie_key") == "YoutubeTab")
        .map(|e| str_of(e, "url"))
        .filter(|u| u.starts_with("http"))
        .collect();
    tabs.iter().find(|u| u.ends_with("/videos")).or(tabs.first()).cloned()
}

// ---------- lecture des pages et des flux ----------

/// Entités HTML et XML courantes.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(j, _)| j);
        if let Some(e) = end {
            let ent = &rest[1..e];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = ch {
                out.push(c);
                rest = &rest[e + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// Position de `needle` (en ASCII, sans tenir compte de la casse) à partir de `from`.
pub(crate) fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || from >= h.len() {
        return None;
    }
    (from..=h.len().saturating_sub(n.len())).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Balise ouvrante : nom en minuscules, attributs, fin (après le « > »).
pub(crate) struct Tag {
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) end: usize,
}

impl Tag {
    pub(crate) fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
}

/// Parcourt les balises ouvrantes d'un HTML ou d'un XML, sans se soucier de
/// leur imbrication (assez pour y repérer médias et métadonnées). Le contenu
/// des scripts, des styles, des commentaires et des CDATA est sauté.
pub(crate) struct Tags<'a> {
    s: &'a str,
    pos: usize,
}

pub(crate) fn tags(s: &str) -> Tags<'_> {
    Tags { s, pos: 0 }
}

impl Iterator for Tags<'_> {
    type Item = Tag;

    fn next(&mut self) -> Option<Tag> {
        let s = self.s;
        let b = s.as_bytes();
        loop {
            let lt = s[self.pos..].find('<')? + self.pos;
            if s[lt..].starts_with("<!--") {
                self.pos = s[lt..].find("-->").map(|i| lt + i + 3).unwrap_or(b.len());
                continue;
            }
            if s[lt..].starts_with("<![CDATA[") {
                self.pos = s[lt..].find("]]>").map(|i| lt + i + 3).unwrap_or(b.len());
                continue;
            }
            let mut i = lt + 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b':' | b'-' | b'_')) {
                i += 1;
            }
            if i == lt + 1 {
                self.pos = lt + 1;
                continue;
            }
            let name = s[lt + 1..i].to_ascii_lowercase();
            let mut attrs = Vec::new();
            loop {
                while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
                    i += 1;
                }
                if i >= b.len() {
                    break;
                }
                if b[i] == b'>' {
                    i += 1;
                    break;
                }
                let k0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
                    i += 1;
                }
                let key = s[k0..i].to_ascii_lowercase();
                if key.is_empty() {
                    i += 1;
                    continue;
                }
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                let mut val = String::new();
                if i < b.len() && b[i] == b'=' {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                        let v0 = i + 1;
                        let v1 = s[v0..].find(b[i] as char).map(|x| v0 + x).unwrap_or(b.len());
                        val = decode_entities(&s[v0..v1]);
                        i = (v1 + 1).min(b.len());
                    } else {
                        let v0 = i;
                        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                            i += 1;
                        }
                        val = decode_entities(&s[v0..i]);
                    }
                }
                attrs.push((key, val));
            }
            self.pos = i;
            if name == "script" || name == "style" {
                // le contenu d'un script n'est pas du HTML (le JSON-LD est lu à part)
                self.pos = find_ci(s, &format!("</{name}"), i).unwrap_or(i);
            }
            return Some(Tag { name, attrs, end: i });
        }
    }
}

/// Texte d'un élément (CDATA et balises internes retirées), à partir de la fin
/// de sa balise ouvrante.
pub(crate) fn inner_text(s: &str, tag: &Tag) -> String {
    let close = find_ci(s, &format!("</{}", tag.name), tag.end).unwrap_or(s.len());
    let raw = &s[tag.end..close];
    let raw = raw.replace("<![CDATA[", "").replace("]]>", "");
    let mut text = String::with_capacity(raw.len());
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    decode_entities(&text).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Texte du premier élément `name`.
pub(crate) fn tag_text(s: &str, name: &str) -> Option<String> {
    tags(s).find(|t| t.name == name).map(|t| inner_text(s, &t)).filter(|x| !x.is_empty())
}

/// Ce qu'une page HTML annonce : titre, image, sons et vidéos, flux de podcast.
#[derive(Default, Debug)]
struct PageScan {
    title: String,
    site: String,
    image: String,
    /// og:type (« article », « video.other »…)
    kind: String,
    /// la page se présente comme une vidéo
    videoish: bool,
    /// (adresse, vidéo)
    media: Vec<(String, bool)>,
    feeds: Vec<String>,
}

/// Vidéo intégrée d'un site connu, ramenée à sa page.
fn embed_page(u: &Url) -> Option<(String, bool)> {
    let h = host(u);
    let path = u.path();
    if on_site(&h, &["youtube.com", "youtube-nocookie.com"]) {
        let id = path.strip_prefix("/embed/")?.split('/').next()?;
        return (id.len() == 11).then(|| (format!("https://www.youtube.com/watch?v={id}"), true));
    }
    if h == "player.vimeo.com" {
        let id = path.strip_prefix("/video/")?.trim_end_matches('/');
        return (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| (format!("https://vimeo.com/{id}"), true));
    }
    if on_site(&h, &["dailymotion.com"]) {
        let id = path.strip_prefix("/embed/video/")?;
        return Some((format!("https://www.dailymotion.com/video/{id}"), true));
    }
    if h == "w.soundcloud.com" {
        let (_, v) = u.query_pairs().find(|(k, _)| k == "url")?;
        return Some((v.into_owned(), false));
    }
    None
}

/// Adresses absolues de fichiers son ou vidéo écrites dans la page (attributs,
/// JSON-LD, données des lecteurs, y compris avec des « \/ » échappés).
fn media_urls(html: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = html[from..].find("http") {
        let start = from + i;
        let rest = &html[start..];
        let rb = rest.as_bytes();
        let mut end = rb.len();
        let mut j = 0;
        while j < rb.len() {
            let c = rb[j];
            if c == b'\\' {
                if rb.get(j + 1) == Some(&b'/') {
                    j += 2;
                    continue;
                }
                if rest[j..].len() >= 6 && rest[j..j + 6].eq_ignore_ascii_case("\\u002f") {
                    j += 6;
                    continue;
                }
                end = j;
                break;
            }
            if matches!(c, b'"' | b'\'' | b'<' | b'>' | b')' | b'`') || c.is_ascii_whitespace() {
                end = j;
                break;
            }
            j += 1;
        }
        from = start + end.max(4);
        let raw = rest[..end].replace("\\/", "/").replace("\\u002F", "/").replace("\\u002f", "/");
        let raw = decode_entities(&raw);
        if !(raw.starts_with("http://") || raw.starts_with("https://")) {
            continue;
        }
        if let Ok(u) = Url::parse(&raw) {
            if live_host(&u) {
                continue;
            }
            if let Some(video) = ext_kind(&u) {
                out.push((u.to_string(), video));
            }
        }
    }
    out
}

fn scan_html(base: &Url, html: &str) -> PageScan {
    let mut p = PageScan::default();
    let mut found: Vec<(String, bool)> = Vec::new();
    let mut title_tag = String::new();
    let abs = |v: &str| base.join(v.trim()).ok();
    let add = |found: &mut Vec<(String, bool)>, u: Url, video_hint: Option<bool>| {
        if live_host(&u) {
            return;
        }
        if let Some(e) = embed_page(&u) {
            found.push(e);
        } else if let Some(v) = ext_kind(&u) {
            found.push((u.to_string(), video_hint.unwrap_or(v)));
        }
    };
    for tag in tags(html) {
        match tag.name.as_str() {
            "title" if title_tag.is_empty() => title_tag = inner_text(html, &tag),
            "meta" => {
                let key = tag.attr("property").or(tag.attr("name")).unwrap_or("").to_lowercase();
                let val = tag.attr("content").unwrap_or("").trim().to_string();
                if val.is_empty() {
                    continue;
                }
                match key.as_str() {
                    "og:title" | "twitter:title" if p.title.is_empty() => p.title = val,
                    "og:site_name" => p.site = val,
                    "og:image" | "og:image:url" | "og:image:secure_url" | "twitter:image" if p.image.is_empty() => {
                        p.image = abs(&val).map(|u| u.to_string()).unwrap_or_default()
                    }
                    "og:type" => {
                        p.videoish |= val.starts_with("video");
                        p.kind = val.to_lowercase();
                    }
                    "twitter:card" => p.videoish |= val == "player",
                    "og:audio" | "og:audio:url" | "og:audio:secure_url" => {
                        if let Some(u) = abs(&val) {
                            add(&mut found, u, Some(false));
                        }
                    }
                    "og:video" | "og:video:url" | "og:video:secure_url" | "twitter:player:stream" => {
                        p.videoish = true;
                        if let Some(u) = abs(&val) {
                            add(&mut found, u, Some(true));
                        }
                    }
                    _ => {}
                }
            }
            "audio" | "video" | "source" => {
                if let Some(u) = tag.attr("src").or(tag.attr("data-src")).and_then(abs) {
                    let hint = match tag.name.as_str() {
                        "audio" => Some(false),
                        "video" => Some(true),
                        _ => None,
                    };
                    add(&mut found, u, hint);
                }
            }
            "iframe" | "embed" => {
                if let Some(u) = tag.attr("src").or(tag.attr("data-src")).and_then(abs) {
                    if let Some(e) = embed_page(&u) {
                        found.push(e);
                    }
                }
            }
            "script" if tag.attr("type").is_some_and(|t| t.contains("ld+json")) => {
                let close = find_ci(html, "</script", tag.end).unwrap_or(html.len());
                if let Ok(v) = serde_json::from_str::<Value>(html[tag.end..close].trim()) {
                    let mut stack = vec![&v];
                    while let Some(x) = stack.pop() {
                        match x {
                            Value::Array(a) => stack.extend(a.iter().rev()),
                            Value::Object(o) => {
                                let kind = o.get("@type").and_then(Value::as_str).unwrap_or("");
                                let hint = if kind.contains("Audio") { Some(false) } else if kind.contains("Video") { Some(true) } else { None };
                                for k in ["contentUrl", "embedUrl"] {
                                    if let Some(u) = o.get(k).and_then(Value::as_str).and_then(abs) {
                                        add(&mut found, u, hint);
                                    }
                                }
                                stack.extend(o.values().filter(|v| v.is_object() || v.is_array()));
                            }
                            _ => {}
                        }
                    }
                }
            }
            "link" => {
                let rel = tag.attr("rel").unwrap_or("").to_lowercase();
                let typ = tag.attr("type").unwrap_or("").to_lowercase();
                if rel.contains("alternate") && (typ.contains("rss") || typ.contains("atom")) {
                    if let Some(u) = tag.attr("href").and_then(abs) {
                        p.feeds.push(u.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    // sans média déclaré (balises, Open Graph, JSON-LD, lecteurs), les adresses
    // écrites ailleurs dans la page (données des lecteurs maison)
    if found.is_empty() {
        found = media_urls(html);
    }
    let mut seen = HashSet::new();
    let key = |u: &str| match Url::parse(u) {
        // une vidéo intégrée se distingue par sa requête (« ?v=… »), un fichier par son chemin
        Ok(x) if ext_kind(&x).is_some() => format!("{}{}", host(&x), x.path()),
        _ => u.to_string(),
    };
    p.media = found.into_iter().filter(|(u, _)| seen.insert(key(u))).take(40).collect();
    if p.title.is_empty() {
        p.title = title_tag;
    }
    p
}

fn is_feed(ctype: &str, body: &str) -> bool {
    if ctype.contains("rss") || ctype.contains("atom") {
        return true;
    }
    let head: String = body.trim_start().chars().take(4000).collect();
    let xmlish = ctype.contains("xml") || head.starts_with("<?xml") || head.starts_with("<rss") || head.starts_with("<feed");
    xmlish && (head.contains("<rss") || head.contains("<feed") || head.contains("<channel"))
}

pub(crate) fn parse_duration(s: &str) -> f64 {
    let parts: Option<Vec<f64>> = s.trim().split(':').map(|p| p.trim().parse::<f64>().ok()).collect();
    parts.map(|p| p.iter().fold(0.0, |acc, x| acc * 60.0 + x)).unwrap_or(0.0)
}

/// Épisodes d'un flux RSS ou Atom (ceux qui ont un fichier son ou vidéo).
pub fn feed_info(xml: &str, feed_url: &str) -> Option<LinkInfo> {
    let base = Url::parse(feed_url).ok();
    let abs = |v: &str| match &base {
        Some(b) => b.join(v.trim()).map(|u| u.to_string()).unwrap_or_else(|_| v.trim().to_string()),
        None => v.trim().to_string(),
    };
    let atom = find_ci(xml, "<item", 0).is_none();
    let item_tag = if atom { "entry" } else { "item" };
    let first = tags(xml).find(|t| t.name == item_tag).map(|t| t.end).unwrap_or(xml.len());
    let head = &xml[..first];
    let title = tag_text(head, "title").unwrap_or_default();
    let author = tag_text(head, "itunes:author").unwrap_or_default();
    let image = tags(head)
        .find(|t| t.name == "itunes:image" && t.attr("href").is_some())
        .and_then(|t| t.attr("href").map(abs))
        .or_else(|| tags(head).find(|t| t.name == "image").and_then(|t| {
            let close = find_ci(head, "</image", t.end)?;
            tag_text(&head[t.end..close], "url").map(|u| abs(&u))
        }))
        .unwrap_or_default();

    let mut media = Vec::new();
    let mut pos = 0;
    while let Some(start) = tags(&xml[pos..]).find(|t| t.name == item_tag).map(|t| pos + t.end) {
        let end = find_ci(xml, &format!("</{item_tag}"), start).unwrap_or(xml.len());
        let block = &xml[start..end];
        pos = end.max(start + 1).min(xml.len());
        let mut file: Option<(String, String)> = None;
        let mut page = String::new();
        let mut img = String::new();
        for tg in tags(block) {
            match tg.name.as_str() {
                "enclosure" if file.is_none() => {
                    if let Some(u) = tg.attr("url") {
                        file = Some((abs(u), tg.attr("type").unwrap_or("").to_lowercase()));
                    }
                }
                "media:content" if file.is_none() => {
                    let typ = tg.attr("type").unwrap_or("").to_lowercase();
                    let medium = tg.attr("medium").unwrap_or("");
                    if let Some(u) = tg.attr("url") {
                        if typ.starts_with("audio") || typ.starts_with("video") || medium == "audio" || medium == "video" {
                            file = Some((abs(u), if typ.is_empty() { format!("{medium}/") } else { typ }));
                        }
                    }
                }
                "link" => {
                    let rel = tg.attr("rel").unwrap_or("alternate");
                    if let Some(h) = tg.attr("href") {
                        if rel == "enclosure" && file.is_none() {
                            file = Some((abs(h), tg.attr("type").unwrap_or("").to_lowercase()));
                        } else if rel == "alternate" && page.is_empty() {
                            page = abs(h);
                        }
                    } else if page.is_empty() {
                        page = inner_text(block, &tg);
                    }
                }
                "itunes:image" if img.is_empty() => img = tg.attr("href").map(abs).unwrap_or_default(),
                _ => {}
            }
        }
        let Some((url, typ)) = file else { continue };
        let video = typ.starts_with("video") || Url::parse(&url).ok().and_then(|u| ext_kind(&u)) == Some(true);
        let date = ["pubdate", "published", "updated", "dc:date"].iter().find_map(|n| tag_text(block, n)).map(|d| ymd(&d)).unwrap_or_default();
        media.push(MediaItem {
            url: url.clone(),
            title: tag_text(block, "title").unwrap_or_else(|| Url::parse(&url).map(|u| file_title(&u)).unwrap_or_default()),
            duration: tag_text(block, "itunes:duration").map(|d| parse_duration(&d)).unwrap_or(0.0),
            video,
            direct: true,
            image: if img.is_empty() { image.clone() } else { img },
            date,
            page: if page.is_empty() { url } else { page },
            collection: title.clone(),
        });
        if media.len() >= MAX_ITEMS {
            break;
        }
    }
    if media.is_empty() {
        return None;
    }
    Some(LinkInfo {
        url: feed_url.into(),
        site: if author.is_empty() { title.clone() } else { author },
        title,
        image,
        media,
        list: true,
        ..Default::default()
    })
}

// ---------- titres semblables (Spotify, annuaire d'Apple) ----------

fn norm(s: &str) -> String {
    let flat: String = s
        .nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ressemblance de deux titres, de 0 à 1.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (short, long) = if a.len() < b.len() { (&a, &b) } else { (&b, &a) };
    if short.len() >= 6 && long.contains(short.as_str()) {
        return 0.9;
    }
    let sa: HashSet<&str> = a.split(' ').collect();
    let sb: HashSet<&str> = b.split(' ').collect();
    sa.intersection(&sb).count() as f64 / sa.union(&sb).count() as f64
}

fn days_apart(a: &str, b: &str) -> Option<i64> {
    let p = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
    Some((p(a)? - p(b)?).num_days().abs())
}

/// L'épisode d'une liste qui correspond à un titre (et, s'ils sont connus, à
/// une durée et une date : les fuseaux horaires décalent parfois d'un jour).
fn best_episode(items: &[MediaItem], title: &str, duration: f64, date: &str) -> Option<MediaItem> {
    let mut best: Option<(f64, &MediaItem)> = None;
    for m in items {
        let sim = similarity(&m.title, title);
        let same_len = duration > 0.0 && m.duration > 0.0 && (m.duration - duration).abs() <= (duration * 0.03).max(30.0);
        let same_day = days_apart(&m.date, date).is_some_and(|d| d <= 1);
        if !(sim >= 0.8 || (sim >= 0.5 && (same_len || same_day))) {
            continue;
        }
        let score = sim + if same_len { 0.2 } else { 0.0 } + if same_day { 0.1 } else { 0.0 };
        if best.map_or(true, |(b, _)| score > b) {
            best = Some((score, m));
        }
    }
    best.map(|(_, m)| m.clone())
}

// ---------- podcasts : annuaire d'Apple ----------

struct Show {
    name: String,
    feed: String,
    image: String,
}

fn itunes_url(params: &[(&str, &str)]) -> String {
    let mut u = Url::parse("https://itunes.apple.com/search").expect("adresse fixe");
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    u.to_string()
}

/// Podcasts de l'annuaire d'Apple dont le nom ressemble à `name`, du plus proche au moins proche.
async fn itunes_shows(c: &reqwest::Client, name: &str) -> Vec<Show> {
    let url = itunes_url(&[("media", "podcast"), ("entity", "podcast"), ("limit", "10"), ("term", name)]);
    let Ok(v) = get_json(c, &url).await else { return Vec::new() };
    let mut shows: Vec<Show> = v
        .get("results")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|r| Show { name: str_of(r, "collectionName"), feed: str_of(r, "feedUrl"), image: str_of(r, "artworkUrl600") })
                .filter(|s| s.feed.starts_with("http"))
                .collect()
        })
        .unwrap_or_default();
    shows.sort_by(|a, b| similarity(&b.name, name).total_cmp(&similarity(&a.name, name)));
    shows
}

async fn read_feed(c: &reqwest::Client, url: &str) -> Option<LinkInfo> {
    feed_info(&get_text(c, url).await.ok()?, url)
}

/// Flux public d'un podcast d'après son nom.
async fn find_feed(c: &reqwest::Client, show: &str) -> Option<LinkInfo> {
    for s in itunes_shows(c, show).await.iter().take(3) {
        if similarity(&s.name, show) < 0.6 {
            break;
        }
        if let Some(mut f) = read_feed(c, &s.feed).await {
            if f.image.is_empty() {
                f.image = s.image.clone();
            }
            return Some(f);
        }
    }
    None
}

/// Un épisode d'après son titre et le nom de son émission : dans le flux public
/// du podcast, sinon dans l'annuaire des épisodes d'Apple.
async fn find_episode(c: &reqwest::Client, show: &str, title: &str, duration: f64, date: &str) -> Option<MediaItem> {
    for s in itunes_shows(c, show).await.iter().take(4) {
        if similarity(&s.name, show) < 0.3 {
            break;
        }
        let Some(f) = read_feed(c, &s.feed).await else { continue };
        if let Some(mut m) = best_episode(&f.media, title, duration, date) {
            if m.image.is_empty() {
                m.image = s.image.clone();
            }
            return Some(m);
        }
    }
    let url = itunes_url(&[("media", "podcast"), ("entity", "podcastEpisode"), ("limit", "25"), ("term", title)]);
    let v = get_json(c, &url).await.ok()?;
    let items: Vec<MediaItem> = v
        .get("results")?
        .as_array()?
        .iter()
        .filter(|r| similarity(&str_of(r, "collectionName"), show) >= 0.5)
        .filter_map(apple_episode)
        .collect();
    best_episode(&items, title, duration, date)
}

fn apple_episode(r: &Value) -> Option<MediaItem> {
    let url = str_of(r, "episodeUrl");
    if !url.starts_with("http") {
        return None;
    }
    let ext = str_of(r, "episodeFileExtension").to_lowercase();
    Some(MediaItem {
        url,
        title: str_of(r, "trackName"),
        duration: num(r, "trackTimeMillis") / 1000.0,
        video: VIDEO_EXT.contains(&ext.as_str()) || str_of(r, "episodeContentType") == "video",
        direct: true,
        image: str_of(r, "artworkUrl600"),
        date: ymd(&str_of(r, "releaseDate")),
        page: str_of(r, "trackViewUrl"),
        collection: str_of(r, "collectionName"),
    })
}

/// Lien Apple Podcasts : une émission (« id… ») ou l'un de ses épisodes (« ?i=… »).
async fn apple(c: &reqwest::Client, url: &Url) -> Result<Option<LinkInfo>> {
    let id = url
        .path_segments()
        .and_then(|mut s| s.find_map(|p| p.strip_prefix("id").filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).map(str::to_string)));
    let Some(id) = id else { return Ok(None) };
    let wanted = url.query_pairs().find(|(k, _)| k == "i").map(|(_, v)| v.into_owned());
    let v = get_json(c, &format!("https://itunes.apple.com/lookup?id={id}&entity=podcastEpisode&limit=200")).await?;
    let results = v.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    let show = results.iter().find(|r| str_of(r, "wrapperType") == "track");
    let name = show.map(|s| str_of(s, "collectionName")).unwrap_or_default();
    let feed = show.map(|s| str_of(s, "feedUrl")).unwrap_or_default();
    let art = show.map(|s| str_of(s, "artworkUrl600")).unwrap_or_default();
    let episodes: Vec<(String, MediaItem)> = results
        .iter()
        .filter(|r| str_of(r, "wrapperType") == "podcastEpisode")
        .filter_map(|r| Some((str_of(r, "trackId"), apple_episode(r)?)))
        .collect();
    if let Some(w) = &wanted {
        if let Some((_, m)) = episodes.iter().find(|(tid, _)| tid == w) {
            let mut m = m.clone();
            m.page = url.to_string();
            return Ok(Some(LinkInfo {
                url: url.to_string(),
                title: m.title.clone(),
                site: name,
                image: if m.image.is_empty() { art } else { m.image.clone() },
                media: vec![m],
                ..Default::default()
            }));
        }
    }
    // l'émission entière : son flux (tous les épisodes), sinon les 200 derniers de l'annuaire
    if feed.starts_with("http") {
        if let Some(mut f) = read_feed(c, &feed).await {
            f.url = url.to_string();
            if f.image.is_empty() {
                f.image = art;
            }
            return Ok(Some(f));
        }
    }
    if episodes.is_empty() {
        return Err(anyhow!(t("Cette émission ne propose aucun épisode à écouter.", "This show has no episode to listen to.")));
    }
    Ok(Some(LinkInfo {
        url: url.to_string(),
        title: name.clone(),
        site: name,
        image: art,
        media: episodes.into_iter().map(|(_, m)| m).collect(),
        list: true,
        ..Default::default()
    }))
}

// ---------- Spotify ----------

const SPOTIFY_KINDS: &[&str] = &["episode", "show", "track", "album", "playlist", "artist"];

fn spotify_ref(u: &Url) -> Option<(String, String)> {
    if host(u) != "open.spotify.com" {
        return None;
    }
    let segs: Vec<&str> = u.path_segments()?.filter(|s| !s.is_empty()).collect();
    let i = segs.iter().position(|s| SPOTIFY_KINDS.contains(s))?;
    let id = segs.get(i + 1)?;
    id.bytes().all(|b| b.is_ascii_alphanumeric()).then(|| (segs[i].to_string(), id.to_string()))
}

/// Adresse open.spotify.com écrite dans une page (liens courts spotify.link).
fn spotify_in(body: &str) -> Option<(String, String)> {
    let mut from = 0;
    while let Some(i) = body[from..].find("open.spotify.com/") {
        let start = from + i;
        let end = body[start..].find(|c: char| matches!(c, '"' | '\'' | '?' | '<' | '\\') || c.is_whitespace()).map(|e| start + e).unwrap_or(body.len());
        if let Some(r) = Url::parse(&format!("https://{}", &body[start..end])).ok().and_then(|u| spotify_ref(&u)) {
            return Some(r);
        }
        from = start + 1;
    }
    None
}

/// Données publiques d'un contenu Spotify : celles de son lecteur intégré.
async fn spotify_entity(c: &reqwest::Client, kind: &str, id: &str) -> Result<Value> {
    let html = get_text(c, &format!("https://open.spotify.com/embed/{kind}/{id}")).await?;
    let json = html
        .find("id=\"__NEXT_DATA__\"")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
        .and_then(|s| html[s..].find("</script>").map(|e| &html[s..s + e]));
    let v: Option<Value> = json.and_then(|j| serde_json::from_str(j).ok());
    v.and_then(|v| v.pointer("/props/pageProps/state/data/entity").cloned())
        .ok_or_else(|| anyhow!(t("Spotify n'a pas répondu comme prévu. Réessayez dans un instant.", "Spotify didn't answer as expected. Try again in a moment.")))
}

fn spotify_image(e: &Value) -> String {
    for list in [e.pointer("/visualIdentity/image"), e.get("relatedEntityCoverArt"), e.pointer("/coverArt/sources")].into_iter().flatten() {
        let best = list.as_array().and_then(|a| a.iter().max_by_key(|x| x.get("maxWidth").or(x.get("width")).and_then(Value::as_u64).unwrap_or(0)));
        if let Some(u) = best.map(|b| str_of(b, "url")).filter(|u| !u.is_empty()) {
            return u;
        }
    }
    String::new()
}

/// Un morceau : le même, trouvé sur YouTube au moment de l'import.
fn song(title: &str, artist: &str, duration: f64, image: &str, page: &str, collection: &str) -> MediaItem {
    MediaItem {
        url: format!("ytsearch1:{artist} {title} audio"),
        title: if artist.is_empty() { title.to_string() } else { format!("{title} · {artist}") },
        duration,
        video: false,
        direct: false,
        image: image.into(),
        date: String::new(),
        page: page.into(),
        collection: collection.into(),
    }
}

async fn spotify(c: &reqwest::Client, url: &Url) -> Result<LinkInfo> {
    let unreadable = || anyhow!(t("Ce lien Spotify n'a pas pu être lu.", "This Spotify link couldn't be read."));
    let (kind, id) = match spotify_ref(url) {
        Some(r) => r,
        None => {
            // lien court : la redirection, ou l'adresse écrite dans la page
            let resp = c.get(url.as_str()).send().await?;
            let found = spotify_ref(resp.url());
            match found {
                Some(r) => r,
                None => spotify_in(&body_text(resp).await?).ok_or_else(unreadable)?,
            }
        }
    };
    let page = format!("https://open.spotify.com/{kind}/{id}");
    let e = spotify_entity(c, &kind, &id).await?;
    let name = ["title", "name"].iter().map(|k| str_of(&e, k)).find(|s| !s.is_empty()).unwrap_or_default();
    let image = spotify_image(&e);
    match kind.as_str() {
        "episode" => {
            let show = str_of(&e, "subtitle");
            let date = ymd(e.pointer("/releaseDate/isoString").and_then(Value::as_str).unwrap_or(""));
            let Some(mut item) = find_episode(c, &show, &name, num(&e, "duration") / 1000.0, &date).await else {
                return Err(anyhow!(t(
                    "Cet épisode n'existe que sur Spotify, qui protège ses fichiers contre la copie. Lumen ne l'a trouvé dans aucun flux public de podcast.",
                    "This episode only exists on Spotify, which protects its files against copying. Lumen didn't find it in any public podcast feed."
                )));
            };
            item.page = page.clone();
            item.collection = show.clone();
            if item.image.is_empty() {
                item.image = image.clone();
            }
            Ok(LinkInfo { url: page, title: name, site: show, image, media: vec![item], via: "rss".into(), ..Default::default() })
        }
        "show" => {
            // le lecteur d'une émission montre son dernier épisode : le nom de l'émission est son sous-titre
            let sub = str_of(&e, "subtitle");
            let show = if str_of(&e, "type") == "episode" && !sub.is_empty() { sub } else { name };
            let Some(mut f) = find_feed(c, &show).await else {
                return Err(anyhow!(t(
                    "Ce podcast n'existe que sur Spotify, qui protège ses fichiers contre la copie. Lumen ne l'a trouvé dans aucun flux public.",
                    "This podcast only exists on Spotify, which protects its files against copying. Lumen didn't find it in any public feed."
                )));
            };
            f.url = page;
            f.via = "rss".into();
            if f.image.is_empty() {
                f.image = image;
            }
            Ok(f)
        }
        "track" => {
            let artists = e
                .get("artists")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let item = song(&name, &artists, num(&e, "duration") / 1000.0, &image, &page, "");
            Ok(LinkInfo { url: page, title: name, site: artists, image, media: vec![item], via: "youtube".into(), ..Default::default() })
        }
        "album" | "playlist" => {
            let media: Vec<MediaItem> = e
                .get("trackList")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|x| str_of(x, "entityType") != "episode")
                        .filter(|x| !str_of(x, "title").is_empty())
                        .map(|x| song(&str_of(x, "title"), &str_of(x, "subtitle"), num(x, "duration") / 1000.0, &image, &page, &name))
                        .take(MAX_ITEMS)
                        .collect()
                })
                .unwrap_or_default();
            if media.is_empty() {
                return Err(unreadable());
            }
            Ok(LinkInfo { url: page, title: name, site: str_of(&e, "subtitle"), image, media, list: true, via: "youtube".into(), ..Default::default() })
        }
        _ => Err(anyhow!(t(
            "Sur Spotify, choisissez un épisode, un podcast, un morceau, un album ou une playlist.",
            "On Spotify, choose an episode, a podcast, a song, an album or a playlist."
        ))),
    }
}

// ---------- téléchargements ----------

fn ext_for(u: &Url, ctype: &str, video: bool) -> String {
    if let Some(e) = u.path_segments().and_then(|mut s| s.next_back()).and_then(|l| l.rsplit_once('.')).map(|(_, e)| e.to_lowercase()) {
        if AUDIO_EXT.contains(&e.as_str()) || VIDEO_EXT.contains(&e.as_str()) {
            return e;
        }
    }
    let by_type = [
        ("audio/mpeg", "mp3"),
        ("audio/mp3", "mp3"),
        ("audio/mp4", "m4a"),
        ("audio/x-m4a", "m4a"),
        ("audio/aac", "aac"),
        ("audio/ogg", "ogg"),
        ("audio/opus", "opus"),
        ("audio/wav", "wav"),
        ("audio/x-wav", "wav"),
        ("audio/flac", "flac"),
        ("video/mp4", "mp4"),
        ("video/quicktime", "mov"),
        ("video/webm", "webm"),
    ];
    by_type.iter().find(|(m, _)| ctype.starts_with(m)).map(|(_, e)| e.to_string()).unwrap_or_else(|| if video { "mp4" } else { "mp3" }.into())
}

/// Télécharge un fichier son ou vidéo dans la bibliothèque de Lumen.
pub async fn download(data_dir: &Path, url: &str, stem: &str, video: bool, on_progress: &mut (dyn FnMut(f64) + Send)) -> Result<PathBuf> {
    let c = reqwest::Client::builder().user_agent(UA).connect_timeout(Duration::from_secs(20)).read_timeout(Duration::from_secs(60)).build()?;
    let resp = c.get(url).send().await.map_err(|e| anyhow!(crate::tr!("Fichier inaccessible : {e}", "File unreachable: {e}")))?;
    if !resp.status().is_success() {
        return Err(anyhow!(crate::tr!("Le serveur a répondu {}.", "The server answered {}.", resp.status())));
    }
    let ctype = resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if ctype.starts_with("text/") {
        return Err(anyhow!(t("Ce lien ne mène pas à un fichier son ou vidéo.", "This link doesn't lead to a sound or video file.")));
    }
    if live_response(&resp) {
        return Err(live_error());
    }
    let dir = media::media_dir(data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    let ext = ext_for(resp.url(), &ctype, video);
    let dest = dir.join(format!("{stem}.{}.{ext}", if video { "video" } else { "audio" }));
    let tmp = dest.with_extension(format!("{ext}.part"));
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut got = 0u64;
    let mut stream = resp.bytes_stream();
    let res: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            got += chunk.len() as u64;
            if total > 0 {
                on_progress(got as f64 / total as f64 * 100.0);
            }
        }
        file.flush().await?;
        Ok(())
    }
    .await;
    drop(file);
    if let Err(e) = res {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow!(crate::tr!("Téléchargement interrompu : {e}", "Download interrupted: {e}")));
    }
    tokio::fs::rename(&tmp, &dest).await?;
    Ok(dest)
}

/// Télécharge une couverture (podcast, Spotify, site vidéo), réduite comme
/// celles qu'on choisit à la main (1 280 px, JPEG). Rien en cas d'échec.
pub async fn fetch_cover(data_dir: &Path, url: &str) -> Option<PathBuf> {
    let c = client().ok()?;
    let resp = c.get(url).send().await.ok()?;
    let ctype = resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !resp.status().is_success() || !ctype.starts_with("image/") {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.is_empty() || bytes.len() > 30_000_000 {
        return None;
    }
    let dir = media::media_dir(data_dir);
    std::fs::create_dir_all(&dir).ok()?;
    let ext = match ctype.as_str() {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    };
    let stem = media::new_stem();
    let raw = dir.join(format!("{stem}.cover-src.{ext}"));
    let out = dir.join(format!("{stem}.cover.jpg"));
    std::fs::write(&raw, &bytes).ok()?;
    // outil d'images de macOS : réduit et convertit en JPEG
    let sips = tokio::process::Command::new("/usr/bin/sips")
        .args(["-Z", "1280", "-s", "format", "jpeg", "-s", "formatOptions", "85"])
        .arg(&raw)
        .arg("--out")
        .arg(&out)
        .output()
        .await;
    if sips.is_ok_and(|o| o.status.success()) && out.exists() {
        let _ = std::fs::remove_file(&raw);
        return Some(out);
    }
    let _ = std::fs::remove_file(&out);
    let kept = dir.join(format!("{stem}.cover.{ext}"));
    std::fs::rename(&raw, &kept).ok()?;
    Some(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_completed() {
        assert_eq!(normalize("lemonde.fr/article").unwrap().as_str(), "https://lemonde.fr/article");
        assert_eq!(normalize("spotify:episode:2feqlkMdhv8I5sMtRlxrs6").unwrap().as_str(), "https://open.spotify.com/episode/2feqlkMdhv8I5sMtRlxrs6");
        assert_eq!(normalize("feed://example.com/rss").unwrap().as_str(), "https://example.com/rss");
        assert!(normalize("bonjour").is_err());
        assert!(normalize("ftp://example.com/a").is_err());
        let u = Url::parse("https://open.spotify.com/intl-fr/episode/2feqlkMdhv8I5sMtRlxrs6?si=abc").unwrap();
        assert_eq!(spotify_ref(&u), Some(("episode".into(), "2feqlkMdhv8I5sMtRlxrs6".into())));
        assert_eq!(spotify_in("<a href=\"https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=x\">"), Some(("track".into(), "4uLU6hMCjMI75M1A2tKUQC".into())));
    }

    #[test]
    fn entities_and_durations() {
        assert_eq!(decode_entities("Tom &amp; Jerry &#233;t&eacute; &#x41;"), "Tom & Jerry ét&eacute; A");
        assert_eq!(decode_entities("café & crème"), "café & crème");
        assert_eq!(parse_duration("1872"), 1872.0);
        assert_eq!(parse_duration("31:12"), 1872.0);
        assert_eq!(parse_duration("00:31:12"), 1872.0);
        assert_eq!(ymd("Wed, 10 Sep 2025 15:51:45 +0200"), "2025-09-10");
        assert_eq!(ymd("2025-09-10T13:51:00Z"), "2025-09-10");
        assert_eq!(ymd("20250910"), "2025-09-10");
    }

    #[test]
    fn page_media_are_found() {
        // page à la RFI : le fichier est dans le JSON-LD (barres échappées), une vidéo YouTube intégrée
        let html = r#"<html><head><title>Journal &amp; info</title>
            <meta property="og:title" content="Journal en français facile">
            <meta property="og:type" content="article"><meta property="og:image" content="/img/cover.jpg">
            <link rel="alternate" type="application/rss+xml" href="/podcast">
            <script type="application/ld+json">{"@graph":[{"@type":"AudioObject","contentUrl":"https:\/\/aod.example.net\/jff\/journal_20261002.mp3","x":"<b>"}]}</script>
            </head><body><!-- <audio src="https://x.net/hidden.mp3"> -->
            <iframe data-src="//www.youtube.com/embed/dQw4w9WgXcQ?rel=0"></iframe>
            <audio controls><source src="sons/extrait.m4a?v=2" type="audio/mp4"></audio>
            <a href="https://aod.example.net/jff/journal_20261002.mp3?dl=1">Télécharger</a>
            <script>var live = "https://rfimonde64k.ice.infomaniak.ch/rfimonde-64.mp3", jingle = "https://static.rfi.fr/player/virgule.mp3";</script>
            <p>Texte</p></body></html>"#;
        let base = Url::parse("https://www.rfi.fr/fr/podcasts/jff/episode").unwrap();
        let p = scan_html(&base, html);
        assert_eq!(p.title, "Journal en français facile");
        assert_eq!(p.kind, "article");
        assert_eq!(p.image, "https://www.rfi.fr/img/cover.jpg");
        assert_eq!(p.feeds, vec!["https://www.rfi.fr/podcast".to_string()]);
        let urls: Vec<&str> = p.media.iter().map(|(u, _)| u.as_str()).collect();
        assert!(urls.contains(&"https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(urls.contains(&"https://www.rfi.fr/fr/podcasts/jff/sons/extrait.m4a?v=2"));
        assert!(urls.contains(&"https://aod.example.net/jff/journal_20261002.mp3"));
        // un même fichier n'est proposé qu'une fois, une radio en direct jamais
        assert_eq!(p.media.iter().filter(|(u, _)| u.contains("journal_20261002")).count(), 1);
        assert!(!urls.iter().any(|u| u.contains("infomaniak") || u.contains("virgule")));
        // sans média déclaré, les adresses du code de la page, sauf les radios en direct
        let loose = scan_html(&base, r"<script>a='https:\/\/x.net\/a.mp3';b='https://ice.x.net/live.mp3'</script>");
        assert_eq!(loose.media, vec![("https://x.net/a.mp3".to_string(), false)]);
    }

    #[test]
    fn feeds_are_read() {
        let xml = r#"<?xml version="1.0"?><rss xmlns:itunes="x"><channel><title>InnerFrench</title>
            <itunes:author>Hugo</itunes:author><itunes:image href="https://img.example/show.jpg"/>
            <item><title><![CDATA[E179 La France est-elle anti-enfants ?]]></title>
              <link>https://podcast.example/e179</link><pubDate>Wed, 10 Sep 2025 15:51:45 +0200</pubDate>
              <enclosure url="https://cdn.example/e179.mp3" length="1" type="audio/mpeg"/><itunes:duration>31:12</itunes:duration></item>
            <item><title>Sans fichier</title></item>
            <item><title>Vidéo</title><enclosure url="https://cdn.example/v.mp4" type="video/mp4"/></item>
            </channel></rss>"#;
        assert!(is_feed("application/rss+xml", xml));
        let f = feed_info(xml, "https://podcast.example/feed.xml").unwrap();
        assert_eq!(f.title, "InnerFrench");
        assert_eq!(f.site, "Hugo");
        assert_eq!(f.media.len(), 2);
        let e = &f.media[0];
        assert_eq!(e.title, "E179 La France est-elle anti-enfants ?");
        assert_eq!((e.duration, e.date.as_str(), e.page.as_str()), (1872.0, "2025-09-10", "https://podcast.example/e179"));
        assert_eq!(e.image, "https://img.example/show.jpg");
        assert!(e.direct && !e.video && f.media[1].video);
        // Spotify donne le titre, la durée et la date : on retrouve l'épisode
        let hit = best_episode(&f.media, "E179 La France est-elle anti-enfants ?", 1872.1, "2025-09-10").unwrap();
        assert_eq!(hit.url, "https://cdn.example/e179.mp3");
        assert!(best_episode(&f.media, "Un tout autre épisode", 0.0, "").is_none());
    }

    #[test]
    fn titles_compare() {
        assert_eq!(similarity("InnerFrench", "innerFrench"), 1.0);
        assert_eq!(similarity("La France est-elle anti-enfants", "E179 La France est-elle anti-enfants ?"), 0.9);
        assert!(similarity("Le journal", "Un podcast de cuisine") < 0.3);
    }

    /// Téléchargements réels (ignoré par défaut) : un épisode, une couverture réduite.
    /// cargo test --lib link_download_live -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn link_download_live() {
        let dir = std::env::temp_dir().join("lumen-link-download");
        let _ = std::fs::remove_dir_all(&dir);
        let feed = "https://www.rfi.fr/fr/podcasts/journal-fran%C3%A7ais-facile/podcast";
        let xml = get_text(&client().unwrap(), feed).await.unwrap();
        let ep = feed_info(&xml, feed).unwrap().media.remove(0);
        let mut last = 0.0;
        let mut on = |p: f64| last = p;
        let path = download(&dir, &ep.url, &media::new_stem(), ep.video, &mut on).await.unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        println!("« {} » → {} ({} Mo, progression finale {last:.0} %)", ep.title, path.display(), size / 1_000_000);
        assert!(size > 1_000_000 && last > 99.0);
        let cover = fetch_cover(&dir, "https://is1-ssl.mzstatic.com/image/thumb/Podcasts126/v4/5f/4b/51/5f4b5121-b307-4b0c-4ba9-3fba7aba161d/mza_1141892237621262066.jpg/3000x3000bb.jpg").await.unwrap();
        let info = std::process::Command::new("/usr/bin/sips").args(["-g", "pixelWidth", "-g", "pixelHeight"]).arg(&cover).output().unwrap();
        println!("couverture {} :\n{}", cover.display(), String::from_utf8_lossy(&info.stdout));
        assert!(String::from_utf8_lossy(&info.stdout).contains("pixelWidth: 1280"));
    }

    /// Essai réel (ignoré par défaut) : LUMEN_TEST_URL="lien1 lien2" cargo test --lib link_live -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn link_live() {
        let Ok(urls) = std::env::var("LUMEN_TEST_URL") else { return };
        let dir = std::env::temp_dir().join("lumen-link-live");
        std::fs::create_dir_all(&dir).unwrap();
        for u in urls.split_whitespace() {
            let mut quiet = |_e: ImportEvent| {};
            match probe(&dir, u, None, &mut quiet).await {
                Ok(i) => {
                    println!("\n{u}\n  « {} » · {} · via {:?} · liste {} · {} Ko de page · note {:?}", i.title, i.site, i.via, i.list, i.html.len() / 1000, i.note);
                    for m in i.media.iter().take(4) {
                        println!("  - {} · {:.0} s · vidéo {} · direct {} · {} · {}", m.title, m.duration, m.video, m.direct, m.date, m.url);
                    }
                }
                Err(e) => println!("\n{u}\n  erreur : {e}"),
            }
        }
    }
}
