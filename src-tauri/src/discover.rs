//! Découvrir : des leçons venues d'ailleurs. Tout au long de la journée, Lumen
//! lit les sources choisies pour chaque langue (chaînes YouTube pour apprenants,
//! podcasts, actualités faciles ou ordinaires, vulgarisation, classements des
//! chansons) et range leurs nouveautés par niveau, de A1 à C1. Chaque source a son
//! rythme (les actualités toutes les 3 h, les chaînes deux fois par jour, les
//! classements chaque jour) ; un flux qui n'a pas changé n'est pas relu (ETag).
//! Rien n'est téléchargé avant que l'apprenant choisisse : la vidéo, l'épisode,
//! la chanson ou l'article devient une leçon par l'import habituel.
//!
//! Le catalogue est écrit ici, source par source, avec sa fourchette de
//! niveaux ; les titres des contenus pour apprenants l'affinent (« for
//! Beginners (A1-A2) », « Intermediate »). Les éléments trouvés vivent dans
//! `discover.db`, à part de la progression : un simple cache, ni sauvegardé ni
//! compté comme un changement de la base (la sauvegarde suit `total_changes`).
//! Ce qu'une source ne liste plus reste un moment (une leçon pour apprenants ne
//! vieillit pas, une actualité si) : les rayons s'étoffent au fil des jours.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures_util::{stream, StreamExt};
use parking_lot::Mutex;
use reqwest::Url;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::i18n::t;
use crate::media::{self, ImportEvent};
use crate::{langid, link, lyrics};

// ---------- niveaux ----------

pub const A1: u8 = 1;
pub const A2: u8 = 2;
pub const B1: u8 = 3;
pub const B2: u8 = 4;
pub const C1: u8 = 5;

/// Éléments lus par source à chaque lecture (les plus récents).
const PER_SOURCE: usize = 16;
/// Première lecture d'une chaîne : de quoi garnir les rayons d'emblée.
const FIRST_READ: usize = 30;
/// Chansons lues dans un classement.
const CHART_ITEMS: usize = 40;
/// Paroles vérifiées au plus par classement et par lecture (LRCLIB est un service bénévole).
const LYRICS_CHECKS: usize = 40;
/// Une actualisation demandée juste après une autre ne relit pas les mêmes sources.
const FRESH: i64 = 5 * 60;
/// Vidéos gardées : ni bandes-annonces ni directs de plusieurs heures.
const MIN_SECS: f64 = 90.0;
const MAX_SECS: f64 = 2.0 * 3600.0;

// ---------- catalogue ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    YouTube,
    Podcast,
    Articles,
    /// classement des chansons d'un pays (playlist de YouTube Music Charts)
    Chart,
}

/// Rayon de la vue Découvrir.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shelf {
    /// pour apprenants : comprehensible input, conversations lentes, podcasts d'apprentissage
    Learn,
    /// actualités, faciles ou ordinaires
    News,
    /// vulgarisation, documentaires, récits
    Culture,
    /// chansons du moment, avec leurs paroles
    Music,
}

impl Shelf {
    fn code(self) -> &'static str {
        match self {
            Shelf::Learn => "learn",
            Shelf::News => "news",
            Shelf::Culture => "culture",
            Shelf::Music => "music",
        }
    }

    /// Une source se relit au plus souvent… (les actualités vieillissent vite)
    fn every(self) -> i64 {
        match self {
            Shelf::News => 3 * 3600,
            Shelf::Music => 24 * 3600,
            _ => 10 * 3600,
        }
    }

    /// Ce qu'on garde : (jours, éléments au plus par source). Une leçon pour
    /// apprenants ou un documentaire ne vieillit pas ; les actualités, si.
    fn keep(self) -> (i64, usize) {
        match self {
            Shelf::News => (5, 24),
            Shelf::Music => (60, 40),
            _ => (150, 60),
        }
    }
}

/// Où lire le niveau d'un élément.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grade {
    /// la fourchette de la source seulement (actualités, culture)
    Source,
    /// niveaux et mots-clés des titres (« Beginners », « (A2) ») : contenus pour apprenants
    Titles,
    /// « – level 2 » (News in Levels)
    Levels,
}

#[derive(Debug)]
pub struct Source {
    /// identifiant stable (clé des éléments en cache)
    pub id: &'static str,
    pub lang: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// identifiant de la chaîne YouTube (UC…), ou adresse du flux
    pub url: &'static str,
    pub lo: u8,
    pub hi: u8,
    pub shelf: Shelf,
    pub grade: Grade,
    /// la page de chaque épisode porte aussi son texte (DW, RFI, Sveriges Radio) :
    /// l'import propose le son avec ce texte, la lanterne calée dessus
    pub page_text: bool,
    /// langue d'explication en plus de la langue étudiée : la source n'est
    /// proposée qu'aux apprenants dont c'est la langue d'interface
    pub via: Option<&'static str>,
}

#[allow(clippy::too_many_arguments)]
const fn src(id: &'static str, lang: &'static str, name: &'static str, kind: Kind, url: &'static str, lo: u8, hi: u8, shelf: Shelf) -> Source {
    let grade = match shelf {
        Shelf::Learn => Grade::Titles,
        _ => Grade::Source,
    };
    Source { id, lang, name, kind, url, lo, hi, shelf, grade, page_text: false, via: None }
}

#[allow(clippy::too_many_arguments)]
const fn yt(id: &'static str, lang: &'static str, name: &'static str, channel: &'static str, lo: u8, hi: u8, shelf: Shelf) -> Source {
    src(id, lang, name, Kind::YouTube, channel, lo, hi, shelf)
}

#[allow(clippy::too_many_arguments)]
const fn pod(id: &'static str, lang: &'static str, name: &'static str, feed: &'static str, lo: u8, hi: u8, shelf: Shelf) -> Source {
    src(id, lang, name, Kind::Podcast, feed, lo, hi, shelf)
}

/// Classement des chansons d'un pays (YouTube Music Charts) : seules celles dont
/// les paroles sont dans la langue étudiée sont proposées.
const fn chart(id: &'static str, lang: &'static str, name: &'static str, playlist: &'static str) -> Source {
    Source { id, lang, name, kind: Kind::Chart, url: playlist, lo: A2, hi: C1, shelf: Shelf::Music, grade: Grade::Source, page_text: false, via: None }
}

#[allow(clippy::too_many_arguments)]
const fn art(id: &'static str, lang: &'static str, name: &'static str, feed: &'static str, lo: u8, hi: u8, shelf: Shelf) -> Source {
    src(id, lang, name, Kind::Articles, feed, lo, hi, shelf)
}

impl Source {
    const fn text(self) -> Self {
        Source { page_text: true, ..self }
    }
    const fn via(self, lang: &'static str) -> Self {
        Source { via: Some(lang), ..self }
    }
    const fn levels(self) -> Self {
        Source { grade: Grade::Levels, ..self }
    }
}

use Shelf::{Culture, Learn, News};

