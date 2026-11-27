//! Linux : les médias de l'app (audio et vidéo des leçons) servis par un petit
//! serveur HTTP local. L'élément audio/vidéo de WebKitGTK refuse le protocole
//! des ressources de Tauri : il y attend des réponses aux requêtes de plage
//! (Range), que ce protocole ne donne pas — un son isolé peut passer par un
//! blob (voir pronounce.ts), pas une vidéo entière. Le serveur répond
//! lui-même aux plages, sur 127.0.0.1, sans rien d'autre que tokio.
//!
//! Le même serveur relaye les flux distants enregistrés (`/flux/<identifiant>`,
//! voir `relay`) : l'aperçu de Découvrir, lu par segments depuis l'interface
//! (streamer.rs), y demande des plages d'octets que Lumen va chercher lui-même.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Démarre le serveur sur un port libre de 127.0.0.1 et renvoie l'adresse de
/// base (`http://127.0.0.1:port/media`). Les fichiers ne sont lus que dans
/// `media_dir`.
pub fn start(media_dir: PathBuf) -> Option<String> {
    let listener = tauri::async_runtime::block_on(async { TcpListener::bind(("127.0.0.1", 0)).await }).ok()?;
    let addr = listener.local_addr().ok()?;
    let base = format!("http://{addr}/media");
    tauri::async_runtime::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let dir = media_dir.clone();
            tauri::async_runtime::spawn(async move {
                let _ = serve(stream, &dir).await;
            });
        }
    });
    Some(base)
}

/// Type d'un fichier par son extension (celles que Lumen crée ou importe).
fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "wav" => "audio/wav",
        "m4a" | "mp4" | "m4b" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "webm" => "video/webm",
        "mkv" | "mov" => "video/x-matroska",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Une requête lue : HEAD, OPTIONS ou non, le chemin, et l'en-tête Range s'il y en a un.
struct Ask {
    head: bool,
    options: bool,
    path: String,
    range: Option<String>,
}

/// Lit la requête ligne par ligne jusqu'à la ligne vide qui suit les en-têtes.
async fn read_line(reader: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<()> {
    buf.clear();
    loop {
        let mut byte = [0u8; 1];
        let n = reader.read(&mut byte).await?;
        if n == 0 {
            break;
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
        if buf.len() > 8192 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "en-tête trop long"));
        }
    }
    Ok(())
}

async fn read_ask(reader: &mut TcpStream) -> Option<Ask> {
    let mut line = Vec::new();
    read_line(reader, &mut line).await.ok()?;
    let first = String::from_utf8_lossy(&line);
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let raw_path = parts.next()?.to_string();
    let mut range = None;
    for _ in 0..64 {
        let mut header = Vec::new();
        if read_line(reader, &mut header).await.is_err() {
            return None;
        }
        if header == b"\r\n" || header == b"\n" || header.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(&header);
        if let Some((name, value)) = text.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }
    Some(Ask {
        head: method == "HEAD",
        options: method == "OPTIONS",
        path: raw_path,
        range,
    })
}
/// La plage demandée dans un en-tête Range (« bytes=début-fin », « bytes=début- »
/// ou « bytes=-fin »), en octets de début et de fin dans un fichier de `total`
/// octets. Plusieurs plages séparées par des virgules : la première seule
/// (l'élément média n'en demande jamais plusieurs).
pub(crate) fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    if b.is_empty() {
        let start = a.trim().parse().ok()?;
        Some((start, total.saturating_sub(1)))
    } else if a.is_empty() {
        let len: u64 = b.trim().parse().ok()?;
        let len = len.min(total);
        Some((total - len, total.saturating_sub(1)))
    } else {
        let start = a.trim().parse().ok()?;
        let end: u64 = b.trim().parse().ok()?;
        Some((start, end.min(total.saturating_sub(1))))
    }
}

/// Le chemin demandé, nettoyé de ce qui sortirait du dossier des médias
/// (segments vides, « .. »). `None` si tout le chemin est vide ou remonte.
pub(crate) fn safe_path(rel: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty() && *s != "..").collect();
    (!parts.is_empty()).then(|| parts.iter().collect())
}

