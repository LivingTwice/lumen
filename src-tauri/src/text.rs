//! Normalisation et découpage du texte en mots (UAX #29), identiques partout
//! dans l'application : la clé d'un mot est sa forme en minuscules, sans
//! accent tonique (russe), avec l'apostrophe typographique unifiée.

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Clé de recherche d'un mot ou d'une expression.
pub fn normalize(s: &str) -> String {
    let lower = s.trim().to_lowercase().replace('’', "'");
    let stripped: String = lower.nfd().filter(|c| *c != '\u{0301}' && *c != '\u{0300}').collect();
    stripped.nfc().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Serialize, Clone, Debug)]
pub struct Token {
    /// texte tel qu'il apparaît
    pub t: String,
    /// vrai si c'est un mot à apprendre
    pub w: bool,
    /// clé normalisée (vide si ce n'est pas un mot)
    pub k: String,
    /// début et fin en unités UTF-16 (compatibles avec les chaînes JavaScript)
    pub s: usize,
    pub e: usize,
}

fn is_learnable(piece: &str) -> bool {
    piece.chars().any(|c| c.is_alphabetic())
}

/// Langues où l'élision (l'uomo, dell'acqua, d'água) doit être séparée.
fn splits_elision(lang: &str) -> bool {
    matches!(lang, "it" | "fr" | "pt" | "ca")
}

pub fn tokenize(text: &str, lang: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut utf16_pos = 0usize;
    let push = |piece: &str, out: &mut Vec<Token>, pos: &mut usize| {
        let len16: usize = piece.encode_utf16().count();
        let w = is_learnable(piece);
        out.push(Token {
            t: piece.to_string(),
            w,
            k: if w { normalize(piece) } else { String::new() },
            s: *pos,
            e: *pos + len16,
        });
        *pos += len16;
    };
    for piece in text.split_word_bounds() {
        if splits_elision(lang) && is_learnable(piece) {
            if let Some(idx) = piece.find(|c| c == '\'' || c == '’') {
                let (head, rest) = piece.split_at(idx);
                let apos_len = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                let tail = &rest[apos_len..];
                if !head.is_empty() && head.chars().count() <= 6 && is_learnable(tail) {
                    let head_with_apos = &piece[..idx + apos_len];
                    push(head_with_apos, &mut out, &mut utf16_pos);
                    push(tail, &mut out, &mut utf16_pos);
                    continue;
                }
            }
        }
        push(piece, &mut out, &mut utf16_pos);
    }
    out
}

/// Liste des clés de mots (avec doublons) d'un texte.
pub fn word_keys(text: &str, lang: &str) -> Vec<String> {
    tokenize(text, lang).into_iter().filter(|t| t.w).map(|t| t.k).collect()
}

/// Nombre de mots d'un texte.
pub fn word_count(text: &str, lang: &str) -> usize {
    tokenize(text, lang).iter().filter(|t| t.w).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn elision_italian() {
        let t: Vec<_> = tokenize("Dell'acqua e l'uomo.", "it").into_iter().filter(|t| t.w).map(|t| t.k).collect();
        assert_eq!(t, vec!["dell'", "acqua", "e", "l'", "uomo"]);
    }
    #[test]
    fn english_contraction_kept() {
        let t: Vec<_> = tokenize("Don't stop", "en").into_iter().filter(|t| t.w).map(|t| t.k).collect();
        assert_eq!(t, vec!["don't", "stop"]);
    }
    #[test]
    fn russian_stress() {
        assert_eq!(normalize("до\u{301}м"), "дом");
    }
    #[test]
    fn utf16_offsets() {
        let toks = tokenize("Größe 😀 ok", "de");
        let last = toks.last().unwrap();
        assert_eq!(last.t, "ok");
        assert_eq!(last.s, 9);
    }
}
