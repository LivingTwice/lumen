import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { Icon } from "./components/Icon";
import { Sidebar } from "./components/Sidebar";
import { Orb, Toasts } from "./components/ui";
import { isTauri } from "./lib/api";
import { MEDIA_EXT, TEXT_EXT, extOf } from "./lib/importers";
import { startBackupEvents } from "./lib/backup";
import { setUiLang, t, type UiLang } from "./lib/i18n";
import { useApp } from "./lib/store";
import { startUpdateChecks } from "./lib/updater";
import { ImportSheet } from "./views/ImportSheet";
import { Chat } from "./views/Chat";
import { Library } from "./views/Library";
import { Onboarding } from "./views/Onboarding";
import { Playlists } from "./views/Playlists";
import { Progress } from "./views/Progress";
import { Settings } from "./views/Settings";
import { Vocabulary } from "./views/Vocabulary";
import { Reader } from "./views/reader/Reader";

function useTheme() {
  const theme = useApp((s) => s.settings.theme);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || (theme !== "light" && mq.matches);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
    };
    apply();
    mq.addEventListener("change", apply);
    if (isTauri) {
      import("@tauri-apps/api/window")
        .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(theme === "system" ? null : (theme as "light" | "dark")))
        .catch(() => {});
    }
    return () => mq.removeEventListener("change", apply);
  }, [theme]);
}

function useFileDrop(enabled: boolean) {
  const openImport = useApp((s) => s.openImport);
  const toast = useApp((s) => s.toast);
  const [over, setOver] = useState(false);
  useEffect(() => {
    if (!isTauri || !enabled) return;
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/webview").then(async ({ getCurrentWebview }) => {
      unlisten = await getCurrentWebview().onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter" || p.type === "over") setOver(true);
        else if (p.type === "leave") setOver(false);
        else if (p.type === "drop") {
          setOver(false);
          const ok = p.paths.filter((f) => [...MEDIA_EXT, ...TEXT_EXT].includes(extOf(f)));
          if (ok.length) openImport(ok);
          else toast(t("Ce type de fichier n'est pas encore pris en charge.", "This type of file isn't supported yet."), "error");
        }
      });
    });
    return () => unlisten?.();
  }, [enabled, openImport, toast]);
  return over;
}

export function App() {
  const ready = useApp((s) => s.ready);
  const init = useApp((s) => s.init);
  const view = useApp((s) => s.view);
  const onboarded = useApp((s) => s.settings.onboarded);
  const replay = useApp((s) => s.replay);
  // langue de l'interface : appliquée avant le rendu des vues (t() la lit au rendu)
  const ui = useApp((s) => s.settings.ui_lang) as UiLang;
  if (ui) setUiLang(ui);
  const [failed, setFailed] = useState<string | null>(null);
  // barre latérale repliée dans une leçon (réglage reader_sidebar), visible partout ailleurs
  const sideHidden = useApp((s) => s.view === "reader" && s.settings.reader_sidebar === "0");
  // pendant le repli, le contenu est rogné ; ensuite les menus peuvent déborder
  const [sliding, setSliding] = useState(false);
  const firstSide = useRef(true);
  useEffect(() => {
    if (firstSide.current) {
      firstSide.current = false;
      return;
    }
    setSliding(true);
    const t = window.setTimeout(() => setSliding(false), 600);
    return () => window.clearTimeout(t);
  }, [sideHidden]);

  useTheme();
  const dropping = useFileDrop(ready && !!onboarded);

  useEffect(() => {
    if (isTauri && navigator.userAgent.includes("Mac")) document.documentElement.classList.add("vibrant");
    init()
      .then(() => {
        startUpdateChecks();
        startBackupEvents();
      })
      .catch((e) => setFailed(String(e)));
  }, [init]);

  // évite le menu contextuel du navigateur en dehors des champs de saisie
  useEffect(() => {
    if (!isTauri) return;
    const block = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!t.closest("input, textarea")) e.preventDefault();
    };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);

  if (failed) {
    return (
      <div className="empty" style={{ height: "100%", justifyContent: "center" }}>
        <Orb size={40} />
        <h3>{t("Lumen n'a pas pu démarrer", "Lumen couldn't start")}</h3>
        <p>{failed}</p>
      </div>
    );
  }

  if (!ready) {
    return (
      <div style={{ height: "100%", display: "grid", placeItems: "center", background: "var(--bg)" }}>
        <Orb size={34} />
      </div>
    );
  }

  if (!onboarded || replay) return <Onboarding />;

  // changer de langue remonte toute l'interface : chaque texte se recalcule
  return (
    <div key={ui} className={`app ${sideHidden ? "side-hidden" : ""}`}>
      <div className={`sidebar-shell ${sideHidden || sliding ? "clip" : ""}`} inert={sideHidden}>
        <Sidebar />
      </div>
      <main className="main">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={view}
            style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
          >
            {view === "library" && <Library />}
            {view === "playlists" && <Playlists />}
            {view === "reader" && <Reader />}
            {view === "chat" && <Chat />}
            {view === "vocab" && <Vocabulary />}
            {view === "progress" && <Progress />}
            {view === "settings" && <Settings />}
          </motion.div>
        </AnimatePresence>
      </main>
      <ImportSheet />
      <Toasts />
      <AnimatePresence>
        {dropping && (
          <motion.div className="dropzone-overlay" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
            <motion.div className="dropzone-card" initial={{ scale: 0.94 }} animate={{ scale: 1 }} transition={{ type: "spring", stiffness: 300, damping: 22 }}>
              <Orb size={44} />
              <h3>{t("Déposez pour importer", "Drop to import")}</h3>
              <p className="muted">{t("Livres, articles, PDF, sous-titres, audio et vidéo", "Books, articles, PDFs, subtitles, audio and video")}</p>
              <Icon name="import" size={22} />
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
