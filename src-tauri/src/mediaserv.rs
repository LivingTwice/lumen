//! Linux : les médias de l'app (audio et vidéo des leçons) servis par un petit
//! serveur HTTP local. L'élément audio/vidéo de WebKitGTK refuse le protocole
//! des ressources de Tauri : il y attend des réponses aux requêtes de plage
//! (Range), que ce protocole ne donne pas — un son isolé peut passer par un
//! blob (voir pronounce.ts), pas une vidéo entière. Le serveur répond
//! lui-même aux plages, sur 127.0.0.1, sans rien d'autre que tokio.

use std::path::{Path, PathBuf};

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

/// Une requête lue : HEAD ou non, le chemin, et l'en-tête Range s'il y en a un.
struct Ask {
    head: bool,
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
        path: raw_path,
        range,
    })
}

async fn serve(mut stream: TcpStream, media_dir: &Path) -> std::io::Result<()> {
    let Some(ask) = read_ask(&mut stream).await else {
        return Ok(());
    };
    // `/media/<chemin dans le dossier des médias>` ; rien n'en sort
    let rel = ask.path.trim_start_matches("/media/").trim_start_matches('/');
    let file_path: PathBuf = rel.split('/').filter(|s| !s.is_empty() && *s != "..").collect();
    let file_path = media_dir.join(file_path);
    let meta = match tokio::fs::metadata(&file_path).await {
        Ok(m) if m.is_file() => m,
        _ => return write_head(&mut stream, 404, "Not Found", "text/plain", 0, None).await,
    };
    let total = meta.len();
    let mime = mime(&file_path);

    // la plage demandée : « bytes=début-fin », « bytes=début- » ou « bytes=-fin »
    let range: Option<(u64, u64)> = ask
        .range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .filter(|spec| !spec.contains(','))
        .and_then(|spec| spec.split_once('-'))
        .and_then(|(a, b)| {
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
        });

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

/// Écrit l'en-tête de la réponse.
async fn write_head(stream: &mut TcpStream, status: u16, reason: &str, mime: &str, length: u64, content_range: Option<&str>) -> std::io::Result<()> {
    let mut text = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n");
    if let Some(range) = content_range {
        text.push_str(&format!("Content-Range: {range}\r\n"));
    }
    text.push_str("\r\n");
    stream.write_all(text.as_bytes()).await
}
