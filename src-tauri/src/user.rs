//! Profil de l'apprenant : réglages `user_*`, écrits par l'interface
//! (`src/lib/user.ts`). Rien de communautaire : il reste dans la base, sur ce
//! Mac, et voyage avec la sauvegarde. Le chat local le lit pour s'adresser à
//! l'apprenant ; la sauvegarde en garde le nom et l'avatar, pour qu'on
//! reconnaisse la sienne parmi celles de plusieurs Mac.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::ai::Learner;
use crate::db;

const NAME_MAX: usize = 32;
const WHY_MAX: usize = 140;
const INTEREST_MAX: usize = 30;
const INTERESTS_MAX: usize = 8;
/// Une photo plus lourde n'a pas été préparée par Lumen : elle reste hors du manifeste.
const PHOTO_MAX: usize = 200_000;

/// Centres d'intérêt proposés (miroir de `interestSuggestions` dans lib/user.ts).
const INTERESTS: [(&str, &str, &str); 16] = [
    ("#cooking", "cuisine", "cooking"),
    ("#travel", "voyages", "travel"),
    ("#history", "histoire", "history"),
    ("#film", "cinéma et séries", "film and TV"),
    ("#music", "musique", "music"),
    ("#books", "littérature", "books"),
    ("#sport", "sport", "sports"),
    ("#science", "sciences", "science"),
    ("#nature", "nature", "nature"),
    ("#art", "art et design", "art and design"),
    ("#tech", "technologie", "technology"),
    ("#news", "actualité", "current affairs"),
    ("#games", "jeux vidéo", "video games"),
    ("#work", "travail et économie", "work and business"),
    ("#philosophy", "philosophie", "philosophy"),
    ("#health", "santé et bien-être", "health and wellbeing"),
];

/// Une seule ligne, sans espaces superflus, coupée à `max` caractères.
fn one_line(s: &str, max: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Libellé d'un centre d'intérêt : une suggestion (« #cooking ») dans la langue
/// de l'apprenant, sinon le texte qu'il a tapé.
fn interest_label(id: &str, native: &str) -> String {
    if id.starts_with('#') {
        INTERESTS.iter().find(|(k, ..)| *k == id).map(|(_, fr, en)| if native == "en" { *en } else { *fr }).unwrap_or_default().to_string()
    } else {
        one_line(id, INTEREST_MAX)
    }
}

/// Ce que le chat sait de l'apprenant.
pub fn learner(c: &Connection, native: &str, known: i64) -> Learner {
    let get = |k: &str| db::setting(c, k).unwrap_or_default();
    let interests = serde_json::from_str::<Vec<String>>(&get("user_interests"))
        .unwrap_or_default()
        .iter()
        .map(|x| interest_label(x, native))
        .filter(|x| !x.is_empty())
        .take(INTERESTS_MAX)
        .collect();
    Learner {
        known,
        name: one_line(&get("user_name"), NAME_MAX),
        why: one_line(&get("user_why"), WHY_MAX),
        interests,
        feminine: match get("user_agree").as_str() {
            "f" => Some(true),
            "m" => Some(false),
            _ => None,
        },
    }
}

/// Nom et avatar, gardés dans le manifeste de chaque sauvegarde.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Card {
    pub name: String,
    /// « style:teinte:graine » (lib/user.ts)
    pub avatar: String,
    /// photo réduite en data URL, si l'avatar est une photo
    pub photo: String,
}

pub fn card(c: &Connection) -> Card {
    let get = |k: &str| db::setting(c, k).unwrap_or_default();
    let avatar = get("user_avatar");
    let photo = get("user_photo");
    let photo = if avatar.starts_with("photo:") && photo.starts_with("data:image/") && photo.len() <= PHOTO_MAX { photo } else { String::new() };
    Card { name: one_line(&get("user_name"), NAME_MAX), avatar: avatar.chars().take(40).collect(), photo }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learner_and_card() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        // rien de choisi : un profil vide
        let l = learner(&c, "fr", 120);
        assert_eq!((l.known, l.name.as_str(), l.why.as_str(), l.interests.len(), l.feminine), (120, "", "", 0, None));
        assert_eq!(card(&c), Card::default());

        db::setting_set(&c, "user_name", "  Léa \n Rossi  ").unwrap();
        db::setting_set(&c, "user_why", "Parler avec ma famille\nà Naples").unwrap();
        db::setting_set(&c, "user_interests", r##"["#cooking","Opéra","#inconnu","  "]"##).unwrap();
        db::setting_set(&c, "user_agree", "f").unwrap();
        db::setting_set(&c, "user_avatar", "dawn:34:12345").unwrap();
        db::setting_set(&c, "user_photo", "data:image/jpeg;base64,AAAA").unwrap();
        let l = learner(&c, "fr", 900);
        assert_eq!(l.name, "Léa Rossi");
        assert_eq!(l.why, "Parler avec ma famille à Naples");
        assert_eq!(l.interests, vec!["cuisine", "Opéra"]);
        assert_eq!(l.feminine, Some(true));
        assert_eq!(learner(&c, "en", 900).interests, vec!["cooking", "Opéra"]);
        // la photo ne part dans le manifeste que si c'est elle, l'avatar
        assert_eq!(card(&c), Card { name: "Léa Rossi".into(), avatar: "dawn:34:12345".into(), photo: String::new() });
        db::setting_set(&c, "user_avatar", "photo:34:12345").unwrap();
        assert_eq!(card(&c).photo, "data:image/jpeg;base64,AAAA");
        // réglages abîmés : rien d'inventé
        db::setting_set(&c, "user_interests", "pas du JSON").unwrap();
        db::setting_set(&c, "user_agree", "x").unwrap();
        let l = learner(&c, "fr", 900);
        assert!(l.interests.is_empty() && l.feminine.is_none());
    }
}