/// Les sources de chaque langue. Chaînes et flux vérifiés un par un (activité,
/// langue parlée, niveau) ; `cargo test --lib discover_live -- --ignored`
/// les relit toutes.
pub static SOURCES: &[Source] = &[
    // ----- anglais -----
    yt("en-ci", "en", "English Comprehensible Input", "UCSW8FB6e8tUGEaDsoe7SlWw", A1, A2, Learn),
    yt("en-bbcle", "en", "BBC Learning English", "UCHaHD477h-FeBbVh9Sh7syA", A2, B2, Learn),
    yt("en-teded", "en", "TED-Ed", "UCsooa4yRKGN_zEE8iknghZA", B2, C1, Culture),
    yt("en-kurz", "en", "Kurzgesagt – In a Nutshell", "UCsXVk37bltHxD1rDPwtNM8Q", B2, C1, Culture),
    yt("en-bbc", "en", "BBC News", "UC16niRr50-MSBwiO3YDb3RA", C1, C1, News),
    pod("en-6min", "en", "6 Minute English", "https://podcasts.files.bbci.co.uk/p02pc9tn.rss", B1, B2, Learn),
    pod("en-conv", "en", "Learning English Conversations", "https://podcasts.files.bbci.co.uk/p02pc9zn.rss", B1, B2, Learn),
    pod("en-easy-pod", "en", "Easy English", "https://feeds.fireside.fm/easyenglish/rss", B1, B2, Learn),
    pod("en-luke", "en", "Luke's English Podcast", "https://feeds.acast.com/public/shows/62b0ada25c7ea10012f541cb", B2, C1, Learn),
    art("en-levels", "en", "News in Levels", "https://www.newsinlevels.com/feed/", A1, B1, News).levels(),
    art("en-bbc-art", "en", "BBC News", "https://feeds.bbci.co.uk/news/world/rss.xml", C1, C1, News),
    pod("en-lw-a1", "en", "LinguaWire · English A1", "https://feeds.acast.com/public/shows/6a203ce23d098b7011022fcb", A1, A1, News),
    pod("en-lw-a2", "en", "LinguaWire · English A2", "https://feeds.acast.com/public/shows/6a29bcc43f4eb34728ccd5e7", A2, A2, News),
    pod("en-lw-b1", "en", "LinguaWire · English B1+", "https://feeds.acast.com/public/shows/6a29bd4132e30dceaf22a1db", B1, B2, News),
    pod("en-news-easy", "en", "News in Easy English", "https://anchor.fm/s/10409f880/podcast/rss", A2, B1, News),
    pod("en-sbs-easy", "en", "SBS News in Easy English", "https://sbs-ondemand.streamguys1.com/sbs-news-easy-english/", A2, B1, News),
    // ----- espagnol -----
    yt("es-dreaming", "es", "Dreaming Spanish", "UCouyFdE9-Lrjo3M_2idKq1A", A1, B2, Learn),
    yt("es-easy", "es", "Easy Spanish", "UCAL4AMMMXKxHDu3FqZV6CbQ", A2, B1, Learn),
    yt("es-juan", "es", "Español con Juan", "UCoHJ7PkM6T92LwgJgrnDhWA", B1, B2, Learn),
    yt("es-kurz", "es", "En Pocas Palabras – Kurzgesagt", "UCZcvCpFcLxOKGbMocVgLjEA", B2, C1, Culture),
    yt("es-bbc", "es", "BBC News Mundo", "UCUBIrDsIVzRpKsClMlSlTpQ", C1, C1, News),
    pod("es-lw", "es", "LinguaWire · Spanish A1", "https://feeds.acast.com/public/shows/6a1f616c9942b8f91f48cc09", A1, A1, News),
    pod("es-news-easy", "es", "News In Easy Spanish", "https://feeds.castos.com/j3w40", A2, B1, News),
    pod("es-easy-pod", "es", "Easy Spanish", "https://feeds.fireside.fm/easyspanish/rss", B1, B2, Learn),
    pod("es-listos", "es", "Españolistos", "https://feeds.soundcloud.com/users/soundcloud:users:250273737/sounds.rss", B1, B2, Learn),
    pod(
        "es-ambulante",
        "es",
        "Radio Ambulante",
        "https://www.omnycontent.com/d/playlist/e73c998e-6e60-432f-8610-ae210140c5b1/b3c9b6e7-72ba-45c4-aff9-b1e7012d213b/092b66a8-4329-4183-bb12-b1e7012d216f/podcast.rss",
        C1,
        C1,
        Culture,
    ),
    art("es-bbc-art", "es", "BBC News Mundo", "https://www.bbc.com/mundo/index.xml", C1, C1, News),
    art("es-elpais", "es", "El País", "https://feeds.elpais.com/mrss-s/pages/ep/site/elpais.com/portada", C1, C1, News),
    pod("es-lw-a2", "es", "LinguaWire · Spanish A2", "https://feeds.acast.com/public/shows/6a2aca9071d362181f88a91d", A2, A2, News),
    pod("es-lw-b1", "es", "LinguaWire · Spanish B1+", "https://feeds.acast.com/public/shows/6a2ad0dc479dfe546fa3a017", B1, B2, News),
    pod("es-nis", "es", "News in Slow Spanish Latino", "https://rss.libsyn.com/shows/45040/destinations/146316.xml", B1, B1, News),
    pod("es-holaquepasa", "es", "Hola Qué Pasa", "https://holaquepasa.com/feed/podcast/", A2, B1, News),
    pod("es-coffee", "es", "Coffee Break Spanish", "https://feeds.acast.com/public/shows/985e7c00-8945-4e0d-a4da-b93049180ce1", A1, B1, Learn).via("en"),
    yt("es-quantum", "es", "QuantumFracture", "UCbdSYaPD-lr1kW27UJuk8Pw", C1, C1, Culture),
    yt("es-academia", "es", "Academia Play", "UCv05qOuJ6Igbe-EyQibJgwQ", B2, C1, Culture),
    // ----- français -----
    yt("fr-dreaming", "fr", "Dreaming French", "UCG7lancLEOKXZ7lEEyzvwjA", A1, B1, Learn),
    yt("fr-easy", "fr", "Easy French", "UCoUWq2QawqdC3-nRXKk-JUw", A2, B1, Learn),
    yt("fr-inner", "fr", "innerFrench", "UCI4xp8qHD1MDErkqxb1dPbA", B1, B2, Learn),
    yt("fr-pierre", "fr", "Français avec Pierre", "UCVgW9ZQaGBk6fsiPgE2mYDg", B1, B2, Learn),
    yt("fr-kurz", "fr", "Tout Simplement – Kurzgesagt", "UCzMHLUJ8xTqDq6xsk0N5IdQ", B2, C1, Culture),
    yt("fr-teded", "fr", "Restons Curieux – TED-Ed", "UCeGog9GPaCAPzTcSG237c_Q", B2, C1, Culture),
    yt("fr-hugo", "fr", "HugoDécrypte", "UCAcAnMF0OrCtUep3Y4M-ZPw", C1, C1, News),
    pod(
        "fr-rfi",
        "fr",
        "Journal en français facile",
        "https://apis.fle.rfi.fr/products/get_product/fle_getpodcast_by_nid_author_rfi?token_application=applepodcast_fle&program.entrepriseId=WBMZ39-FLE-FR-20220627",
        A2,
        B1,
        News,
    )
    .text(),
    pod("fr-news-easy", "fr", "News In Easy French", "https://feeds.castos.com/4x8do", A2, B1, News),
    pod("fr-easy-pod", "fr", "Easy French", "https://feeds.fireside.fm/easyfrench/rss", B1, B2, Learn),
    pod("fr-inner-pod", "fr", "InnerFrench", "https://podcast.innerfrench.com/feed.xml", B1, B2, Learn),
    pod("fr-authentique", "fr", "Français Authentique", "https://francaisauthentique.libsyn.com/rss", B1, B2, Learn),
    art("fr-1j1a", "fr", "1jour1actu", "https://www.1jour1actu.com/feed/", B1, B1, News),
    art("fr-lemonde", "fr", "Le Monde", "https://www.lemonde.fr/rss/une.xml", C1, C1, News),
    art("fr-franceinfo", "fr", "franceinfo", "https://www.francetvinfo.fr/titres.rss", C1, C1, News),
    pod("fr-lw-a1", "fr", "LinguaWire · French A1", "https://feeds.acast.com/public/shows/6a29a7413f4eb34728c4200e", A1, A1, News),
    pod("fr-lw-a2", "fr", "LinguaWire · French A2", "https://feeds.acast.com/public/shows/6a2ae89472ff36e11a1955fc", A2, A2, News),
    pod("fr-lw-b1", "fr", "LinguaWire · French B1+", "https://feeds.acast.com/public/shows/6a306ae5252d86e8466f67d0", B1, B2, News),
    pod("fr-nis", "fr", "News in Slow French", "https://rss.libsyn.com/shows/30723/destinations/63713.xml", B1, B1, News),
    pod("fr-coffee", "fr", "Coffee Break French", "https://feeds.acast.com/public/shows/47990e88-454b-4e3b-bf78-75a172c33184", A1, B1, Learn).via("en"),
    yt("fr-notabene", "fr", "Nota Bene", "UCP46_MXP_WG_auH88FnfS1A", B2, C1, Culture),
    yt("fr-science", "fr", "ScienceEtonnante", "UCaNlbnghtwlsGF-KzAFThqA", C1, C1, Culture),
    yt("fr-franceinfo-yt", "fr", "franceinfo", "UCO6K_kkdP-lnSCiO3tPx7WA", C1, C1, News),
    // ----- allemand -----
    yt("de-ci", "de", "Comprehensible German", "UChyx8ibmFMTTe3aS_l_eaKA", A1, A2, Learn),
    yt("de-dw", "de", "Deutsch lernen mit der DW", "UCxUWIEL-USsiPak0Qy6_vVg", A1, B1, Learn),
    yt("de-easy", "de", "Easy German", "UCbxb2fqe9oNgglAoYqsYOtQ", A2, B1, Learn),
    yt("de-logo", "de", "logo!", "UCuziK4bUFRr3z62bE7FMDPQ", B1, B1, News),
    yt("de-kurz", "de", "Dinge Erklärt – Kurzgesagt", "UCwRH985XgMYXQ6NxXDo8npw", B2, C1, Culture),
    yt("de-tagesschau", "de", "tagesschau", "UC5NOEUbkLheQcaaRldYW5GA", C1, C1, News),
    pod(
        "de-leicht",
        "de",
        "Nachrichtenleicht",
        "https://www.deutschlandfunk.de/podcast-nachrichtenleicht-der-wochenrueckblick-in-einfacher-sprache-100.xml",
        A2,
        A2,
        News,
    )
    .text(),
    pod("de-lgn", "de", "Langsam gesprochene Nachrichten", "https://rss.dw.com/xml/DKpodcast_lgn_de", B1, B1, News).text(),
    pod("de-topthema", "de", "Top-Thema mit Vokabeln", "https://rss.dw.com/xml/DKpodcast_topthemamitvokabeln_de", B1, B2, News).text(),
    pod("de-slow", "de", "Slow German", "https://slowgerman.com/feed/podcast/?redirect=no", A2, B1, Learn),
    pod("de-easy-pod", "de", "Easy German", "https://proxyfeed.svmaudio.com/feeds/easygerman/feed.xml", B1, B2, Learn),
    art("de-tagesschau-art", "de", "tagesschau.de", "https://www.tagesschau.de/index~rss2.xml", C1, C1, News),
    art("de-dw-art", "de", "DW", "https://rss.dw.com/rdf/rss-de-all", B2, C1, News),
    pod("de-news-easy", "de", "News In Easy German", "https://feeds.castos.com/x4pwm", A2, B1, News),
    pod("de-lw-a1", "de", "LinguaWire · German A1", "https://feeds.acast.com/public/shows/6a2b43d5479dfe546fd07156", A1, A1, News),
    pod("de-lw-a2", "de", "LinguaWire · German A2", "https://feeds.acast.com/public/shows/6a305c08cd02369494108cd9", A2, A2, News),
    pod("de-lw-b1", "de", "LinguaWire · German B1+", "https://feeds.acast.com/public/shows/6a3068dca893cd95ca82b064", B1, B2, News),
    pod("de-coffee", "de", "Coffee Break German", "https://feeds.acast.com/public/shows/0c3c53a1-180f-435a-9453-cec3883b4ada", A1, B1, Learn).via("en"),
    yt("de-wissen", "de", "MrWissen2go", "UCZHpIFMfoJJ_1QxNGLJTzyA", C1, C1, Culture),
    yt("de-terrax", "de", "Terra X History", "UCA3mpqm67CpJ13YfA8qAnow", B2, C1, Culture),
    yt("de-simplicissimus", "de", "Simplicissimus", "UCKGMHVipEvuZudhHD05FOYA", B2, C1, Culture),
    // ----- italien -----
    yt("it-si", "it", "Italiano sì", "UCkQz9XgvKsE8V0CFpCXX6MQ", A1, B1, Learn),
    yt("it-easy", "it", "Easy Italian", "UChpDG_WQkf_2tgLUr9xTR0g", A2, B1, Learn),
    yt("it-automatico", "it", "Italiano Automatico", "UChJtl-bJFgQit_BmjL5axtg", B1, B2, Learn),
    yt("it-podcast", "it", "Podcast Italiano", "UCegEedDeryCYSVT7eBCm-RQ", B1, C1, Learn),
    yt("it-geopop", "it", "Geopop", "UCx7EWheHmjCW3vX8K2d09vg", C1, C1, Culture),
    pod("it-news-easy", "it", "News In Easy Italian", "https://feeds.castos.com/w8v6p", A2, B1, News),
    pod("it-easy-pod", "it", "Easy Italian", "https://feeds.fireside.fm/easyitalian/rss", B1, B2, Learn),
    pod("it-podcast-pod", "it", "Podcast Italiano", "https://rss.buzzsprout.com/2413795.rss", B1, C1, Learn),
    pod("it-automatico-pod", "it", "Italiano Automatico", "https://italianoautomatico.podomatic.com/rss2.xml", B1, B2, Learn),
    art("it-ansa", "it", "ANSA", "https://www.ansa.it/sito/ansait_rss.xml", C1, C1, News),
    pod("it-lw-a1", "it", "LinguaWire · Italian A1", "https://feeds.acast.com/public/shows/6a1eca9fd610a774037c9d02", A1, A1, News),
    pod("it-nis", "it", "News in Slow Italian", "https://rss.libsyn.com/shows/41785/destinations/127311.xml", B1, B1, News),
    pod("it-coffee", "it", "Coffee Break Italian", "https://feeds.acast.com/public/shows/86766c5f-1580-450f-9376-bd74b57fcfbb", A1, B1, Learn).via("en"),
    yt("it-novalectio", "it", "Nova Lectio", "UCRCWJCFoZUvkkWzIqzfBy6g", C1, C1, Culture),
    yt("it-barbero", "it", "Lezioni di Alessandro Barbero", "UCbViHPTqsPbHNZ095bJRxRg", C1, C1, Culture),
    // ----- portugais -----
    yt("pt-speaking", "pt", "Speaking Brazilian", "UCGs6EbIt75S4IMKPRUU0JNQ", A2, B1, Learn),
    yt("pt-practice", "pt", "Practice Portuguese", "UCRR8BOCXjSMU8O2TdzAFbww", A2, B1, Learn),
    yt("pt-easy", "pt", "Easy Portuguese", "UCGItHJHk5zoYHRQD6ZQ-mrA", A2, B1, Learn),
    yt("pt-kurz", "pt", "Em Poucas Palavras – Kurzgesagt", "UCCYb8bp7v9maNk5n3nfw-Lg", B2, C1, Culture),
    yt("pt-nerdologia", "pt", "Nerdologia", "UClu474HMt895mVxZdlIHXEA", C1, C1, Culture),
    yt("pt-bbc", "pt", "BBC News Brasil", "UCthbIFAxbXTTQEC7EcQvP1Q", C1, C1, News),
    pod("pt-lw", "pt", "LinguaWire · Portuguese A1", "https://feeds.acast.com/public/shows/6a29baab518f9f10eb9a1a78", A1, A1, News),
    pod("pt-news-easy", "pt", "News In Easy Portuguese", "https://feeds.castos.com/w8vxp", A2, B1, News),
    pod("pt-practice-pod", "pt", "Practice Portuguese", "https://www.practiceportuguese.com/feed/podcast/", A2, B1, Learn),
    pod("pt-speaking-pod", "pt", "Speaking Brazilian Podcast", "https://feed.podbean.com/speakingbrazilian/feed.xml", B1, B2, Learn),
    art("pt-bbc-art", "pt", "BBC News Brasil", "https://www.bbc.com/portuguese/index.xml", C1, C1, News),
    art("pt-g1", "pt", "g1", "https://g1.globo.com/rss/g1/", C1, C1, News),
    pod("pt-lw-b1", "pt", "LinguaWire · Portuguese B1+", "https://feeds.acast.com/public/shows/6a71c167e9d2c023deb14860", B1, B2, News),
    yt("pt-manual", "pt", "Manual do Mundo", "UCKHhA5hN2UohhFDfNXB_cvQ", B2, C1, Culture),
    yt("pt-ciencia", "pt", "Ciência Todo Dia", "UCn9Erjy00mpnWeLnRqhsA1g", C1, C1, Culture),
    // ----- russe -----
    yt("ru-ci", "ru", "Comprehensible Russian", "UCDNbk-uX4D6nsthi8L03fng", A1, B1, Learn),
    yt("ru-easy", "ru", "Easy Russian", "UCxvt-g7JsPNnEn8tUtZZBBg", A2, B1, Learn),
    yt("ru-max", "ru", "Russian With Max", "UCklUqFEcJqFnWKEBozw5p4g", B1, B2, Learn),
    yt("ru-bbc", "ru", "BBC News – Русская служба", "UC8zQiuT0m1TELequJ5sp5zw", C1, C1, News),
    pod("ru-slow", "ru", "Slow Russian", "https://rss.libsyn.com/shows/75566/destinations/334707.xml", A2, B1, Learn),
    pod("ru-news-easy", "ru", "News In Easy Russian", "https://feeds.castos.com/4x8ko", A2, B1, News),
    pod("ru-easy-pod", "ru", "Easy Russian", "https://feeds.fireside.fm/easyrussian/rss", B1, B2, Learn),
    pod("ru-max-pod", "ru", "Russian With Max", "https://anchor.fm/s/6f65684/podcast/rss", B1, B2, Learn),
    art("ru-bbc-art", "ru", "Би-би-си", "https://www.bbc.com/russian/index.xml", C1, C1, News),
    art("ru-meduza", "ru", "Медуза", "https://meduza.io/rss/all", C1, C1, News),
    yt("ru-arzamas", "ru", "Arzamas", "UCVgvnGSFU41kIhEc09aztEg", C1, C1, Culture),
    // ----- néerlandais -----
    yt("nl-dutchly", "nl", "Dutchly", "UC1UlyJen2hvHT6wt2X-dU1Q", A1, B1, Learn),
    yt("nl-easy", "nl", "Easy Dutch", "UC1x1Tso1WzjvU7GhcJBVQhg", A2, B1, Learn),
    yt("nl-makkelijk", "nl", "NOS Journaal in Makkelijke Taal", "UCch2JvY2ZSwcjf5gb93HGQw", A2, B1, News),
    yt("nl-jeugd", "nl", "NOS Jeugdjournaal", "UC-bbHiTZGWKbsCjpzUfrk6Q", B1, B1, News),
    pod("nl-news-easy", "nl", "News In Easy Dutch", "https://feeds.castos.com/372d9", A2, B1, News),
    pod("nl-easy-pod", "nl", "Easy Dutch", "https://feeds.fireside.fm/easydutch/rss", B1, B2, Learn),
    art("nl-jeugd-art", "nl", "NOS Jeugdjournaal", "https://feeds.nos.nl/jeugdjournaal", B1, B1, News),
    art("nl-nos", "nl", "NOS", "https://feeds.nos.nl/nosnieuwsalgemeen", C1, C1, News),
    // ----- suédois -----
    yt("sv-katrin", "sv", "Slow Swedish with Katrin", "UCbG0VOqIo9EqEtfE3Ru2BaQ", A2, B1, Learn),
    yt("sv-linguist", "sv", "Swedish Linguist", "UCez7ckQUZhums7Rj0VuSrag", B1, B2, Learn),
    pod("sv-latt", "sv", "Radio Sweden på lätt svenska", "https://public-api.sr.se/rss/radio-sweden-pa-latt-svenska", A2, B1, News).text(),
    pod("sv-klartext", "sv", "Klartext", "https://public-api.sr.se/rss/klartext", A2, B1, News).text(),
    pod("sv-news-easy", "sv", "News In Easy Swedish", "https://feeds.castos.com/nvz3q", A2, B1, News),
    pod("sv-simple", "sv", "Simple Swedish Podcast", "https://feed.podbean.com/arhus12/feed.xml", A2, B1, Learn),
    art("sv-8sidor", "sv", "8 Sidor", "https://8sidor.se/feed/", A2, B1, News),
    art("sv-svt", "sv", "SVT Nyheter", "https://www.svt.se/nyheter/rss.xml", C1, C1, News),
    yt("sv-easyconv", "sv", "Easy Swedish Conversations", "UCCfH7zObgxSrjuKnxSJPZLg", A1, A2, Learn),
    // ----- danois -----
    yt("da-ci", "da", "Danish Comprehensible Input", "UCgDQm18b9DmEFJou0xLt6jg", A1, B1, Learn),
    yt("da-conv", "da", "Danish Conversations", "UCKWa0YHD4uXoG9_AXyB1CxQ", B1, B2, Learn),
    yt("da-essensen", "da", "P3 Essensen", "UC8tSbn8Q4rnsSQPY-1kzXuw", C1, C1, News),
    art("da-dr", "da", "DR Nyheder", "https://www.dr.dk/nyheder/service/feeds/allenyheder", C1, C1, News),
    yt("da-simple", "da", "Simple Danish Podcast", "UCHb91IBGvNFHH-7T-5N51JA", A2, B1, Learn),
    // ----- finnois -----
    yt("fi-satu", "fi", "Helppoa suomea", "UCXtkoH6vbOihYtM54PMX6Eg", A2, B1, Learn),
    yt("fi-jarno", "fi", "Suomea Jarnon kanssa", "UCoDbIqejIAS9eZgYNzE2FCg", A2, B1, Learn),
    yt("fi-keskustelut", "fi", "Suomen Keskustelut", "UC3sevuoFN7fKUIYX_ZgxJeA", B1, B2, Learn),
    yt("fi-yle", "fi", "Yle Uutiset", "UC_mdoC_BWTfTj7eT3XGA23A", C1, C1, News),
    art("fi-selko", "fi", "Yle Selkouutiset", "https://yle.fi/rss/selkouutiset", A2, B1, News),
    art("fi-yle-art", "fi", "Yle Uutiset", "https://yle.fi/rss/uutiset/paauutiset", C1, C1, News),
    // ----- estonien -----
    yt("et-lala", "et", "LÄLÄ", "UCcB-4wKX_hn9g6FnhGwluSA", A2, B1, Learn),
    yt("et-err", "et", "ERR", "UCYal7J64EbvvlygsfiFriIw", C1, C1, News),
    art("et-err-art", "et", "ERR uudised", "https://www.err.ee/rss", C1, C1, News),
    pod("et-eli", "et", "Estonian with Eli", "https://media.rss.com/estonianwitheli/feed.xml", A2, B1, Learn).via("en"),
    // ----- letton -----
    yt("lv-lva", "lv", "Latviešu valodas aģentūra", "UC24idzqmWOwTIURmxnXXdsg", A2, B1, Learn),
    yt("lv-ltv", "lv", "LTV Ziņu dienests", "UCOSAAyJoybqsY5sZ76BaqFA", C1, C1, News),
    art("lv-lsm", "lv", "LSM", "https://www.lsm.lv/rss/", C1, C1, News),
    // ----- lituanien -----
    yt("lt-paulius", "lt", "Lithuanian with Paulius", "UCIoF257Ir2im5lVegjhjSGA", A2, B1, Learn),
    yt("lt-lrt", "lt", "LRT", "UC4KnMZaxcv1KZDAJsgHXcOA", C1, C1, News),
    art("lt-lrt-art", "lt", "LRT", "https://www.lrt.lt/?rss", C1, C1, News),
    art("lt-15min", "lt", "15min", "https://www.15min.lt/rss", C1, C1, News),
    yt("lt-spoken", "lt", "Spoken Lithuanian", "UCHkglUChAgAcvocwpUHnoCQ", A1, A2, Learn).via("en"),
    pod("lt-paulius-pod", "lt", "Lithuanian with Paulius", "https://anchor.fm/s/efb8393c/podcast/rss", A2, B1, Learn),
    pod("lt-cup", "lt", "A Cup of Lithuanian", "https://anchor.fm/s/10cb2a090/podcast/rss", A2, B1, Learn),
    // ----- polonais -----
    yt("pl-lingoput", "pl", "LingoPut", "UCJgre6She3TQCVEXxk6kc8Q", A1, B1, Learn),
    yt("pl-easy", "pl", "Easy Polish", "UCPG9JpJITL7xETpVylMyrqA", A2, B1, Learn),
    yt("pl-kamil", "pl", "Polish with Kamil", "UC-PVeQYDVXYd4lY9wtQ4wwA", A2, B1, Learn),
    yt("pl-tvn24", "pl", "tvn24", "UC3R8278fJUWn2ysrOCJrmAQ", C1, C1, News),
    pod("pl-news-easy", "pl", "News In Easy Polish", "https://feeds.castos.com/9qkrv", A2, B1, News),
    pod("pl-real", "pl", "Real Polish", "https://anchor.fm/s/f6829c30/podcast/rss", B1, B2, Learn),
    art("pl-tvn24-art", "pl", "TVN24", "https://tvn24.pl/najnowsze.xml", C1, C1, News),
    // ----- tchèque -----
    yt("cs-czechin", "cs", "Czech-in", "UCIAPVo6C9UV_taKgInRLTcQ", A1, B1, Learn),
    yt("cs-easy", "cs", "Easy Czech", "UC-va43n42YE5sBAekuQX8qg", A2, B1, Learn),
    yt("cs-ct24", "cs", "ČT24", "UC0HQLHvU5_MMUJxLz6yl0GA", C1, C1, News),
    art("cs-irozhlas", "cs", "iROZHLAS", "https://www.irozhlas.cz/rss/irozhlas", C1, C1, News),
    pod("cs-slowczech", "cs", "slowczech", "https://feeds.blubrry.com/feeds/slowczech.xml", A2, B1, Learn).via("en"),
    pod("cs-michal", "cs", "Čeština s Michalem", "https://rss.buzzsprout.com/2566674.rss", A1, A2, Learn),
    pod("cs-dread", "cs", "Slow Czech for beginners", "https://anchor.fm/s/11319b798/podcast/rss", A1, A2, Learn),
    // ----- slovaque -----
    yt("sk-stories", "sk", "Learn Slovak with Stories", "UCUPgmAhUy6NkMxzRTmks55w", A1, B1, Learn),
    yt("sk-aktuality", "sk", "Aktuality.sk", "UC2lCFhJIC4adt_oP1dwSOVg", C1, C1, News),
    art("sk-aktuality-art", "sk", "Aktuality.sk", "https://www.aktuality.sk/rss/", C1, C1, News),
    yt("sk-filip", "sk", "Learn Slovak with Filip", "UCdVPMrIQmLFC-9yNrBS8Q_Q", A1, A2, Learn).via("en"),
    // ----- slovène -----
    yt("sl-dialog", "sl", "Slovenščina skozi dialog", "UCrUDbUp04kcr3rA1fwlP9dw", A2, B1, Learn),
    art("sl-rtv", "sl", "RTV SLO", "https://img.rtvslo.si/feeds/00.xml", C1, C1, News),
    // ----- croate -----
    yt("hr-hrt", "hr", "HRT vijesti", "UCSI1vb6CELskEFQpRceUj0g", C1, C1, News),
    art("hr-index", "hr", "Index.hr", "https://www.index.hr/rss", C1, C1, News),
    pod("hr-lagani", "hr", "Lagani hrvatski · SBS", "https://sbs-ondemand.streamguys1.com/lagani-hrvatski/", A2, B1, Learn),
    pod("hr-cakula", "hr", "Ćakula Café", "https://anchor.fm/s/10cf3f4c8/podcast/rss", A2, B1, Learn),
    // ----- hongrois -----
    yt("hu-heart", "hu", "Hungarian by Heart", "UCprH3w8hVn0aaAE6wGlXzUw", A2, B1, Learn),
    yt("hu-telex", "hu", "Telex", "UCM-1sd-cXSuCsfWp8QMY_OQ", C1, C1, News),
    art("hu-telex-art", "hu", "Telex", "https://telex.hu/rss", C1, C1, News),
    yt("hu-easy", "hu", "Easy Hungarian", "UCA0lqVr47gnhS5_Bsghnbzg", A2, B1, Learn),
    pod("hu-plain", "hu", "Plain Hungarian", "https://rss.buzzsprout.com/2138870.rss", B1, B2, Learn),
    pod("hu-patrik", "hu", "Hungarian with Patrik", "https://media.rss.com/hungarianwithpatrik/feed.xml", A2, B1, Learn).via("en"),
    // ----- roumain -----
    yt("ro-digi", "ro", "Digi24", "UCbvKamSrJkwT6ed2BMMZXwg", C1, C1, News),
    art("ro-digi-art", "ro", "Digi24", "https://www.digi24.ro/rss", C1, C1, News),
    pod("ro-acum", "ro", "Acum înțeleg!", "https://anchor.fm/s/d9aa65fc/podcast/rss", B1, B2, Learn),
    pod("ro-weekly", "ro", "Romanian Weekly Podcast", "https://feed.podbean.com/romanianweekly/feed.xml", A2, B1, Learn),
    // ----- bulgare -----
    yt("bg-az", "bg", "Аз говоря български", "UC7R7TQDHdgPVnb0YxbPz1bA", A2, B1, Learn),
    yt("bg-bnt", "bg", "БНТ", "UC8jnuRbBzMICRHsC0qqVd9Q", C1, C1, News),
    art("bg-dnevnik", "bg", "Dnevnik", "https://www.dnevnik.bg/rss/", C1, C1, News),
    yt("bg-bistra", "bg", "Learn Bulgarian with Bistra", "UCBTjB-a1MDXzbITxrCg86aQ", A1, A2, Learn).via("en"),
    // ----- ukrainien -----
    yt("uk-hanna", "uk", "Immersive Ukrainian with Hanna", "UCTJzD-YVoDGolkN7lQ4ZasQ", A2, B1, Learn),
    yt("uk-slow", "uk", "Slow Ukrainian", "UCqGyrVsLBUk2FcGWGfN14SQ", A2, B1, Learn),
    yt("uk-bbc", "uk", "BBC News Україна", "UCZctsW8Tpx8Tz9Ln4KUmR3g", C1, C1, News),
    yt("uk-suspilne", "uk", "Суспільне Новини", "UCPY6gj8G7dqwPxg9KwHrj5Q", C1, C1, News),
    art("uk-bbc-art", "uk", "BBC News Україна", "https://www.bbc.com/ukrainian/index.xml", C1, C1, News),
    yt("uk-speak", "uk", "Speak Ukrainian", "UCSTYIEpLtn_dqY7mMBoF3kA", A1, B1, Learn).via("en"),
    pod("uk-yevhen", "uk", "Slow Ukrainian with Yevhen", "https://anchor.fm/s/973b404c/podcast/rss", A2, B1, Learn),
    // ----- grec -----
    yt("el-ci", "el", "Greek Comprehensible Input", "UC99qhCTVJeZ_DCQkRLRm3Ew", A1, B1, Learn),
    yt("el-easy", "el", "Easy Greek", "UCoTlC0saIu6WNa5ttDDe6fQ", A2, B1, Learn),
    yt("el-ert", "el", "ΕΡΤ", "UC0jVU-mK53vDQZcSZB5mVHg", C1, C1, News),
    pod("el-supereasy", "el", "Super Easy Greek", "https://feeds.fireside.fm/supereasygreek/rss", A1, A2, Learn),
    pod("el-news-easy", "el", "News In Easy Greek", "https://feeds.castos.com/0pmk8", A2, B1, News),
    pod("el-easy-pod", "el", "Easy Greek", "https://feeds.fireside.fm/easygreek/rss", B1, B2, Learn),
    art("el-kathimerini", "el", "Καθημερινή", "https://www.kathimerini.gr/infeeds/rss/nx-rss-feed.xml", C1, C1, News),
    // ----- turc -----
    yt("tr-ci", "tr", "Comprehensible Turkish", "UCAXD2TuTkY_Tld20lYNL15g", A1, B1, Learn),
    yt("tr-easy", "tr", "Easy Turkish", "UC1U-NW0ci6d1aHLc2gOHv8Q", A2, B1, Learn),
    yt("tr-bbc", "tr", "BBC News Türkçe", "UCeMQiXmFNTtN3OHlNJxnnUw", C1, C1, News),
    pod("tr-news-easy", "tr", "News In Easy Turkish", "https://feeds.castos.com/r3m45", A2, B1, News),
    pod("tr-easy-pod", "tr", "Easy Turkish", "https://feeds.fireside.fm/easyturkish/rss", B1, B2, Learn),
    art("tr-bbc-art", "tr", "BBC News Türkçe", "https://www.bbc.com/turkce/index.xml", C1, C1, News),
    // ----- arabe -----
    yt("ar-ci", "ar", "Arabic Comprehensible Input", "UCQdEQ6KW8fpwuVSdtalXupg", A1, B1, Learn),
    yt("ar-kurz", "ar", "بكل بساطة – Kurzgesagt", "UCn5ASYdp7CzbFH2qtqjIJ9w", B2, C1, Culture),
    yt("ar-bbc", "ar", "BBC News عربي", "UCelk6aHijZq-GJBBB9YpReA", C1, C1, News),
    yt("ar-ajdoc", "ar", "الجزيرة الوثائقية", "UC0LSnqrwqtMwl2YwfUpO66g", C1, C1, Culture),
    art("ar-bbc-art", "ar", "BBC News عربي", "https://www.bbc.com/arabic/index.xml", C1, C1, News),
    // ----- hindi -----
    yt("hi-ci", "hi", "Comprehensible Hindi", "UCVt9SQjiL-R1GuH-5Uieovw", A1, B1, Learn),
    yt("hi-kurz", "hi", "आसान शब्दों में – Kurzgesagt", "UCIR1LQvYrHOWBpOGq5nFo0Q", B2, C1, Culture),
    yt("hi-bbc", "hi", "BBC News हिंदी", "UCN7B-QD0Qgn2boVH5Q0pOWg", C1, C1, News),
    art("hi-bbc-art", "hi", "BBC News हिंदी", "https://www.bbc.com/hindi/index.xml", C1, C1, News),
    // ----- indonésien -----
    yt("id-easy", "id", "Easy Indonesian", "UCfSZ2wSyWK7cRi-TcLp30Ew", A2, B1, Learn),
    yt("id-simply", "id", "Simply Indonesian", "UCvJaMlS_vr28GX7jFcuJv8Q", A2, B1, Learn),
    yt("id-bbc", "id", "BBC News Indonesia", "UC46q-QSvoJz-1iSxPeuOqWA", C1, C1, News),
    art("id-bbc-art", "id", "BBC News Indonesia", "https://www.bbc.com/indonesia/index.xml", C1, C1, News),
    pod("id-windah", "id", "Bahasa Indonesia Bersama Windah", "https://anchor.fm/s/520f510c/podcast/rss", B1, B2, Learn),
    // ----- vietnamien -----
    yt("vi-lilian", "vi", "Lilian Vietnamese", "UC4di2z7dbPra5xhp6BN7XwQ", A1, B1, Learn),
    yt("vi-understand", "vi", "Actually Understand Vietnamese", "UCiJJCAigdFRvR1CfH25IIbg", A2, B1, Learn),
    yt("vi-bbc", "vi", "BBC News Tiếng Việt", "UCpoNfKwZbecrcFpzm0ET4uw", C1, C1, News),
    art("vi-bbc-art", "vi", "BBC News Tiếng Việt", "https://www.bbc.com/vietnamese/index.xml", C1, C1, News),
    art("vi-vnexpress", "vi", "VnExpress", "https://vnexpress.net/rss/tin-moi-nhat.rss", C1, C1, News),
    yt("vi-slow", "vi", "Slow Vietnamese", "UC3U2lryw5Hksu71fMwsGT9A", A2, B1, Learn),
    // ----- coréen -----
    yt("ko-ttmik", "ko", "Talk To Me In Korean", "UC5r3WHrX4Z7peSYpDlgktGw", A1, B1, Learn).via("en"),
    yt("ko-taewoong", "ko", "태웅쌤 Comprehensible Input Korean", "UC737T1zTN6MQ1uWorHVAXvA", A1, B1, Learn),
    yt("ko-ttmik100", "ko", "Talk To Me In 100% Korean", "UCX6NVksmz8GzQsqYy24DWlA", A2, B1, Learn),
    yt("ko-immersion", "ko", "몰입한국어", "UC-3wHyVaLCiujjjPYXtif5w", B1, B2, Learn),
    yt("ko-kurz", "ko", "한눈에 보는 세상 – Kurzgesagt", "UC8rKCy_tipwTEY3RdkNCKmw", B2, C1, Culture),
    yt("ko-bbc", "ko", "BBC News 코리아", "UCIDOGTbwTBHZ5YoR9Xjcp-w", C1, C1, News),
    art("ko-bbc-art", "ko", "BBC News 코리아", "https://feeds.bbci.co.uk/korean/rss.xml", C1, C1, News),
    art("ko-yonhap", "ko", "연합뉴스", "https://www.yna.co.kr/rss/news.xml", C1, C1, News),
    yt("ko-easy", "ko", "Easy Korean!", "UCHxNPAnqTB4ohG5xl6MyuXg", A1, A2, Learn),
    // ----- japonais -----
    yt("ja-jikan", "ja", "にほんごのじかん", "UCdZHET-9_Comx6UaVTSETiQ", A1, B1, Learn),
    yt("ja-teppei", "ja", "Teppei", "UCH88l3_ltyJm67gAFzDFNRw", A2, B1, Learn),
    yt("ja-easypod", "ja", "EASY JAPANESE PODCAST", "UC16-9M0osgdFbLKXvCwpXaw", B1, B2, Learn),
    yt("ja-kurz", "ja", "世界をわかりやすく – Kurzgesagt", "UCzw2KK537iRgsrYnWaEMs8Q", B2, C1, Culture),
    yt("ja-teded", "ja", "好奇心を持ち続けよう – TED-Ed", "UCwFlWUGyXPHdsRAgmFxG_jw", B2, C1, Culture),
    pod("ja-teppei-pod", "ja", "Nihongo con Teppei", "http://nihongoconteppei.com/feed/podcast", A2, B1, Learn),
    art("ja-nhk", "ja", "NHK ニュース", "https://news.web.nhk/n-data/conf/na/rss/cat0.xml", C1, C1, News),
    art("ja-bbc", "ja", "BBC News Japan", "https://feeds.bbci.co.uk/japanese/rss.xml", C1, C1, News),
    yt("ja-easy", "ja", "Easy Japanese", "UCBet5Paucgz8feYQhVd0Y3A", B1, B2, Learn),
    pod("ja-news-easy", "ja", "News In Easy Japanese", "https://feeds.castos.com/241n8", A2, B1, News),
    // ----- musique : classements de YouTube Music Charts (pays où ils existent) -----
    chart("en-chart-us", "en", "Top 100 · États-Unis", "PL4fGSI1pDJn6O1LS0XSdF3RyO0Rq_LDeI"),
    chart("en-chart-uk", "en", "Top 100 · Royaume-Uni", "PL4fGSI1pDJn6_f5P3MnzXg9l3GDfnSlXa"),
    chart("es-chart-es", "es", "Top 100 · Espagne", "PL4fGSI1pDJn6sMPCoD7PdSlEgyUylgxuT"),
    chart("es-chart-mx", "es", "Top 100 · Mexique", "PL4fGSI1pDJn6fko1AmNa_pdGPZr5ROFvd"),
    chart("fr-chart", "fr", "Top 100 · France", "PL4fGSI1pDJn7bK3y1Hx-qpHBqfr6cesNs"),
    chart("de-chart", "de", "Top 100 · Allemagne", "PL4fGSI1pDJn6KpOXlp0MH8qA9tngXaUJ-"),
    chart("it-chart", "it", "Top 100 · Italie", "PL4fGSI1pDJn5JiDypHxveEplQrd7XQMlX"),
    chart("pt-chart-br", "pt", "Top 100 · Brésil", "PL4fGSI1pDJn7rGBE8kEC0CqTa1nMh9AKB"),
    chart("pt-chart-pt", "pt", "Top 100 · Portugal", "PL4fGSI1pDJn7H0X0bZN4C-I6YeldOvPku"),
    chart("ru-chart", "ru", "Top 100 · Russie", "PL4fGSI1pDJn5C8dBiYt0BTREyCHbZ47qc"),
    chart("nl-chart", "nl", "Top 100 · Pays-Bas", "PL4fGSI1pDJn7CXu1B1U0lYQ0qfPB9TVfa"),
    chart("sv-chart", "sv", "Top 100 · Suède", "PL4fGSI1pDJn7S_JFSuBHol2RH9WphaqzS"),
    chart("da-chart", "da", "Top 100 · Danemark", "PL4fGSI1pDJn51jFsgXEIR7WdKBychJiMU"),
    chart("fi-chart", "fi", "Top 100 · Finlande", "PL4fGSI1pDJn4T5TECl_90hfJsPUu1yi2y"),
    chart("et-chart", "et", "Top 100 · Estonie", "PL4fGSI1pDJn4fpNbyI8YHStVF-wyzHJtd"),
    chart("pl-chart", "pl", "Top 100 · Pologne", "PL4fGSI1pDJn68fmsRw9f6g-NzU5UA45v1"),
    chart("cs-chart", "cs", "Top 100 · Tchéquie", "PL4fGSI1pDJn5wV1AgglmIN_8okwTkz9WT"),
    chart("hu-chart", "hu", "Top 100 · Hongrie", "PL4fGSI1pDJn6K3QY1nHyhOGQqNCBGbMKi"),
    chart("ro-chart", "ro", "Top 100 · Roumanie", "PL4fGSI1pDJn5G2T6hrqwSS7ajUA7y4S5l"),
    chart("uk-chart", "uk", "Top 100 · Ukraine", "PL4fGSI1pDJn4E_HoW5HB-w5vFPkYfo3dB"),
    chart("tr-chart", "tr", "Top 100 · Turquie", "PL4fGSI1pDJn5tdVDtIAZArERm_vv4uFCR"),
    chart("ar-chart-eg", "ar", "Top 100 · Égypte", "PL4fGSI1pDJn510j-1L8bMgKTyeRwPrXWY"),
    chart("ar-chart-sa", "ar", "Top 100 · Arabie saoudite", "PL4fGSI1pDJn7xNK-XdqvCsqa7I8Nx3IyW"),
    chart("hi-chart", "hi", "Top 100 · Inde", "PL4fGSI1pDJn4pTWyM3t61lOyZ6_4jcNOw"),
    chart("id-chart", "id", "Top 100 · Indonésie", "PL4fGSI1pDJn5QPpj0R4vVgRWk8sSq549G"),
    chart("vi-chart", "vi", "Top 100 · Viêt Nam", "PL4fGSI1pDJn4bRKr6tjRWaGlqRg_zY_is"),
    chart("ko-chart", "ko", "Top 100 · Corée du Sud", "PL4fGSI1pDJn5S09aId3dUGp40ygUqmPGc"),
    chart("ja-chart", "ja", "Top 100 · Japon", "PL4fGSI1pDJn4-UIb6RKHdxam-oAUULIGB"),
];

