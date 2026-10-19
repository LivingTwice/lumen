//! iPhone et iPad : ce que fait l'interface d'iOS elle-même, par le pont Swift du
//! projet Xcode (gen/apple/Sources/lumen/LumenNative.swift) : choisir un dossier
//! dans l'app Fichiers et y garder l'accès (signet), faire descendre un fichier
//! d'iCloud, retour haptique, page web dans Lumen (connexion à ChatGPT). (Le modèle de l'appareil, sa mémoire et sa chaleur :
//! `proc::device_info`.)

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

extern "C" {
    fn lumen_pick_folder(cb: extern "C" fn(*mut c_void, *const c_char, *const c_char), user: *mut c_void);
    fn lumen_access_begin(mark: *const c_char, out: *mut c_char, cap: isize) -> i32;
    fn lumen_access_end(path: *const c_char);
    fn lumen_bookmark(path: *const c_char, out: *mut c_char, cap: isize) -> i32;
    fn lumen_materialize(path: *const c_char) -> i32;
    fn lumen_haptic(kind: i32);
    fn lumen_web_open(url: *const c_char, cb: extern "C" fn(*mut c_void), user: *mut c_void) -> i32;
    fn lumen_web_close();
    /// mémoire que iOS laisse encore à l'app (octets)
    fn os_proc_available_memory() -> usize;
}

type Picked = tokio::sync::oneshot::Sender<Option<(String, String)>>;

extern "C" fn on_picked(user: *mut c_void, mark: *const c_char, path: *const c_char) {
    // SAFETY : `user` est la boîte créée par `pick_folder`, rendue une seule fois
    let tx = unsafe { Box::from_raw(user as *mut Picked) };
    let got = (!mark.is_null() && !path.is_null()).then(|| {
        // SAFETY : chaînes C valides pendant le rappel
        unsafe { (CStr::from_ptr(mark).to_string_lossy().into_owned(), CStr::from_ptr(path).to_string_lossy().into_owned()) }
    });
    let _ = tx.send(got.filter(|(m, _)| !m.is_empty()));
}

/// L'app Fichiers, pour choisir un dossier : son signet (base64) et son chemin.
/// `None` : l'apprenant a renoncé.
pub async fn pick_folder() -> Option<(String, String)> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let user = Box::into_raw(Box::new(tx)) as *mut c_void;
    // SAFETY : le rappel reprend la boîte une seule fois
    unsafe { lumen_pick_folder(on_picked, user) };
    rx.await.ok().flatten()
}

/// Accès ouvert à un dossier choisi par l'apprenant, refermé à la fin.
pub struct Access {
    path: CString,
}

impl Drop for Access {
    fn drop(&mut self) {
        // SAFETY : chemin rendu par lumen_access_begin
        unsafe { lumen_access_end(self.path.as_ptr()) }
    }
}

/// Ouvre l'accès au dossier d'un signet. Renvoie l'accès, le chemin du dossier, et
/// un nouveau signet quand l'ancien est périmé (à enregistrer).
pub fn access(mark: &str) -> Option<(Access, PathBuf, Option<String>)> {
    // essais dans le simulateur (version de développement seulement) : « dev:<chemin> »
    // désigne un dossier de l'app, sans signet
    if cfg!(debug_assertions) {
        if let Some(p) = mark.strip_prefix("dev:") {
            return Some((Access { path: CString::new("").ok()? }, PathBuf::from(p), None));
        }
    }
    let mark = CString::new(mark).ok()?;
    let mut buf = vec![0 as c_char; 4096];
    // SAFETY : tampon de la taille annoncée
    let r = unsafe { lumen_access_begin(mark.as_ptr(), buf.as_mut_ptr(), buf.len() as isize) };
    if r == 0 {
        return None;
    }
    // SAFETY : chaîne terminée par un zéro, écrite par lumen_access_begin
    let path = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_owned();
    let fresh = (r == 2).then(|| {
        let mut out = vec![0 as c_char; 16384];
        // SAFETY : tampon de la taille annoncée
        let ok = unsafe { lumen_bookmark(path.as_ptr(), out.as_mut_ptr(), out.len() as isize) };
        // SAFETY : chaîne terminée par un zéro
        (ok == 1).then(|| unsafe { CStr::from_ptr(out.as_ptr()) }.to_string_lossy().into_owned())
    });
    let p = PathBuf::from(path.to_string_lossy().into_owned());
    Some((Access { path }, p, fresh.flatten()))
}

/// Fait descendre ce fichier resté dans le nuage (iCloud Drive, Google Drive, Dropbox…
/// vus par l'app Fichiers) et attend qu'il soit là : vrai s'il est lisible.
pub fn materialize(p: &Path) -> bool {
    let Ok(c) = CString::new(p.to_string_lossy().as_bytes()) else { return false };
    // SAFETY : chaîne C valide pendant l'appel (bloquant)
    unsafe { lumen_materialize(c.as_ptr()) == 1 }
}

/// Retour haptique : 0 sélection, 1 léger, 2 moyen, 3 réussite.
pub fn haptic(kind: i32) {
    // SAFETY : simple appel, sans pointeur
    unsafe { lumen_haptic(kind) }
}

extern "C" fn on_web_closed(user: *mut c_void) {
    // SAFETY : `user` vient d'`Arc::into_raw` dans `web_open`, rendu une seule fois
    let stop = unsafe { std::sync::Arc::from_raw(user as *const std::sync::atomic::AtomicBool) };
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Ouvre une page web posée sur Lumen (connexion à ChatGPT : Lumen reste au premier
/// plan, son adresse locale reçoit la réponse). Si l'apprenant la ferme, `stop` passe
/// à vrai. Seulement des adresses https.
pub fn web_open(url: &str, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) -> bool {
    let Ok(url) = CString::new(url) else { return false };
    let user = std::sync::Arc::into_raw(stop) as *mut c_void;
    // SAFETY : chaîne valide pendant l'appel ; le rappel reprend la référence une fois
    let ok = unsafe { lumen_web_open(url.as_ptr(), on_web_closed, user) } == 1;
    if !ok {
        // SAFETY : refusée tout de suite, le rappel ne viendra pas
        drop(unsafe { std::sync::Arc::from_raw(user as *const std::sync::atomic::AtomicBool) });
    }
    ok
}

/// Referme la page ouverte par `web_open`.
pub fn web_close() {
    // SAFETY : simple appel, sans pointeur
    unsafe { lumen_web_close() }
}

/// Mémoire que iOS laisse encore à Lumen (octets).
pub fn available_memory() -> u64 {
    // SAFETY : fonction du système, sans argument
    unsafe { os_proc_available_memory() as u64 }
}

/// Nom de l'appareil dans une phrase : « iPhone » ou « iPad » (iOS ne donne plus
/// le nom choisi par son propriétaire sans autorisation spéciale).
pub fn device_word() -> &'static str {
    if crate::proc::device_info().machine.starts_with("iPad") {
        "iPad"
    } else {
        "iPhone"
    }
}
