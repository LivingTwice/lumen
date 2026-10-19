//! YouTube sans yt-dlp, pour l'iPhone et l'iPad : iOS interdit de lancer un autre
//! programme. Lumen y parle directement à l'API de YouTube (InnerTube), comme ses
//! propres apps : recherche, onglet « Vidéos » d'une chaîne, playlist, informations
//! et fichiers d'une vidéo. `run` comprend les quelques commandes de yt-dlp
//! qu'emploie media.rs et répond comme lui (JSON de `-J`, lignes de `--print`) :
//! search.rs, discover.rs, link.rs et media.rs ne changent pas.
//!
//! Sur Mac et Windows, yt-dlp reste le chemin (il suit YouTube de près) ; ce module
//! s'y essaie avec `LUMEN_YT_NATIVE=1`, et ses tests réels (`ytnative_live`) passent
//! par les vraies adresses de YouTube.
//!
//! Mesuré en octobre 2026 : le client « visionOS » de l'API (celui que yt-dlp emploie
//! sans moteur JavaScript) donne des adresses directes, déjà signées, qui se
//! téléchargent en entier, à condition de présenter l'identifiant de visiteur lu sur
//! une page de YouTube (sinon : « confirmez que vous n'êtes pas un robot »). Le
//! client « iOS » ne laisse télécharger que le premier mégaoctet sans jeton
//! d'attestation : il ne sert plus que de repli pour l'aperçu (flux HLS). La
//! recherche et les chaînes passent par le client « Web » (`videoRenderer`,
//! `lockupViewModel`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use reqwest::Url;
use serde_json::{json, Value};

use crate::i18n::t;

const WEB_VERSION: &str = "2.20261015.01.00";
const WEB_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

/// Un client de l'API qui donne des adresses de lecture directes.
struct Client {
    name: &'static str,
    id: &'static str,
    version: &'static str,
    ua: &'static str,
    device: fn() -> Value,
}

const VISION_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

const PLAYERS: [Client; 3] = [
    Client {
        name: "VISIONOS",
        id: "101",
        version: "1.02",
        ua: VISION_UA,
        device: || json!({ "deviceMake": "Apple", "deviceModel": "RealityDevice17,1", "userAgent": VISION_UA, "osName": "visionOS", "osVersion": "26.5.23O471" }),
    },
    Client {
        name: "IOS",
        id: "5",
        version: "20.10.4",
        ua: "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
        device: || json!({ "deviceMake": "Apple", "deviceModel": "iPhone16,2", "osName": "iPhone", "osVersion": "18.3.2.22D82" }),
    },
    Client {
        name: "ANDROID_VR",
        id: "28",
        version: "1.65.10",
        ua: "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
        device: || json!({ "deviceMake": "Oculus", "deviceModel": "Quest 3", "androidSdkVersion": 32, "osName": "Android", "osVersion": "12L" }),
    },
];

/// Onglet « Vidéos » d'une chaîne (paramètre de l'API, celui du site).
const VIDEOS_TAB: &str = "EgZ2aWRlb3PyBgQKAjoA";
/// Morceaux d'un téléchargement : YouTube ralentit les fichiers demandés d'un bloc.
const CHUNK: u64 = 10 << 20;

/// Le module remplace yt-dlp : toujours sur iPhone et iPad, sur demande ailleurs (essais).
pub fn is_on() -> bool {
    cfg!(target_os = "ios") || std::env::var_os("LUMEN_YT_NATIVE").is_some()
}

// ---------- la ligne de commande de yt-dlp ----------

/// Ce que demandait la ligne de commande (seulement ce qu'emploie media.rs).
#[derive(Default, Debug, PartialEq)]
struct Args {
    /// `-J` : répondre en JSON sans télécharger
    json: bool,
    /// `--flat-playlist` : une liste n'est pas dépliée
    flat: bool,
    /// `--playlist-items a:b` (à partir de 1)
    items: Option<(usize, usize)>,
    /// `youtube:lang=xx` : titres dans cette langue
    lang: Option<String>,
    /// `youtubetab:approximate_date` : dates approchées (« 3 days ago »)
    approx_dates: bool,
    /// `-f`, `-S`
    format: String,
    sort: String,
    /// `--no-simulate` : télécharger
    download: bool,
    /// `--print` (avec `after_move:` éventuel)
    prints: Vec<String>,
    /// `-o`
    output: String,
    url: String,
}

fn parse_args(args: &[String]) -> Args {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let s = args[i].as_str();
        let next = args.get(i + 1).cloned().unwrap_or_default();
        match s {
            "-J" => a.json = true,
            "--flat-playlist" => a.flat = true,
            "--no-simulate" => a.download = true,
            "--playlist-items" | "--extractor-args" | "-f" | "-S" | "--print" | "-o" | "--js-runtimes" | "--cookies-from-browser" | "--progress-template" => {
                match s {
                    "--playlist-items" => {
                        let (x, y) = next.split_once(':').unwrap_or((next.as_str(), next.as_str()));
                        a.items = Some((x.parse().unwrap_or(1).max(1), y.parse().unwrap_or(usize::MAX)));
                    }
                    "--extractor-args" => {
                        if let Some(l) = next.strip_prefix("youtube:lang=") {
                            a.lang = Some(l.to_string());
                        } else if next.contains("approximate_date") {
                            a.approx_dates = true;
                        }
                    }
                    "-f" => a.format = next,
                    "-S" => a.sort = next,
                    "--print" => a.prints.push(next),
                    "-o" => a.output = next,
                    _ => {}
                }
                i += 1;
            }
            _ if !s.starts_with('-') => a.url = s.to_string(),
            _ => {}
        }
        i += 1;
    }
    a
}

// ---------- adresses ----------

/// Ce que désigne une adresse.
#[derive(Debug, PartialEq)]
enum Target {
    /// recherche (page de résultats avec ses filtres `sp`, ou `ytsearchN:`)
    Search { query: String, sp: Option<String>, limit: Option<usize> },
    /// chaîne : identifiant `UC…`, ou adresse à résoudre (`@pseudo`, `/c/`, `/user/`)
    Channel { id: Option<String>, url: String, videos: bool },
    Playlist(String),
    Video(String),
    Dailymotion(String),
}

