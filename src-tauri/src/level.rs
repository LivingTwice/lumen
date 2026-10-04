//! Niveau estimé d'un apprenant dans une langue, pour Découvrir. Les mots connus
//! sont enregistrés tels qu'ils apparaissent dans les textes : « parlo »,
//! « parlava », « parlato » comptent trois fois. Les langues n'ont pas toutes
//! autant de formes par mot (le russe ou le finnois bien plus que l'anglais) :
//! on compte donc les lemmes (mots de base), grâce aux formes fléchies des
//! dictionnaires hors ligne, et les seuils sont les mêmes pour toutes les langues.
//! Japonais (découpé caractère par caractère) : les kanji ; vietnamien (syllabe
//! par syllabe) : les syllabes.

use std::collections::HashMap;

use serde::Serialize;

use crate::dict::{self, Lemmatizer};

/// Lemmes connus au début de A2, B1, B2, C1. Les mots marqués connus en lisant
/// (comme sur LingQ) comptent aussi ceux qu'on reconnaît sans les maîtriser : les
/// seuils sont un peu plus hauts que les tailles de vocabulaire des manuels.
const LEMMAS: [i64; 4] = [1000, 2200, 4000, 7000];
/// Kanji connus (repères du JLPT : N5 ≈ 100, N4 ≈ 300, N3 ≈ 650, N2 ≈ 1 000).
const KANJI: [i64; 4] = [120, 350, 700, 1100];
/// Syllabes vietnamiennes connues (un mot en compte une ou deux).
const SYLLABLES: [i64; 4] = [400, 900, 1600, 2500];

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Estimate {
    /// mots connus, tels qu'enregistrés (chaque forme compte)
    pub forms: i64,
    /// ce qui sert au niveau : lemmes, kanji ou syllabes
    pub units: i64,
    /// "lemmas", "kanji" ou "syllables"
    pub unit: String,
    /// calculé avec un dictionnaire (sinon approché d'après la langue)
    pub exact: bool,
    /// 1 (A1) à 5 (C1)
    pub level: u8,
    /// début du niveau atteint, et seuil du suivant (0 au plus haut)
    pub floor: i64,
    pub next: i64,
}

/// Formes par lemme, en moyenne, dans un vocabulaire d'apprenant (mesuré sur des
/// vocabulaires réels) : sert tant que le dictionnaire de la langue n'est pas là.
pub fn forms_per_lemma(lang: &str) -> f64 {
    match lang {
        "en" | "id" | "hi" => 1.25,
        "es" | "it" | "pt" | "fr" | "nl" | "sv" | "da" => 1.4,
        "de" | "ro" | "bg" => 1.5,
        "el" | "ar" | "ko" => 1.8,
        "pl" | "cs" | "sk" | "sl" | "hr" | "uk" | "ru" | "lv" | "lt" => 2.2,
        "fi" | "et" | "hu" | "tr" => 2.5,
        _ => 1.0,
    }
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' | '\u{f900}'..='\u{faff}')
}

fn place(units: i64, steps: &[i64; 4]) -> (u8, i64, i64) {
    let reached = steps.iter().filter(|s| units >= **s).count();
    let floor = if reached == 0 { 0 } else { steps[reached - 1] };
    let next = steps.get(reached).copied().unwrap_or(0);
    (reached as u8 + 1, floor, next)
}

/// Niveau d'après les mots connus : (mot, lemme enregistré s'il y en a un).
pub fn estimate(lang: &str, known: &[(String, String)], lemmatizer: Option<&Lemmatizer>) -> Estimate {
    let forms = known.len() as i64;
    let (units, unit, exact, steps) = match lang {
        "ja" => {
            let mut kanji: Vec<char> = known.iter().flat_map(|(w, _)| w.chars()).filter(|c| is_han(*c)).collect();
            kanji.sort_unstable();
            kanji.dedup();
            (kanji.len() as i64, "kanji", true, &KANJI)
        }
        "vi" => (forms, "syllables", true, &SYLLABLES),
        _ => match lemmatizer {
            Some(lem) => {
                // lemme → au moins une de ses formes est connue du dictionnaire
                let mut lemmas: HashMap<String, bool> = HashMap::new();
                for (word, saved) in known {
                    let (l, attested) = if !saved.trim().is_empty() {
                        (dict::key(saved, lang), true)
                    } else {
                        match lem.lemma(word) {
                            Some(l) => (l, true),
                            None => (dict::key(word, lang), false),
                        }
                    };
                    let e = lemmas.entry(l).or_insert(false);
                    *e |= attested;
                }
                // un mot que les dictionnaires ignorent (nom propre, forme rare) compte pour moitié
                let sure = lemmas.values().filter(|a| **a).count() as f64;
                let unsure = lemmas.len() as f64 - sure;
                ((sure + unsure * 0.5).round() as i64, "lemmas", true, &LEMMAS)
            }
            None => ((forms as f64 / forms_per_lemma(lang)).round() as i64, "lemmas", false, &LEMMAS),
        },
    };
    let (level, floor, next) = place(units, steps);
    Estimate { forms, units, unit: unit.into(), exact, level, floor, next }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<(String, String)> {
        list.iter().map(|w| (w.to_string(), String::new())).collect()
    }

    #[test]
    fn thresholds() {
        assert_eq!(place(0, &LEMMAS), (1, 0, 1000));
        assert_eq!(place(1000, &LEMMAS), (2, 1000, 2200));
        assert_eq!(place(6999, &LEMMAS), (4, 4000, 7000));
        assert_eq!(place(9000, &LEMMAS), (5, 7000, 0));
    }

    #[test]
    fn without_dictionary() {
        // 3 300 formes russes ≈ 1 500 lemmes : A2, pas B1 comme en comptant les formes
        let many: Vec<(String, String)> = (0..3300).map(|i| (format!("слово{i}"), String::new())).collect();
        let e = estimate("ru", &many, None);
        assert_eq!((e.units, e.level, e.exact), (1500, 2, false));
        // japonais : les kanji distincts
        let e = estimate("ja", &words(&["日", "本", "日", "の", "カメラ"]), None);
        assert_eq!((e.units, e.unit.as_str()), (2, "kanji"));
    }

    /// Niveau estimé pour chaque langue d'une copie des données (lecture seule) :
    /// `LUMEN_LEVEL_DATA="$HOME/Library/Application Support/app.lumen.reader" cargo test --lib level_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn level_live() {
        let data = std::path::PathBuf::from(std::env::var("LUMEN_LEVEL_DATA").expect("LUMEN_LEVEL_DATA"));
        let c = rusqlite::Connection::open_with_flags(data.join("lumen.db"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let dicts = crate::dict::Dicts::new(std::path::PathBuf::from("resources/dicts"), crate::dict::dict_dir(&data));
        for lang in crate::text::LANGS {
            let known = crate::db::known_terms(&c, lang).unwrap();
            if known.is_empty() {
                continue;
            }
            let started = std::time::Instant::now();
            let lem = dicts.lemmatizer(lang);
            let e = estimate(lang, &known, lem.as_ref());
            let old = [800, 2500, 6000, 12000].iter().filter(|s| e.forms >= **s).count() + 1;
            println!(
                "{lang} : {} formes → {} {} ({}) · niveau {} (avant : {old}) · prochain à {} · {:.2} s",
                e.forms,
                e.units,
                e.unit,
                if e.exact { "dictionnaire" } else { "approché" },
                e.level,
                e.next,
                started.elapsed().as_secs_f64()
            );
        }
    }
}
