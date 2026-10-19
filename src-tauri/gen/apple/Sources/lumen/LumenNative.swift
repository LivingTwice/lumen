// Pont natif de Lumen sur iPhone et iPad : ce que l'interface d'iOS fait elle-même
// et que Rust appelle comme des fonctions C (src/ios.rs). Rien d'autre ici :
// le reste de l'app est le même code que sur Mac et Windows.
//
// - choisir un dossier dans l'app Fichiers (le dossier « Lumen » d'iCloud Drive, où le
//   Mac sauvegarde) et y garder l'accès d'un lancement à l'autre (signet) ;
// - faire descendre d'iCloud un fichier qui n'y est encore qu'en nuage ;
// - un léger retour haptique (toucher un mot, tourner une page) ;
// - une page web dans Lumen (connexion à ChatGPT) : Lumen reste au premier plan, son
//   adresse locale peut recevoir la réponse d'OpenAI.

import Foundation
import SafariServices
import UIKit
import UniformTypeIdentifiers

// MARK: - Choix d'un dossier

private final class FolderPicker: NSObject, UIDocumentPickerDelegate {
    let done: (URL?) -> Void
    init(_ done: @escaping (URL?) -> Void) { self.done = done }
    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        done(urls.first)
        current = nil
    }
    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        done(nil)
        current = nil
    }
}

private var current: FolderPicker?

private func topController() -> UIViewController? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let window = scenes.flatMap { $0.windows }.first { $0.isKeyWindow } ?? scenes.first?.windows.first
    var top = window?.rootViewController
    while let presented = top?.presentedViewController { top = presented }
    return top
}

public typealias LumenPicked = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Void

/// Présente l'app Fichiers pour choisir un dossier. Rappelle `cb(user, signet, chemin)`
/// (signet en base64), ou `cb(user, nil, nil)` si l'apprenant renonce.
@_cdecl("lumen_pick_folder")
public func lumen_pick_folder(_ cb: LumenPicked, _ user: UnsafeMutableRawPointer?) {
    DispatchQueue.main.async {
        guard let top = topController() else {
            cb(user, nil, nil)
            return
        }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [UTType.folder])
        let delegate = FolderPicker { url in
            guard let url = url else {
                cb(user, nil, nil)
                return
            }
            let granted = url.startAccessingSecurityScopedResource()
            defer { if granted { url.stopAccessingSecurityScopedResource() } }
            let mark = (try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil))?.base64EncodedString() ?? ""
            mark.withCString { m in url.path.withCString { p in cb(user, m, p) } }
        }
        current = delegate
        picker.delegate = delegate
        picker.modalPresentationStyle = .formSheet
        top.present(picker, animated: true)
    }
}

// MARK: - Accès durable (signets)

private var open: [String: URL] = [:]
private let lock = NSLock()

/// Ouvre l'accès au dossier d'un signet (base64) et en écrit le chemin dans `out`
/// (`cap` octets). Renvoie 1 si le dossier est accessible, 2 s'il l'est mais que le
/// signet doit être refait (`lumen_bookmark` du chemin), 0 sinon.
@_cdecl("lumen_access_begin")
public func lumen_access_begin(_ mark: UnsafePointer<CChar>, _ out: UnsafeMutablePointer<CChar>, _ cap: Int) -> Int32 {
    guard let data = Data(base64Encoded: String(cString: mark)) else { return 0 }
    var stale = false
    guard let url = try? URL(resolvingBookmarkData: data, options: [], relativeTo: nil, bookmarkDataIsStale: &stale) else { return 0 }
    guard url.startAccessingSecurityScopedResource() else { return 0 }
    let path = url.path
    lock.lock()
    if let old = open[path] { old.stopAccessingSecurityScopedResource() }
    open[path] = url
    lock.unlock()
    let bytes = Array(path.utf8CString)
    if bytes.count > cap { return 0 }
    for (i, b) in bytes.enumerated() { out[i] = b }
    return stale ? 2 : 1
}