async fn serve(mut stream: TcpStream, media_dir: &Path) -> std::io::Result<()> {
    let Some(ask) = read_ask(&mut stream).await else {
        return Ok(());
    };
    // `/flux/<identifiant>` : un flux distant que Lumen a enregistré (le relais)
    let path = ask.path.split('?').next().unwrap_or("");
    if let Some(id) = path.strip_prefix("/flux/") {
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return relay(stream, id, &ask).await;
        }
        return write_relay_head(&mut stream, 404, "Not Found", &[], ALLOW).await;
    }
    // `/media/<chemin dans le dossier des médias>` ; rien n'en sort
    let rel = ask.path.trim_start_matches("/media/").trim_start_matches('/');
    let file_path = match safe_path(rel) {
        Some(p) => media_dir.join(p),
        None => return write_head(&mut stream, 404, "Not Found", "text/plain", 0, None).await,
    };
    let meta = match tokio::fs::metadata(&file_path).await {
        Ok(m) if m.is_file() => m,
        _ => return write_head(&mut stream, 404, "Not Found", "text/plain", 0, None).await,
    };
    let total = meta.len();
    let mime = mime(&file_path);
    let range = ask.range.as_deref().and_then(|r| parse_range(r, total));

    let (status, reason, start, end) = match range {
        Some((a, b)) if a <= b && a < total => (206, "Partial Content", a, b),
        Some(_) => return write_head(&mut stream, 416, "Range Not Satisfiable", "text/plain", 0, None).await,
        None => (200, "OK", 0, total.saturating_sub(1)),
    };
    let length = if total == 0 { 0 } else { end - start + 1 };
    let content_range = (status == 206).then(|| format!("bytes {start}-{end}/{total}"));

    write_head(&mut stream, status, reason, mime, length, content_range.as_deref()).await?;
    if !ask.head && length > 0 {
        let mut file = tokio::fs::File::open(&file_path).await?;
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut remaining = length;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
    }
    stream.shutdown().await
}

/// L'autorisation de lecture d'une page : ce serveur n'a pas la même origine que
/// l'interface, qui ne pourrait pas lire ses réponses autrement.
const ALLOW: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length\r\n";

/// Les flux distants que le relais sert : identifiant → adresse.
static RELAY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Enregistre une adresse distante pour le relais et renvoie l'identifiant à donner
/// à l'interface (`/flux/<identifiant>`) : elle y demande des plages d'octets, que Lumen
/// va chercher à cette adresse. La même adresse garde le même identifiant ; le relais
/// ne sert qu'une adresse passée par ici, jamais une que la page donnerait.
pub(crate) fn relay_register(url: &str) -> String {
    let mut map = RELAY.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(id) = map.iter().find(|(_, known)| known.as_str() == url).map(|(id, _)| id.clone()) {
        return id;
    }
    // un hachage de l'adresse : court, illisible, et seul Lumen sait ce qu'il sert
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hash);
    let id = format!("{:016x}", hash.finish());
    map.insert(id.clone(), url.to_string());
    id
}

/// L'adresse enregistrée sous cet identifiant.
fn relay_url(id: &str) -> Option<String> {
    RELAY.get()?.lock().unwrap().get(id).cloned()
}

/// Le client HTTP du relais : un seul, pour que la connexion reste ouverte d'un segment au suivant.
pub(crate) fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = C.get() {
        return Ok(c.clone());
    }
    let c = reqwest::Client::builder()
        .user_agent("Lumen/0.1 (+language reader)")
        .connect_timeout(Duration::from_secs(20))
        // silence de trente secondes au milieu d'une réponse : abandon
        .read_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(120))
        .build()?;
    Ok(C.get_or_init(|| c).clone())
}

