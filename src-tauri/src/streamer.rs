//! Linux : l'aperçu de Découvrir lu par segments depuis l'interface, comme le lecteur de
//! youtube.com (`lib/streamer.ts`), au lieu d'être téléchargé par le moteur de WebKitGTK.
//!
//! Pourquoi : donné à l'élément vidéo, un fichier distant est téléchargé entier par le
//! moteur. Au démarrage d'un MP4 fragmenté, son tampon de téléchargement laisse parfois la
//! lecture en suspens jusqu'à la fin du fichier (défaut de GStreamer, `gstdownloadbuffer.c`,
//! présent de 1.28.7 à 1.28.9 au moins) : l'image ne vient pas une fois sur deux. Et le site
//! sert une grande réponse au débit d'environ deux fois celui du flux : rien d'avance.
//!
//! Ici, Lumen lit l'index du fichier (la boîte `sidx` qui suit l'en-tête : où commence
//! chaque segment et combien il dure) et le donne à l'interface, qui prend les segments au
//! fur et à mesure, par le relais du serveur local (`mediaserv::relay_register`). La page
//! ne peut pas les prendre elle-même : un navigateur ne l'autorise à lire les données d'un
//! autre site que si ce site le permet, et YouTube ne le permet pas.
//!
//! `LUMEN_PREVIEW_STREAMER` : posée à `1` par le paquet (`nix/lumen.nix`) et par le shell de
//! développement ; `0` (ou `false`) la coupe, et l'aperçu revient à la lecture directe.

use crate::mediaserv;
use crate::search::{Segment, Stream, Track};
use anyhow::{bail, Result};
use reqwest::header::{CONTENT_RANGE, RANGE};

/// La variable qui commande la lecture par segments.
const ENV: &str = "LUMEN_PREVIEW_STREAMER";

/// Le début d'un fichier : assez pour y trouver l'en-tête et l'index (l'en-tête d'un MP4
/// fragmenté est court : pas de table d'échantillons, elle est dans l'index).
const HEAD: u64 = 64 * 1024;

/// Ce que la variable demande. Vide, `0`, `false`, `non` ou `off` : la lecture directe.
fn wanted(value: &str) -> bool {
    !matches!(value.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no" | "off")
}

/// La lecture par segments est-elle demandée ?
pub(crate) fn enabled() -> bool {
    wanted(&std::env::var(ENV).unwrap_or_default())
}

/// L'aperçu lu par segments si le flux s'y prête et que la variable le demande ; tel quel
/// sinon (un son seul, un autre site, un fichier sans index : l'adresse directe, comme avant).
/// `media_base` : l'adresse du serveur local (`http://127.0.0.1:<port>/media`).
pub(crate) async fn decorate(stream: Stream, media_base: &str) -> Stream {
    let both = |s: &Stream| !s.video.is_empty() && !s.audio.is_empty() && !s.vcodec.is_empty() && !s.acodec.is_empty();
    if !enabled() || !both(&stream) {
        return stream;
    }
    let mut stream = stream;
    let Some(origin) = media_base.strip_suffix("/media").filter(|o| !o.is_empty()) else {
        return stream;
    };
    let Ok(client) = mediaserv::http_client() else {
        return stream;
    };
    // les deux à la fois : le survol d'une carte les demande avant l'ouverture de l'aperçu
    let (video, audio) = tokio::join!(
        read_track(&client, &stream.video, &stream.vcodec, origin),
        read_track(&client, &stream.audio, &stream.acodec, origin)
    );
    if let (Some(video), Some(audio)) = (video, audio) {
        stream.segments = Some(crate::search::Segments { video, audio });
    }
    stream
}

/// L'en-tête et la table des segments d'un fichier, son adresse passant par le relais.
async fn read_track(client: &reqwest::Client, url: &str, codec: &str, origin: &str) -> Option<Track> {
    let head = fetch(client, url, 0, HEAD - 1).await.ok()?;
    let (init_end, sidx_at) = layout(&head.bytes)?;
    // l'index tient dans ce qu'on a déjà, ou il se demande à part
    let index = match head.bytes.get(sidx_at.0 as usize..sidx_at.1 as usize) {
        Some(index) => index.to_vec(),
        None => fetch(client, url, sidx_at.0, sidx_at.1 - 1).await.ok()?.bytes,
    };
    let segments = index_of(&index, sidx_at.1, head.size)?;
    Some(Track {
        url: format!("{origin}/flux/{}", mediaserv::relay_register(url)),
        codec: codec.to_string(),
        init_end,
        segments,
    })
}

/// Une plage d'un fichier distant, et la taille du fichier.
struct Fetched {
    bytes: Vec<u8>,
    size: u64,
}

async fn fetch(client: &reqwest::Client, url: &str, start: u64, end: u64) -> Result<Fetched> {
    let resp = client.get(url).header(RANGE, format!("bytes={start}-{end}")).send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("{} {status}", crate::tr!("le site refuse une plage du fichier ({status}).", "the site refuses a range of the file ({status})."));
    }
    // 206 : la taille totale est en fin de Content-Range ; 200 : le fichier tenait dans la plage
    let size = resp
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|range| range.rsplit('/').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let bytes = resp.bytes().await?;
    let size = if size > 0 { size } else { bytes.len() as u64 };
    Ok(Fetched {
        bytes: bytes.to_vec(),
        size,
    })
}

