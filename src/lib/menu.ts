// Barre des menus du Mac : Lumen, Fichier, Édition, Présentation, Format,
// Leçon, Fenêtre, Aide. Construite ici plutôt qu'en Rust : elle parle la langue
// de l'interface, suit l'état de l'app (coches de la vue, de la police, de la
// couleur de la page… ; menu Leçon actif seulement dans une leçon) et ses
// actions sont celles des boutons. Les raccourcis ⌘ mènent d'un geste à ce qui
// demande sinon plusieurs clics (import d'un fichier, sauvegarde, taille du texte).
import type { CheckMenuItemOptions, Menu, MenuItemOptions, PredefinedMenuItemOptions, Submenu, SubmenuOptions } from "@tauri-apps/api/menu";
import { useEffect } from "react";
import { create } from "zustand";
import { api, errorText, isTauri } from "./api";
import { revealBackup, useBackup } from "./backup";
import { useChat } from "./chat";
import { pickFiles, pickSavePath } from "./dialogs";
import { isEn, t } from "./i18n";
import { MEDIA_EXT, TEXT_EXT } from "./importers";
import { langInfo } from "./langs";
import { isWindows, joinPath } from "./platform";
import { LOOK_DEFAULTS, PAPERS, playbackRates, READ_FONTS, SIZE_MAX, SIZE_MIN } from "./reading";
import { useApp, type View } from "./store";
import { useUpdate } from "./updater";
import type { LangCode } from "./types";

/** Ce que la leçon ouverte permet, pour activer les entrées du menu Leçon. */
export interface LessonMenu {
  /** leçon audio ou vidéo (avancer, reculer) */
  media: boolean;
  video: boolean;
}

/** Actions du lecteur, appelées par le menu Leçon. */
export interface LessonActions {
  toggle(): void;
  skip(secs: number): void;
  prev(): void;
  next(): void;
  restart(): void;
  finish(): void;
  chat(): void;
  simplify(): void;
  cinema(): void;
  playlist(): void;
  cover(): void;
  remove(): void;
}

interface MenuState {
  lesson: LessonMenu | null;
  /** feuille « Raccourcis clavier » ouverte */
  shortcuts: boolean;
  /** reconstruit le menu (une coche cliquée s'éteint d'elle-même sur macOS) */
  tick: number;
}

export const useMenu = create<MenuState>(() => ({ lesson: null, shortcuts: false, tick: 0 }));

export function openShortcuts() {
  useMenu.setState({ shortcuts: true });
}

export function closeShortcuts() {
  useMenu.setState({ shortcuts: false });
}

// actions de la leçon ouverte : relues au clic, sans reconstruire le menu
let lessonActions: LessonActions | null = null;

/** Branche le lecteur sur le menu Leçon (`flags` : null tant que la leçon n'est pas chargée). */
export function useLessonMenu(flags: LessonMenu | null, actions: LessonActions) {
  useEffect(() => {
    lessonActions = actions;
  });
  const key = flags ? `${flags.media}:${flags.video}` : "";
  useEffect(() => {
    useMenu.setState({ lesson: key ? { media: key.startsWith("true"), video: key.endsWith("true") } : null });
  }, [key]);
  useEffect(
    () => () => {
      lessonActions = null;
      useMenu.setState({ lesson: null });
    },
    [],
  );
}

const onLesson = (run: (a: LessonActions) => void) => () => {
  if (lessonActions) run(lessonActions);
};

// ---------- actions partagées avec les vues ----------

/** Exporte le vocabulaire de la langue en CSV (compatible Anki). */
export async function exportVocabulary(lang: LangCode) {
  const { toast } = useApp.getState();
  const path = await pickSavePath(t(`lumen-${lang}-vocabulaire.csv`, `lumen-${lang}-vocabulary.csv`));
  if (!path) return;
  try {
    await api().exportVocab(lang, path);
    toast(t("Vocabulaire exporté (compatible Anki)", "Vocabulary exported (Anki compatible)"), "light");
  } catch (e) {
    toast(errorText(e), "error");
  }
}