/// Referme l'accès ouvert par `lumen_access_begin`.
@_cdecl("lumen_access_end")
public func lumen_access_end(_ path: UnsafePointer<CChar>) {
    let p = String(cString: path)
    lock.lock()
    let url = open.removeValue(forKey: p)
    lock.unlock()
    url?.stopAccessingSecurityScopedResource()
}

/// Signet d'un dossier déjà ouvert (signet devenu périmé) : base64 dans `out`.
@_cdecl("lumen_bookmark")
public func lumen_bookmark(_ path: UnsafePointer<CChar>, _ out: UnsafeMutablePointer<CChar>, _ cap: Int) -> Int32 {
    lock.lock()
    let url = open[String(cString: path)]
    lock.unlock()
    guard let url = url, let data = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) else { return 0 }
    let bytes = Array(data.base64EncodedString().utf8CString)
    if bytes.count > cap { return 0 }
    for (i, b) in bytes.enumerated() { out[i] = b }
    return 1
}

// MARK: - Fichiers restés dans le nuage

/// Fait descendre un fichier resté dans le nuage (iCloud Drive, ou l'app d'un autre
/// nuage dans Fichiers : Google Drive, Dropbox…) et attend qu'il soit là. iCloud se
/// demande directement ; les autres nuages fournissent le fichier à une lecture
/// coordonnée. Renvoie 1 si le fichier est lisible.
@_cdecl("lumen_materialize")
public func lumen_materialize(_ path: UnsafePointer<CChar>) -> Int32 {
    let url = URL(fileURLWithPath: String(cString: path))
    try? FileManager.default.startDownloadingUbiquitousItem(at: url)
    var ok = false
    var failure: NSError?
    NSFileCoordinator(filePresenter: nil).coordinate(readingItemAt: url, options: [], error: &failure) { local in
        ok = FileManager.default.isReadableFile(atPath: local.path)
    }
    return ok && failure == nil ? 1 : 0
}

// MARK: - Toucher

/// Retour haptique : 0 sélection (toucher un mot), 1 léger, 2 moyen, 3 réussite.
@_cdecl("lumen_haptic")
public func lumen_haptic(_ kind: Int32) {
    DispatchQueue.main.async {
        switch kind {
        case 0: UISelectionFeedbackGenerator().selectionChanged()
        case 1: UIImpactFeedbackGenerator(style: .light).impactOccurred()
        case 2: UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        default: UINotificationFeedbackGenerator().notificationOccurred(.success)
        }
    }
}

// MARK: - Page web dans Lumen

public typealias LumenClosed = @convention(c) (UnsafeMutableRawPointer?) -> Void

private final class WebPage: NSObject, SFSafariViewControllerDelegate {
    let closed: () -> Void
    init(_ closed: @escaping () -> Void) { self.closed = closed }
    // seulement quand l'apprenant la ferme lui-même (« OK »)
    func safariViewControllerDidFinish(_ controller: SFSafariViewController) {
        closed()
        web = nil
    }
}

private var web: (SFSafariViewController, WebPage)?

/// Ouvre `url` dans une page Safari posée sur Lumen. `cb(user)` est rappelé si
/// l'apprenant la ferme lui-même ; `lumen_web_close` la referme sans rappel.
@_cdecl("lumen_web_open")
public func lumen_web_open(_ url: UnsafePointer<CChar>, _ cb: LumenClosed, _ user: UnsafeMutableRawPointer?) -> Int32 {
    guard let link = URL(string: String(cString: url)), link.scheme == "https" else { return 0 }
    DispatchQueue.main.async {
        guard let top = topController() else {
            cb(user)
            return
        }
        let page = SFSafariViewController(url: link)
        let delegate = WebPage { cb(user) }
        page.delegate = delegate
        page.dismissButtonStyle = .cancel
        page.modalPresentationStyle = .pageSheet
        web = (page, delegate)
        top.present(page, animated: true)
    }
    return 1
}

/// Referme la page ouverte par `lumen_web_open` (connexion terminée).
@_cdecl("lumen_web_close")
public func lumen_web_close() {
    DispatchQueue.main.async {
        guard let (page, _) = web else { return }
        web = nil
        page.dismiss(animated: true)
    }
}
