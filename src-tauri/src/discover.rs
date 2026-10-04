//! Découvrir : des leçons venues d'ailleurs. Une fois par jour, Lumen lit
//! quelques sources choisies pour chaque langue (chaînes YouTube pour
//! apprenants, podcasts, actualités faciles ou ordinaires, vulgarisation) et
//! range leurs nouveautés par niveau, de A1 à C1. Rien n'est téléchargé avant
//! que l'apprenant choisisse : la vidéo, l'épisode ou l'article devient une
//! leçon par l'import habituel (`import_link`, ou l'article lu par Readability).
//!
//! Le catalogue est écrit ici, source par source, avec sa fourchette de
//! niveaux ; les titres des contenus pour apprenants l'affinent (« for
//! Beginners (A1-A2) », « Intermediate »). Les éléments trouvés vivent dans
//! `discover.db`, à part de la progression : un simple cache, ni sauvegardé ni
//! compté comme un changement de la base (la sauvegarde suit `total_changes`).

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
use crate::link;
use crate::media::{self, ImportEvent};

// ---------- niveaux ----------

pub const A1: u8 = 1;
pub const A2: u8 = 2;
pub const B1: u8 = 3;
pub const B2: u8 = 4;
pub const C1: u8 = 5;

/// Éléments gardés par source à chaque lecture (les plus récents).
const PER_SOURCE: usize = 12;
/// Une langue est relue une fois par jour (un peu moins de 24 h, pour suivre l'heure du lancement).
const EVERY: i64 = 20 * 3600;
/// Après une lecture où rien n'a répondu (hors ligne), nouvel essai au plus tôt…
const RETRY: i64 = 50 * 60;
/// Une actualisation demandée juste après une autre ne relit rien.
const FRESH: i64 = 90;
/// Vidéos gardées : ni bandes-annonces ni directs de plusieurs heures.
const MIN_SECS: f64 = 90.0;
const MAX_SECS: f64 = 2.0 * 3600.0;

// ---------- catalogue ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    YouTube,
    Podcast,
    Articles,
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
}