fn video_id_ok(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn target_of(raw: &str) -> Result<Target> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix("ytsearch") {
        let (n, q) = rest.split_once(':').unwrap_or(("1", rest));
        return Ok(Target::Search { query: q.trim().to_string(), sp: None, limit: Some(n.parse().unwrap_or(1)) });
    }
    let unsupported = || {
        anyhow!(t(
            "Sur iPhone et iPad, Lumen lit les vidéos de YouTube et de Dailymotion. Pour ce site, importez la leçon depuis votre Mac : elle viendra par la sauvegarde.",
            "On iPhone and iPad, Lumen reads YouTube and Dailymotion videos. For this site, import the lesson on your Mac: it will come over with the backup."
        ))
    };
    let url = Url::parse(raw).map_err(|_| unsupported())?;
    let host = url.host_str().unwrap_or("").trim_start_matches("www.").trim_start_matches("m.").to_lowercase();
    let q = |k: &str| url.query_pairs().find(|(x, _)| x == k).map(|(_, v)| v.to_string());
    let segs: Vec<String> = url.path_segments().map(|s| s.filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    if host == "youtu.be" {
        let id = segs.first().cloned().unwrap_or_default();
        return if video_id_ok(&id) { Ok(Target::Video(id)) } else { Err(unsupported()) };
    }
    if host == "dai.ly" {
        return segs.first().map(|id| Target::Dailymotion(id.clone())).ok_or_else(unsupported);
    }
    if host.ends_with("dailymotion.com") {
        if segs.first().map(String::as_str) == Some("video") {
            if let Some(id) = segs.get(1) {
                return Ok(Target::Dailymotion(id.split('_').next().unwrap_or(id).to_string()));
            }
        }
        return Err(unsupported());
    }
    if host == "music.youtube.com" {
        // la recherche de YouTube Music n'est pas lue ici : Lumen essaie ensuite la recherche ordinaire
        if let Some(v) = q("v").filter(|v| video_id_ok(v)) {
            return Ok(Target::Video(v));
        }
        return Err(anyhow!("YouTube Music"));
    }
    if !(host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtube-nocookie.com") {
        return Err(unsupported());
    }
    let first = segs.first().map(String::as_str).unwrap_or("");
    match first {
        "watch" => q("v").filter(|v| video_id_ok(v)).map(Target::Video).ok_or_else(unsupported),
        "shorts" | "embed" | "live" | "v" => segs.get(1).filter(|v| video_id_ok(v)).cloned().map(Target::Video).ok_or_else(unsupported),
        "results" => Ok(Target::Search { query: q("search_query").or_else(|| q("q")).unwrap_or_default(), sp: q("sp"), limit: None }),
        "playlist" => q("list").map(Target::Playlist).ok_or_else(unsupported),
        "channel" => Ok(Target::Channel {
            id: segs.get(1).cloned(),
            url: raw.to_string(),
            videos: true,
        }),
        _ if first.starts_with('@') || first == "c" || first == "user" => Ok(Target::Channel { id: None, url: raw.to_string(), videos: true }),
        _ => {
            if let Some(v) = q("v").filter(|v| video_id_ok(v)) {
                return Ok(Target::Video(v));
            }
            Err(unsupported())
        }
    }
}

// ---------- appels à l'API ----------

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(40)).build()?)
}

/// Pays des résultats : celui de la langue (titres et classements du pays).
fn region(hl: &str) -> String {
    crate::search::country(hl).to_uppercase()
}

/// Identifiant de visiteur de YouTube (`VISITOR_DATA` d'une de ses pages), gardé une
/// demi-heure : sans lui, l'API demande de « confirmer qu'on n'est pas un robot ».
async fn visitor(c: &reqwest::Client) -> Option<String> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<parking_lot::Mutex<Option<(std::time::Instant, String)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| parking_lot::Mutex::new(None));
    if let Some((at, v)) = cache.lock().as_ref() {
        if at.elapsed() < Duration::from_secs(30 * 60) {
            return Some(v.clone());
        }
    }
    let html = c
        .get("https://www.youtube.com/?hl=en&persist_hl=1")
        .header("User-Agent", WEB_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let start = html.find("\"VISITOR_DATA\":\"")? + "\"VISITOR_DATA\":\"".len();
    let v = html[start..].split('"').next()?.to_string();
    if v.is_empty() {
        return None;
    }
    *cache.lock() = Some((std::time::Instant::now(), v.clone()));
    Some(v)
}

async fn web(c: &reqwest::Client, endpoint: &str, mut body: Value, hl: &str) -> Result<Value> {
    let vd = visitor(c).await;
    body["context"] = json!({ "client": { "clientName": "WEB", "clientVersion": WEB_VERSION, "hl": hl, "gl": region(hl) } });
    if let Some(v) = &vd {
        body["context"]["client"]["visitorData"] = json!(v);
    }
    let url = format!("https://www.youtube.com/youtubei/v1/{endpoint}?prettyPrint=false");
    let mut req = c
        .post(url)
        .header("User-Agent", WEB_UA)
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", WEB_VERSION)
        .header("Origin", "https://www.youtube.com");
    if let Some(v) = &vd {
        req = req.header("X-Goog-Visitor-Id", v);
    }
    let r = req.json(&body).send().await?;
    if !r.status().is_success() {
        return Err(anyhow!(crate::tr!("YouTube a répondu {}", "YouTube answered {}", r.status())));
    }
    Ok(r.json().await?)
}

async fn player(c: &reqwest::Client, cl: &Client, id: &str, hl: &str) -> Result<Value> {
    let vd = visitor(c).await;
    let mut client = json!({ "clientName": cl.name, "clientVersion": cl.version, "hl": hl, "gl": region(hl) });
    if let (Value::Object(m), Value::Object(d)) = (&mut client, (cl.device)()) {
        m.extend(d);
    }
    if let Some(v) = &vd {
        client["visitorData"] = json!(v);
    }
    let body = json!({ "context": { "client": client }, "videoId": id, "contentCheckOk": true, "racyCheckOk": true });
    let mut req = c
        .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
        .header("User-Agent", cl.ua)
        .header("X-Youtube-Client-Name", cl.id)
        .header("X-Youtube-Client-Version", cl.version)
        .header("Origin", "https://www.youtube.com");
    if let Some(v) = &vd {
        req = req.header("X-Goog-Visitor-Id", v);
    }
    let r = req.json(&body).send().await?;
    if !r.status().is_success() {
        return Err(anyhow!(crate::tr!("YouTube a répondu {}", "YouTube answered {}", r.status())));
    }
    Ok(r.json().await?)
}

// ---------- lecture des réponses ----------

fn text(v: &Value) -> String {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return s.trim().to_string();
    }
    if let Some(s) = v.get("content").and_then(Value::as_str) {
        return s.trim().to_string();
    }
    if let Some(runs) = v.get("runs").and_then(Value::as_array) {
        return runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect::<String>().trim().to_string();
    }
    v.as_str().map(|s| s.trim().to_string()).unwrap_or_default()
}

fn at<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
    let mut cur = v;
    for p in path {
        cur = match p.parse::<usize>() {
            Ok(i) => cur.get(i).unwrap_or(&Value::Null),
            Err(_) => cur.get(*p).unwrap_or(&Value::Null),
        };
    }
    cur
}

/// « 1:02:03 », « 12:04 » en secondes ; 0 si ce n'est pas une durée.
fn clock(s: &str) -> f64 {
    let parts: Vec<&str> = s.trim().split([':', '.']).collect();
    if parts.len() < 2 || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return 0.0;
    }
    parts.iter().fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0))
}