/// Les sources d'une langue pour un apprenant dont l'interface est en `ui`.
pub fn sources<'a>(lang: &'a str, ui: &'a str) -> impl Iterator<Item = &'static Source> + 'a {
    SOURCES.iter().filter(move |s| s.lang == lang && s.via.map_or(true, |v| v == ui))
}

fn source(id: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.id == id)
}

// ---------- niveaux lus dans les titres ----------

const ABSOLUTE: &[&str] = &[
    "superbeginner",
    "super beginner",
    "complete beginner",
    "absolute beginner",
    "total beginner",
    "super easy",
    "super facile",
    "súper fácil",
    "absolute anfänger",
    "débutant complet",
    "débutants complets",
    "principiante absoluto",
    "principianti assoluti",
    "iniciante absoluto",
];
const BEGINNER: &[&str] = &[
    "beginner",
    "débutant",
    "principiant",
    "anfänger",
    "iniciante",
    "начинающ",
    "для новичков",
    "nybörjare",
    "początkując",
    "začátečník",
    "aloittelij",
    "初心者",
    "初級",
    "초급",
    "pemula",
    "başlangıç",
    "αρχάρι",
];
const INTERMEDIATE: &[&str] = &[
    "intermediate",
    "intermédiaire",
    "intermedio",
    "intermedia",
    "intermediário",
    "intermediario",
    "mittelstufe",
    "中級",
    "중급",
    "średniozaawansowan",
    "orta seviye",
    "μεσαί",
];
const ADVANCED: &[&str] = &[
    "advanced",
    "avancé",
    "avanzado",
    "avanzato",
    "avançado",
    "fortgeschritten",
    "上級",
    "고급",
    "продвинут",
    "zaawansowan",
    "gevorderd",
    "ileri seviye",
    "προχωρημέν",
];
const SLOW: &[&str] = &["slow ", "lent ", "langsam", "lento", "lentamente", "despacio", "медленн", "powoli", "yavaş"];