/** Met le curseur dans le champ de recherche de la vue (sinon : Découvrir). */
export function focusSearch() {
  const find = () => document.querySelector<HTMLInputElement>("input[data-find]");
  const now = find();
  if (now) {
    now.focus();
    now.select();
    return;
  }
  useApp.getState().go("discover");
  // le temps que la vue arrive
  let tries = 0;
  const wait = () => {
    const el = find();
    if (el) el.focus();
    else if (++tries < 30) window.setTimeout(wait, 50);
  };
  window.setTimeout(wait, 50);
}

async function openFiles() {
  const paths = await pickFiles([{ name: t("Textes, livres, audio et vidéo", "Texts, books, audio and video"), extensions: [...TEXT_EXT, ...MEDIA_EXT] }]);
  if (paths.length) useApp.getState().openImport(paths);
}

function newChat() {
  const app = useApp.getState();
  useChat.getState().fresh(null);
  app.go("chat");
  useChat.getState().focus();
}

async function backupNow() {
  const app = useApp.getState();
  const s = useBackup.getState().status;
  if (!s?.enabled || !s.dir) {
    app.openSettings("backup");
    return;
  }
  if (await useBackup.getState().save()) app.toast(t("Progression sauvegardée", "Progress backed up"), "light");
}

async function checkUpdates() {
  const up = useUpdate.getState();
  const { toast, info } = useApp.getState();
  await up.check(true);
  const s = useUpdate.getState();
  if (s.phase === "available") {
    toast(t(`Lumen ${s.version} est disponible.`, `Lumen ${s.version} is available.`), "light", { label: t("Installer", "Install"), run: () => void useUpdate.getState().install() });
  } else if (s.phase === "uptodate") {
    toast(t(`Lumen est à jour (version ${info?.version ?? ""}).`, `Lumen is up to date (version ${info?.version ?? ""}).`), "light");
  } else if (s.phase === "error") {
    toast(t("Impossible de vérifier les mises à jour. Vérifiez la connexion à Internet.", "Couldn't check for updates. Check the Internet connection."), "error");
  }
}

async function revealData() {
  const dir = useApp.getState().info?.data_dir;
  if (!dir || !isTauri) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(joinPath(dir, "lumen.db")).catch(() => {});
}

async function switchLang(code: LangCode) {
  const app = useApp.getState();
  if (app.lang() === code) return;
  await app.setSetting("lang", code);
  await app.refreshKnown();
  const v = app.view;
  // une leçon, une conversation ou une playlist d'une autre langue n'ont plus leur place
  app.go(v === "reader" || v === "chat" || v === "playlists" ? "library" : v);
}

/** Rouvre la leçon en cours (celle du dernier lancement, s'il le faut). */
function resumeLesson() {
  const a = useApp.getState();
  const id = a.lessonId ?? Number(a.settings.last_lesson);
  if (id) a.openLesson(id);
}

function toggleSidebar() {
  const app = useApp.getState();
  void app.setSetting("reader_sidebar", app.setting("reader_sidebar") === "0" ? "1" : "0");
}

async function toggleFullscreen() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  await w.setFullscreen(!(await w.isFullscreen()));
}

function setSize(next: (now: number) => number) {
  const app = useApp.getState();
  const now = Number(app.setting("font_size")) || 23;
  const v = Math.min(SIZE_MAX, Math.max(SIZE_MIN, next(now)));
  if (v !== now) void app.setSetting("font_size", String(v));
}

// ---------- construction ----------

type Item = MenuItemOptions | CheckMenuItemOptions | PredefinedMenuItemOptions | SubmenuOptions;

const sep: PredefinedMenuItemOptions = { item: "Separator" };