impl Shelf {
    fn code(self) -> &'static str {
        match self {
            Shelf::Learn => "learn",
            Shelf::News => "news",
            Shelf::Culture => "culture",
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
    // ----- danois -----
    yt("da-ci", "da", "Danish Comprehensible Input", "UCgDQm18b9DmEFJou0xLt6jg", A1, B1, Learn),
    yt("da-conv", "da", "Danish Conversations", "UCKWa0YHD4uXoG9_AXyB1CxQ", B1, B2, Learn),
    yt("da-essensen", "da", "P3 Essensen", "UC8tSbn8Q4rnsSQPY-1kzXuw", C1, C1, News),
    art("da-dr", "da", "DR Nyheder", "https://www.dr.dk/nyheder/service/feeds/allenyheder", C1, C1, News),
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
    // ----- letton -----
    yt("lv-lva", "lv", "Latviešu valodas aģentūra", "UC24idzqmWOwTIURmxnXXdsg", A2, B1, Learn),
    yt("lv-ltv", "lv", "LTV Ziņu dienests", "UCOSAAyJoybqsY5sZ76BaqFA", C1, C1, News),
    art("lv-lsm", "lv", "LSM", "https://www.lsm.lv/rss/", C1, C1, News),
    // ----- lituanien -----
    yt("lt-paulius", "lt", "Lithuanian with Paulius", "UCIoF257Ir2im5lVegjhjSGA", A2, B1, Learn),
    yt("lt-lrt", "lt", "LRT", "UC4KnMZaxcv1KZDAJsgHXcOA", C1, C1, News),
    art("lt-lrt-art", "lt", "LRT", "https://www.lrt.lt/?rss", C1, C1, News),
    art("lt-15min", "lt", "15min", "https://www.15min.lt/rss", C1, C1, News),
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
    // ----- slovaque -----
    yt("sk-stories", "sk", "Learn Slovak with Stories", "UCUPgmAhUy6NkMxzRTmks55w", A1, B1, Learn),
    yt("sk-aktuality", "sk", "Aktuality.sk", "UC2lCFhJIC4adt_oP1dwSOVg", C1, C1, News),
    art("sk-aktuality-art", "sk", "Aktuality.sk", "https://www.aktuality.sk/rss/", C1, C1, News),
    // ----- slovène -----
    yt("sl-dialog", "sl", "Slovenščina skozi dialog", "UCrUDbUp04kcr3rA1fwlP9dw", A2, B1, Learn),
    art("sl-rtv", "sl", "RTV SLO", "https://img.rtvslo.si/feeds/00.xml", C1, C1, News),
    // ----- croate -----
    yt("hr-hrt", "hr", "HRT vijesti", "UCSI1vb6CELskEFQpRceUj0g", C1, C1, News),
    art("hr-index", "hr", "Index.hr", "https://www.index.hr/rss", C1, C1, News),
    // ----- hongrois -----
    yt("hu-heart", "hu", "Hungarian by Heart", "UCprH3w8hVn0aaAE6wGlXzUw", A2, B1, Learn),
    yt("hu-telex", "hu", "Telex", "UCM-1sd-cXSuCsfWp8QMY_OQ", C1, C1, News),
    art("hu-telex-art", "hu", "Telex", "https://telex.hu/rss", C1, C1, News),
    // ----- roumain -----
    yt("ro-digi", "ro", "Digi24", "UCbvKamSrJkwT6ed2BMMZXwg", C1, C1, News),
    art("ro-digi-art", "ro", "Digi24", "https://www.digi24.ro/rss", C1, C1, News),
    // ----- bulgare -----
    yt("bg-az", "bg", "Аз говоря български", "UC7R7TQDHdgPVnb0YxbPz1bA", A2, B1, Learn),
    yt("bg-bnt", "bg", "БНТ", "UC8jnuRbBzMICRHsC0qqVd9Q", C1, C1, News),
    art("bg-dnevnik", "bg", "Dnevnik", "https://www.dnevnik.bg/rss/", C1, C1, News),
    // ----- ukrainien -----
    yt("uk-hanna", "uk", "Immersive Ukrainian with Hanna", "UCTJzD-YVoDGolkN7lQ4ZasQ", A2, B1, Learn),
    yt("uk-slow", "uk", "Slow Ukrainian", "UCqGyrVsLBUk2FcGWGfN14SQ", A2, B1, Learn),
    yt("uk-bbc", "uk", "BBC News Україна", "UCZctsW8Tpx8Tz9Ln4KUmR3g", C1, C1, News),
    yt("uk-suspilne", "uk", "Суспільне Новини", "UCPY6gj8G7dqwPxg9KwHrj5Q", C1, C1, News),
    art("uk-bbc-art", "uk", "BBC News Україна", "https://www.bbc.com/ukrainian/index.xml", C1, C1, News),
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
    // ----- vietnamien -----
    yt("vi-lilian", "vi", "Lilian Vietnamese", "UC4di2z7dbPra5xhp6BN7XwQ", A1, B1, Learn),
    yt("vi-understand", "vi", "Actually Understand Vietnamese", "UCiJJCAigdFRvR1CfH25IIbg", A2, B1, Learn),
    yt("vi-bbc", "vi", "BBC News Tiếng Việt", "UCpoNfKwZbecrcFpzm0ET4uw", C1, C1, News),
    art("vi-bbc-art", "vi", "BBC News Tiếng Việt", "https://www.bbc.com/vietnamese/index.xml", C1, C1, News),
    art("vi-vnexpress", "vi", "VnExpress", "https://vnexpress.net/rss/tin-moi-nhat.rss", C1, C1, News),
    // ----- coréen -----
    yt("ko-ttmik", "ko", "Talk To Me In Korean", "UC5r3WHrX4Z7peSYpDlgktGw", A1, B1, Learn).via("en"),
    yt("ko-taewoong", "ko", "태웅쌤 Comprehensible Input Korean", "UC737T1zTN6MQ1uWorHVAXvA", A1, B1, Learn),
    yt("ko-ttmik100", "ko", "Talk To Me In 100% Korean", "UCX6NVksmz8GzQsqYy24DWlA", A2, B1, Learn),
    yt("ko-immersion", "ko", "몰입한국어", "UC-3wHyVaLCiujjjPYXtif5w", B1, B2, Learn),
    yt("ko-kurz", "ko", "한눈에 보는 세상 – Kurzgesagt", "UC8rKCy_tipwTEY3RdkNCKmw", B2, C1, Culture),
    yt("ko-bbc", "ko", "BBC News 코리아", "UCIDOGTbwTBHZ5YoR9Xjcp-w", C1, C1, News),
    art("ko-bbc-art", "ko", "BBC News 코리아", "https://feeds.bbci.co.uk/korean/rss.xml", C1, C1, News),
    art("ko-yonhap", "ko", "연합뉴스", "https://www.yna.co.kr/rss/news.xml", C1, C1, News),
    // ----- japonais -----
    yt("ja-jikan", "ja", "にほんごのじかん", "UCdZHET-9_Comx6UaVTSETiQ", A1, B1, Learn),
    yt("ja-teppei", "ja", "Teppei", "UCH88l3_ltyJm67gAFzDFNRw", A2, B1, Learn),
    yt("ja-easypod", "ja", "EASY JAPANESE PODCAST", "UC16-9M0osgdFbLKXvCwpXaw", B1, B2, Learn),
    yt("ja-kurz", "ja", "世界をわかりやすく – Kurzgesagt", "UCzw2KK537iRgsrYnWaEMs8Q", B2, C1, Culture),
    yt("ja-teded", "ja", "好奇心を持ち続けよう – TED-Ed", "UCwFlWUGyXPHdsRAgmFxG_jw", B2, C1, Culture),
    pod("ja-teppei-pod", "ja", "Nihongo con Teppei", "http://nihongoconteppei.com/feed/podcast", A2, B1, Learn),
    art("ja-nhk", "ja", "NHK ニュース", "https://news.web.nhk/n-data/conf/na/rss/cat0.xml", C1, C1, News),
    art("ja-bbc", "ja", "BBC News Japan", "https://feeds.bbci.co.uk/japanese/rss.xml", C1, C1, News),
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
fn title_levels(title: &str) -> Option<(u8, u8)> {
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
}

/// Dernières vidéos d'une chaîne, telles que yt-dlp les liste (`media::yt_latest`).
fn youtube_entries(v: &Value) -> Vec<Found> {
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
            if !(MIN_SECS..=MAX_SECS).contains(&duration) {
                return None;
            }
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
            Some(Found {
                key: id.into(),
                title: title.into(),
                url: url.clone(),
                page: url,
                image,
                duration,
                published,
                video: true,
                ..Default::default()
            })
        })
        .take(PER_SOURCE)
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

async fn channel(data_dir: &Path, ytdlp: &Path, url: &str, lang: Option<&str>) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(60), media::yt_latest(data_dir, ytdlp, url, PER_SOURCE + 6, lang))
        .await
        .map_err(|_| anyhow!(t("la chaîne ne répond pas", "the channel doesn't answer")))?
}