/// Dans le début d'un fichier : la fin de l'en-tête (ftyp puis moov) et la place de l'index
/// (la boîte sidx, de son premier octet au premier qui n'en fait plus partie). `None` si
/// l'en-tête n'y est pas en entier ou si l'index n'y commence pas.
fn layout(head: &[u8]) -> Option<(u64, (u64, u64))> {
    let mut at = 0u64;
    let (mut moov, mut sidx) = (None, None);
    while at as usize + 8 <= head.len() {
        let from = at as usize;
        let size = u32::from_be_bytes(head[from..from + 4].try_into().ok()?) as u64;
        let kind = &head[from + 4..from + 8];
        // la taille sur 8 octets quand les 4 ne suffisent pas ; 0 (jusqu'à la fin) : hors de portée
        let (size, header) = if size == 1 {
            (u64::from_be_bytes(head.get(from + 8..from + 16)?.try_into().ok()?), 16)
        } else {
            (size, 8)
        };
        let end = at.checked_add(size).filter(|_| size >= header)?;
        match kind {
            b"moov" => moov = Some(end),
            b"sidx" => sidx = Some((at, end)),
            // les segments commencent : l'index devait être avant
            b"moof" | b"mdat" => break,
            _ => {}
        }
        at = end;
    }
    let moov_end = moov.filter(|end| *end <= head.len() as u64)?;
    Some((moov_end, sidx?))
}