// actions des éléments : le natif renvoie l'identifiant de l'élément choisi (événement
// « menu » de lib.rs) ; Tauri oublie les actions passées aux éléments d'un sous-menu
let actions = new Map<string, () => void>();
let binding = new Map<string, () => void>();
let ids = 0;

function bind(run: () => void): string {
  const id = `lumen-${++ids}`;
  binding.set(id, run);
  return id;
}

function act(text: string, run: () => unknown, opts: { key?: string; on?: boolean } = {}): MenuItemOptions {
  return { id: bind(() => void run()), text, accelerator: opts.key, enabled: opts.on ?? true };
}

/** Choix coché ; le menu est refait après le clic (macOS décoche une coche cliquée deux fois). */
function pick(text: string, checked: boolean, run: () => unknown, opts: { key?: string; on?: boolean } = {}): CheckMenuItemOptions {
  return {
    id: bind(() => {
      void run();
      useMenu.setState((s) => ({ tick: s.tick + 1 }));
    }),
    text,
    checked,
    accelerator: opts.key,
    enabled: opts.on ?? true,
  };
}

function sub(text: string, items: Item[], enabled = true): SubmenuOptions {
  return { text, items, enabled };
}

const LOOK_KEYS = ["read_font", "read_paper", "font_size", "line_height", "read_width", "reader_layout", "word_style", "theme", "reader_sidebar", "media_rate", "tts_rate"] as const;

/** Ce dont le menu dépend : il n'est refait que si cela change. */
function snapshot() {
  const app = useApp.getState();
  const m = useMenu.getState();
  const b = useBackup.getState().status;
  const s = app.settings;
  const get = (k: string) => app.setting(k);
  return {
    en: isEn(),
    // accueil, nouvel accueil ou visite guidée : seulement l'essentiel
    calm: !app.ready || !s.onboarded || app.replay || app.tour !== null,
    view: app.view,
    lesson: app.view === "reader" ? m.lesson : null,
    resume: app.view !== "reader" && !!(app.lessonId ?? s.last_lesson),
    lang: app.lang(),
    langs: app.langs(),
    look: Object.fromEntries(LOOK_KEYS.map((k) => [k, get(k)])) as Record<(typeof LOOK_KEYS)[number], string>,
    backup: { on: !!b?.enabled && !!b.dir, saved: !!b?.dir && b.last_at !== null },
    tick: m.tick,
  };
}

type Snap = ReturnType<typeof snapshot>;

function appMenu(s: Snap): SubmenuOptions {
  const app = () => useApp.getState();
  return sub("Lumen", [
    {
      item: {
        // la version est lue dans le paquet de l'app
        About: {
          name: "Lumen",
          credits: t(
            "Apprendre les langues en lisant et en écoutant. Toute l'intelligence tourne sur votre Mac.",
            "Learn languages by reading and listening. All the intelligence runs on your Mac.",
          ),
        },
      },
      text: t("À propos de Lumen", "About Lumen"),
    },
    act(t("Rechercher les mises à jour…", "Check for Updates…"), checkUpdates, { on: !s.calm }),
    sep,
    act(t("Réglages…", "Settings…"), () => app().openSettings(), { key: "CmdOrCtrl+,", on: !s.calm }),
    act(t("Profil…", "Profile…"), () => app().openSettings("profile"), { on: !s.calm }),
    sep,
    { item: "Services", text: t("Services", "Services") },
    sep,
    { item: "Hide", text: t("Masquer Lumen", "Hide Lumen") },
    { item: "HideOthers", text: t("Masquer les autres", "Hide Others") },
    { item: "ShowAll", text: t("Tout afficher", "Show All") },
    sep,
    { item: "Quit", text: t("Quitter Lumen", "Quit Lumen") },
  ]);
}