/// Ce qu'une source propose aujourd'hui. `first` : première lecture de cette source.
async fn fetch(data_dir: &Path, ytdlp: Option<&Path>, client: &reqwest::Client, src: &Source, first: bool) -> Result<Vec<Found>> {
    match src.kind {
        Kind::YouTube => {
            let ytdlp = ytdlp.ok_or_else(|| anyhow!(t("composants vidéo indisponibles", "video components unavailable")))?;
            let url = format!("https://www.youtube.com/channel/{}/videos", src.url);
            let english = src.lang == "en";
            let mut found = youtube_entries(&channel(data_dir, ytdlp, &url, (!english).then_some(src.lang)).await?);
            // les titres dans la langue de la chaîne n'ont pas de date : à la première lecture, une
            // seconde liste (en anglais) les date ; ensuite, une vidéo nouvelle date du jour où Lumen la voit
            if first && !english && !found.is_empty() {
                if let Ok(v) = channel(data_dir, ytdlp, &url, None).await {
                    let dates: HashMap<String, i64> = youtube_entries(&v).into_iter().map(|f| (f.key, f.published)).collect();
                    for f in &mut found {
                        f.published = dates.get(&f.key).copied().unwrap_or(0);
                    }
                }
            }
            Ok(found)
        }
        Kind::Podcast | Kind::Articles => {
            let xml = tokio::time::timeout(Duration::from_secs(40), link::get_text(client, src.url))
                .await
                .map_err(|_| anyhow!(t("le flux ne répond pas", "the feed doesn't answer")))??;
            Ok(parse_feed(&xml, src.url, src.kind == Kind::Podcast))
        }
    }
}

async fn fetch_one(
    data_dir: &Path,
    ytdlp: Option<&Path>,
    client: &reqwest::Client,
    youtube: &tokio::sync::Semaphore,
    src: &'static Source,
    first: bool,
) -> (&'static Source, Result<Vec<Found>>) {
    if src.kind != Kind::YouTube {
        return (src, fetch(data_dir, ytdlp, client, src, first).await);
    }
    let _slot = youtube.acquire().await;
    let r = fetch(data_dir, ytdlp, client, src, first).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    (src, r)
}

// ---------- cache ----------

/// Un élément proposé à l'apprenant.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub source: String,
    pub source_name: String,
    /// "learn", "news" ou "culture"
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
    /// une lecture venait de se faire : rien n'a été relu
    pub skipped: bool,
}