/// Niveau qu'un titre annonce : « (A1-A2) », « A0–A1 », « for Beginners », « Intermediate ».
pub(crate) fn title_levels(title: &str) -> Option<(u8, u8)> {
    let low = title.to_lowercase();
    // niveaux du CECR écrits tels quels
    let ch: Vec<char> = low.chars().collect();
    let mut cefr: Vec<u8> = Vec::new();
    for i in 0..ch.len().saturating_sub(1) {
        let before = i == 0 || !ch[i - 1].is_alphanumeric();
        let after = ch.get(i + 2).map_or(true, |c| !c.is_alphanumeric());
        if !(before && after) {
            continue;
        }
        let l = match (ch[i], ch[i + 1]) {
            ('a', '0' | '1') => Some(A1),
            ('a', '2') => Some(A2),
            ('b', '1') => Some(B1),
            ('b', '2') => Some(B2),
            ('c', '1' | '2') => Some(C1),
            _ => None,
        };
        cefr.extend(l);
    }
    if let (Some(&lo), Some(&hi)) = (cefr.iter().min(), cefr.iter().max()) {
        return Some((lo, hi));
    }
    let has = |words: &[&str]| words.iter().any(|w| low.contains(w));
    let mut said: Vec<(u8, u8)> = Vec::new();
    if has(ABSOLUTE) {
        said.push((A1, A1));
    } else if has(BEGINNER) {
        said.push((A1, A2));
    }
    if has(INTERMEDIATE) {
        said.push((B1, B2));
    }
    if has(ADVANCED) {
        said.push((B2, C1));
    }
    if said.is_empty() && has(SLOW) {
        said.push((A2, B1));
    }
    said.into_iter().reduce(|(a, b), (x, y)| (a.min(x), b.max(y)))
}

/// « Grammar for Your Level – level 2 » (News in Levels : 1 à 3).
fn news_level(title: &str) -> Option<(u8, u8)> {
    let low = title.to_lowercase();
    let i = low.rfind("level ")?;
    match low[i + 6..].chars().next()? {
        '1' => Some((A1, A1)),
        '2' => Some((A2, A2)),
        '3' => Some((B1, B1)),
        _ => None,
    }
}

/// Fourchette de niveaux d'un élément : celle de sa source, affinée par son
/// titre ; un titre qui s'en écarte franchement l'emporte (« Super Easy German »).
pub fn grade(src: &Source, title: &str) -> (u8, u8) {
    let said = match src.grade {
        Grade::Source => None,
        Grade::Titles => title_levels(title),
        Grade::Levels => news_level(title),
    };
    match said {
        None => (src.lo, src.hi),
        Some((a, b)) => {
            let (lo, hi) = (a.max(src.lo), b.min(src.hi));
            if lo <= hi {
                (lo, hi)
            } else {
                (a, b)
            }
        }
    }
}

// ---------- lecture des sources ----------

/// Un élément tel que la source le décrit.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Found {
    /// identifiant dans la source (vidéo, guid, adresse)
    key: String,
    title: String,
    /// vidéo YouTube, fichier son, ou article
    url: String,
    /// page de l'épisode ou de l'article
    page: String,
    image: String,
    summary: String,
    duration: f64,
    /// secondes depuis 1970, 0 si inconnue
    published: i64,
    video: bool,
    /// chaîne YouTube (pour lire l'artiste d'un clip)
    channel: String,
    /// chanson : artiste et titre (paroles vérifiées)
    artist: String,
    track: String,
}

/// Dernières vidéos d'une chaîne, telles que yt-dlp les liste (`media::yt_latest`).
fn youtube_entries(v: &Value, max: usize) -> Vec<Found> {
    youtube_all(v).into_iter().filter(|f| (MIN_SECS..=MAX_SECS).contains(&f.duration)).take(max).collect()
}

/// Toutes les vidéos d'une liste, durée connue ou non (titres en danois, finnois,
/// indonésien : yt-dlp lit mal les durées écrites à la façon du pays).
fn youtube_all(v: &Value) -> Vec<Found> {
    let Some(entries) = v.get("entries").and_then(Value::as_array) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|e| {
            let id = e.get("id")?.as_str()?;
            let title = e.get("title")?.as_str()?.trim();
            if id.len() != 11 || title.is_empty() || title == "[Private video]" || title == "[Deleted video]" {
                return None;
            }
            let live = e.get("live_status").and_then(Value::as_str).unwrap_or("");
            if matches!(live, "is_live" | "is_upcoming" | "post_live") {
                return None;
            }
            let duration = e.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
            let published = ["timestamp", "release_timestamp"].iter().find_map(|k| e.get(*k).and_then(Value::as_i64)).unwrap_or(0);
            // la plus grande miniature proposée, sinon celle qui existe toujours
            let image = e
                .get("thumbnails")
                .and_then(Value::as_array)
                .and_then(|a| {
                    a.iter()
                        .filter(|t| t.get("url").and_then(Value::as_str).is_some_and(|u| u.starts_with("https://")))
                        .max_by_key(|t| t.get("width").and_then(Value::as_i64).unwrap_or(0))
                })
                .and_then(|t| t.get("url").and_then(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
            let url = format!("https://www.youtube.com/watch?v={id}");
            let channel = ["channel", "uploader"].iter().find_map(|k| e.get(*k).and_then(Value::as_str)).unwrap_or("").to_string();
            Some(Found {
                key: id.into(),
                title: title.into(),
                url: url.clone(),
                page: url,
                image,
                duration,
                published,
                video: true,
                channel,
                ..Default::default()
            })
        })
        .collect()
}

/// Date d'un flux (RFC 2822 pour le RSS, RFC 3339 pour l'Atom) en secondes, 0 si illisible.
fn parse_date(s: &str) -> i64 {
    let s = s.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc2822(s) {
        return d.timestamp();
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return d.timestamp();
    }
    // jour de la semaine erroné (« Sat, 4 Oct »), ou date sans fuseau
    let rest = s.split_once(", ").map(|(_, r)| r).unwrap_or(s);
    if let Ok(d) = chrono::DateTime::parse_from_str(rest, "%d %b %Y %H:%M:%S %z") {
        return d.timestamp();
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s.get(..19).unwrap_or(s), fmt) {
            return d.and_utc().timestamp();
        }
    }
    chrono::NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(12, 0, 0))
        .map_or(0, |d| d.and_utc().timestamp())
}