function fileMenu(s: Snap): SubmenuOptions {
  const app = () => useApp.getState();
  const on = !s.calm;
  return sub(t("Fichier", "File"), [
    act(t("Nouvelle leçon…", "New Lesson…"), () => app().openImport(null, "text"), { key: "CmdOrCtrl+N", on }),
    act(t("Ouvrir un fichier…", "Open File…"), openFiles, { key: "CmdOrCtrl+O", on }),
    act(t("Importer un lien…", "Import a Link…"), () => app().openImport(null, "link"), { key: "CmdOrCtrl+L", on }),
    act(t("Créer un podcast sur mesure…", "Create a Custom Podcast…"), () => app().openImport(null, "podcast"), { on }),
    sep,
    act(t("Nouvelle conversation", "New Conversation"), newChat, { key: "Shift+CmdOrCtrl+N", on }),
    sep,
    act(t("Exporter le vocabulaire (CSV, Anki)…", "Export Vocabulary (CSV, Anki)…"), () => exportVocabulary(app().lang()), { on }),
    act(s.backup.on ? t("Sauvegarder maintenant", "Back Up Now") : t("Sauvegarder la progression…", "Back Up Progress…"), backupNow, { key: "CmdOrCtrl+S", on }),
    act(t("Afficher la sauvegarde dans le Finder", "Show Backup in Finder"), () => revealBackup(useBackup.getState().status?.dir ?? ""), { on: on && s.backup.saved }),
    act(t("Afficher les données de Lumen dans le Finder", "Show Lumen Data in Finder"), revealData, { on }),
    sep,
    { item: "CloseWindow", text: t("Fermer la fenêtre", "Close Window") },
  ]);
}

function editMenu(s: Snap): SubmenuOptions {
  return sub(t("Édition", "Edit"), [
    { item: "Undo", text: t("Annuler", "Undo") },
    { item: "Redo", text: t("Rétablir", "Redo") },
    sep,
    { item: "Cut", text: t("Couper", "Cut") },
    { item: "Copy", text: t("Copier", "Copy") },
    { item: "Paste", text: t("Coller", "Paste") },
    { item: "SelectAll", text: t("Tout sélectionner", "Select All") },
    sep,
    act(t("Rechercher…", "Find…"), focusSearch, { key: "CmdOrCtrl+F", on: !s.calm }),
  ]);
}

function viewMenu(s: Snap): SubmenuOptions {
  const app = () => useApp.getState();
  const on = !s.calm;
  const views: [View, string, string][] = [
    ["library", t("Bibliothèque", "Library"), "1"],
    ["discover", t("Découvrir", "Discover"), "2"],
    ["playlists", t("Playlists", "Playlists"), "3"],
    ["chat", t("Chat", "Chat"), "4"],
    ["vocab", t("Vocabulaire", "Vocabulary"), "5"],
    ["progress", t("Progrès", "Progress"), "6"],
  ];
  const { theme, reader_sidebar: sidebar } = s.look;
  const themes: [string, string][] = [
    ["light", t("Clair", "Light")],
    ["dark", t("Sombre", "Dark")],
    ["system", t("Comme le Mac", "Same as the Mac")],
  ];
  return sub(t("Présentation", "View"), [
    ...views.map(([v, label, key]) => pick(label, on && s.view === v, () => app().go(v), { key: `CmdOrCtrl+${key}`, on })),
    sep,
    sub(
      t("Langue étudiée", "Language"),
      [
        ...s.langs.map((code) => pick(langInfo(code).name, code === s.lang, () => switchLang(code), { on })),
        sep,
        act(t("Ajouter une langue…", "Add a Language…"), () => app().openSettings("langs"), { on }),
      ],
      on,
    ),
    sep,
    act(
      sidebar === "0" ? t("Afficher la barre latérale", "Show Sidebar") : t("Masquer la barre latérale", "Hide Sidebar"),
      toggleSidebar,
      // hors leçon, la barre latérale est toujours là
      { key: "Ctrl+CmdOrCtrl+S", on: on && s.view === "reader" },
    ),
    sub(
      t("Apparence", "Appearance"),
      themes.map(([v, label]) => pick(label, theme === v, () => app().setSetting("theme", v), { on })),
      on,
    ),
    sep,
    { item: "Fullscreen", text: t("Passer en mode plein écran", "Enter Full Screen") },
  ]);
}