/// Nombre de vues d'un texte : « 39.313 visualizzazioni » ; « 1,2 Mio. Aufrufe » (approché).
fn count_of(s: &str) -> i64 {
    let s = s.trim();
    let first = s.find(|c: char| c.is_ascii_digit());
    let Some(start) = first else { return 0 };
    let rest = &s[start..];
    let num_end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == '\u{a0}' || c == '\u{202f}' || c == ' ' || c == '\'')).unwrap_or(rest.len());
    let num = rest[..num_end].trim();
    let unit = rest[num_end..].trim_start().to_lowercase();
    let mult = if ["k", "mil", "tsd", "тыс", "tis", "ezer", "bin", "rb", "천", "万", "만"].iter().any(|u| unit.starts_with(u)) {
        if unit.starts_with('万') || unit.starts_with("만") {
            1e4
        } else {
            1e3
        }
    } else if ["m", "mln", "mio", "млн", "mil."].iter().any(|u| unit.starts_with(u)) && !unit.starts_with("mila") {
        1e6
    } else if ["b", "md", "mrd", "млрд", "億", "억"].iter().any(|u| unit.starts_with(u)) {
        if unit.starts_with('億') || unit.starts_with("억") {
            1e8
        } else {
            1e9
        }
    } else {
        1.0
    };
    if mult == 1.0 {
        return num.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
    }
    // abrégé : « 1,2 » ou « 1.2 » est un nombre décimal
    let dec: String = num.chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}' && *c != '\u{202f}').map(|c| if c == ',' { '.' } else { c }).collect();
    (dec.parse::<f64>().unwrap_or(0.0) * mult) as i64
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// « 3 days ago », « Streamed 2 weeks ago » (anglais seulement, comme yt-dlp) : date approchée.
fn ago(s: &str) -> Option<i64> {
    let s = s.to_lowercase();
    let words: Vec<&str> = s.split_whitespace().collect();
    let i = words.iter().position(|w| w.bytes().all(|b| b.is_ascii_digit()))?;
    let n: i64 = words[i].parse().ok()?;
    let unit = words.get(i + 1)?;
    let secs = match unit.trim_end_matches('s') {
        "second" => 1,
        "minute" => 60,
        "hour" => 3600,
        "day" => 86_400,
        "week" => 7 * 86_400,
        "month" => 30 * 86_400,
        "year" => 365 * 86_400,
        _ => return None,
    };
    Some(now() - n * secs)
}

fn thumbs(list: &Value) -> Value {
    Value::Array(
        list.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        let url = t.get("url")?.as_str()?;
                        let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_string() };
                        Some(json!({ "url": url, "width": t.get("width").cloned().unwrap_or(Value::Null), "height": t.get("height").cloned().unwrap_or(Value::Null) }))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// Une vidéo d'une liste, sous la forme d'une entrée de `--flat-playlist`.
fn entry(id: &str, title: String, duration: f64, channel: String, views: i64, published: &str, description: String, thumbnails: Value, live: bool, approx: bool) -> Value {
    let mut e = json!({
        "_type": "url",
        "ie_key": "Youtube",
        "id": id,
        "url": format!("https://www.youtube.com/watch?v={id}"),
        "title": title,
        "duration": if duration > 0.0 { json!(duration) } else { Value::Null },
        "channel": channel,
        "uploader": channel,
        "view_count": if views > 0 { json!(views) } else { Value::Null },
        "description": if description.is_empty() { Value::Null } else { json!(description) },
        "thumbnails": thumbnails,
        "live_status": if live { json!("is_live") } else { Value::Null },
    });
    if approx {
        if let Some(ts) = ago(published) {
            e["timestamp"] = json!(ts);
        }
    }
    e
}

/// `videoRenderer` (recherche, anciennes grilles) et ses cousins.
fn from_renderer(r: &Value, approx: bool) -> Option<Value> {
    let id = r.get("videoId")?.as_str()?;
    if !video_id_ok(id) {
        return None;
    }
    let title = text(&r["title"]);
    let mut length = text(&r["lengthText"]);
    if length.is_empty() {
        if let Some(o) = r["thumbnailOverlays"].as_array() {
            length = o.iter().map(|x| text(&x["thumbnailOverlayTimeStatusRenderer"]["text"])).find(|s| clock(s) > 0.0).unwrap_or_default();
        }
    }
    let live = r["badges"].as_array().is_some_and(|b| b.iter().any(|x| at(x, &["metadataBadgeRenderer", "style"]).as_str() == Some("BADGE_STYLE_TYPE_LIVE_NOW")))
        || r.get("upcomingEventData").is_some();
    let channel = [&r["ownerText"], &r["longBylineText"], &r["shortBylineText"]].iter().map(|v| text(v)).find(|s| !s.is_empty()).unwrap_or_default();
    let description = r["detailedMetadataSnippets"]
        .get(0)
        .map(|s| text(&s["snippetText"]))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| text(&r["descriptionSnippet"]));
    let duration = if let Some(secs) = r["lengthSeconds"].as_str().and_then(|s| s.parse::<f64>().ok()) { secs } else { clock(&length) };
    Some(entry(id, title, duration, channel, count_of(&text(&r["viewCountText"])), &text(&r["publishedTimeText"]), description, thumbs(&r["thumbnail"]["thumbnails"]), live, approx))
}

/// Badges posés sur une miniature (`thumbnailBadgeViewModel`) : durée, « EN DIRECT ».
fn badges(v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) => {
            if let Some(b) = m.get("thumbnailBadgeViewModel") {
                out.push((b["text"].as_str().unwrap_or("").to_string(), b["badgeStyle"].as_str().unwrap_or("").to_string()));
            }
            for x in m.values() {
                badges(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| badges(x, out)),
        _ => {}
    }
}

/// `lockupViewModel` : la forme récente des chaînes et des playlists.
fn from_lockup(l: &Value, approx: bool, channel: &str) -> Option<Value> {
    if l["contentType"].as_str() != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let id = l["contentId"].as_str()?;
    if !video_id_ok(id) {
        return None;
    }
    let md = &l["metadata"]["lockupMetadataViewModel"];
    let title = text(&md["title"]);
    let mut b = Vec::new();
    badges(&l["contentImage"], &mut b);
    let duration = b.iter().map(|(t, _)| clock(t)).find(|d| *d > 0.0).unwrap_or(0.0);
    let live = b.iter().any(|(_, s)| s.contains("LIVE"));
    // lignes : la chaîne (dans une playlist), puis « vues • date »
    let rows: Vec<Vec<String>> = md["metadata"]["contentMetadataViewModel"]["metadataRows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| r["metadataParts"].as_array().map(|p| p.iter().map(|x| text(&x["text"])).collect()).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    let (by, stats) = match rows.len() {
        0 => (String::new(), Vec::new()),
        1 => (String::new(), rows[0].clone()),
        _ => (rows[0].first().cloned().unwrap_or_default(), rows[rows.len() - 1].clone()),
    };
    let views = stats.first().map(|s| count_of(s)).unwrap_or(0);
    let published = stats.get(1).cloned().unwrap_or_default();
    let channel = if by.is_empty() { channel.to_string() } else { by };
    let thumbnails = thumbs(&l["contentImage"]["thumbnailViewModel"]["image"]["sources"]);
    Some(entry(id, title, duration, channel, views, &published, String::new(), thumbnails, live, approx))
}