/// La table des segments d'un index sidx : où chacun commence et combien il dure.
/// `after` : la place, dans le fichier, du premier octet qui suit la boîte (le premier
/// segment commence à la distance que dit l'index) ; `size` : la taille du fichier.
fn index_of(sidx: &[u8], after: u64, size: u64) -> Option<Vec<Segment>> {
    let be16 = |from: usize| Some(u16::from_be_bytes(sidx.get(from..from + 2)?.try_into().ok()?) as u64);
    let be32 = |from: usize| Some(u32::from_be_bytes(sidx.get(from..from + 4)?.try_into().ok()?) as u64);
    let be64 = |from: usize| Some(u64::from_be_bytes(sidx.get(from..from + 8)?.try_into().ok()?) as u64);
    // taille (4), type (4), version (1), drapeaux (3), identifiant de référence (4), échelle de temps (4)
    let version = *sidx.get(8)?;
    let timescale = be32(16)?;
    // le premier instant et la distance du premier segment : sur 8 octets (version 1), sinon sur 4 ;
    // puis 2 octets réservés, le nombre de références (2), et les références (12 octets chacune)
    let (count_at, first_time, first_offset) = if version == 1 { (38, be64(20)?, be64(28)?) } else { (30, be32(20)?, be32(24)?) };
    let n = be16(count_at)? as usize;
    let refs = count_at + 2;
    if timescale == 0 || refs + 12 * n > sidx.len() {
        return None;
    }
    let mut at = after.checked_add(first_offset)?;
    let mut time = first_time as f64 / timescale as f64;
    let mut segments = Vec::with_capacity(n);
    for i in 0..n {
        // la taille du segment sur 31 bits ; le bit de poids fort : un renvoi à un autre index
        let reference = be32(refs + 12 * i)?;
        let duration = be32(refs + 12 * i + 4)?;
        if reference & 0x8000_0000 != 0 {
            return None;
        }
        let end = at.checked_add(reference & 0x7fff_ffff)?;
        segments.push(Segment {
            start: at,
            end,
            time,
            duration: duration as f64 / timescale as f64,
        });
        at = end;
        time += duration as f64 / timescale as f64;
    }
    (at <= size).then_some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une boîte MP4 : taille, type, contenu.
    fn boîte(kind: &[u8], contenu: &[u8]) -> Vec<u8> {
        let mut b = ((contenu.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(contenu);
        b
    }

    /// Des morceaux bout à bout.
    fn coller(morceaux: &[&[u8]]) -> Vec<u8> {
        morceaux.iter().flat_map(|m| m.iter().copied()).collect()
    }

    /// Un index de `n` segments de `taille` octets, à l'échelle de temps 10 : chacun dure
    /// `durée` dixièmes de seconde ; le premier à l'instant `premier`, `offset` octets après la boîte.
    fn sidx(version: u8, n: u16, taille: u32, durée: u32, premier: u32, offset: u32) -> Vec<u8> {
        let mut corps = vec![version, 0, 0, 0];
        corps.extend_from_slice(&1u32.to_be_bytes());
        corps.extend_from_slice(&10u32.to_be_bytes());
        if version == 1 {
            corps.extend_from_slice(&(premier as u64).to_be_bytes());
            corps.extend_from_slice(&(offset as u64).to_be_bytes());
        } else {
            corps.extend_from_slice(&premier.to_be_bytes());
            corps.extend_from_slice(&offset.to_be_bytes());
        }
        corps.extend_from_slice(&[0, 0]);
        corps.extend_from_slice(&n.to_be_bytes());
        for _ in 0..n {
            corps.extend_from_slice(&taille.to_be_bytes());
            corps.extend_from_slice(&durée.to_be_bytes());
            corps.extend_from_slice(&0x8000_0000u32.to_be_bytes());
        }
        boîte(b"sidx", &corps)
    }

    #[test]
    fn the_variable_decides() {
        // posée par le paquet et par le shell de développement ; absente ou coupée : la lecture directe
        assert!(wanted("1"));
        assert!(wanted(" yes "));
        assert!(!wanted(""));
        assert!(!wanted("0"));
        assert!(!wanted("FALSE"));
        assert!(!wanted("off"));
    }

    #[test]
    fn a_fragmented_file_gives_its_header_and_index() {
        let ftyp = boîte(b"ftyp", &[0; 8]);
        let moov = boîte(b"moov", &[0; 20]);
        let index = sidx(0, 3, 100, 25, 0, 0);
        let mut head = coller(&[&ftyp, &moov, &index]);
        head.extend_from_slice(&boîte(b"moof", &[0; 30]));
        let header_end = (ftyp.len() + moov.len()) as u64;
        let (init_end, sidx_at) = layout(&head).unwrap();
        assert_eq!(init_end, header_end);
        assert_eq!(sidx_at, (header_end, header_end + index.len() as u64));
        // des boîtes entre l'en-tête et l'index, ou après lui : peu importe
        let free = boîte(b"free", &[0; 4]);
        let mut head = coller(&[&ftyp, &free, &moov, &index, &free, &boîte(b"moof", &[0; 8])]);
        let (init_end, sidx_at) = layout(&head).unwrap();
        assert_eq!(init_end, (ftyp.len() + free.len() + moov.len()) as u64);
        assert_eq!(sidx_at.0, init_end);
        // l'index commencé mais pas en entier (son en-tête seul) : sa place se trouve quand même
        head.truncate(sidx_at.0 as usize + 8);
        assert_eq!(layout(&head).unwrap().1, sidx_at);
        // pas d'index, ou pas d'en-tête : rien
        assert!(layout(&coller(&[&ftyp, &moov, &boîte(b"moof", &[0; 8])])).is_none());
        assert!(layout(&coller(&[&ftyp, &index, &boîte(b"moof", &[0; 8])])).is_none());
        // l'en-tête coupé : rien
        let mut cut = coller(&[&ftyp, &moov]);
        cut.truncate(ftyp.len() + 10);
        assert!(layout(&cut).is_none());
        // des octets au hasard : rien, sans paniquer
        assert!(layout(&[0u8; 40]).is_none());
        assert!(layout(&[255u8; 40]).is_none());
        assert!(layout(b"moovsidx").is_none());
    }

    #[test]
    fn the_index_gives_each_segment_its_place_and_time() {
        // trois segments de 100 octets et 2,5 s, juste après la boîte, qui finit à l'octet 500
        let index = sidx(0, 3, 100, 25, 0, 0);
        let segments = index_of(&index, 500, 800).unwrap();
        assert_eq!(
            segments,
            vec![
                Segment { start: 500, end: 600, time: 0.0, duration: 2.5 },
                Segment { start: 600, end: 700, time: 2.5, duration: 2.5 },
                Segment { start: 700, end: 800, time: 5.0, duration: 2.5 },
            ]
        );
        // la version 1 : mêmes champs sur 8 octets ; le premier segment loin après la boîte, et pas à zéro
        let index = sidx(1, 2, 100, 25, 50, 40);
        let segments = index_of(&index, 500, 1000).unwrap();
        assert_eq!(segments[0], Segment { start: 540, end: 640, time: 5.0, duration: 2.5 });
        assert_eq!(segments[1], Segment { start: 640, end: 740, time: 7.5, duration: 2.5 });
        // un renvoi à un autre index (bit de poids fort), un index coupé, un dernier segment
        // au-delà du fichier, une échelle de temps nulle : rien
        // la taille de la première référence est aux octets 32 à 36 : en-tête (8), version, drapeaux,
        // identifiant et échelle (12), premier instant et distance (8), réservé (2), nombre (2)
        let mut reference = sidx(0, 1, 100, 25, 0, 0);
        reference[32..36].copy_from_slice(&0x8000_0064u32.to_be_bytes());
        assert!(index_of(&reference, 500, 800).is_none());
        assert!(index_of(&sidx(0, 3, 100, 25, 0, 0)[..40], 500, 800).is_none());
        assert!(index_of(&sidx(0, 3, 100, 25, 0, 0), 500, 700).is_none());
        assert!(index_of(&sidx(0, 0, 100, 25, 0, 0), 500, 800).unwrap().is_empty());
        assert!(index_of(&[], 500, 800).is_none());
        assert!(index_of(&[255u8; 60], 500, 800).is_none());
    }

    /// Sur une vraie vidéo (`LUMEN_TEST_URL` : l'adresse de sa page) : ce que Lumen en tire,
    /// et un contrôle sur le fichier (chaque segment contrôlé commence un fragment).
    #[tokio::test]
    #[ignore]
    async fn streamer_live() {
        let url = std::env::var("LUMEN_TEST_URL").expect("LUMEN_TEST_URL");
        let data_dir = std::env::temp_dir().join(format!("lumen-streamer-{}", std::process::id()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let stream = crate::search::stream(&data_dir, &url, false, None).await.unwrap();
        let stream = decorate(stream, "http://127.0.0.1:1/media").await;
        let segments = stream.segments.expect("l'index n'a pas été trouvé");
        let client = mediaserv::http_client().unwrap();
        for (name, track, source) in [("image", &segments.video, &stream.video), ("son", &segments.audio, &stream.audio)] {
            let last = track.segments.last().unwrap();
            let total: f64 = track.segments.iter().map(|s| s.duration).sum();
            println!(
                "{name} : {}, en-tête de {} octets, {} segments ({:.1} s en moyenne, {:.1} s pour {:.1} s annoncées), le dernier de {} à {}",
                track.codec,
                track.init_end,
                track.segments.len(),
                total / track.segments.len() as f64,
                total,
                stream.duration,
                last.start,
                last.end
            );
            println!("   {}", track.url);
            assert!(track.segments.len() > 1);
            assert!((total - stream.duration).abs() < 2.0, "{total} contre {}", stream.duration);
            // l'en-tête commence par ftyp ; le premier, un du milieu et le dernier segment par moof
            let head = fetch(&client, source, 0, 7).await.unwrap();
            assert_eq!(&head.bytes[4..8], b"ftyp");
            let middle = &track.segments[track.segments.len() / 2];
            for seg in [&track.segments[0], middle, last] {
                let got = fetch(&client, source, seg.start, seg.start + 7).await.unwrap();
                assert_eq!(&got.bytes[4..8], b"moof", "le segment à {} n'en commence pas un", seg.start);
                assert!(got.size >= last.end);
            }
        }
        // la table entière, pour la comparer à celle de sessions/probe/segments.py
        println!("SEGMENTS {}", serde_json::to_string(&segments).unwrap());
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