/// Contenu brut d'un élément (HTML compris), sans ses CDATA ; HTML échappé (« &lt;p&gt; ») rendu lisible.
fn raw_inner(block: &str, name: &str) -> Option<String> {
    let tag = link::tags(block).find(|t| t.name == name)?;
    let close = link::find_ci(block, &format!("</{name}"), tag.end).unwrap_or(block.len());
    let raw = block[tag.end..close].replace("<![CDATA[", "").replace("]]>", "");
    Some(if raw.contains("&lt;") { link::decode_entities(&raw) } else { raw })
}

/// Texte lisible d'un fragment HTML, coupé à `max` caractères sur un mot.
fn plain(html: &str, max: usize) -> String {
    let mut text = String::with_capacity(html.len().min(4096));
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let flat = link::decode_entities(&text).split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches(|c: char| !c.is_alphanumeric()))
}

fn is_image(u: &str) -> bool {
    let path = u.split(['?', '#']).next().unwrap_or(u).to_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"].iter().any(|e| path.ends_with(e))
}

fn is_media(u: &str) -> Option<bool> {
    Url::parse(u).ok().and_then(|x| link::ext_kind(&x))
}

/// Les miniatures de la BBC arrivent en 240 px : la même image en 800 px reste nette sur une carte.
fn sharper(image: String) -> String {
    if image.contains("ichef.bbci.co.uk") && image.contains("/240/") {
        image.replacen("/240/", "/800/", 1)
    } else {
        image
    }
}

/// Éléments d'un flux RSS ou Atom, du plus récent au plus ancien. Podcast : les
/// épisodes qui ont un son ou une vidéo ; articles : ceux qui ont une page.
pub(crate) fn parse_feed(xml: &str, feed_url: &str, podcast: bool) -> Vec<Found> {
    let base = Url::parse(feed_url).ok();
    let abs = |v: &str| match &base {
        Some(b) => b.join(v.trim()).map(|u| u.to_string()).unwrap_or_else(|_| v.trim().to_string()),
        None => v.trim().to_string(),
    };
    let atom = link::find_ci(xml, "<item", 0).is_none();
    let item_tag = if atom { "entry" } else { "item" };
    let first = link::tags(xml).find(|t| t.name == item_tag).map(|t| t.end).unwrap_or(xml.len());
    let head = &xml[..first];
    // image de l'émission : celle des épisodes qui n'en ont pas
    let show_image = link::tags(head)
        .find(|t| t.name == "itunes:image" && t.attr("href").is_some())
        .and_then(|t| t.attr("href").map(abs))
        .or_else(|| {
            link::tags(head).find(|t| t.name == "image").and_then(|t| {
                let close = link::find_ci(head, "</image", t.end)?;
                link::tag_text(&head[t.end..close], "url").map(|u| abs(&u))
            })
        })
        .unwrap_or_default();

    let mut out: Vec<Found> = Vec::new();
    let mut pos = 0;
    while let Some(start) = link::tags(&xml[pos..]).find(|t| t.name == item_tag).map(|t| pos + t.end) {
        let end = link::find_ci(xml, &format!("</{item_tag}"), start).unwrap_or(xml.len());
        let block = &xml[start..end];
        pos = end.max(start + 1).min(xml.len());
        let mut media: Option<(String, bool)> = None;
        let mut page = String::new();
        let mut image = String::new();
        for tg in link::tags(block) {
            let url = tg.attr("url").map(abs);
            let typ = tg.attr("type").unwrap_or("").to_lowercase();
            match tg.name.as_str() {
                "enclosure" | "media:content" => {
                    let Some(u) = url else { continue };
                    let medium = tg.attr("medium").unwrap_or("");
                    if typ.starts_with("image") || medium == "image" || (typ.is_empty() && medium.is_empty() && is_image(&u)) {
                        if image.is_empty() {
                            image = u;
                        }
                    } else if media.is_none() {
                        let video = typ.starts_with("video") || medium == "video";
                        if typ.starts_with("audio") || video || medium == "audio" || is_media(&u).is_some() {
                            media = Some((u.clone(), video || is_media(&u) == Some(true)));
                        }
                    }
                }
                "media:thumbnail" if image.is_empty() => image = url.unwrap_or_default(),
                "itunes:image" if image.is_empty() => image = tg.attr("href").map(abs).unwrap_or_default(),
                "link" if page.is_empty() => {
                    let rel = tg.attr("rel").unwrap_or("alternate");
                    match tg.attr("href") {
                        Some(h) if rel == "alternate" => page = abs(h),
                        Some(h) if rel == "enclosure" && media.is_none() => media = Some((abs(h), typ.starts_with("video"))),
                        Some(_) => {}
                        None => page = abs(&link::inner_text(block, &tg)),
                    }
                }
                _ => {}
            }
        }
        let html =
            ["content:encoded", "description", "summary", "content", "itunes:summary"].iter().find_map(|n| raw_inner(block, n)).unwrap_or_default();
        if image.is_empty() {
            image = link::tags(&html).find(|t| t.name == "img").and_then(|t| t.attr("src").map(abs)).unwrap_or_default();
        }
        let Some(title) = link::tag_text(block, "title") else { continue };
        let page = if page.starts_with("http") { page } else { String::new() };
        let (url, video) = match (podcast, media) {
            (true, Some((u, v))) => (u, v),
            (false, _) if !page.is_empty() => (page.clone(), false),
            _ => continue,
        };
        let key = link::tag_text(block, "guid").or_else(|| link::tag_text(block, "id")).unwrap_or_else(|| url.clone());
        let published = ["pubdate", "published", "updated", "dc:date"].iter().find_map(|n| link::tag_text(block, n)).map_or(0, |d| parse_date(&d));
        let image = if image.is_empty() && podcast { show_image.clone() } else { image };
        out.push(Found {
            key,
            title,
            url,
            page,
            image: sharper(image),
            summary: plain(&html, 240),
            duration: link::tag_text(block, "itunes:duration").map_or(0.0, |d| link::parse_duration(&d)),
            published,
            video,
            ..Default::default()
        });
        if out.len() >= PER_SOURCE * 3 {
            break;
        }
    }
    // du plus récent au plus ancien (l'ordre du flux départage les dates inconnues)
    out.sort_by_key(|f| std::cmp::Reverse(f.published));
    out.truncate(PER_SOURCE);
    out
}

async fn channel(data_dir: &Path, ytdlp: &Path, url: &str, max: usize, lang: Option<&str>) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(60), media::yt_latest(data_dir, ytdlp, url, max + 6, lang))
        .await
        .map_err(|_| anyhow!(t("la chaîne ne répond pas", "the channel doesn't answer")))?
}

/// Ce qu'a donné la lecture d'une source.
pub(crate) enum Fetched {
    /// éléments, et de quoi demander au prochain passage « seulement si ça a changé » (ETag, date)
    Items(Vec<Found>, Option<(String, String)>),
    /// le flux n'a pas changé depuis la dernière lecture
    Unchanged,
}

/// Où en est la lecture d'une source.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct SrcState {
    /// dernier essai, dernière réussite (secondes)
    checked: i64,
    ok: i64,
    /// échecs d'affilée
    fails: i64,
    etag: String,
    modified: String,
}

/// La source est à relire : son rythme est passé (plus tard après des échecs),
/// ou l'apprenant l'a demandé et elle n'a pas été lue depuis quelques minutes.
fn is_due(src: &Source, st: &SrcState, now: i64, force: bool) -> bool {
    if force {
        return now - st.checked >= FRESH;
    }
    let wait = if st.fails > 0 { (45 * 60 * (1i64 << (st.fails - 1).min(4))).min(12 * 3600) } else { src.shelf.every() };
    now - st.checked >= wait
}

/// Flux RSS ou Atom, lu seulement s'il a changé depuis la dernière fois.
async fn feed(client: &reqwest::Client, src: &Source, prev: &SrcState) -> Result<Fetched> {
    use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
    let mut req = client.get(src.url);
    if !prev.etag.is_empty() {
        req = req.header(IF_NONE_MATCH, &prev.etag);
    }
    if !prev.modified.is_empty() {
        req = req.header(IF_MODIFIED_SINCE, &prev.modified);
    }
    let slow = || anyhow!(t("le flux ne répond pas", "the feed doesn't answer"));
    let resp = tokio::time::timeout(Duration::from_secs(40), req.send()).await.map_err(|_| slow())??;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::Unchanged);
    }
    if !resp.status().is_success() {
        return Err(anyhow!(crate::tr!("le flux a répondu {}", "the feed answered {}", resp.status())));
    }
    let header = |h| resp.headers().get(h).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok()).unwrap_or("").to_string();
    let tags = (header(ETAG), header(LAST_MODIFIED));
    let bytes = tokio::time::timeout(Duration::from_secs(40), resp.bytes()).await.map_err(|_| slow())??;
    let xml = String::from_utf8_lossy(&bytes);
    Ok(Fetched::Items(parse_feed(&xml, src.url, src.kind == Kind::Podcast), Some(tags)))
}

/// Les chansons d'un classement dont les paroles existent et sont dans la langue
/// de la source (un classement italien compte aussi des chansons espagnoles).
async fn chart_songs(data_dir: &Path, ytdlp: &Path, store: &Store, src: &Source) -> Result<Vec<Found>> {
    let url = format!("https://www.youtube.com/playlist?list={}", src.url);
    let v = tokio::time::timeout(Duration::from_secs(60), media::yt_flat(data_dir, ytdlp, &url, 1, CHART_ITEMS, None))
        .await
        .map_err(|_| anyhow!(t("le classement ne répond pas", "the chart doesn't answer")))??;
    let entries = youtube_entries(&v, CHART_ITEMS);
    if entries.is_empty() {
        return Err(anyhow!(t("classement vide", "empty chart")));
    }
    let mut todo = Vec::new();
    for f in &entries {
        if store.song(&f.key).is_none() {
            todo.push(f.clone());
        }
    }
    let lc = lyrics::client()?;
    let lang = src.lang;
    let checked: Vec<(String, bool, String, String)> = stream::iter(todo.into_iter().take(LYRICS_CHECKS))
        .map(|f| {
            let lc = lc.clone();
            async move {
                let (artist, track) = lyrics::split_title(&f.title, &f.channel);
                let found = tokio::time::timeout(Duration::from_secs(15), lyrics::find(&lc, &artist, &track, "", f.duration)).await.ok().flatten();
                let ok = found.is_some_and(|l| !l.is_empty() && lyrics::word_count(&l, lang) >= 30 && langid::matches(&l.text(), lang) == Some(true));
                (f.key, ok, artist, track)
            }
        })
        .buffered(4)
        .collect()
        .await;
    for (video, ok, artist, track) in &checked {
        store.set_song(video, *ok, artist, track)?;
    }
    // l'ordre du classement
    Ok(entries
        .into_iter()
        .filter_map(|mut f| {
            let (ok, artist, track) = store.song(&f.key)?;
            if !ok {
                return None;
            }
            f.artist = artist;
            f.track = track;
            Some(f)
        })
        .collect())
}

/// Ce qu'une source propose aujourd'hui. `first` : première lecture de cette source.
async fn fetch(data_dir: &Path, ytdlp: Option<&Path>, client: &reqwest::Client, store: &Store, src: &Source, prev: &SrcState, first: bool) -> Result<Fetched> {
    match src.kind {
        Kind::YouTube => {
            let ytdlp = ytdlp.ok_or_else(|| anyhow!(t("composants vidéo indisponibles", "video components unavailable")))?;
            let url = format!("https://www.youtube.com/channel/{}/videos", src.url);
            let english = src.lang == "en";
            let max = if first { FIRST_READ } else { PER_SOURCE };
            let mut found = youtube_all(&channel(data_dir, ytdlp, &url, max, (!english).then_some(src.lang)).await?);
            // les titres dans la langue de la chaîne n'ont pas de date (et parfois pas de durée lisible) :
            // une seconde liste, en anglais, les complète à la première lecture ou s'il manque des durées ;
            // ensuite, une vidéo nouvelle date du jour où Lumen la voit
            let unknown = found.iter().filter(|f| !(MIN_SECS..=MAX_SECS).contains(&f.duration)).count();
            if !english && !found.is_empty() && (first || unknown * 3 > found.len()) {
                if let Ok(v) = channel(data_dir, ytdlp, &url, max, None).await {
                    let known: HashMap<String, (f64, i64)> = youtube_all(&v).into_iter().map(|f| (f.key, (f.duration, f.published))).collect();
                    for f in &mut found {
                        if let Some((d, p)) = known.get(&f.key) {
                            if *d > 0.0 {
                                f.duration = *d;
                            }
                            if first {
                                f.published = *p;
                            }
                        }
                    }
                }
            }
            found.retain(|f| (MIN_SECS..=MAX_SECS).contains(&f.duration));
            found.truncate(max);
            Ok(Fetched::Items(found, None))
        }
        Kind::Chart => {
            let ytdlp = ytdlp.ok_or_else(|| anyhow!(t("composants vidéo indisponibles", "video components unavailable")))?;
            Ok(Fetched::Items(chart_songs(data_dir, ytdlp, store, src).await?, None))
        }
        Kind::Podcast | Kind::Articles => feed(client, src, prev).await,
    }
}

/// YouTube refuse un moment les rafales : deux lectures à la fois au plus, toutes langues confondues.
fn youtube_gate() -> &'static tokio::sync::Semaphore {
    static GATE: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Semaphore::new(2))
}