/// Les vidéos d'une réponse, dans l'ordre, et le jeton de la page suivante.
fn collect(v: &Value, approx: bool, channel: &str, out: &mut Vec<Value>, token: &mut Option<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match k.as_str() {
                    "videoRenderer" | "gridVideoRenderer" | "compactVideoRenderer" | "playlistVideoRenderer" => {
                        if x.get("isPlayable").and_then(Value::as_bool) != Some(false) {
                            out.extend(from_renderer(x, approx));
                        }
                    }
                    "lockupViewModel" => out.extend(from_lockup(x, approx, channel)),
                    // pas de Shorts ni d'annonces
                    "reelShelfRenderer" | "shortsLockupViewModel" | "reelItemRenderer" | "adSlotRenderer" | "promotedSparklesWebRenderer" => {}
                    "continuationItemRenderer" => {
                        let tk = [at(x, &["continuationEndpoint", "continuationCommand", "token"]), at(x, &["button", "buttonRenderer", "command", "continuationCommand", "token"])]
                            .into_iter()
                            .find_map(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| {
                                at(x, &["continuationEndpoint", "commandExecutorCommand", "commands"])
                                    .as_array()
                                    .and_then(|cs| cs.iter().find_map(|c| at(c, &["continuationCommand", "token"]).as_str().map(str::to_string)))
                            });
                        if tk.is_some() {
                            *token = tk;
                        }
                    }
                    _ => collect(x, approx, channel, out, token),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, approx, channel, out, token)),
        _ => {}
    }
}

/// Parcourt une liste (recherche ou page) jusqu'à `want` vidéos, page après page.
async fn walk_list(c: &reqwest::Client, endpoint: &str, first: Value, hl: &str, want: usize, approx: bool, channel: &str) -> Result<(Value, Vec<Value>)> {
    let head = web(c, endpoint, first, hl).await?;
    let mut out = Vec::new();
    let mut token = None;
    collect(&head, approx, channel, &mut out, &mut token);
    let mut pages = 0;
    while out.len() < want && pages < 12 {
        let Some(tk) = token.take() else { break };
        pages += 1;
        let more = web(c, endpoint, json!({ "continuation": tk }), hl).await?;
        let before = out.len();
        collect(&more, approx, channel, &mut out, &mut token);
        if out.len() == before && token.is_none() {
            break;
        }
    }
    // une même vidéo peut revenir d'une page à l'autre
    let mut seen = std::collections::HashSet::new();
    out.retain(|e| seen.insert(e["id"].as_str().unwrap_or("").to_string()));
    Ok((head, out))
}

fn slice(entries: Vec<Value>, items: Option<(usize, usize)>) -> Vec<Value> {
    let (from, to) = items.unwrap_or((1, usize::MAX));
    entries.into_iter().skip(from.saturating_sub(1)).take(to.saturating_sub(from.saturating_sub(1))).collect()
}

/// Identifiant `UC…` d'une chaîne nommée par son adresse (`@pseudo`, `/c/`, `/user/`).
async fn resolve_channel(c: &reqwest::Client, url: &str) -> Result<String> {
    let base = url.split("/videos").next().unwrap_or(url);
    let v = web(c, "navigation/resolve_url", json!({ "url": base }), "en").await?;
    at(&v, &["endpoint", "browseEndpoint", "browseId"])
        .as_str()
        .filter(|id| id.starts_with("UC"))
        .map(str::to_string)
        .ok_or_else(|| anyhow!(t("Lumen ne trouve pas cette chaîne YouTube.", "Lumen can't find this YouTube channel.")))
}

/// Une liste, comme `yt-dlp -J --flat-playlist`.
async fn listing(target: Target, a: &Args) -> Result<Value> {
    let c = client()?;
    let approx = a.approx_dates;
    // dates approchées : yt-dlp ne les lit qu'en anglais ; sinon la langue demandée
    let hl = if approx { "en".to_string() } else { a.lang.clone().unwrap_or_else(|| "en".into()) };
    let want = a.items.map_or(30, |(_, to)| to.min(300));
    match target {
        Target::Search { query, sp, limit } => {
            let mut body = json!({ "query": query });
            body["params"] = json!(sp.unwrap_or_else(|| "EgIQAQ==".into()));
            let want = limit.unwrap_or(want);
            let (_, entries) = walk_list(&c, "search", body, &hl, want, approx, "").await?;
            let entries = match limit {
                Some(n) => entries.into_iter().take(n).collect(),
                None => slice(entries, a.items),
            };
            Ok(json!({ "_type": "playlist", "id": query, "title": query, "entries": entries }))
        }
        Target::Channel { id, url, .. } => {
            let id = match id.filter(|i| i.starts_with("UC")) {
                Some(i) => i,
                None => resolve_channel(&c, &url).await?,
            };
            let first = json!({ "browseId": id, "params": VIDEOS_TAB });
            let head = web(&c, "browse", first.clone(), &hl).await?;
            let meta = &head["metadata"]["channelMetadataRenderer"];
            let name = text(&meta["title"]);
            let mut out = Vec::new();
            let mut token = None;
            collect(&head, approx, &name, &mut out, &mut token);
            let mut pages = 0;
            while out.len() < want && pages < 12 {
                let Some(tk) = token.take() else { break };
                pages += 1;
                let more = web(&c, "browse", json!({ "continuation": tk }), &hl).await?;
                collect(&more, approx, &name, &mut out, &mut token);
            }
            Ok(json!({
                "_type": "playlist",
                "id": id,
                "title": name,
                "channel": name,
                "uploader": name,
                "channel_id": id,
                "webpage_url": format!("https://www.youtube.com/channel/{id}/videos"),
                "thumbnails": thumbs(&meta["avatar"]["thumbnails"]),
                "description": text(&meta["description"]),
                "entries": slice(out, a.items),
            }))
        }
        Target::Playlist(list) => {
            let (head, entries) = walk_list(&c, "browse", json!({ "browseId": format!("VL{list}") }), &hl, want, approx, "").await?;
            let title = [text(&head["metadata"]["playlistMetadataRenderer"]["title"]), text(&at(&head, &["header", "pageHeaderRenderer", "pageTitle"]))]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            Ok(json!({
                "_type": "playlist",
                "id": list,
                "title": title,
                "webpage_url": format!("https://www.youtube.com/playlist?list={list}"),
                "entries": slice(entries, a.items),
            }))
        }
        Target::Video(id) => info(&c, &id, &hl).await.map(|(v, _)| v),
        Target::Dailymotion(id) => dailymotion(&c, &id).await,
    }
}

// ---------- une vidéo ----------