function formatMenu(s: Snap): SubmenuOptions {
  const app = () => useApp.getState();
  const set = (k: string, v: string) => () => app().setSetting(k, v);
  const on = !s.calm;
  const { read_font: font, read_paper: paper, font_size: size, line_height: lh, read_width: width, reader_layout: layout, word_style: marks } = s.look;
  const n = Number(size) || 23;
  const options = (key: string, now: string, list: [string, string][]) => list.map(([v, label]) => pick(label, now === v, set(key, v), { on }));
  return sub("Format", [
    sub(
      t("Police", "Font"),
      READ_FONTS.map((f) => pick(f.label, font === f.id, set("read_font", f.id), { on })),
      on,
    ),
    sub(
      t("Couleur de la page", "Page Color"),
      PAPERS.map((p) => pick(p.label, paper === p.id, set("read_paper", p.id), { on })),
      on,
    ),
    sep,
    act(t("Agrandir le texte", "Bigger Text"), () => setSize((v) => v + 1), { key: "CmdOrCtrl+=", on: on && n < SIZE_MAX }),
    act(t("Réduire le texte", "Smaller Text"), () => setSize((v) => v - 1), { key: "CmdOrCtrl+-", on: on && n > SIZE_MIN }),
    act(t("Taille d'origine", "Default Size"), () => setSize(() => Number(LOOK_DEFAULTS.font_size)), { key: "CmdOrCtrl+0", on: on && size !== LOOK_DEFAULTS.font_size }),
    sep,
    sub(
      t("Interligne", "Line Spacing"),
      options("line_height", lh, [
        ["1.55", t("Serré", "Tight")],
        ["1.75", t("Normal", "Normal")],
        ["1.95", t("Aéré", "Airy")],
      ]),
      on,
    ),
    sub(
      t("Largeur des lignes", "Line Width"),
      options("read_width", width || "normal", [
        ["narrow", t("Étroite", "Narrow")],
        ["normal", t("Normale", "Normal")],
        ["wide", t("Large", "Wide")],
      ]),
      on,
    ),
    sub(
      t("Mise en page", "Layout"),
      options("reader_layout", layout, [
        ["pages", t("Pages", "Pages")],
        ["scroll", t("Défilement", "Scrolling")],
      ]),
      on,
    ),
    sub(
      t("Marquage des mots", "Word Marking"),
      options("word_style", marks, [
        ["tint", t("Teinte", "Tint")],
        ["line", t("Soulignés", "Underlined")],
      ]),
      on,
    ),
    sep,
    act(
      t("Rétablir l'affichage d'origine", "Restore Default Display"),
      () => Promise.all(Object.entries(LOOK_DEFAULTS).map(([k, v]) => app().setSetting(k, v))),
      { on: on && Object.entries(LOOK_DEFAULTS).some(([k, v]) => app().setting(k) !== v) },
    ),
  ]);
}