pub struct Store {
    conn: Mutex<Connection>,
    /// une lecture à la fois, toutes langues confondues (YouTube n'aime pas les rafales)
    gate: tokio::sync::Mutex<()>,
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
"#;

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
                Ok::<_, anyhow::Error>(c)
            })
            .expect("base en mémoire");
        Store { conn: Mutex::new(conn), gate: tokio::sync::Mutex::new(()), running: Mutex::new(HashMap::new()) }
    }

    fn connect(path: &Path) -> Result<Connection> {
        let c = Connection::open(path)?;
        c.execute_batch(SCHEMA)?;
        Ok(c)
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

    /// La langue attend sa lecture du jour.
    pub fn due(&self, lang: &str) -> bool {
        let (at, tried) = self.run(lang);
        let n = now();
        n - at >= EVERY && n - tried >= RETRY
    }

    /// Range ce qu'une source propose aujourd'hui ; ce qu'elle ne propose plus
    /// s'en va. Renvoie le nombre d'éléments nouveaux.
    fn save(&self, src: &Source, found: &[Found], at: i64) -> Result<usize> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        let known_source = tx.query_row("SELECT 1 FROM items WHERE source=?1 LIMIT 1", [src.id], |_| Ok(())).optional()?.is_some();
        let mut keep = HashSet::new();
        let mut added = 0;
        for (rank, f) in found.iter().enumerate() {
            let id = item_id(src.id, &f.key);
            if !keep.insert(id.clone()) {
                continue;
            }
            let known = tx.query_row("SELECT 1 FROM items WHERE id=?1", [&id], |_| Ok(())).optional()?.is_some();
            if !known {
                added += 1;
            }
            let (lo, hi) = grade(src, &f.title);
            // sans date dans la source : un élément apparu depuis la dernière lecture (quotidienne) date d'aujourd'hui
            let published = if f.published == 0 && known_source && !known { at } else { f.published };
            let kind = match src.kind {
                Kind::YouTube => "video",
                Kind::Podcast if f.video => "video",
                Kind::Podcast => "audio",
                Kind::Articles => "text",
            };
            // une date approximative (« il y a 3 jours ») est plus juste à la première lecture : on la garde
            tx.execute(
                "INSERT INTO items(id,lang,source,kind,title,url,page,image,summary,duration,published,lo,hi,rank,fetched_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                 ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, title=excluded.title, url=excluded.url, page=excluded.page,
                   image=excluded.image, summary=excluded.summary, duration=excluded.duration,
                   published=CASE WHEN items.published > 0 THEN items.published ELSE excluded.published END,
                   lo=excluded.lo, hi=excluded.hi, rank=excluded.rank",
                params![id, src.lang, src.id, kind, f.title, f.url, f.page, f.image, f.summary, f.duration, published, lo, hi, rank as i64, at],
            )?;
        }
        let old: Vec<String> = {
            let mut st = tx.prepare("SELECT id FROM items WHERE source=?1")?;
            let rows = st.query_map([src.id], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).filter(|id| !keep.contains(id)).collect()
        };
        for id in old {
            tx.execute("DELETE FROM items WHERE id=?1", [id])?;
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
            "SELECT id,source,kind,title,url,page,image,summary,duration,published,lo,hi,fetched_at,lesson_id
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
            ))
        })?;
        let mut items = Vec::new();
        for row in rows {
            let (id, sid, kind, title, url, page, image, summary, duration, published, lo, hi, fetched_at, lesson) = row?;
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
            });
        }
        drop(st);
        drop(c);
        Ok(Feed { items, refreshed_at: self.run(lang).0, refreshing: self.refreshing(lang), sources: sources(lang, ui).count() })
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