async fn fetch_one(
    data_dir: &Path,
    ytdlp: Option<&Path>,
    client: &reqwest::Client,
    store: &Store,
    src: &'static Source,
    prev: SrcState,
) -> (&'static Source, SrcState, Result<Fetched>) {
    let first = !store.has(src.id);
    if !matches!(src.kind, Kind::YouTube | Kind::Chart) {
        let r = fetch(data_dir, ytdlp, client, store, src, &prev, first).await;
        return (src, prev, r);
    }
    let _slot = youtube_gate().acquire().await;
    let r = fetch(data_dir, ytdlp, client, store, src, &prev, first).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    (src, prev, r)
}

// ---------- cache ----------

/// Un élément proposé à l'apprenant.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub source: String,
    pub source_name: String,
    /// "learn", "news", "culture" ou "music"
    pub shelf: String,
    /// "video", "audio" ou "text"
    pub kind: String,
    pub title: String,
    /// vidéo YouTube, fichier son, ou article
    pub url: String,
    /// page de l'épisode ou de l'article (vide si inconnue)
    pub page: String,
    pub image: String,
    pub summary: String,
    pub duration: f64,
    pub published: i64,
    /// niveaux de 1 (A1) à 5 (C1)
    pub lo: u8,
    pub hi: u8,
    /// la page porte aussi le texte de l'épisode
    pub page_text: bool,
    /// première apparition dans Lumen
    pub fetched_at: i64,
    /// leçon déjà créée à partir de cet élément
    pub lesson_id: Option<i64>,
    /// chanson : artiste et titre (ses paroles existent, dans la langue étudiée)
    pub artist: String,
    pub track: String,
}

#[derive(Serialize, Debug)]
pub struct Feed {
    pub items: Vec<Item>,
    /// dernière lecture réussie (secondes, 0 : jamais)
    pub refreshed_at: i64,
    pub refreshing: bool,
    /// sources de cette langue
    pub sources: usize,
}

#[derive(Serialize, Debug, Default)]
pub struct Report {
    /// éléments nouveaux
    pub added: usize,
    pub sources: usize,
    /// sources qui n'ont pas répondu
    pub failed: Vec<String>,
    /// toutes les sources venaient d'être lues : rien n'a été relu
    pub skipped: bool,
}

pub struct Store {
    conn: Mutex<Connection>,
    /// une lecture à la fois par langue (une demande attend celle qui tourne)
    turns: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    /// langues en cours de lecture (ou qui attendent leur tour)
    running: Mutex<HashMap<String, usize>>,
}