function lessonMenu(s: Snap): SubmenuOptions {
  const app = () => useApp.getState();
  const l = s.lesson;
  const on = !s.calm && !!l;
  const media = on && !!l?.media;
  const rateKey = l?.media ? "media_rate" : "tts_rate";
  // mêmes valeurs par défaut que le lecteur
  const rate = (l?.media ? s.look.media_rate : s.look.tts_rate) || (l?.media ? "1" : "0.95");
  const rates = playbackRates(!!l?.media);
  return sub(t("Leçon", "Lesson"), [
    act(t("Reprendre la lecture", "Resume Reading"), resumeLesson, { key: "CmdOrCtrl+R", on: !s.calm && s.resume }),
    sep,
    act(t("Lire ou mettre en pause", "Play or Pause"), onLesson((a) => a.toggle()), { on }),
    act(t("Reculer de 5 secondes", "Back 5 Seconds"), onLesson((a) => a.skip(-5)), { on: media }),
    act(t("Avancer de 5 secondes", "Forward 5 Seconds"), onLesson((a) => a.skip(5)), { on: media }),
    sub(
      t("Vitesse de lecture", "Playback Speed"),
      rates.map((r) => pick(`${t(r.replace(".", ","), r)}×`, rate === r, () => app().setSetting(rateKey, r), { on })),
      on,
    ),
    sep,
    act(t("Page précédente", "Previous Page"), onLesson((a) => a.prev()), { on }),
    act(t("Page suivante", "Next Page"), onLesson((a) => a.next()), { on }),
    act(t("Revenir au début", "Back to the Beginning"), onLesson((a) => a.restart()), { on }),
    act(t("Terminer la page", "Finish the Page"), onLesson((a) => a.finish()), { on }),
    sep,
    act(t("Discuter de la leçon", "Chat About the Lesson"), onLesson((a) => a.chat()), { on }),
    act(t("Simplifier…", "Simplify…"), onLesson((a) => a.simplify()), { on }),
    act(t("Regarder en plein écran", "Watch Full Screen"), onLesson((a) => a.cinema()), { on: on && !!l?.video }),
    sep,
    act(t("Ajouter à une playlist…", "Add to a Playlist…"), onLesson((a) => a.playlist()), { on }),
    act(t("Choisir une couverture…", "Choose a Cover…"), onLesson((a) => a.cover()), { on }),
    sep,
    act(t("Supprimer la leçon…", "Delete Lesson…"), onLesson((a) => a.remove()), { on }),
  ]);
}

function windowMenu(): SubmenuOptions {
  return sub(t("Fenêtre", "Window"), [
    { item: "Minimize", text: t("Placer dans le Dock", "Minimize") },
    { item: "Maximize", text: t("Réduire/agrandir", "Zoom") },
    sep,
    { item: "BringAllToFront", text: t("Tout ramener au premier plan", "Bring All to Front") },
  ]);
}

function helpMenu(s: Snap): SubmenuOptions {
  const app = () => useApp.getState();
  const on = !s.calm;
  return sub(t("Aide", "Help"), [
    act(t("Raccourcis clavier", "Keyboard Shortcuts"), openShortcuts, { key: "CmdOrCtrl+/", on }),
    act(t("Comment marche Lumen ?", "How Lumen Works"), () => app().openGuide(0), { on }),
    act(t("Visite guidée", "Guided Tour"), () => app().startTour(), { on }),
    act(t("Nouveautés de Lumen", "What's New in Lumen"), () => app().openNews("all"), { on }),
  ]);
}

// une seule construction à la fois ; le menu précédent est libéré ensuite
let installed: { menu: Menu; parts: Submenu[] } | null = null;
let chain: Promise<void> = Promise.resolve();

async function install() {
  const { Menu, Submenu } = await import("@tauri-apps/api/menu");
  binding = new Map();
  const s = snapshot();
  const win = await Submenu.new(windowMenu());
  const help = await Submenu.new(helpMenu(s));
  const items = s.calm ? [appMenu(s), editMenu(s)] : [appMenu(s), fileMenu(s), editMenu(s), viewMenu(s), formatMenu(s), lessonMenu(s)];
  const menu = await Menu.new({ items: [...items, win, ...(s.calm ? [] : [help])] });
  await menu.setAsAppMenu();
  // macOS y ajoute la liste des fenêtres, et le champ de recherche dans les menus
  await win.setAsWindowsMenuForNSApp();
  if (!s.calm) await help.setAsHelpMenuForNSApp();
  const old = installed;
  installed = { menu, parts: [win, help] };
  actions = binding;
  if (old) for (const r of [old.menu, ...old.parts]) void r.close().catch(() => {});
}