/// Le relais d'un flux distant. Un navigateur n'autorise une page à lire les données
/// d'un autre site que si ce site le permet, et YouTube ne le permet pas : l'interface
/// ne peut donc pas prendre elle-même les segments d'une vidéo. Elle les demande à
/// Lumen, qui n'est pas une page, et qui les lui rend avec l'autorisation de lecture.
async fn relay(mut stream: TcpStream, id: &str, ask: &Ask) -> std::io::Result<()> {
    let Some(url) = relay_url(id) else {
        return write_relay_head(&mut stream, 404, "Not Found", &[], ALLOW).await;
    };
    // la demande préalable qu'une page envoie avant de lire : tout est permis ici
    if ask.options {
        let allow = format!("{ALLOW}Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nAccess-Control-Allow-Headers: range\r\nAccess-Control-Max-Age: 3600\r\n");
        return write_relay_head(&mut stream, 204, "No Content", &[], &allow).await;
    }
    let Ok(client) = http_client() else {
        return write_relay_head(&mut stream, 502, "Bad Gateway", &[], ALLOW).await;
    };
    let request = if ask.head { client.head(&url) } else { client.get(&url) };
    // la plage demandée est transmise telle quelle : c'est l'en-tête Range de la page
    let request = match &ask.range {
        Some(range) => request.header(reqwest::header::RANGE, range),
        None => request,
    };
    let Ok(resp) = request.send().await else {
        return write_relay_head(&mut stream, 502, "Bad Gateway", &[], ALLOW).await;
    };
    let status = resp.status().as_u16();
    // ce que le site a répondu, sous les noms habituels de ces en-têtes
    let headers: Vec<(&str, String)> = [("Content-Type", "content-type"), ("Content-Length", "content-length"), ("Content-Range", "content-range")]
        .into_iter()
        .filter_map(|(name, header)| Some((name, resp.headers().get(header)?.to_str().ok()?.to_string())))
        .collect();
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        403 => "Forbidden",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        _ => "Response",
    };
    write_relay_head(&mut stream, status, reason, &headers, ALLOW).await?;
    if ask.head {
        return stream.shutdown().await;
    }
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        // coupé chez le site d'origine, ou la page a renoncé à cette demande (un saut en
        // remplace une autre) : la réponse s'arrête là
        let Ok(chunk) = chunk else { break };
        if stream.write_all(&chunk).await.is_err() {
            break;
        }
    }
    stream.shutdown().await
}

