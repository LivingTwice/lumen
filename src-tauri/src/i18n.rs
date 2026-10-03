//! Langue de l'interface (réglage `ui_lang`, "fr" ou "en") : messages montrés à
//! l'utilisateur, langue des traductions de l'IA, du chat et des dictionnaires.
//! Les marqueurs internes (« annulé », « interrompu », `NO_MODEL:`) ne changent pas :
//! l'interface les reconnaît.

use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

pub fn set(code: &str) {
    EN.store(code == "en", Ordering::Relaxed);
}

pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// Code de la langue de l'apprenant : celle des traductions ("fr" ou "en").
pub fn native() -> &'static str {
    if en() {
        "en"
    } else {
        "fr"
    }
}

/// Texte fixe dans la langue de l'interface : `t("Le texte est vide.", "The text is empty.")`.
pub fn t(fr: &'static str, en: &'static str) -> &'static str {
    if self::en() {
        en
    } else {
        fr
    }
}

/// Texte mis en forme dans la langue de l'interface :
/// `tr!("Page inaccessible : {e}", "Page unreachable: {e}")`.
#[macro_export]
macro_rules! tr {
    ($fr:literal, $en:literal $(,)?) => {
        if $crate::i18n::en() { format!($en) } else { format!($fr) }
    };
    ($fr:literal, $en:literal, $($arg:tt)+) => {
        if $crate::i18n::en() { format!($en, $($arg)+) } else { format!($fr, $($arg)+) }
    };
}