/** Installe la barre des menus et la tient à jour (appelé une fois, dans App). */
export function useAppMenu() {
  useEffect(() => {
    if (!isTauri || !navigator.userAgent.includes("Mac")) return;
    let last = "";
    let timer = 0;
    const check = () => {
      const key = JSON.stringify(snapshot());
      if (key === last) return;
      last = key;
      window.clearTimeout(timer);
      // plusieurs réglages changent souvent ensemble (« Rétablir ») : un seul menu refait
      timer = window.setTimeout(() => {
        chain = chain.then(install).catch(() => {});
      }, 60);
    };
    check();
    const stops = [useApp.subscribe(check), useMenu.subscribe(check), useBackup.subscribe(check)];
    let unlisten: (() => void) | undefined;
    let gone = false;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<string>("menu", (e) => actions.get(e.payload)?.()).then((u) => (gone ? u() : (unlisten = u))),
    );
    return () => {
      gone = true;
      stops.forEach((stop) => stop());
      unlisten?.();
      window.clearTimeout(timer);
    };
  }, []);
}

// ---------- Windows : les raccourcis sans la barre des menus ----------

const VIEW_KEYS: View[] = ["library", "discover", "playlists", "chat", "vocab", "progress"];

/**
 * Sous Windows, pas de barre des menus : ses raccourcis passent par Ctrl
 * (Ctrl+N, Ctrl+O, Ctrl+1 à 6…), F11 pour le plein écran, Ctrl+B pour la barre
 * latérale d'une leçon. Les chiffres se lisent à leur place sur le clavier
 * (`code`) : sur un clavier AZERTY, Ctrl+1 se tape sans Maj. Les raccourcis du
 * navigateur (recharger, imprimer, chercher dans la page) n'ont pas leur place
 * dans une app. Appelé une fois, dans App.
 */
export function useWindowsKeys() {
  useEffect(() => {
    if (!isWindows) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key === "F11") {
        e.preventDefault();
        void toggleFullscreen();
        return;
      }
      // F5 rechargerait l'app (gardé pendant le développement)
      if (e.key === "F5" && !import.meta.env.DEV) {
        e.preventDefault();
        return;
      }
      if (!e.ctrlKey || e.metaKey || e.altKey) return;
      const key = e.key.toLowerCase();
      if (key === "p" || key === "g" || (key === "r" && e.shiftKey)) {
        e.preventDefault();
        return;
      }
      const app = useApp.getState();
      const run = (f: () => unknown) => {
        e.preventDefault();
        void f();
      };
      // accueil, nouvel accueil ou visite guidée : seulement l'essentiel, comme le menu du Mac
      if (snapshot().calm) {
        if (key === "r" || key === "f") e.preventDefault();
        return;
      }
      if (key === "/" || e.code === "Slash") return run(openShortcuts);
      if (e.shiftKey && key === "n") return run(newChat);
      if (key === "=" || key === "+" || e.code === "NumpadAdd") return run(() => setSize((v) => v + 1));
      if (key === "-" || e.code === "NumpadSubtract") return run(() => setSize((v) => v - 1));
      if (e.code === "Digit0" || e.code === "Numpad0") return run(() => setSize(() => Number(LOOK_DEFAULTS.font_size)));
      const digit = /^(?:Digit|Numpad)([1-6])$/.exec(e.code);
      if (digit) return run(() => app.go(VIEW_KEYS[Number(digit[1]) - 1]));
      if (e.shiftKey) return;
      switch (key) {
        case ",":
          return run(() => app.openSettings());
        case "n":
          return run(() => app.openImport(null, "text"));
        case "o":
          return run(openFiles);
        case "l":
          return run(() => app.openImport(null, "link"));
        case "s":
          return run(backupNow);
        case "f":
          return run(focusSearch);
        case "r":
          return run(resumeLesson);
        case "b":
          if (app.view === "reader") run(toggleSidebar);
          return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