/// Un fichier proposé par YouTube, sous la forme d'un format de yt-dlp.
fn format_of(f: &Value) -> Option<Value> {
    let url = f["url"].as_str()?;
    let itag = f["itag"].as_i64()?;
    let mime = f["mimeType"].as_str().unwrap_or("");
    let (kind, rest) = mime.split_once('/').unwrap_or(("", ""));
    let container = rest.split(';').next().unwrap_or("");
    let codecs: Vec<String> = mime
        .split("codecs=\"")
        .nth(1)
        .map(|c| c.trim_end_matches('"').split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();
    // pistes doublées ou au volume compressé : seulement la piste d'origine
    if f["audioTrack"].is_object() && f["audioTrack"]["audioIsDefault"].as_bool() == Some(false) {
        return None;
    }
    if f["isDrc"].as_bool() == Some(true) {
        return None;
    }
    let (vcodec, acodec) = match (kind, codecs.len()) {
        ("audio", _) => ("none".to_string(), codecs.first().cloned().unwrap_or_default()),
        ("video", 2) => (codecs[0].clone(), codecs[1].clone()),
        ("video", _) => (codecs.first().cloned().unwrap_or_default(), "none".to_string()),
        _ => return None,
    };
    let ext = match (kind, container) {
        ("audio", "mp4") => "m4a",
        (_, "webm") => "webm",
        _ => "mp4",
    };
    let size = f["contentLength"].as_str().and_then(|s| s.parse::<u64>().ok());
    Some(json!({
        "format_id": itag.to_string(),
        "url": url,
        "ext": ext,
        "vcodec": vcodec,
        "acodec": acodec,
        "width": f["width"].clone(),
        "height": f["height"].clone(),
        "fps": f["fps"].clone(),
        "tbr": f["bitrate"].as_f64().map(|b| b / 1000.0),
        "filesize": size,
        "protocol": "https",
        "language": f["audioTrack"]["id"].as_str().and_then(|l| l.split('.').next()).unwrap_or(""),
    }))
}

/// Informations et fichiers d'une vidéo (client visionOS, puis iOS et Android VR). Renvoie aussi
/// l'agent du client, que les téléchargements doivent reprendre.
async fn info(c: &reqwest::Client, id: &str, hl: &str) -> Result<(Value, &'static str)> {
    let mut last_err = None;
    for cl in &PLAYERS {
        let v = match player(c, cl, id, hl).await {
            Ok(v) => v,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let status = v["playabilityStatus"]["status"].as_str().unwrap_or("");
        let sd = &v["streamingData"];
        let mut formats: Vec<Value> = sd["formats"].as_array().into_iter().chain(sd["adaptiveFormats"].as_array()).flatten().filter_map(format_of).collect();
        if let Some(hls) = sd["hlsManifestUrl"].as_str() {
            formats.push(json!({ "format_id": "hls", "url": hls, "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "protocol": "m3u8_native" }));
        }
        if status != "OK" || formats.is_empty() {
            let reason = text(&v["playabilityStatus"]["reason"]);
            last_err = Some(if reason.is_empty() { anyhow!(t("Cette vidéo n'est pas disponible.", "This video isn't available.")) } else { anyhow!("{reason}") });
            continue;
        }
        let d = &v["videoDetails"];
        let language = formats.iter().filter_map(|f| f["language"].as_str()).find(|l| !l.is_empty()).unwrap_or("").to_string();
        let live = d["isLive"].as_bool() == Some(true) || d["isUpcoming"].as_bool() == Some(true);
        let page = format!("https://www.youtube.com/watch?v={id}");
        return Ok((
            json!({
                "_type": "video",
                "id": id,
                "title": d["title"].as_str().unwrap_or(""),
                "description": d["shortDescription"].as_str().unwrap_or(""),
                "channel": d["author"].as_str().unwrap_or(""),
                "uploader": d["author"].as_str().unwrap_or(""),
                "channel_id": d["channelId"].as_str().unwrap_or(""),
                "duration": d["lengthSeconds"].as_str().and_then(|s| s.parse::<f64>().ok()),
                "view_count": d["viewCount"].as_str().and_then(|s| s.parse::<i64>().ok()),
                "thumbnails": thumbs(&d["thumbnail"]["thumbnails"]),
                "thumbnail": format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
                "webpage_url": page,
                "original_url": page,
                "extractor_key": "Youtube",
                "live_status": if live { "is_live" } else { "not_live" },
                "language": language,
                "formats": formats,
            }),
            cl.ua,
        ));
    }
    Err(last_err.unwrap_or_else(|| anyhow!(t("Cette vidéo n'est pas disponible.", "This video isn't available."))))
}

/// Ce que demande la sélection de yt-dlp (`-f`, `-S`), réduit à ce qu'emploie Lumen.
#[derive(Debug, PartialEq)]
enum Want {
    /// le son seul, de préférence en AAC (m4a)
    Audio,
    /// l'aperçu : un flux lisible par la fenêtre (HLS, ou fichier qui porte le son), sinon image et son séparés
    Stream,
    /// l'image seule d'une leçon : codecs acceptés, côté court au plus
    Video { codecs: Vec<&'static str>, max_side: i64 },
}

fn want_of(format: &str, sort: &str) -> Want {
    if format.starts_with("ba") {
        return Want::Audio;
    }
    if format.contains("+ba") {
        return Want::Stream;
    }
    let mut codecs = vec!["avc1"];
    if format.contains("vp0?9") || format.contains("vp9") {
        codecs.push("vp9");
    }
    if format.contains("av01") {
        codecs.push("av01");
    }
    let max_side = sort.split(',').find_map(|p| p.strip_prefix("res:")).and_then(|n| n.parse().ok()).unwrap_or(1080);
    Want::Video { codecs, max_side }
}

fn num(f: &Value, k: &str) -> i64 {
    f[k].as_i64().or_else(|| f[k].as_f64().map(|x| x as i64)).unwrap_or(0)
}

fn codec_family(vcodec: &str) -> &'static str {
    if vcodec.starts_with("avc1") {
        "avc1"
    } else if vcodec.starts_with("vp9") || vcodec.starts_with("vp09") {
        "vp9"
    } else if vcodec.starts_with("av01") {
        "av01"
    } else {
        ""
    }
}

/// Les fichiers choisis : un seul, ou l'image puis le son.
fn pick(formats: &[Value], want: &Want) -> Option<Vec<Value>> {
    let audio = |f: &&Value| f["vcodec"] == "none" && f["protocol"] == "https";
    let best_audio = || {
        formats
            .iter()
            .filter(audio)
            .max_by_key(|f| (f["ext"] == "m4a", num(f, "tbr")))
            .cloned()
    };
    match want {
        Want::Audio => best_audio().or_else(|| formats.iter().find(|f| f["acodec"] != "none" && f["vcodec"] != "none" && f["protocol"] == "https").cloned()).map(|f| vec![f]),
        Want::Stream => {
            if let Some(h) = formats.iter().find(|f| f["protocol"] == "m3u8_native") {
                return Some(vec![h.clone()]);
            }
            // un fichier qui porte image et son (360p) ; sinon image H.264 de 720p au plus et son à part
            if let Some(m) = formats.iter().filter(|f| f["acodec"] != "none" && f["vcodec"] != "none" && f["protocol"] == "https").max_by_key(|f| num(f, "height")) {
                return Some(vec![m.clone()]);
            }
            let v = formats
                .iter()
                .filter(|f| f["acodec"] == "none" && codec_family(f["vcodec"].as_str().unwrap_or("")) == "avc1" && num(f, "height") <= 720)
                .max_by_key(|f| num(f, "height"))?;
            Some(vec![v.clone(), best_audio()?])
        }
        Want::Video { codecs, max_side } => {
            let side = |f: &Value| num(f, "width").min(num(f, "height"));
            let video_only = |f: &&Value| f["acodec"] == "none" && f["vcodec"] != "none" && f["protocol"] == "https";
            let ok: Vec<&Value> = formats.iter().filter(video_only).filter(|f| codecs.contains(&codec_family(f["vcodec"].as_str().unwrap_or("")))).collect();
            let pool: Vec<&Value> = if ok.is_empty() { formats.iter().filter(video_only).collect() } else { ok };
            // la meilleure définition jusqu'à `max_side`, puis le fichier le plus léger, 30 images/s de préférence
            let fit: Vec<&&Value> = pool.iter().filter(|f| side(f) <= *max_side).collect();
            let best_side = fit.iter().map(|f| side(f)).max()?;
            // taille inconnue : après celles qu'on connaît
            fit.into_iter()
                .filter(|f| side(f) == best_side)
                .min_by_key(|f| (num(f, "fps") > 30, if num(f, "filesize") > 0 { num(f, "filesize") } else { i64::MAX }))
                .map(|f| vec![(*f).clone()])
        }
    }
}

/// La réponse de `yt-dlp -J -f …` : l'information et le ou les fichiers choisis.
fn chosen(mut v: Value, picked: &[Value]) -> Value {
    let m = v.as_object_mut().expect("objet");
    if picked.len() == 1 {
        for k in ["format_id", "url", "ext", "vcodec", "acodec", "width", "height", "fps", "filesize", "protocol"] {
            m.insert(k.into(), picked[0][k].clone());
        }
    } else {
        m.insert("requested_formats".into(), Value::Array(picked.to_vec()));
        m.insert("vcodec".into(), picked[0]["vcodec"].clone());
        m.insert("acodec".into(), picked[1]["acodec"].clone());
        m.insert("width".into(), picked[0]["width"].clone());
        m.insert("height".into(), picked[0]["height"].clone());
        let size = num(&picked[0], "filesize") + num(&picked[1], "filesize");
        m.insert("filesize".into(), if size > 0 { json!(size) } else { Value::Null });
    }
    v
}

// ---------- Dailymotion (aperçu seulement) ----------

async fn dailymotion(c: &reqwest::Client, id: &str) -> Result<Value> {
    let v: Value = c.get(format!("https://www.dailymotion.com/player/metadata/video/{id}")).header("User-Agent", WEB_UA).send().await?.json().await?;
    if let Some(e) = v.get("error") {
        return Err(anyhow!("{}", text(&e["title"]).trim()));
    }
    let hls = at(&v, &["qualities", "auto", "0", "url"]).as_str().ok_or_else(|| anyhow!(t("Cette vidéo n'est pas disponible.", "This video isn't available.")))?;
    let page = format!("https://www.dailymotion.com/video/{id}");
    Ok(json!({
        "_type": "video",
        "id": id,
        "title": v["title"].as_str().unwrap_or(""),
        "description": v["description"].as_str().unwrap_or(""),
        "channel": at(&v, &["owner", "screenname"]).as_str().unwrap_or(""),
        "uploader": at(&v, &["owner", "screenname"]).as_str().unwrap_or(""),
        "duration": v["duration"].as_f64(),
        "timestamp": v["created_time"].as_i64(),
        "thumbnails": [{ "url": at(&v, &["posters", "720"]).as_str().unwrap_or("") }],
        "webpage_url": page,
        "extractor_key": "Dailymotion",
        "language": v["language"].as_str().unwrap_or(""),
        "formats": [{ "format_id": "hls", "url": hls, "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "protocol": "m3u8_native" }],
    }))
}

// ---------- téléchargement ----------

/// Télécharge `url` dans `dest`, par morceaux (paramètre `range` de YouTube), en
/// reprenant un morceau qui échoue. `Err` 403 : l'adresse a été refusée.
async fn fetch(c: &reqwest::Client, url: &str, ua: &str, size: u64, dest: &Path, on_progress: &mut (dyn FnMut(f64) + Send)) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut done: u64 = 0;
    loop {
        let end = if size > 0 { (done + CHUNK).min(size) - 1 } else { done + CHUNK - 1 };
        let sep = if url.contains('?') { '&' } else { '?' };
        let mut tries = 0;
        let bytes = loop {
            tries += 1;
            let r = c.get(format!("{url}{sep}range={done}-{end}")).header("User-Agent", ua).timeout(Duration::from_secs(120)).send().await;
            match r {
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) => break b,
                    Err(e) if tries < 3 => {
                        eprintln!("youtube : morceau interrompu ({e}), nouvel essai");
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                },
                Ok(r) if r.status().as_u16() == 403 => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(anyhow!("403"));
                }
                Ok(r) if tries < 3 => eprintln!("youtube : morceau refusé ({}), nouvel essai", r.status()),
                Ok(r) => return Err(anyhow!(crate::tr!("YouTube a répondu {}", "YouTube answered {}", r.status()))),
                Err(_) if tries < 3 => {}
                Err(e) => return Err(e.into()),
            }
            tokio::time::sleep(Duration::from_millis(600 * tries)).await;
        };
        if bytes.is_empty() {
            break;
        }
        file.write_all(&bytes).await?;
        done += bytes.len() as u64;
        if size > 0 {
            on_progress((done as f64 / size as f64 * 100.0).min(100.0));
            if done >= size {
                break;
            }
        } else if (bytes.len() as u64) < CHUNK {
            break;
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, dest).await?;
    on_progress(100.0);
    Ok(())
}

/// `yt-dlp -f … --no-simulate --print … -o modèle adresse` : télécharge le fichier choisi.
async fn download(a: &Args, on_progress: &mut (dyn FnMut(f64) + Send)) -> Result<Vec<String>> {
    let target = target_of(&a.url)?;
    let Target::Video(id) = target else {
        return Err(anyhow!(t(
            "Sur iPhone et iPad, Lumen télécharge seulement les vidéos de YouTube. Importez celle-ci depuis votre Mac : elle viendra par la sauvegarde.",
            "On iPhone and iPad, Lumen only downloads YouTube videos. Import this one on your Mac: it will come over with the backup."
        )));
    };
    let c = client()?;
    let want = want_of(&a.format, &a.sort);
    // une adresse refusée (403) : un autre client de l'API en donne de nouvelles
    let mut last = None;
    for attempt in 0..PLAYERS.len() {
        let (v, ua) = match info_from(&c, &id, attempt).await {
            Ok(x) => x,
            Err(e) => {
                last = Some(e);
                continue;
            }
        };
        let formats: Vec<Value> = v["formats"].as_array().cloned().unwrap_or_default();
        let Some(picked) = pick(&formats, &want).filter(|p| p.len() == 1) else {
            last = Some(anyhow!(t("Aucun fichier à télécharger pour cette vidéo.", "No file to download for this video.")));
            continue;
        };
        let f = &picked[0];
        let ext = f["ext"].as_str().unwrap_or("mp4");
        let dest = PathBuf::from(a.output.replace("%(ext)s", ext));
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        match fetch(&c, f["url"].as_str().unwrap_or(""), ua, num(f, "filesize").max(0) as u64, &dest, on_progress).await {
            Ok(()) => {
                let title = v["title"].as_str().unwrap_or("");
                let path = dest.display().to_string();
                return Ok(a
                    .prints
                    .iter()
                    .map(|p| p.strip_prefix("after_move:").unwrap_or(p).replace("%(title)s", title).replace("%(filepath)s", &path).replace("%(id)s", &id))
                    .collect());
            }
            Err(e) => last = Some(e),
        }
    }
    let e = last.unwrap_or_else(|| anyhow!(t("Téléchargement impossible.", "Download failed.")));
    if e.to_string() == "403" {
        return Err(anyhow!(t(
            "YouTube refuse ce téléchargement pour l'instant. Réessayez dans quelques minutes.",
            "YouTube is refusing this download for now. Try again in a few minutes."
        )));
    }
    Err(e)
}

/// Informations d'une vidéo par un client précis de la liste (`PLAYERS`), puis les suivants.
async fn info_from(c: &reqwest::Client, id: &str, start: usize) -> Result<(Value, &'static str)> {
    let mut last = None;
    for cl in PLAYERS.iter().skip(start) {
        match player(c, cl, id, "en").await {
            Ok(v) if v["playabilityStatus"]["status"] == "OK" => {
                let sd = &v["streamingData"];
                let formats: Vec<Value> = sd["formats"].as_array().into_iter().chain(sd["adaptiveFormats"].as_array()).flatten().filter_map(format_of).collect();
                if formats.is_empty() {
                    continue;
                }
                return Ok((json!({ "title": v["videoDetails"]["title"], "formats": formats }), cl.ua));
            }
            Ok(v) => last = Some(anyhow!("{}", text(&v["playabilityStatus"]["reason"]))),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!(t("Cette vidéo n'est pas disponible.", "This video isn't available."))))
}

// ---------- point d'entrée ----------

/// Répond à une ligne de commande de yt-dlp (voir `media::run_ytdlp`) : les lignes
/// qu'il aurait écrites (son JSON, ses `--print`).
pub async fn run(args: &[String], on_progress: &mut (dyn FnMut(f64) + Send)) -> Result<Vec<String>> {
    let a = parse_args(args);
    if a.download {
        return download(&a, on_progress).await;
    }
    let target = target_of(&a.url)?;
    let v = if a.flat || !a.json {
        listing(target, &a).await?
    } else {
        let c = client()?;
        let hl = a.lang.clone().unwrap_or_else(|| "en".into());
        let v = match target {
            Target::Video(id) => info(&c, &id, &hl).await?.0,
            Target::Dailymotion(id) => dailymotion(&c, &id).await?,
            other => listing(other, &a).await?,
        };
        if v["formats"].is_array() && !a.format.is_empty() {
            let formats: Vec<Value> = v["formats"].as_array().cloned().unwrap_or_default();
            let picked = pick(&formats, &want_of(&a.format, &a.sort)).ok_or_else(|| anyhow!(t("Aucun format lisible pour cette vidéo.", "No playable format for this video.")))?;
            chosen(v, &picked)
        } else {
            v
        }
    };
    Ok(vec![serde_json::to_string(&v)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn command_lines() {
        let a = parse_args(&s(&["-J", "--flat-playlist", "--playlist-items", "21:40", "--extractor-args", "youtube:lang=it", "https://www.youtube.com/results?search_query=cucina&sp=EgIQAQ%3D%3D"]));
        assert!(a.json && a.flat && !a.download);
        assert_eq!(a.items, Some((21, 40)));
        assert_eq!(a.lang.as_deref(), Some("it"));
        let a = parse_args(&s(&["-f", "ba[ext=m4a]/b", "--no-simulate", "--print", "TITLE:%(title)s", "--print", "after_move:FILE:%(filepath)s", "-o", "/m/x.audio.%(ext)s", "https://youtu.be/PyADfuVljko"]));
        assert!(a.download);
        assert_eq!(a.prints.len(), 2);
        assert_eq!(a.output, "/m/x.audio.%(ext)s");
        assert_eq!(target_of(&a.url).unwrap(), Target::Video("PyADfuVljko".into()));
        assert_eq!(
            target_of("https://www.youtube.com/results?search_query=cucina+italiana&sp=EgIQAQ%3D%3D").unwrap(),
            Target::Search { query: "cucina italiana".into(), sp: Some("EgIQAQ==".into()), limit: None }
        );
        assert_eq!(target_of("ytsearch3:Laura Pausini - La solitudine").unwrap(), Target::Search { query: "Laura Pausini - La solitudine".into(), sp: None, limit: Some(3) });
        assert!(matches!(target_of("https://www.youtube.com/channel/UCkQz9XgvKsE8V0CFpCXX6MQ/videos").unwrap(), Target::Channel { id: Some(_), .. }));
        assert!(matches!(target_of("https://www.youtube.com/@EasyItalian").unwrap(), Target::Channel { id: None, .. }));
        assert_eq!(target_of("https://www.youtube.com/playlist?list=PL4fGSI1pDJn5BPviUFX4a3IMnAgyknC68").unwrap(), Target::Playlist("PL4fGSI1pDJn5BPviUFX4a3IMnAgyknC68".into()));
        assert_eq!(target_of("https://www.dailymotion.com/video/x8abcd_titre").unwrap(), Target::Dailymotion("x8abcd".into()));
        assert!(target_of("https://vimeo.com/123").is_err());
    }

    #[test]
    fn counts_and_clocks() {
        assert_eq!(clock("12:04"), 724.0);
        assert_eq!(clock("1:02:03"), 3723.0);
        assert_eq!(clock("LIVE"), 0.0);
        assert_eq!(count_of("39.313 visualizzazioni"), 39_313);
        assert_eq!(count_of("1,234,567 views"), 1_234_567);
        assert_eq!(count_of("1,2 Mio. Aufrufe"), 1_200_000);
        assert_eq!(count_of("12K views"), 12_000);
        assert_eq!(count_of("3,4 mila visualizzazioni"), 3_400);
        assert_eq!(count_of("Nessuna visualizzazione"), 0);
        assert!(ago("3 days ago").is_some_and(|t| (now() - t - 3 * 86_400).abs() < 5));
        assert!(ago("Streamed 2 weeks ago").is_some());
        assert_eq!(ago("2 giorni fa"), None);
    }

    #[test]
    fn renderers() {
        let v: Value = serde_json::from_str(
            r##"{"contents":[{"videoRenderer":{"videoId":"WcdtGjDobxc","title":{"runs":[{"text":"50 anni che ristrutturo cucine"}]},
                "longBylineText":{"runs":[{"text":"CLM Design"}]},"publishedTimeText":{"simpleText":"5 months ago"},
                "lengthText":{"simpleText":"12:04"},"viewCountText":{"simpleText":"39,313 views"},
                "thumbnail":{"thumbnails":[{"url":"https://i.ytimg.com/vi/WcdtGjDobxc/hq720.jpg","width":720,"height":404}]}}},
              {"lockupViewModel":{"contentId":"HCCDMYejB64","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
                "contentImage":{"thumbnailViewModel":{"image":{"sources":[{"url":"https://i.ytimg.com/vi/HCCDMYejB64/hq720.jpg","width":720,"height":404}]},
                  "overlays":[{"thumbnailBottomOverlayViewModel":{"badges":[{"thumbnailBadgeViewModel":{"text":"13:09","badgeStyle":"THUMBNAIL_OVERLAY_BADGE_STYLE_DEFAULT"}}]}}]}},
                "metadata":{"lockupMetadataViewModel":{"title":{"content":"#75 | A Strange Ice Cream Stand"},
                  "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"666 views"}},{"text":{"content":"2 days ago"}}]}]}}}}}},
              {"reelItemRenderer":{"videoId":"shortshort1"}},
              {"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"TOKEN"}}}}]}"##,
        )
        .unwrap();
        let mut out = Vec::new();
        let mut token = None;
        collect(&v, true, "Italiano sì", &mut out, &mut token);
        assert_eq!(token.as_deref(), Some("TOKEN"));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["duration"], 724.0);
        assert_eq!(out[0]["channel"], "CLM Design");
        assert_eq!(out[0]["view_count"], 39_313);
        assert!(out[0]["timestamp"].is_i64());
        assert_eq!(out[1]["id"], "HCCDMYejB64");
        assert_eq!(out[1]["duration"], 789.0);
        assert_eq!(out[1]["channel"], "Italiano sì");
        assert_eq!(out[1]["view_count"], 666);
        // la recherche de Lumen les lit telles quelles
        let hits = crate::search::youtube_hits(&json!({ "entries": out }), "it", false);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn choices() {
        let f = |itag: i64, mime: &str, w: i64, h: i64, size: u64| {
            format_of(&json!({ "itag": itag, "url": format!("https://g/{itag}"), "mimeType": mime, "width": w, "height": h, "contentLength": size.to_string(), "bitrate": 1000 })).unwrap()
        };
        let formats = vec![
            f(137, r#"video/mp4; codecs="avc1.640028""#, 1920, 1080, 170_000_000),
            f(248, r#"video/webm; codecs="vp09.00.40.08""#, 1920, 1080, 98_000_000),
            f(136, r#"video/mp4; codecs="avc1.4D401F""#, 1280, 720, 42_000_000),
            f(140, r#"audio/mp4; codecs="mp4a.40.2""#, 0, 0, 9_000_000),
            f(251, r#"audio/webm; codecs="opus""#, 0, 0, 8_000_000),
            f(18, r#"video/mp4; codecs="avc1.42001E, mp4a.40.2""#, 640, 360, 30_000_000),
        ];
        assert_eq!(pick(&formats, &Want::Audio).unwrap()[0]["format_id"], "140");
        // aperçu : le fichier qui porte image et son
        assert_eq!(pick(&formats, &Want::Stream).unwrap()[0]["format_id"], "18");
        // leçon : le VP9, plus léger, quand la fenêtre le lit ; sinon le H.264
        let (fmt, sort) = crate::media::video_format(&["vp9".into()], 1080);
        assert_eq!(pick(&formats, &want_of(&fmt, &sort)).unwrap()[0]["format_id"], "248");
        let (fmt, sort) = crate::media::video_format(&[], 720);
        assert_eq!(pick(&formats, &want_of(&fmt, &sort)).unwrap()[0]["format_id"], "136");
        // image et son séparés : `requested_formats`, comme yt-dlp
        let both = chosen(json!({ "title": "x" }), &[formats[2].clone(), formats[3].clone()]);
        let s = crate::search::stream_of_for_tests(&both);
        assert_eq!((s.0.as_str(), s.1.as_str()), ("https://g/136", "https://g/140"));
    }

    /// Recherche, chaîne, playlist, informations d'une vidéo et téléchargement du son,
    /// par les vraies adresses de YouTube (titres et nombres seulement) :
    /// `cargo test --lib ytnative_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn ytnative_live() {
        let mut quiet = |_p: f64| {};
        let started = std::time::Instant::now();
        let out = run(&s(&["-J", "--flat-playlist", "--playlist-items", "1:25", "--extractor-args", "youtube:lang=it", "https://www.youtube.com/results?search_query=cucina&sp=EgIQAQ%3D%3D"]), &mut quiet).await.unwrap();
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        let hits = crate::search::youtube_hits(&v, "it", false);
        println!("recherche : {} entrées, {} résultats pour Lumen ({:.1} s)", v["entries"].as_array().unwrap().len(), hits.len(), started.elapsed().as_secs_f64());
        for h in hits.iter().take(3) {
            println!("   · {} — {} · {} s · {} vues", h.title.chars().take(50).collect::<String>(), h.author, h.duration, h.count);
        }
        assert!(hits.len() >= 10);
        let out = run(&s(&["-J", "--flat-playlist", "--playlist-items", "1:36", "--extractor-args", "youtubetab:approximate_date", "https://www.youtube.com/channel/UCkQz9XgvKsE8V0CFpCXX6MQ/videos"]), &mut quiet).await.unwrap();
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        let e = v["entries"].as_array().unwrap();
        let dated = e.iter().filter(|x| x["timestamp"].is_i64()).count();
        let timed = e.iter().filter(|x| x["duration"].is_f64()).count();
        println!("chaîne « {} » : {} vidéos, {dated} datées, {timed} avec durée", v["title"], e.len());
        assert!(e.len() >= 30 && dated > 20 && timed > 20);
        let out = run(&s(&["-J", "--flat-playlist", "--playlist-items", "1:40", "https://www.youtube.com/playlist?list=PL4fGSI1pDJn5BPviUFX4a3IMnAgyknC68"]), &mut quiet).await.unwrap();
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        println!("playlist « {} » : {} vidéos", v["title"], v["entries"].as_array().unwrap().len());
        let out = run(&s(&["-J", "--flat-playlist", "--playlist-items", "1:300", "https://www.youtube.com/@EasyItalian"]), &mut quiet).await.unwrap();
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        println!("@EasyItalian : « {} », {} vidéos", v["title"], v["entries"].as_array().unwrap().len());
        // aperçu : un flux lisible
        let out = run(&s(&["-J", "-f", "bv*[vcodec^=avc1][height<=720][protocol^=m3u8]+ba[ext=m4a][protocol=https]/b", "https://www.youtube.com/watch?v=PyADfuVljko"]), &mut quiet).await.unwrap();
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        let s_ = crate::search::stream_of_for_tests(&v);
        println!("aperçu : « {} » · image {} · son {} · langue {:?}", v["title"], !s_.0.is_empty(), !s_.1.is_empty(), v["language"]);
        // le son d'une leçon
        let dir = std::env::temp_dir().join("lumen-ytnative-live");
        let _ = std::fs::create_dir_all(&dir);
        let started = std::time::Instant::now();
        let mut last = 0.0;
        let out = run(
            &s(&["-f", "ba[ext=m4a]/ba[acodec^=mp4a]/ba[ext=mp3]/b[ext=mp4]", "--no-simulate", "--print", "TITLE:%(title)s", "--print", "after_move:FILE:%(filepath)s", "-o", &dir.join("t.audio.%(ext)s").display().to_string(), "https://www.youtube.com/watch?v=PyADfuVljko"]),
            &mut |p| last = p,
        )
        .await
        .unwrap();
        let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).unwrap();
        let size = std::fs::metadata(file).unwrap().len();
        println!("son : {} octets en {:.1} s (progression {last} %), {}", size, started.elapsed().as_secs_f64(), out[0]);
        assert!(size > 1_000_000);
    }
}