/// Relit les sources d'une langue. `on_event` : installation des composants
/// vidéo s'il le faut (étape « tools »), puis avancement (0 à 100).
pub async fn refresh(data_dir: &Path, store: &Store, lang: &str, ui: &str, on_event: &mut (dyn FnMut(ImportEvent) + Send)) -> Result<Report> {
    let _busy = Busy::new(store, lang);
    let _turn = store.gate.lock().await;
    let started = now();
    let (at, _) = store.run(lang);
    if started - at < FRESH {
        return Ok(Report { skipped: true, ..Default::default() });
    }
    let list: Vec<&'static Source> = sources(lang, ui).collect();
    if list.is_empty() {
        return Ok(Report::default());
    }
    let ytdlp = if list.iter().any(|s| s.kind == Kind::YouTube) {
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
    // deux chaînes YouTube à la fois au plus, avec une pause entre deux : sans quoi YouTube refuse un moment
    let youtube = tokio::sync::Semaphore::new(2);
    let jobs: Vec<_> = list.iter().map(|&s| fetch_one(data_dir, ytdlp.as_deref(), &client, &youtube, s, !store.has(s.id))).collect();
    let mut results = stream::iter(jobs).buffer_unordered(6);

    let total = list.len();
    let mut report = Report { sources: total, ..Default::default() };
    let (mut done, mut answered) = (0, 0);
    while let Some((s, r)) = results.next().await {
        done += 1;
        match r {
            Ok(found) if !found.is_empty() => {
                report.added += store.save(s, &found, started)?;
                answered += 1;
            }
            // un flux vide ou illisible garde ce qu'il proposait hier
            _ => report.failed.push(s.name.to_string()),
        }
        on_event(ImportEvent::Progress { value: done as f64 / total as f64 * 100.0 });
    }
    store.set_run(lang, if answered > 0 { started } else { at }, started)?;
    Ok(report)
}

/// Lecture quotidienne, en arrière-plan, des langues étudiées (la langue
/// active d'abord). Un événement `discover` suit chaque langue relue.
pub async fn auto_loop(app: tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    tokio::time::sleep(Duration::from_secs(40)).await;
    loop {
        let st = app.state::<crate::state::AppState>();
        let (on, langs): (bool, Vec<String>) = {
            let c = st.db.lock();
            // lecture quotidienne coupée dans les Réglages : seulement sur demande
            let on = crate::db::setting(&c, "discover_auto").as_deref() != Some("0");
            let active = crate::db::setting(&c, "lang").unwrap_or_default();
            let all = crate::db::setting(&c, "langs").unwrap_or_default();
            let mut v: Vec<String> = std::iter::once(active.as_str()).chain(all.split(',')).filter(|l| !l.is_empty()).map(str::to_string).collect();
            let mut seen = HashSet::new();
            v.retain(|l| seen.insert(l.clone()));
            (on, v)
        };
        for lang in langs.into_iter().filter(|_| on) {
            if st.discover.due(&lang) && sources(&lang, crate::i18n::native()).next().is_some() {
                let mut quiet = |_e: ImportEvent| {};
                let _ = refresh(&st.data_dir, &st.discover, &lang, crate::i18n::native(), &mut quiet).await;
                let _ = app.emit("discover", &lang);
            }
        }
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
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
        let f = youtube_entries(&v);
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

        // le lendemain : un nouvel élément, un ancien parti ; la date et la première apparition restent
        let second = [found("ccccccccccc", "Nuovo video", 400), found("aaaaaaaaaaa", "Super Easy Italian 9", 999)];
        assert_eq!(store.save(easy, &second, 2000).unwrap(), 1);
        let feed = store.list("it", "fr", &[]).unwrap();
        assert_eq!(feed.items.len(), 2);
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

        // lecture du jour
        assert!(store.due("it"));
        store.set_run("it", now(), now()).unwrap();
        assert!(!store.due("it"));
        assert!(store.list("it", "fr", &[]).unwrap().refreshed_at > 0);
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
        let r = refresh(&dir, &store, &lang, "fr", &mut log).await.unwrap();
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
        assert!(refresh(&dir, &store, &lang, "fr", &mut |_| {}).await.unwrap().skipped);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Relit les sources pour de vrai (toutes, ou `LUMEN_DISCOVER=it,de`) et
    /// affiche ce qu'elles proposent : `cargo test --lib discover_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn discover_live() {
        let only = std::env::var("LUMEN_DISCOVER").unwrap_or_default();
        let langs: Vec<&str> = if only.is_empty() { crate::text::LANGS.to_vec() } else { only.split(',').collect() };
        let dir = std::env::temp_dir().join("lumen-discover-live");
        std::fs::create_dir_all(&dir).unwrap();
        let ytdlp = crate::tools::find_ytdlp(&dir);
        let client = link::client().unwrap();
        let mut broken = Vec::new();
        for lang in langs {
            for s in SOURCES.iter().filter(|s| s.lang == lang) {
                let r = fetch(&dir, ytdlp.as_deref(), &client, s, true).await;
                if s.kind == Kind::YouTube {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                }
                match r {
                    Ok(f) if !f.is_empty() => {
                        let newest = f.iter().map(|x| x.published).max().unwrap_or(0);
                        let age = if newest > 0 { (now() - newest) / 86400 } else { -1 };
                        let first = &f[0];
                        let (lo, hi) = grade(s, &first.title);
                        println!(
                            "OK  {:<18} {:>2} él. · {:>4} j · img {} · {}-{} · {}",
                            s.id,
                            f.len(),
                            age,
                            if first.image.is_empty() { "non" } else { "oui" },
                            lo,
                            hi,
                            first.title.chars().take(70).collect::<String>()
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