const SCHEMA: &str = r#"
    PRAGMA journal_mode = WAL;
    CREATE TABLE IF NOT EXISTS items(
        id TEXT PRIMARY KEY,
        lang TEXT NOT NULL,
        source TEXT NOT NULL,
        kind TEXT NOT NULL,
        title TEXT NOT NULL,
        url TEXT NOT NULL,
        page TEXT NOT NULL DEFAULT '',
        image TEXT NOT NULL DEFAULT '',
        summary TEXT NOT NULL DEFAULT '',
        duration REAL NOT NULL DEFAULT 0,
        published INTEGER NOT NULL DEFAULT 0,
        lo INTEGER NOT NULL,
        hi INTEGER NOT NULL,
        rank INTEGER NOT NULL DEFAULT 0,
        fetched_at INTEGER NOT NULL,
        lesson_id INTEGER,
        hidden INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS items_lang ON items(lang, source);
    CREATE TABLE IF NOT EXISTS runs(lang TEXT PRIMARY KEY, at INTEGER NOT NULL DEFAULT 0, tried INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS sources(
        id TEXT PRIMARY KEY,
        checked INTEGER NOT NULL DEFAULT 0,
        ok INTEGER NOT NULL DEFAULT 0,
        fails INTEGER NOT NULL DEFAULT 0,
        etag TEXT NOT NULL DEFAULT '',
        modified TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE IF NOT EXISTS songs(
        video TEXT PRIMARY KEY,
        ok INTEGER NOT NULL,
        artist TEXT NOT NULL DEFAULT '',
        track TEXT NOT NULL DEFAULT '',
        checked INTEGER NOT NULL
    );
"#;

/// Colonnes ajoutées depuis la première version du cache.
fn upgrade(c: &Connection) -> Result<()> {
    for (col, decl) in [("artist", "TEXT NOT NULL DEFAULT ''"), ("track", "TEXT NOT NULL DEFAULT ''")] {
        let has = c.query_row("SELECT 1 FROM pragma_table_info('items') WHERE name=?1", [col], |_| Ok(())).optional()?.is_some();
        if !has {
            c.execute_batch(&format!("ALTER TABLE items ADD COLUMN {col} {decl}"))?;
        }
    }
    Ok(())
}

/// Une chanson sans paroles trouvées est revérifiée au bout de deux semaines.
const SONG_RETRY: i64 = 14 * 86400;

/// Clé d'un élément : sa source et son identifiant dans la source.
fn item_id(source: &str, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(source.as_bytes());
    h.update([0u8]);
    h.update(key.as_bytes());
    hex::encode(&h.finalize()[..10])
}

/// Adresse comparable d'une page : sans paramètres de suivi ; une vidéo YouTube par son identifiant.
fn same_page(u: &str) -> String {
    let Ok(url) = Url::parse(u.trim()) else { return u.trim().to_lowercase() };
    let host = url.host_str().unwrap_or("").trim_start_matches("www.").trim_start_matches("m.").to_lowercase();
    if host == "youtu.be" {
        return format!("yt:{}", url.path().trim_matches('/'));
    }
    if host.ends_with("youtube.com") {
        if let Some((_, v)) = url.query_pairs().find(|(k, _)| k == "v") {
            return format!("yt:{v}");
        }
    }
    format!("{host}{}", url.path().trim_end_matches('/'))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Marque une langue « en cours de lecture » le temps d'une actualisation.
struct Busy<'a> {
    store: &'a Store,
    lang: String,
}

impl<'a> Busy<'a> {
    fn new(store: &'a Store, lang: &str) -> Self {
        *store.running.lock().entry(lang.to_string()).or_insert(0) += 1;
        Busy { store, lang: lang.to_string() }
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        let mut r = self.store.running.lock();
        if let Some(n) = r.get_mut(&self.lang) {
            *n -= 1;
            if *n == 0 {
                r.remove(&self.lang);
            }
        }
    }
}

impl Store {
    /// Ouvre le cache ; illisible, il est recréé (il ne contient rien qu'on ne puisse relire).
    pub fn open(data_dir: &Path) -> Store {
        let path = data_dir.join("discover.db");
        let conn = Self::connect(&path)
            .or_else(|_| {
                for ext in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
                }
                Self::connect(&path)
            })
            .or_else(|_| {
                let c = Connection::open_in_memory()?;
                c.execute_batch(SCHEMA)?;
                upgrade(&c)?;
                Ok::<_, anyhow::Error>(c)
            })
            .expect("base en mémoire");
        Store { conn: Mutex::new(conn), turns: Mutex::new(HashMap::new()), running: Mutex::new(HashMap::new()) }
    }

    fn connect(path: &Path) -> Result<Connection> {
        let c = Connection::open(path)?;
        c.execute_batch(SCHEMA)?;
        upgrade(&c)?;
        Ok(c)
    }

    fn turn(&self, lang: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        self.turns.lock().entry(lang.to_string()).or_default().clone()
    }

    fn src_state(&self, id: &str) -> SrcState {
        self.conn
            .lock()
            .query_row("SELECT checked, ok, fails, etag, modified FROM sources WHERE id=?1", [id], |r| {
                Ok(SrcState { checked: r.get(0)?, ok: r.get(1)?, fails: r.get(2)?, etag: r.get(3)?, modified: r.get(4)? })
            })
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn set_state(&self, id: &str, st: &SrcState) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO sources(id, checked, ok, fails, etag, modified) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET checked=excluded.checked, ok=excluded.ok, fails=excluded.fails, etag=excluded.etag, modified=excluded.modified",
            params![id, st.checked, st.ok, st.fails, st.etag, st.modified],
        )?;
        Ok(())
    }

    /// Paroles d'un clip déjà vérifiées : (dans la bonne langue, artiste, titre).
    fn song(&self, video: &str) -> Option<(bool, String, String)> {
        let row: Option<(bool, String, String, i64)> = self
            .conn
            .lock()
            .query_row("SELECT ok, artist, track, checked FROM songs WHERE video=?1", [video], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional()
            .ok()
            .flatten();
        let (ok, artist, track, checked) = row?;
        (ok || now() - checked < SONG_RETRY).then_some((ok, artist, track))
    }

    fn set_song(&self, video: &str, ok: bool, artist: &str, track: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO songs(video, ok, artist, track, checked) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(video) DO UPDATE SET ok=excluded.ok, artist=excluded.artist, track=excluded.track, checked=excluded.checked",
            params![video, ok, artist, track, now()],
        )?;
        Ok(())
    }

    /// Dernière lecture réussie d'une des sources de la langue (0 : jamais).
    fn refreshed_at(&self, lang: &str, ui: &str) -> i64 {
        let ids: Vec<&str> = sources(lang, ui).map(|s| s.id).collect();
        let c = self.conn.lock();
        let best = ids
            .iter()
            .filter_map(|id| c.query_row("SELECT ok FROM sources WHERE id=?1", [id], |r| r.get::<_, i64>(0)).optional().ok().flatten())
            .max()
            .unwrap_or(0);
        drop(c);
        best.max(self.run(lang).0)
    }

    /// La source a déjà été lue (ses éléments sont en cache).
    fn has(&self, source: &str) -> bool {
        self.conn.lock().query_row("SELECT 1 FROM items WHERE source=?1 LIMIT 1", [source], |_| Ok(())).optional().ok().flatten().is_some()
    }

    pub fn refreshing(&self, lang: &str) -> bool {
        self.running.lock().contains_key(lang)
    }

    /// (dernière lecture réussie, dernier essai)
    fn run(&self, lang: &str) -> (i64, i64) {
        self.conn
            .lock()
            .query_row("SELECT at, tried FROM runs WHERE lang=?1", [lang], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten()
            .unwrap_or((0, 0))
    }

    fn set_run(&self, lang: &str, at: i64, tried: i64) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO runs(lang, at, tried) VALUES(?1,?2,?3) ON CONFLICT(lang) DO UPDATE SET at=excluded.at, tried=excluded.tried",
            params![lang, at, tried],
        )?;
        Ok(())
    }

    /// Une source au moins de la langue est à relire (lecture d'arrière-plan).
    pub fn due(&self, lang: &str, ui: &str) -> bool {
        let n = now();
        sources(lang, ui).any(|s| is_due(s, &self.src_state(s.id), n, false))
    }

    /// Range ce qu'une source propose aujourd'hui. Ce qu'elle ne propose plus reste
    /// un moment (une leçon pour apprenants ne vieillit pas, une actualité si),
    /// dans la limite d'un nombre d'éléments par source. Renvoie le nombre d'éléments nouveaux.
    fn save(&self, src: &Source, found: &[Found], at: i64) -> Result<usize> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        let known_source = tx.query_row("SELECT 1 FROM items WHERE source=?1 LIMIT 1", [src.id], |_| Ok(())).optional()?.is_some();
        let mut listed = HashSet::new();
        let mut added = 0;
        for (rank, f) in found.iter().enumerate() {
            let id = item_id(src.id, &f.key);
            if !listed.insert(id.clone()) {
                continue;
            }
            let known = tx.query_row("SELECT 1 FROM items WHERE id=?1", [&id], |_| Ok(())).optional()?.is_some();
            if !known {
                added += 1;
            }
            let (lo, hi) = grade(src, &f.title);
            // sans date dans la source : un élément apparu depuis la dernière lecture date d'aujourd'hui
            let published = if f.published == 0 && known_source && !known { at } else { f.published };
            let kind = match src.kind {
                Kind::YouTube | Kind::Chart => "video",
                Kind::Podcast if f.video => "video",
                Kind::Podcast => "audio",
                Kind::Articles => "text",
            };
            // une date approximative (« il y a 3 jours ») est plus juste à la première lecture : on la garde
            tx.execute(
                "INSERT INTO items(id,lang,source,kind,title,url,page,image,summary,duration,published,lo,hi,rank,fetched_at,artist,track)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
                 ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, title=excluded.title, url=excluded.url, page=excluded.page,
                   image=excluded.image, summary=excluded.summary, duration=excluded.duration,
                   published=CASE WHEN items.published > 0 THEN items.published ELSE excluded.published END,
                   lo=excluded.lo, hi=excluded.hi, rank=excluded.rank, artist=excluded.artist, track=excluded.track",
                params![id, src.lang, src.id, kind, f.title, f.url, f.page, f.image, f.summary, f.duration, published, lo, hi, rank as i64, at, f.artist, f.track],
            )?;
        }
        // ménage : trop ancien ou en trop (les plus récents restent) ; une leçon créée
        // ou un élément masqué restent, celui-ci pour ne jamais revenir
        let (days, cap) = src.shelf.keep();
        let rows: Vec<(String, i64, bool, bool)> = {
            let mut st = tx.prepare(
                "SELECT id, CASE WHEN published > 0 THEN published ELSE fetched_at END AS t, lesson_id IS NOT NULL, hidden = 1
                 FROM items WHERE source=?1 ORDER BY t DESC, rank",
            )?;
            let rows = st.query_map([src.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut kept = 0;
        for (id, t, lesson, hidden) in rows {
            let stay = if lesson {
                true
            } else if hidden {
                at - t < 200 * 86400
            } else if (listed.contains(&id) || at - t < days * 86400) && kept < cap {
                kept += 1;
                true
            } else {
                false
            };
            if !stay {
                tx.execute("DELETE FROM items WHERE id=?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Ce qu'il y a à découvrir dans une langue. `lessons` : (identifiant, source)
    /// des leçons de cette langue, pour reconnaître ce qui est déjà importé.
    pub fn list(&self, lang: &str, ui: &str, lessons: &[(i64, String)]) -> Result<Feed> {
        let ids: HashSet<i64> = lessons.iter().map(|(id, _)| *id).collect();
        let by_page: HashMap<String, i64> = lessons.iter().filter(|(_, s)| !s.is_empty()).map(|(id, s)| (same_page(s), *id)).collect();
        let c = self.conn.lock();
        let mut st = c.prepare(
            "SELECT id,source,kind,title,url,page,image,summary,duration,published,lo,hi,fetched_at,lesson_id,artist,track
             FROM items WHERE lang=?1 AND hidden=0 ORDER BY published DESC, rank",
        )?;
        let rows = st.query_map([lang], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, f64>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, u8>(10)?,
                r.get::<_, u8>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, Option<i64>>(13)?,
                r.get::<_, String>(14)?,
                r.get::<_, String>(15)?,
            ))
        })?;
        let mut items = Vec::new();
        for row in rows {
            let (id, sid, kind, title, url, page, image, summary, duration, published, lo, hi, fetched_at, lesson, artist, track) = row?;
            // source retirée du catalogue, ou réservée aux apprenants d'une autre langue d'interface
            let Some(src) = source(&sid).filter(|s| s.via.map_or(true, |v| v == ui)) else { continue };
            let lesson_id = lesson
                .filter(|l| ids.contains(l))
                .or_else(|| [&page, &url].iter().filter(|u| !u.is_empty()).find_map(|u| by_page.get(&same_page(u)).copied()));
            items.push(Item {
                id,
                source: sid,
                source_name: src.name.to_string(),
                shelf: src.shelf.code().to_string(),
                kind,
                title,
                url,
                page,
                image,
                summary,
                duration,
                published,
                lo,
                hi,
                page_text: src.page_text,
                fetched_at,
                lesson_id,
                artist,
                track,
            });
        }
        drop(st);
        drop(c);
        Ok(Feed { items, refreshed_at: self.refreshed_at(lang, ui), refreshing: self.refreshing(lang), sources: sources(lang, ui).count() })
    }

    /// Relie un élément à la leçon créée à partir de lui.
    pub fn mark(&self, id: &str, lesson: i64) -> Result<()> {
        self.conn.lock().execute("UPDATE items SET lesson_id=?1 WHERE id=?2", params![lesson, id])?;
        Ok(())
    }

    /// L'apprenant ne veut plus voir cet élément.
    pub fn hide(&self, id: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE items SET hidden=1 WHERE id=?1", [id])?;
        Ok(())
    }
}

// ---------- actualisation ----------

/// Relit les sources d'une langue qui sont à relire (`force` : demandé par
/// l'apprenant, toutes celles qui n'ont pas été lues depuis quelques minutes).
/// `on_event` : installation des composants vidéo s'il le faut (étape « tools »),
/// avancement (0 à 100), et « found » chaque fois qu'une source apporte du nouveau
/// (la vue se met à jour au fil de la lecture).
pub async fn refresh(
    data_dir: &Path,
    store: &Store,
    lang: &str,
    ui: &str,
    force: bool,
    on_event: &mut (dyn FnMut(ImportEvent) + Send),
) -> Result<Report> {
    let _busy = Busy::new(store, lang);
    let turn = store.turn(lang);
    let _turn = turn.lock().await;
    let started = now();
    let all: Vec<&'static Source> = sources(lang, ui).collect();
    let list: Vec<(&'static Source, SrcState)> =
        all.iter().map(|&s| (s, store.src_state(s.id))).filter(|(s, st)| is_due(s, st, started, force)).collect();
    if list.is_empty() {
        return Ok(Report { skipped: !all.is_empty(), sources: all.len(), ..Default::default() });
    }
    let ytdlp = if list.iter().any(|(s, _)| matches!(s.kind, Kind::YouTube | Kind::Chart)) {
        match crate::tools::find_ytdlp(data_dir) {
            Some(p) => Some(p),
            None => {
                on_event(ImportEvent::Stage { stage: "tools".into() });
                crate::tools::ensure_youtube_tools(data_dir, |p| on_event(ImportEvent::Progress { value: p })).await.ok()
            }
        }
    } else {
        None
    };
    on_event(ImportEvent::Stage { stage: "discover".into() });
    on_event(ImportEvent::Progress { value: 0.0 });

    let client = link::client()?;
    let jobs: Vec<_> = list.iter().map(|(s, st)| fetch_one(data_dir, ytdlp.as_deref(), &client, store, s, st.clone())).collect();
    let mut results = stream::iter(jobs).buffer_unordered(8);

    let total = list.len();
    let mut report = Report { sources: total, ..Default::default() };
    let (mut done, mut answered) = (0, 0);
    while let Some((s, prev, r)) = results.next().await {
        done += 1;
        let mut st = SrcState { checked: started, ..prev.clone() };
        match r {
            Ok(Fetched::Items(found, tags)) if !found.is_empty() => {
                let n = store.save(s, &found, started)?;
                report.added += n;
                answered += 1;
                st.ok = started;
                st.fails = 0;
                if let Some((etag, modified)) = tags {
                    st.etag = etag;
                    st.modified = modified;
                }
                if n > 0 {
                    on_event(ImportEvent::Stage { stage: "found".into() });
                }
            }
            Ok(Fetched::Unchanged) => {
                answered += 1;
                st.ok = started;
                st.fails = 0;
            }
            // un flux vide ou illisible garde ce qu'il proposait ; on réessaiera plus tard
            _ => {
                st.fails = prev.fails + 1;
                report.failed.push(s.name.to_string());
            }
        }
        store.set_state(s.id, &st)?;
        on_event(ImportEvent::Progress { value: done as f64 / total as f64 * 100.0 });
    }
    let (at, _) = store.run(lang);
    store.set_run(lang, if answered > 0 { started } else { at }, started)?;
    Ok(report)
}

/// Lecture en arrière-plan des langues étudiées (la langue active d'abord) :
/// toutes les 20 minutes, les sources dont le rythme est passé (les actualités
/// toutes les 3 h, les chaînes et podcasts deux fois par jour, les classements
/// chaque jour). Un événement `discover` suit chaque langue où du nouveau est arrivé.
pub async fn auto_loop(app: tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    tokio::time::sleep(Duration::from_secs(40)).await;
    loop {
        let st = app.state::<crate::state::AppState>();
        let (on, langs): (bool, Vec<String>) = {
            let c = st.db.lock();
            // lecture d'arrière-plan coupée dans les Réglages : seulement sur demande
            let on = crate::db::setting(&c, "discover_auto").as_deref() != Some("0");
            let active = crate::db::setting(&c, "lang").unwrap_or_default();
            let all = crate::db::setting(&c, "langs").unwrap_or_default();
            let mut v: Vec<String> = std::iter::once(active.as_str()).chain(all.split(',')).filter(|l| !l.is_empty()).map(str::to_string).collect();
            let mut seen = HashSet::new();
            v.retain(|l| seen.insert(l.clone()));
            (on, v)
        };
        let ui = crate::i18n::native();
        for lang in langs.into_iter().filter(|_| on) {
            if st.discover.due(&lang, ui) {
                let mut quiet = |_e: ImportEvent| {};
                if let Ok(r) = refresh(&st.data_dir, &st.discover, &lang, ui, false, &mut quiet).await {
                    if r.added > 0 || r.sources > 0 {
                        let _ = app.emit("discover", &lang);
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(20 * 60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue() {
        let mut ids = HashSet::new();
        for s in SOURCES {
            assert!(ids.insert(s.id), "identifiant en double : {}", s.id);
            assert!(crate::text::LANGS.contains(&s.lang), "langue inconnue : {}", s.id);
            assert!((A1..=C1).contains(&s.lo) && s.lo <= s.hi && s.hi <= C1, "niveaux : {}", s.id);
            assert!(s.id.starts_with(&format!("{}-", s.lang)), "préfixe de langue : {}", s.id);
            match s.kind {
                Kind::YouTube => assert!(s.url.starts_with("UC") && s.url.len() == 24, "chaîne : {}", s.id),
                Kind::Chart => assert!(s.url.starts_with("PL") && s.shelf == Shelf::Music, "classement : {}", s.id),
                _ => assert!(s.url.starts_with("http"), "flux : {}", s.id),
            }
        }
        // toutes les langues de Lumen ont de quoi découvrir, dans les deux langues d'interface
        for lang in crate::text::LANGS {
            for ui in ["fr", "en"] {
                assert!(sources(lang, ui).count() >= 2, "{lang} ({ui}) : trop peu de sources");
            }
        }
        // une source réservée aux anglophones n'apparaît pas aux francophones
        assert!(sources("ko", "fr").all(|s| s.via.is_none()));
        assert!(sources("ko", "en").any(|s| s.via == Some("en")));
    }

    #[test]
    fn levels_from_titles() {
        let easy = source("it-si").unwrap();
        assert_eq!(grade(easy, "🖼️ #74 | An unusual picnic | Italian for Beginners (A2) 🔵"), (A2, A2));
        assert_eq!(grade(easy, "#56 | Small Objects | Italian for Absolute Beginners (A0–A1) 🟢"), (A1, A1));
        assert_eq!(grade(easy, "🎙️ Ep. 136 | Di caccole, cerume e altre parole disgustose (Italian B1)"), (B1, B1));
        // sans indice : la fourchette de la source
        assert_eq!(grade(easy, "🎙️ Ep. 138 | Di sudore, puzza e brufoli"), (A1, B1));
        let dreaming = source("es-dreaming").unwrap();
        assert_eq!(grade(dreaming, "\"Panic at the Airport\" | Spanish for Beginners (A1-A2)"), (A1, A2));
        assert_eq!(grade(dreaming, "Can You Understand This Easy Spanish Story for Beginners?"), (A1, A2));
        assert_eq!(grade(dreaming, "Spanish Podcast: Are We Alone in the Universe?"), (A1, B2));
        let german = source("de-easy").unwrap();
        // un titre qui s'écarte de la source l'emporte
        assert_eq!(grade(german, "Super Easy German (215): Im Supermarkt"), (A1, A1));
        assert_eq!(grade(german, "Easy German Vlog - Comprehensible Input for Beginners & Intermediates"), (A2, B1));
        assert_eq!(grade(german, "Why Germans Love Coffee and Cake (Slow German Conversation)"), (A2, B1));
        // « B2B », « A320 » ne sont pas des niveaux
        assert_eq!(title_levels("B2B sales and the A320"), None);
        // actualités : la source seule, quels que soient les mots du titre
        let bbc = source("en-bbc-art").unwrap();
        assert_eq!(grade(bbc, "Beginner's luck for the advanced economies"), (C1, C1));
        let levels = source("en-levels").unwrap();
        assert_eq!(grade(levels, "Grammar for Your Level – level 1"), (A1, A1));
        assert_eq!(grade(levels, "Storm hits Japan – level 3"), (B1, B1));
        assert_eq!(grade(levels, "English news and easy articles"), (A1, B1));
    }

    #[test]
    fn podcast_feed() {
        let xml = r#"<?xml version="1.0"?><rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel>
            <title>Easy Spanish</title><itunes:image href="https://img.example/show.jpg"/>
            <item><title>265: ¿Tú sabes armar la carpa?</title><guid>ep-265</guid>
              <pubDate>Thu, 01 Oct 2026 15:00:00 +0200</pubDate>
              <link>https://www.easyspanish.fm/265</link>
              <enclosure url="https://cdn.example/265.mp3" type="audio/mpeg" length="1"/>
              <itunes:duration>32:10</itunes:duration>
              <description><![CDATA[<p>Hoy hablamos de <b>acampar</b> &amp; de la montaña.</p>]]></description>
            </item>
            <item><title>Sin sonido</title><link>https://www.easyspanish.fm/x</link></item>
            <item><title>264: ¿Quién quiere, puede?</title><guid>ep-264</guid>
              <pubDate>Sat, 26 Sep 2026 15:00:00 +0200</pubDate>
              <enclosure url="/264.m4a" type="" length="1"/>
              <itunes:duration>1890</itunes:duration>
              <itunes:image href="https://img.example/264.jpg"/>
            </item>
        </channel></rss>"#;
        let f = parse_feed(xml, "https://feeds.example/easyspanish/rss", true);
        assert_eq!(f.len(), 2, "l'épisode sans son est écarté");
        assert_eq!(f[0].title, "265: ¿Tú sabes armar la carpa?");
        assert_eq!(f[0].key, "ep-265");
        assert_eq!(f[0].url, "https://cdn.example/265.mp3");
        assert_eq!(f[0].page, "https://www.easyspanish.fm/265");
        assert_eq!(f[0].image, "https://img.example/show.jpg", "image de l'émission par défaut");
        assert_eq!(f[0].duration, 1930.0);
        assert_eq!(f[0].summary, "Hoy hablamos de acampar & de la montaña.");
        assert!(!f[0].video);
        assert!(f[0].published > f[1].published);
        assert_eq!(f[1].url, "https://feeds.example/264.m4a");
        assert_eq!(f[1].image, "https://img.example/264.jpg");
        assert_eq!(f[1].duration, 1890.0);
    }

    #[test]
    fn article_feeds() {
        // BBC : miniature 240 px agrandie ; NOS : image en pièce jointe, pas un son
        let rss = r#"<rss><channel><title>BBC</title>
            <item><title><![CDATA[Storm hits the coast]]></title>
              <link>https://www.bbc.com/news/articles/abc?at_medium=RSS</link>
              <guid isPermaLink="false">abc</guid><pubDate>Sat, 03 Oct 2026 12:22:44 GMT</pubDate>
              <media:thumbnail width="240" height="135" url="https://ichef.bbci.co.uk/ace/standard/240/cpsprodpb/x.jpg"/>
              <description>Waves up to &lt;b&gt;ten&lt;/b&gt; metres.</description></item>
            <item><title>Kabinet valt</title><link>https://nos.nl/l/123</link>
              <enclosure url="https://cdn.nos.nl/image/123.jpg" type="image/jpeg" length="0"/>
              <pubDate>Sun, 4 Oct 2026 04:21:18 +0200</pubDate></item>
            <item><title>Plaatje in de tekst</title><link>https://nos.nl/l/124</link>
              <description>&lt;p&gt;&lt;img src="https://cdn.nos.nl/image/124.jpg"&gt;Tekst&lt;/p&gt;</description></item>
        </channel></rss>"#;
        let f = parse_feed(rss, "https://feeds.example/rss", false);
        assert_eq!(f.len(), 3);
        let by = |t: &str| f.iter().find(|x| x.title == t).unwrap();
        let storm = by("Storm hits the coast");
        assert_eq!(storm.image, "https://ichef.bbci.co.uk/ace/standard/800/cpsprodpb/x.jpg");
        assert_eq!(storm.summary, "Waves up to ten metres.");
        assert_eq!(storm.url, "https://www.bbc.com/news/articles/abc?at_medium=RSS");
        assert_eq!(by("Kabinet valt").image, "https://cdn.nos.nl/image/123.jpg");
        assert_eq!(by("Plaatje in de tekst").image, "https://cdn.nos.nl/image/124.jpg");
        assert_eq!(by("Plaatje in de tekst").published, 0);
        // un flux d'articles n'a rien à offrir comme podcast
        assert!(parse_feed(rss, "https://feeds.example/rss", true).is_empty());

        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Yle</title>
            <entry><title>Viikon uutinen selkosuomeksi</title><id>urn:yle:1</id>
              <link rel="alternate" href="https://yle.fi/a/74-1"/><updated>2026-10-03T09:00:00+03:00</updated>
              <summary>Uutisia selkeällä suomen kielellä.</summary></entry>
        </feed>"#;
        let f = parse_feed(atom, "https://yle.fi/rss/selkouutiset", false);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].url, "https://yle.fi/a/74-1");
        assert_eq!(f[0].key, "urn:yle:1");
        assert_eq!(f[0].published, parse_date("2026-10-03T06:00:00Z"));
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("Sat, 03 Oct 2026 12:22:44 GMT"), 1_791_030_164);
        assert_eq!(parse_date("2026-10-03T12:22:44Z"), 1_791_030_164);
        // jour de la semaine faux, fréquent dans les flux
        assert_eq!(parse_date("Mon, 03 Oct 2026 12:22:44 +0000"), 1_791_030_164);
        assert_eq!(parse_date("2026-10-03 12:22:44"), 1_791_030_164);
        assert_eq!(parse_date("demain"), 0);
    }

    #[test]
    fn youtube_listing() {
        let v: Value = serde_json::from_str(
            r#"{"entries":[
                {"id":"Tu3IHEb3bko","title":"A Day in Paris in Slow German","duration":834,"timestamp":1790812800,
                 "thumbnails":[{"url":"https://i.ytimg.com/vi/Tu3IHEb3bko/hq720_custom_1.jpg?sqp=a","width":360},
                               {"url":"https://i.ytimg.com/vi/Tu3IHEb3bko/hq720_custom_1.jpg?sqp=b","width":720}]},
                {"id":"shortshort1","title":"Un Short","duration":42},
                {"id":"livelivelv1","title":"En direct","duration":null,"live_status":"is_upcoming"},
                {"id":"x2yz3abcdeF","title":"Sans miniature","duration":991}
            ]}"#,
        )
        .unwrap();
        let f = youtube_entries(&v, 12);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].url, "https://www.youtube.com/watch?v=Tu3IHEb3bko");
        assert_eq!(f[0].image, "https://i.ytimg.com/vi/Tu3IHEb3bko/hq720_custom_1.jpg?sqp=b");
        assert_eq!(f[0].published, 1790812800);
        assert!(f[0].video);
        assert_eq!(f[1].image, "https://i.ytimg.com/vi/x2yz3abcdeF/hqdefault.jpg");
    }

    #[test]
    fn pages_compare() {
        assert_eq!(same_page("https://www.youtube.com/watch?v=Tu3IHEb3bko&t=3"), "yt:Tu3IHEb3bko");
        assert_eq!(same_page("https://youtu.be/Tu3IHEb3bko"), "yt:Tu3IHEb3bko");
        assert_eq!(same_page("https://www.bbc.com/news/articles/abc?at_medium=RSS"), same_page("https://bbc.com/news/articles/abc/"));
    }

    fn temp_store(name: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lumen-discover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Store::open(&dir), dir)
    }

    fn found(key: &str, title: &str, published: i64) -> Found {
        let url = format!("https://www.youtube.com/watch?v={key}");
        Found { key: key.into(), title: title.into(), url: url.clone(), page: url, published, duration: 600.0, video: true, ..Default::default() }
    }

    #[test]
    fn cache_keeps_marks_and_forgets_old_items() {
        let (store, dir) = temp_store("cache");
        let easy = source("it-easy").unwrap();
        let first = [found("aaaaaaaaaaa", "Super Easy Italian 9", 300), found("bbbbbbbbbbb", "Italians Disagree", 200)];
        assert_eq!(store.save(easy, &first, 1000).unwrap(), 2);
        let feed = store.list("it", "fr", &[]).unwrap();
        assert_eq!(feed.items.len(), 2);
        assert_eq!(feed.items[0].title, "Super Easy Italian 9");
        assert_eq!((feed.items[0].lo, feed.items[0].hi), (A1, A1));
        assert_eq!((feed.items[1].lo, feed.items[1].hi), (A2, B1));
        assert_eq!(feed.items[0].shelf, "learn");
        assert_eq!(feed.items[0].kind, "video");
        assert_eq!(feed.items[0].source_name, "Easy Italian");

        // importée hors de Découvrir : reconnue par sa page ; une leçon supprimée ne compte plus
        let lessons = vec![(7, "https://www.youtube.com/watch?v=bbbbbbbbbbb".to_string())];
        let a = feed.items[0].id.clone();
        store.mark(&a, 42).unwrap();
        let feed = store.list("it", "fr", &lessons).unwrap();
        assert_eq!(feed.items[0].lesson_id, None, "leçon 42 supprimée");
        assert_eq!(feed.items[1].lesson_id, Some(7));
        store.mark(&a, 7).unwrap();
        assert_eq!(store.list("it", "fr", &lessons).unwrap().items[0].lesson_id, Some(7));

        // le lendemain : un nouvel élément ; celui que la chaîne ne liste plus reste (une leçon pour
        // apprenants ne vieillit pas) ; la date et la première apparition restent
        let second = [found("ccccccccccc", "Nuovo video", 400), found("aaaaaaaaaaa", "Super Easy Italian 9", 999)];
        assert_eq!(store.save(easy, &second, 2000).unwrap(), 1);
        let feed = store.list("it", "fr", &[]).unwrap();
        assert_eq!(feed.items.len(), 3);
        let kept = feed.items.iter().find(|i| i.id == a).unwrap();
        assert_eq!((kept.published, kept.fetched_at), (300, 1000));

        // titres sans date (YouTube dans la langue de la chaîne) : une vidéo nouvelle date du jour où Lumen la voit
        let third = [found("ddddddddddd", "Senza data", 0), found("ccccccccccc", "Nuovo video", 400)];
        store.save(easy, &third, 2500).unwrap();
        let fresh = store.list("it", "fr", &[]).unwrap();
        assert_eq!(fresh.items[0].title, "Senza data");
        assert_eq!(fresh.items[0].published, 2500);
        store.save(easy, &second, 2600).unwrap();
        let feed = store.list("it", "fr", &[]).unwrap();

        // masqué : ne revient pas, même relu
        let c = feed.items.iter().find(|i| i.title == "Nuovo video").unwrap().id.clone();
        store.hide(&c).unwrap();
        store.save(easy, &second, 3000).unwrap();
        assert!(store.list("it", "fr", &[]).unwrap().items.iter().all(|i| i.id != c));
        assert!(store.list("es", "fr", &[]).unwrap().items.is_empty());

        // un an plus tard : ce que la chaîne ne liste plus s'en va, sauf la leçon créée
        let later = 2600 + 365 * 86400;
        store.save(easy, &[found("eeeeeeeeeee", "Ancora", later)], later).unwrap();
        let ids: Vec<String> = store.list("it", "fr", &lessons).unwrap().items.into_iter().map(|i| i.title).collect();
        assert_eq!(ids, ["Ancora", "Super Easy Italian 9"]);

        // actualités : cinq jours, et vingt-quatre éléments au plus par source
        let ansa = source("it-ansa").unwrap();
        let many: Vec<Found> = (0..30).map(|i| found(&format!("news{i:07}"), &format!("Notizia {i}"), later - i * 3600)).collect();
        store.save(ansa, &many, later).unwrap();
        assert_eq!(store.list("it", "fr", &[]).unwrap().items.iter().filter(|i| i.source == "it-ansa").count(), 24);

        // rythme de lecture par source : relue si son tour est passé, ou à la demande après cinq minutes
        assert!(store.due("it", "fr"));
        let t = now();
        for s in sources("it", "fr") {
            store.set_state(s.id, &SrcState { checked: t, ok: t, ..Default::default() }).unwrap();
        }
        assert!(!store.due("it", "fr"));
        assert!(!is_due(easy, &store.src_state(easy.id), t + 60, true));
        assert!(is_due(easy, &store.src_state(easy.id), t + FRESH, true));
        assert!(is_due(ansa, &store.src_state(ansa.id), t + 3 * 3600, false));
        assert!(!is_due(easy, &store.src_state(easy.id), t + 3 * 3600, false));
        // après des échecs, on attend davantage
        let failing = SrcState { checked: t, fails: 3, ..Default::default() };
        assert!(!is_due(ansa, &failing, t + 2 * 3600, false));
        assert!(is_due(ansa, &failing, t + 3 * 3600 + 1, false));
        assert!(store.list("it", "fr", &[]).unwrap().refreshed_at > 0);

        // chansons : paroles vérifiées gardées ; sans paroles, revérifiées plus tard
        store.set_song("vid00000001", true, "Lumi", "La strada").unwrap();
        assert_eq!(store.song("vid00000001"), Some((true, "Lumi".into(), "La strada".into())));
        store.set_song("vid00000002", false, "", "").unwrap();
        assert_eq!(store.song("vid00000002"), Some((false, String::new(), String::new())));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Un cache d'une version précédente (copie) s'ouvre et se met à niveau :
    /// `LUMEN_DISCOVER_DB=…/discover.db cargo test --lib old_cache_upgrades -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn old_cache_upgrades() {
        let src = std::path::PathBuf::from(std::env::var("LUMEN_DISCOVER_DB").expect("LUMEN_DISCOVER_DB"));
        let dir = std::env::temp_dir().join(format!("lumen-discover-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&src, dir.join("discover.db")).unwrap();
        let store = Store::open(&dir);
        for lang in ["it", "ru"] {
            let feed = store.list(lang, "fr", &[]).unwrap();
            println!("{lang} : {} éléments, lu le {}, à relire : {}", feed.items.len(), feed.refreshed_at, store.due(lang, "fr"));
            assert!(!feed.items.is_empty());
        }
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_cache_is_rebuilt() {
        let dir = std::env::temp_dir().join(format!("lumen-discover-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("discover.db"), b"ceci n'est pas une base").unwrap();
        let store = Store::open(&dir);
        assert!(store.list("it", "fr", &[]).unwrap().items.is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Lecture complète d'une langue comme dans l'app (cache jetable), puis ce que
    /// Découvrir proposerait à chaque niveau :
    /// `LUMEN_DISCOVER=it cargo test --lib discover_refresh_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn discover_refresh_live() {
        let lang = std::env::var("LUMEN_DISCOVER").unwrap_or_else(|_| "it".into());
        let dir = std::env::temp_dir().join(format!("lumen-discover-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir);
        let mut log = |e: ImportEvent| {
            if let ImportEvent::Stage { stage } = e {
                println!("étape : {stage}");
            }
        };
        let started = std::time::Instant::now();
        let r = refresh(&dir, &store, &lang, "fr", true, &mut log).await.unwrap();
        println!("{} nouveaux éléments, {} sources, sans réponse : {:?} ({:.1} s)", r.added, r.sources, r.failed, started.elapsed().as_secs_f64());
        let feed = store.list(&lang, "fr", &[]).unwrap();
        assert!(feed.refreshed_at > 0 && !feed.items.is_empty());
        for level in A1..=C1 {
            let at: Vec<&Item> = feed.items.iter().filter(|i| i.lo <= level && level <= i.hi).collect();
            let by = |shelf: &str| at.iter().filter(|i| i.shelf == shelf).count();
            println!("niveau {level} : {:>3} éléments (apprendre {}, actualités {}, culture {})", at.len(), by("learn"), by("news"), by("culture"));
        }
        let undated = feed.items.iter().filter(|i| i.published == 0).count();
        println!("sans date : {undated} sur {}", feed.items.len());
        // relire aussitôt ne relit rien
        assert!(refresh(&dir, &store, &lang, "fr", true, &mut |_| {}).await.unwrap().skipped);
        let music: Vec<&Item> = feed.items.iter().filter(|i| i.shelf == "music").collect();
        println!("chansons avec paroles dans la langue : {}", music.len());
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Relit les sources pour de vrai (toutes, ou `LUMEN_DISCOVER=it,de`) et
    /// affiche ce qu'elles proposent : `cargo test --lib discover_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn discover_live() {
        let only = std::env::var("LUMEN_DISCOVER").unwrap_or_default();
        // quelques sources seulement, par leurs identifiants
        let ids = std::env::var("LUMEN_DISCOVER_IDS").unwrap_or_default();
        let langs: Vec<&str> = if only.is_empty() { crate::text::LANGS.to_vec() } else { only.split(',').collect() };
        let dir = std::env::temp_dir().join("lumen-discover-live");
        std::fs::create_dir_all(&dir).unwrap();
        let ytdlp = crate::tools::find_ytdlp(&dir);
        let client = link::client().unwrap();
        let store = Store::open(&dir);
        let only_kind = std::env::var("LUMEN_DISCOVER_KIND").unwrap_or_default();
        let mut broken = Vec::new();
        for lang in langs {
            for s in SOURCES.iter().filter(|s| s.lang == lang) {
                if only_kind == "music" && s.kind != Kind::Chart || only_kind == "other" && s.kind == Kind::Chart {
                    continue;
                }
                if !ids.is_empty() && !ids.split(',').any(|i| i == s.id) {
                    continue;
                }
                let r = fetch(&dir, ytdlp.as_deref(), &client, &store, s, &SrcState::default(), true).await;
                if matches!(s.kind, Kind::YouTube | Kind::Chart) {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                }
                match r {
                    Ok(Fetched::Items(f, _)) if !f.is_empty() => {
                        let newest = f.iter().map(|x| x.published).max().unwrap_or(0);
                        let age = if newest > 0 { (now() - newest) / 86400 } else { -1 };
                        let first = &f[0];
                        let (lo, hi) = grade(s, &first.title);
                        // une chanson : artiste et titre seulement (jamais ses paroles)
                        let label = if s.kind == Kind::Chart { format!("{} – {}", first.artist, first.track) } else { first.title.clone() };
                        // titres et résumés reconnus dans la langue de la source (une source surtout en anglais se repère)
                        let judged: Vec<bool> = f.iter().filter_map(|x| langid::matches(&format!("{}. {}", x.title, x.summary), s.lang)).collect();
                        let share = if judged.is_empty() { -1 } else { (judged.iter().filter(|b| **b).count() * 100 / judged.len()) as i64 };
                        println!(
                            "OK  {:<18} {:>2} él. · {:>4} j · img {} · {}-{} · langue {:>3} % · {}",
                            s.id,
                            f.len(),
                            age,
                            if first.image.is_empty() { "non" } else { "oui" },
                            lo,
                            hi,
                            share,
                            label.chars().take(60).collect::<String>()
                        );
                    }
                    Ok(_) => {
                        println!("!!  {:<18} vide", s.id);
                        broken.push(s.id);
                    }
                    Err(e) => {
                        println!("!!  {:<18} {e}", s.id);
                        broken.push(s.id);
                    }
                }
            }
        }
        println!("\nsans réponse : {broken:?}");
    }
}