/// Écrit l'en-tête d'une réponse du relais.
async fn write_relay_head(stream: &mut TcpStream, status: u16, reason: &str, headers: &[(&str, String)], allow: &str) -> std::io::Result<()> {
    let mut text = format!("HTTP/1.1 {status} {reason}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n{allow}");
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    if status != 204 && !headers.iter().any(|(name, _)| *name == "Content-Length") {
        text.push_str("Content-Length: 0\r\n");
    }
    text.push_str("\r\n");
    stream.write_all(text.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn ranges_are_parsed() {
        // « bytes=début-fin » : la plage la plus commune
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-999", 1000), Some((500, 999)));
        // la fin demandée au-delà du fichier : ramenée à la fin du fichier
        assert_eq!(parse_range("bytes=0-9999", 1000), Some((0, 999)));
        // « bytes=début- » : de ce début à la fin
        assert_eq!(parse_range("bytes=100-", 1000), Some((100, 999)));
        // « bytes=-fin » : les N derniers octets
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        // plus que le fichier : tout le fichier
        assert_eq!(parse_range("bytes=-9999", 1000), Some((0, 999)));
        // plusieurs plages : refusées (le serveur n'en sert qu'une)
        assert_eq!(parse_range("bytes=0-10,20-30", 1000), None);
        // pas un en-tête Range de plages d'octets
        assert_eq!(parse_range("items=0-10", 1000), None);
        // nombres illisibles
        assert_eq!(parse_range("bytes=a-b", 1000), None);
        assert_eq!(parse_range("bytes=-", 1000), None);
    }

    #[test]
    fn paths_stay_inside_media_dir() {
        // un simple fichier du dossier
        assert_eq!(safe_path("20261005-120000.mp4"), Some(PathBuf::from("20261005-120000.mp4")));
        // un sous-dossier (le cache de la voix : media/voice/…)
        assert_eq!(safe_path("voice/abc123.wav"), Some(PathBuf::from("voice/abc123.wav")));
        // « .. » est retiré sans être résolu : le chemin reste dans le dossier
        // des médias (plus strict que la résolution Unix, qui pourrait remonter)
        assert_eq!(safe_path("../lumen.db"), Some(PathBuf::from("lumen.db")));
        assert_eq!(safe_path("voice/../../lumen.db"), Some(PathBuf::from("voice/lumen.db")));
        // un chemin tout en remontées : rien à servir
        assert_eq!(safe_path(".."), None);
        assert_eq!(safe_path("../.."), None);
        // segments vides (double barre oblique) : ignorés
        assert_eq!(safe_path("//a//b"), Some(PathBuf::from("a/b")));
        // un chemin vide : rien
        assert_eq!(safe_path(""), None);
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime(Path::new("son.wav")), "audio/wav");
        assert_eq!(mime(Path::new("leçon.m4a")), "audio/mp4");
        assert_eq!(mime(Path::new("clip.MP4")), "audio/mp4");
        assert_eq!(mime(Path::new("film.mkv")), "video/x-matroska");
        assert_eq!(mime(Path::new("image.JPG")), "image/jpeg");
        assert_eq!(mime(Path::new("couverture.webp")), "image/webp");
        assert_eq!(mime(Path::new("inconnu.xyz")), "application/octet-stream");
        assert_eq!(mime(Path::new("sans-extension")), "application/octet-stream");
    }

    /// Un faux site qui sert `contenu` à `/f`, plages comprises ; renvoie son adresse.
    async fn site(contenu: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let Some(ask) = read_ask(&mut sock).await else { continue };
                let total = contenu.len() as u64;
                let (status, reason, start, end) = match ask.range.as_deref().and_then(|r| parse_range(r, total)) {
                    Some((a, b)) if a <= b && a < total => (206, "Partial Content", a, b),
                    Some(_) => {
                        let _ = write_head(&mut sock, 416, "Range Not Satisfiable", "text/plain", 0, None).await;
                        continue;
                    }
                    None => (200, "OK", 0, total - 1),
                };
                let range = (status == 206).then(|| format!("bytes {start}-{end}/{total}"));
                let _ = write_head(&mut sock, status, reason, "video/mp4", end - start + 1, range.as_deref()).await;
                if !ask.head {
                    let _ = sock.write_all(&contenu[start as usize..=end as usize]).await;
                }
            }
        });
        base
    }

    /// Le relais du serveur, sur son propre socket : la fonction `relay` sert chaque demande.
    async fn relais() -> Arc<TcpListener> {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
        let écoute = Arc::clone(&listener);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = écoute.accept().await {
                let Some(ask) = read_ask(&mut sock).await else { continue };
                tokio::spawn(async move {
                    let _ = relay(sock, ask.path.trim_start_matches("/flux/"), &ask).await;
                });
            }
        });
        listener
    }

    /// Sert le vrai relais pour un essai dans le harnais (la page `sessions/probe/streamer-test.html`,
    /// servie par le serveur de développement) : les deux adresses données (`LUMEN_TEST_VIDEO`,
    /// `LUMEN_TEST_AUDIO`) sont enregistrées, leurs adresses relais écrites dans `LUMEN_PROBE_OUT`
    /// (une par ligne), et le serveur tourne `LUMEN_SERVE_SECONDS` (30 par défaut).
    #[test]
    #[ignore]
    fn relay_live() {
        let video = std::env::var("LUMEN_TEST_VIDEO").expect("LUMEN_TEST_VIDEO");
        let audio = std::env::var("LUMEN_TEST_AUDIO").expect("LUMEN_TEST_AUDIO");
        let out = std::env::var("LUMEN_PROBE_OUT").expect("LUMEN_PROBE_OUT");
        let data_dir = std::env::temp_dir().join(format!("lumen-relay-{}", std::process::id()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let base = start(data_dir.join("media")).expect("serveur");
        let origin = base.trim_end_matches("/media");
        let fluxes = [relay_register(&video), relay_register(&audio)].map(|id| format!("{origin}/flux/{id}"));
        std::fs::write(out, fluxes.join("\n")).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(std::env::var("LUMEN_SERVE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(30)));
    }

    /// Une demande HTTP envoyée au relais, et toute sa réponse.
    async fn demande(relais: &Arc<TcpListener>, texte: String) -> String {
        let mut client = TcpStream::connect(relais.local_addr().unwrap()).await.unwrap();
        client.write_all(texte.as_bytes()).await.unwrap();
        let mut réponse = Vec::new();
        client.read_to_end(&mut réponse).await.unwrap();
        String::from_utf8_lossy(&réponse).into_owned()
    }

    #[tokio::test]
    async fn relay_serves_what_lumen_registered() {
        let contenu = b"0123456789abcdefghij";
        let url = format!("{}/f", site(contenu).await);
        let id = relay_register(&url);
        // la même adresse garde le même identifiant ; une autre en a un autre
        assert_eq!(relay_register(&url), id);
        assert_ne!(relay_register(&format!("{}/autre", site(contenu).await)), id);
        let relais = relais().await;

        // une plage : transmise au site, servie avec l'autorisation de lecture d'une page
        let r = demande(&relais, format!("GET /flux/{id} HTTP/1.1\r\nHost: x\r\nRange: bytes=3-5\r\n\r\n")).await;
        assert!(r.starts_with("HTTP/1.1 206 Partial Content\r\n"), "{r}");
        assert!(r.contains("Content-Range: bytes 3-5/20\r\n"), "{r}");
        assert!(r.contains("Content-Length: 3\r\n"), "{r}");
        assert!(r.contains("Access-Control-Allow-Origin: *\r\n"), "{r}");
        assert!(r.ends_with("\r\n\r\n345"), "{r}");

        // sans plage : tout
        let r = demande(&relais, format!("GET /flux/{id} HTTP/1.1\r\nHost: x\r\n\r\n")).await;
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n"), "{r}");
        assert!(r.ends_with(String::from_utf8_lossy(contenu).as_ref()), "{r}");

        // HEAD : l'en-tête seul
        let r = demande(&relais, format!("HEAD /flux/{id} HTTP/1.1\r\nHost: x\r\nRange: bytes=3-5\r\n\r\n")).await;
        assert!(r.starts_with("HTTP/1.1 206 Partial Content\r\n") && r.ends_with("\r\n\r\n"), "{r}");

        // la demande préalable d'une page
        let r = demande(&relais, format!("OPTIONS /flux/{id} HTTP/1.1\r\nHost: x\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: range\r\n\r\n")).await;
        assert!(r.starts_with("HTTP/1.1 204 No Content\r\n"), "{r}");
        assert!(r.contains("Access-Control-Allow-Headers: range\r\n") && r.contains("Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n"), "{r}");

        // une adresse que Lumen n'a pas enregistrée : rien
        let r = demande(&relais, "GET /flux/pas-enregistre HTTP/1.1\r\nHost: x\r\n\r\n".to_string()).await;
        assert!(r.starts_with("HTTP/1.1 404 Not Found\r\n"), "{r}");

        // le site ne répond pas : dit à la page
        let id = relay_register("http://127.0.0.1:1/rien");
        let r = demande(&relais, format!("GET /flux/{id} HTTP/1.1\r\nHost: x\r\n\r\n")).await;
        assert!(r.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{r}");
    }
}

/// Écrit l'en-tête de la réponse.
async fn write_head(stream: &mut TcpStream, status: u16, reason: &str, mime: &str, length: u64, content_range: Option<&str>) -> std::io::Result<()> {
    let mut text = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n");
    if let Some(range) = content_range {
        text.push_str(&format!("Content-Range: {range}\r\n"));
    }
    text.push_str("\r\n");
    stream.write_all(text.as_bytes()).await
}
