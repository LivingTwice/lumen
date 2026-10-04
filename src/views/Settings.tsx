import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "../components/Icon";
import { sampleFor, splitWords } from "../components/Guide";
import { Orb, Segmented, Switch } from "../components/ui";
import { api, isTauri } from "../lib/api";
import { useBackup } from "../lib/backup";
import { confirmAsk } from "../lib/dialogs";
import { count, formatNumber, t, type UiLang } from "../lib/i18n";
import { LANGS, STARTERS, inLang, langInfo, langLower, starterCollection, theLang } from "../lib/langs";
import { useDictStatus } from "../lib/dicts";
import { LEVELS, levelName, unitsLabel, useLevel } from "../lib/discover";
import { useLingq } from "../lib/lingq";
import { PROFILES } from "../lib/profiles";
import { formatBytes, useApp, type SettingsTab } from "../lib/store";
import { naturalVoiceFor, naturalVoices, pronounce } from "../lib/pronounce";
import { loadVoices, sayWord, voicesFor } from "../lib/tts";
import { useUpdate } from "../lib/updater";
import { readerLook, readFont, SIZE_MAX, SIZE_MIN } from "../lib/reading";
import { FontPicker, LayoutPicker, PaperPicker, lineHeightOptions, widthOptions } from "./reader/Display";
import type { LangCode, ModelRow } from "../lib/types";
import { BackupSection } from "./BackupSection";
import { LingqSection } from "./LingqSection";
import { PodcastSection } from "./PodcastSection";

/* Réglages : un menu de catégories à gauche, une page par catégorie à droite.
   Chaque page ne montre que son domaine ; la recherche du menu retrouve un réglage. */

const EASE = [0.2, 0.8, 0.2, 1] as const;

interface Tab {
  id: SettingsTab;
  icon: IconName | "orb";
  label: string;
  /** présentation de la page, sous son titre */
  lead: string;
  /** mots de la recherche, dans les deux langues */
  keys: string;
}

/** Les pages, dans l'ordre du menu, rangées par groupe. */
const groups = (): { title: string | null; tabs: Tab[] }[] => [
  {
    title: null,
    tabs: [
      {
        id: "general",
        icon: "settings",
        label: t("Général", "General"),
        lead: t("La langue de Lumen et sa lumière, de l'aube à la nuit.", "Lumen's language and its light, from dawn to night."),
        keys: "langue interface francais anglais theme apparence clair sombre systeme language french english appearance light dark system",
      },
    ],
  },
  {
    title: t("Apprendre", "Learning"),
    tabs: [
      {
        id: "langs",
        icon: "globe",
        label: t("Langues", "Languages"),
        lead: t(
          "Chaque langue a sa bibliothèque, son vocabulaire et ses progrès. Retirer une langue garde ses leçons et ses mots.",
          "Each language has its own library, vocabulary and progress. Removing a language keeps its lessons and words.",
        ),
        keys: "langues etudiees ajouter retirer dictionnaire hors ligne languages study add remove dictionary offline",
      },
      {
        id: "reading",
        icon: "book",
        label: t("Lecture", "Reading"),
        lead: t(
          "Réglez la page à votre œil. Tout se règle aussi dans une leçon, par le bouton « Aa » en haut à droite.",
          "Adjust the page to your eye. You can also change all of this inside a lesson, with the “Aa” button at the top right.",
        ),
        keys:
          "lecture page pages defilement mise en page police taille texte couleur papier sepia crepuscule nuit encre interligne largeur lignes marquage teinte soulignement terminer connus traduire phrase prononcer automatique reading layout scroll font size color paper sepia dusk night ink line spacing width highlighting tint underline finish known translate sentence pronounce aa",
      },
      {
        id: "voice",
        icon: "speaker",
        label: t("Voix", "Voice"),
        lead: t(
          "La voix naturelle prononce les mots que vous touchez et lit vos leçons de texte. Les voix du système lisent à voix haute sans préparation.",
          "The natural voice pronounces the words you tap and reads your text lessons. The system voices read aloud with no preparation.",
        ),
        keys: "voix naturelle supertonic systeme lecture a voix haute prononciation audio natural voice system read aloud pronunciation speech",
      },
      {
        id: "discover",
        icon: "sparkle",
        label: t("Découvrir", "Discover"),
        lead: t(
          "Des vidéos, des podcasts, des chansons et des articles choisis pour vos langues, rangés par niveau ; et la recherche en ligne.",
          "Videos, podcasts, songs and articles chosen for your languages, sorted by level; and online search.",
        ),
        keys: "decouvrir sources niveau a1 a2 b1 b2 c1 chaque jour podcasts videos articles chansons musique recherche youtube lemmes mots differents discover level daily songs music search lemmas",
      },
      {
        id: "podcasts",
        icon: "podcast",
        label: t("Podcasts", "Podcasts"),
        lead: t(
          "Des podcasts écrits pour vous : un sujet, votre niveau, une durée. Gemini, l'IA en ligne de Google, les écrit et les dit ; ils deviennent des leçons avec leur lanterne.",
          "Podcasts made for you: a topic, your level, a length. Gemini, Google's online AI, writes and voices them; they become lessons with their lantern.",
        ),
        keys: "podcasts podcast gemini notebook notebooklm google ai studio cle api voix creer sur mesure sujet niveau duree key voices create custom topic level length",
      },
    ],
  },
  {
    title: t("Sur ce Mac", "On this Mac"),
    tabs: [
      {
        id: "ai",
        icon: "cpu",
        label: t("IA locale", "Local AI"),
        lead: t(
          "Choisissez la puissance des modèles. Ils sont téléchargés une seule fois puis fonctionnent hors ligne.",
          "Choose how powerful the models are. They are downloaded once, then work offline.",
        ),
        keys: "ia locale modele modeles qwen whisper asr transcription traduction chat profil leger equilibre maximum telecharger local ai model models translation profile light balanced download",
      },
      {
        id: "videos",
        icon: "video",
        label: t("Vidéos en ligne", "Online videos"),
        lead: t(
          "YouTube et la plupart des sites vidéo : le son est transcrit sur votre Mac et l'image téléchargée en haute définition.",
          "YouTube and most video sites: the sound is transcribed on your Mac and the picture downloaded in high definition.",
        ),
        keys: "videos en ligne youtube navigateur cookies yt-dlp composants online videos browser components",
      },
    ],
  },
  {
    title: t("Vos données", "Your data"),
    tabs: [
      {
        id: "backup",
        icon: "cloud",
        label: t("Sauvegarde", "Backup"),
        lead: t(
          "Une copie de votre progression dans votre iCloud Drive : mots, expressions, leçons, playlists, conversations et réglages. Si ce Mac est effacé ou remplacé, vous la retrouvez en un clic. Elle ne passe par aucun serveur : seul votre compte iCloud la reçoit.",
          "A copy of your progress in your iCloud Drive: words, phrases, lessons, playlists, conversations and settings. If this Mac is wiped or replaced, you get it back in one click. It goes through no server: only your iCloud account receives it.",
        ),
        keys: "sauvegarde icloud drive dossier restaurer historique audio videos backup folder restore history",
      },
      {
        id: "lingq",
        icon: "import",
        label: "LingQ",
        lead: t(
          "Retrouvez dans Lumen tout votre parcours LingQ : mots connus et ignorés, LingQ avec leurs traductions et leurs notes, et les leçons de tous vos cours avec leur audio.",
          "Bring your whole LingQ journey into Lumen: known and ignored words, LingQs with their translations and notes, and the lessons of all your courses with their audio.",
        ),
        keys: "lingq import cle api vocabulaire lecons key vocabulary lessons",
      },
    ],
  },
  {
    title: null,
    tabs: [
      {
        id: "about",
        icon: "orb",
        label: t("À propos", "About"),
        lead: t("Lumen, ses nouveautés, sa visite guidée et ceux qui le rendent possible.", "Lumen, what's new, its guided tour and those who make it possible."),
        keys: "a propos version mises a jour nouveautes visite guidee guide accueil credits dossier donnees about updates what's new whats new tour welcome credits data folder",
      },
    ],
  },
];

/** Sans accents ni majuscules, pour la recherche. */
function fold(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase();
}

function matches(tab: Tab, q: string): boolean {
  const hay = fold(`${tab.label} ${tab.lead} ${tab.keys}`);
  return fold(q)
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

/** Petit voyant d'une catégorie : téléchargement en cours, à faire, mise à jour. */
function useBadges(): Partial<Record<SettingsTab, "busy" | "warn" | "light">> {
  const models = useApp((s) => s.models);
  const downloading = useApp((s) => Object.keys(s.downloads).length > 0);
  const backupError = useBackup((s) => !!s.status?.enabled && !!s.status.error && !s.status.running);
  const lingq = useLingq((s) => s.phase === "importing");
  const update = useUpdate((s) => s.phase === "available" || s.phase === "ready");
  const noLlm = models.length > 0 && !models.some((m) => m.kind === "llm" && m.installed);
  return {
    ai: downloading ? "busy" : noLlm ? "warn" : undefined,
    backup: backupError ? "warn" : undefined,
    lingq: lingq ? "busy" : undefined,
    about: update ? "light" : undefined,
  };
}

function TabIcon({ tab, size = 16 }: { tab: Tab; size?: number }) {
  return tab.icon === "orb" ? <Orb size={size - 2} /> : <Icon name={tab.icon} size={size} />;
}

export function Settings() {
  const tab = useApp((s) => s.settingsTab);
  const openSettings = useApp((s) => s.openSettings);
  const still = !!useReducedMotion();
  const [query, setQuery] = useState("");
  const mainRef = useRef<HTMLDivElement>(null);
  const navRef = useRef<HTMLDivElement>(null);
  const badges = useBadges();
  const all = groups();
  const tabs = all.flatMap((g) => g.tabs);
  const current = tabs.find((x) => x.id === tab) ?? tabs[0];
  const found = query.trim() ? tabs.filter((x) => matches(x, query)) : tabs;

  const pick = (id: SettingsTab) => {
    if (id !== tab) openSettings(id);
  };

  // la recherche ouvre d'elle-même la première page trouvée
  const first = found[0]?.id;
  const shown = found.some((x) => x.id === tab);
  useEffect(() => {
    if (query.trim() && first && !shown) openSettings(first);
  }, [query, first, shown, openSettings]);

  // ↑ ↓ dans le menu : page précédente ou suivante (parmi celles trouvées)
  const onNavKey = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    if ((e.target as HTMLElement).closest("input")) return;
    e.preventDefault();
    const i = found.findIndex((x) => x.id === tab);
    const next = found[(i + (e.key === "ArrowDown" ? 1 : -1) + found.length) % found.length];
    if (!next) return;
    pick(next.id);
    navRef.current?.querySelector<HTMLButtonElement>(`[data-tab="${next.id}"]`)?.focus();
  };

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="settings-shell">
        <nav className="set-nav" ref={navRef} aria-label={t("Catégories des réglages", "Settings categories")} onKeyDown={onNavKey}>
          <h1 className="set-nav-title">{t("Réglages", "Settings")}</h1>
          <label className="set-search">
            <Icon name="search" size={14} />
            <input
              value={query}
              placeholder={t("Rechercher", "Search")}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setQuery("");
                if (e.key === "Enter" && found[0]) pick(found[0].id);
              }}
              aria-label={t("Rechercher un réglage", "Search for a setting")}
              spellCheck={false}
            />
            {query && (
              <button className="set-search-clear" onClick={() => setQuery("")} aria-label={t("Effacer la recherche", "Clear the search")}>
                <Icon name="close" size={11} stroke={2.2} />
              </button>
            )}
          </label>
          <div className="set-nav-list" role="tablist" aria-orientation="vertical">
            {all.map((g, gi) => {
              const shown = g.tabs.filter((x) => found.includes(x));
              if (!shown.length) return null;
              return (
                <Fragment key={gi}>
                  {g.title && !query.trim() && <div className="set-nav-group">{g.title}</div>}
                  {!g.title && gi > 0 && !query.trim() && <div className="set-nav-gap" />}
                  {shown.map((x) => {
                    const on = x.id === tab;
                    const badge = badges[x.id];
                    return (
                      <button
                        key={x.id}
                        data-tab={x.id}
                        role="tab"
                        aria-selected={on}
                        tabIndex={on ? 0 : -1}
                        className={`set-nav-item ${on ? "on" : ""}`}
                        onClick={() => pick(x.id)}
                        title={x.label}
                      >
                        {on && <motion.span layoutId="set-nav-pill" className="set-nav-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
                        <span className={`set-tile ${on ? "lit" : ""}`}>
                          <TabIcon tab={x} size={15} />
                        </span>
                        <span className="set-nav-label">{x.label}</span>
                        {badge && <span className={`set-badge ${badge}`} />}
                      </button>
                    );
                  })}
                </Fragment>
              );
            })}
            {!found.length && <p className="set-nav-empty">{t("Aucun réglage ne correspond.", "No setting matches.")}</p>}
          </div>
          <p className="set-nav-foot">{t("Tout fonctionne sur votre Mac, sans compte.", "Everything works on your Mac, with no account.")}</p>
        </nav>

        <div className="set-main" ref={mainRef} role="tabpanel" aria-label={current.label}>
          <AnimatePresence
            mode="wait"
            initial={false}
            onExitComplete={() => {
              // chaque page s'ouvre en haut
              if (mainRef.current) mainRef.current.scrollTop = 0;
            }}
          >
            <motion.div
              key={current.id}
              className="set-pane"
              initial={still ? { opacity: 0 } : { opacity: 0, y: 12, filter: "blur(6px)" }}
              animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
              exit={still ? { opacity: 0 } : { opacity: 0, y: -6, filter: "blur(4px)" }}
              transition={{ duration: 0.26, ease: EASE }}
            >
              <header className="set-pane-head">
                <motion.span
                  className="set-pane-icon"
                  initial={still ? false : { scale: 0.7, opacity: 0 }}
                  animate={{ scale: 1, opacity: 1 }}
                  transition={{ type: "spring", stiffness: 380, damping: 22, delay: 0.05 }}
                >
                  <TabIcon tab={current} size={22} />
                </motion.span>
                <div>
                  <h2>{current.label}</h2>
                  <p>
                    {current.lead}
                    {current.id === "ai" && (
                      <>
                        {" "}
                        <button className="guide-more" onClick={() => useApp.getState().openGuide(3)}>
                          {t("Qu'est-ce qu'un modèle ?", "What is a model?")}
                        </button>
                      </>
                    )}
                  </p>
                </div>
              </header>
              <Pane tab={current.id} />
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </>
  );
}

function Pane({ tab }: { tab: SettingsTab }) {
  switch (tab) {
    case "general":
      return <GeneralPane />;
    case "langs":
      return <LangsPane />;
    case "reading":
      return <ReadingPane />;
    case "voice":
      return <VoicePane />;
    case "discover":
      return <DiscoverPane />;
    case "podcasts":
      return <PodcastSection />;
    case "ai":
      return <AiPane />;
    case "videos":
      return <VideosPane />;
    case "backup":
      return <BackupSection />;
    case "lingq":
      return <LingqSection />;
    case "about":
      return <AboutPane />;
  }
}

/** Une partie d'une page : petit titre, explication facultative, puis ses réglages. */
function Section({ title, note, children }: { title?: string; note?: ReactNode; children: ReactNode }) {
  return (
    <section className="set-section">
      {title && <h3>{title}</h3>}
      {note && <p>{note}</p>}
      {children}
    </section>
  );
}

// ---------- Général ----------

function GeneralPane() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  return (
    <>
      <Section
        title={t("Langue de l'interface", "Interface language")}
        note={t("C'est aussi la langue des traductions, des explications du chat et des dictionnaires.", "It is also the language of translations, chat explanations and dictionaries.")}
      >
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{t("Lumen en", "Lumen in")}</strong>
            </div>
            <Segmented
              id="ui-lang"
              label={t("Langue de l'interface", "Interface language")}
              value={settings.ui_lang || "fr"}
              onChange={(v) => void setSetting("ui_lang", v as UiLang)}
              options={[
                { value: "fr", label: "Français" },
                { value: "en", label: "English" },
              ]}
            />
          </div>
        </div>
      </Section>
      <Section
        title={t("Apparence", "Appearance")}
        note={t("« Système » suit le Mac : l'aube le jour, une nuit chaude le soir.", "“System” follows your Mac: dawn by day, a warm night in the evening.")}
      >
        <ThemePicker value={settings.theme || "system"} onChange={(v) => void setSetting("theme", v)} />
      </Section>
    </>
  );
}

/** Thème : trois petites fenêtres de Lumen, en clair, en sombre, ou partagées. */
function ThemePicker({ value, onChange }: { value: string; onChange(v: string): void }) {
  const opts = [
    { v: "system", label: t("Système", "System") },
    { v: "light", label: t("Clair", "Light") },
    { v: "dark", label: t("Sombre", "Dark") },
  ];
  const art = (base: "light" | "dark") => (
    <span className={`theme-art paper-${base}`}>
      <i className="ta-side">
        <b className="ta-orb" />
        <b />
        <b />
        <b />
      </i>
      <i className="ta-main">
        <b className="ta-title" />
        <span className="ta-line">
          <b className="ta-w s4" />
          <b className="ta-w s0" />
          <b className="ta-w s4" />
        </span>
        <span className="ta-line">
          <b className="ta-w s1" />
          <b className="ta-w s4" />
          <b className="ta-w s0" />
        </span>
      </i>
    </span>
  );
  return (
    <div className="theme-picker" role="radiogroup" aria-label={t("Thème", "Theme")}>
      {opts.map((o) => (
        <button key={o.v} role="radio" aria-checked={value === o.v} className={`theme-opt ${value === o.v ? "on" : ""}`} onClick={() => onChange(o.v)}>
          {value === o.v && <motion.span layoutId="theme-ring" className="look-ring" transition={{ type: "spring", stiffness: 520, damping: 40 }} />}
          {o.v === "system" ? (
            <span className="theme-split">
              {art("light")}
              {art("dark")}
            </span>
          ) : (
            art(o.v as "light" | "dark")
          )}
          <span className="theme-label">{o.label}</span>
        </button>
      ))}
    </div>
  );
}

// ---------- Langues ----------

/** « · dictionnaire hors ligne inclus », « · dictionnaire en téléchargement… » */
function DictNote({ code }: { code: LangCode }) {
  const d = useDictStatus(code);
  if (!d) return null;
  if (!d.exists) return <>{t(" · traduction par l'IA et voix naturelle", " · AI translation and natural voice")}</>;
  if (d.bundled) return <>{t(" · dictionnaire hors ligne inclus", " · offline dictionary included")}</>;
  if (d.ready) return <>{t(" · dictionnaire hors ligne prêt", " · offline dictionary ready")}</>;
  if (d.downloading) return <>{t(" · dictionnaire en téléchargement…", " · dictionary downloading…")}</>;
  return <>{t(" · dictionnaire hors ligne, téléchargé au premier usage", " · offline dictionary, downloaded on first use")}</>;
}

function LangsPane() {
  const setSetting = useApp((s) => s.setSetting);
  const lang = useApp((s) => s.lang)();
  const langs = useApp((s) => s.langs)();
  const refreshKnown = useApp((s) => s.refreshKnown);
  const bump = useApp((s) => s.bumpLibrary);

  const toggleLang = async (code: LangCode) => {
    const has = langs.includes(code);
    if (has && langs.length === 1) return;
    const next = has ? langs.filter((l) => l !== code) : [...langs, code];
    await setSetting("langs", next.join(","));
    if (!has) {
      // la leçon d'accueil, sans doublon si la langue avait déjà été étudiée
      const s = STARTERS[code];
      const exists = (await api().lessonsList(code)).some((x) => x.title === s.title);
      if (!exists) await api().lessonCreate({ lang: code, title: s.title, text: s.text, collection: starterCollection() });
      bump();
    }
    if (has && code === lang) {
      await setSetting("lang", next[0]);
      await refreshKnown();
    }
  };

  return (
    <>
      <Section title={t("Vos langues", "Your languages")}>
        <div className="set-card">
          {langs.map((code) => {
            const l = langInfo(code);
            return (
              <div key={l.code} className="set-row">
                <span className="lang-badge" style={{ background: l.color }}>
                  {l.badge}
                </span>
                <div className="grow">
                  <strong>{l.name}</strong>
                  <span>
                    {l.native}
                    <DictNote code={l.code} />
                  </span>
                </div>
                <Switch on onChange={() => toggleLang(l.code)} label={t(`Ne plus étudier ${langLower(l.code)}`, `Stop studying ${l.name}`)} />
              </div>
            );
          })}
        </div>
      </Section>
      <Section title={t("Ajouter une langue", "Add a language")} note={t(`${LANGS.length} langues, chacune avec son dictionnaire hors ligne et sa voix naturelle.`, `${LANGS.length} languages, each with its offline dictionary and natural voice.`)}>
        <div className="lang-chips">
          {LANGS.filter((l) => !langs.includes(l.code)).map((l) => (
            <button key={l.code} className="lang-chip" onClick={() => toggleLang(l.code)} title={`${l.name} · ${l.native}`}>
              <span className="lang-badge" style={{ background: l.color }}>
                {l.badge}
              </span>
              {l.name}
              <Icon name="plus" size={13} stroke={2.2} />
            </button>
          ))}
        </div>
      </Section>
    </>
  );
}

// ---------- Lecture ----------

/** Aperçu de la page avec l'affichage choisi : police, taille, couleur, interligne, largeur, marquage. */
function LookPreview() {
  const settings = useApp((s) => s.settings);
  const lang = useApp((s) => s.lang)();
  const look = readerLook(settings);
  const { lang: sampleLang, sample } = useMemo(() => sampleFor(lang), [lang]);
  const parts = useMemo(() => splitWords(sample.text), [sample]);
  let n = -1;
  return (
    <div className={`look-preview ${look.className}`} style={look.style} aria-hidden="true">
      <span className="look-preview-tag">{t("Aperçu", "Preview")}</span>
      <div className={`page ${settings.word_style === "line" ? "mark-line" : "mark-tint"}`} lang={sampleLang} dir={langInfo(sampleLang).rtl ? "rtl" : undefined}>
        <p>
          {parts.map((p, k) => {
            if (k % 2 === 0) return p;
            const st = sample.st[++n] ?? 4;
            return (
              <span key={k} className={`w s${st}`}>
                {p}
              </span>
            );
          })}
        </p>
      </div>
    </div>
  );
}

function ReadingPane() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const lang = useApp((s) => s.lang)();
  return (
    <>
      <LookPreview />
      <Section title={t("La page", "The page")}>
        <div className="set-card">
          <div className="set-row set-stack">
            <div className="grow">
              <strong>{t("Mise en page", "Layout")}</strong>
              <span>
                {t(
                  "Pages : le texte tient dans l'écran, on tourne la page avec les flèches (ou deux doigts sur le trackpad). Défilement : une longue page qu'on fait défiler.",
                  "Pages: the text fits on the screen and you turn the page with the arrows (or two fingers on the trackpad). Scrolling: one long page you scroll through.",
                )}
              </span>
            </div>
            <LayoutPicker id="set" value={settings.reader_layout === "scroll" ? "scroll" : "pages"} onChange={(v) => setSetting("reader_layout", v)} />
          </div>
          <div className="set-row set-stack">
            <div className="grow">
              <strong>{t("Police", "Font")}</strong>
            </div>
            <FontPicker id="set" lang={lang} value={settings.read_font || "literata"} onChange={(v) => setSetting("read_font", v)} />
          </div>
          <div className="set-row set-stack">
            <div className="grow">
              <strong>{t("Couleur de la page", "Page color")}</strong>
              <span>{t("« Auto » suit le thème de Lumen. Le fond et le texte vont ensemble, pour bien se lire.", "“Auto” follows Lumen's theme. Background and text come as a pair, so they stay easy to read.")}</span>
            </div>
            <PaperPicker id="set" value={settings.read_paper || "auto"} onChange={(v) => setSetting("read_paper", v)} font={readFont(settings.read_font).stack} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Taille du texte", "Text size")}</strong>
              <span className="num">{settings.font_size} px</span>
            </div>
            <input
              className="range"
              style={{ width: 220 }}
              type="range"
              min={SIZE_MIN}
              max={SIZE_MAX}
              value={settings.font_size}
              onChange={(e) => setSetting("font_size", e.target.value)}
              aria-label={t("Taille du texte", "Text size")}
            />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Interligne", "Line spacing")}</strong>
            </div>
            <Segmented id="lh" value={settings.line_height} onChange={(v) => setSetting("line_height", v)} options={lineHeightOptions()} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Largeur des lignes", "Line width")}</strong>
              <span>{t("Des lignes plus courtes se lisent plus facilement", "Shorter lines are easier to read")}</span>
            </div>
            <Segmented id="rw" value={settings.read_width || "normal"} onChange={(v) => setSetting("read_width", v)} options={widthOptions()} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Marquage des mots", "Word highlighting")}</strong>
              <span>{t("Teinte douce ou simple soulignement", "Soft tint or simple underline")}</span>
            </div>
            <Segmented
              id="ws"
              value={settings.word_style}
              onChange={(v) => setSetting("word_style", v)}
              options={[
                { value: "tint", label: t("Teinte", "Tint") },
                { value: "line", label: t("Soulignement", "Underline") },
              ]}
            />
          </div>
        </div>
      </Section>
      <Section title={t("En lisant", "While reading")}>
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{t("Terminer la page marque les mots bleus comme connus", "Finishing the page marks blue words as known")}</strong>
              <span>{t("Le principe de LingQ : un mot lu sans être consulté est compris.", "The LingQ principle: a word you read without looking it up is understood.")}</span>
            </div>
            <Switch on={settings.finish_marks_known !== "0"} onChange={(v) => setSetting("finish_marks_known", v ? "1" : "0")} label={t("Marquer comme connus", "Mark as known")} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Traduire automatiquement la phrase", "Translate the sentence automatically")}</strong>
              <span>{t("Après chaque mot touché", "After each word you tap")}</span>
            </div>
            <Switch on={settings.auto_sentence !== "0"} onChange={(v) => setSetting("auto_sentence", v ? "1" : "0")} label={t("Traduire la phrase", "Translate the sentence")} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Prononcer le mot touché", "Pronounce the word you tap")}</strong>
              <span>{t("Et le passage surligné, seulement quand l'audio de la leçon est en pause", "And the highlighted passage, only while the lesson audio is paused")}</span>
            </div>
            <Switch on={settings.auto_pronounce !== "0"} onChange={(v) => setSetting("auto_pronounce", v ? "1" : "0")} label={t("Prononcer le mot touché", "Pronounce the word you tap")} />
          </div>
        </div>
      </Section>
    </>
  );
}

// ---------- Voix ----------

function VoicePane() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const models = useApp((s) => s.models);
  const lang = useApp((s) => s.lang)();
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>([]);
  const voiceKey = `voice_${lang}`;

  useEffect(() => {
    loadVoices().then(() => setVoices(voicesFor(lang)));
  }, [lang]);

  return (
    <>
      <Section
        title={t("Voix naturelle", "Natural voice")}
        note={t(
          "Elle prononce les mots et les expressions que vous touchez, et lit toute une leçon de texte avec le bouton « Créer l'audio ». Elle est calculée sur votre Mac, seulement quand vous en avez besoin.",
          "It pronounces the words and phrases you tap, and reads a whole text lesson with the “Create audio” button. It is computed on your Mac, only when you need it.",
        )}
      >
        <div className="set-card">
          {models
            .filter((m) => m.kind === "tts")
            .map((m) => (
              <ModelLine key={m.id} m={m} />
            ))}
          {models.some((m) => m.kind === "tts" && m.installed) && (
            <div className="set-row">
              <div className="grow">
                <strong>{t(`Voix pour ${theLang(lang)}`, `Voice for ${theLang(lang)}`)}</strong>
                <span>{t("10 voix, comprises dans le téléchargement ; elle lit aussi l'audio créé pour vos leçons", "10 voices, included in the download; it also reads the audio created for your lessons")}</span>
              </div>
              <select className="select" value={naturalVoiceFor(lang)} onChange={(e) => setSetting(`tts_voice_${lang}`, e.target.value)} aria-label={t("Voix naturelle", "Natural voice")}>
                {[...new Set(naturalVoices().map((v) => v.group))].map((g) => (
                  <optgroup key={g} label={g}>
                    {naturalVoices()
                      .filter((v) => v.group === g)
                      .map((v) => (
                        <option key={v.id} value={v.id}>
                          {v.name}
                        </option>
                      ))}
                  </optgroup>
                ))}
              </select>
              <button
                className="icon-btn"
                onClick={() => void pronounce(STARTERS[lang].text.split(/[.!?。]/)[0].split(/\s+/).slice(0, 8).join(" "), lang, settings[voiceKey])}
                aria-label={t("Écouter la voix naturelle", "Listen to the natural voice")}
              >
                <Icon name="speaker" />
              </button>
            </div>
          )}
        </div>
      </Section>
      <Section title={t("Lecture à voix haute", "Reading aloud")} note={t("Les voix du système lisent la leçon à voix haute sans préparation.", "The system voices read the lesson aloud with no preparation.")}>
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{t(`Voix du système pour ${theLang(lang)}`, `System voice for ${theLang(lang)}`)}</strong>
              <span>
                {voices.length
                  ? t(`${voices.length} voix disponible${voices.length > 1 ? "s" : ""}`, `${voices.length} voice${voices.length > 1 ? "s" : ""} available`)
                  : t("Aucune voix installée pour cette langue", "No voice installed for this language")}
                {t(" · d'autres voix dans Réglages Système › Accessibilité › Contenu énoncé", " · more voices in System Settings › Accessibility › Spoken Content")}
              </span>
            </div>
            <select className="select" value={settings[voiceKey] ?? ""} onChange={(e) => setSetting(voiceKey, e.target.value)} aria-label={t("Voix", "Voice")}>
              <option value="">{t("Automatique (la plus naturelle)", "Automatic (the most natural)")}</option>
              {voices.map((v) => (
                <option key={v.voiceURI} value={v.voiceURI}>
                  {v.name} · {v.lang}
                </option>
              ))}
            </select>
            <button
              className="icon-btn"
              onClick={() => sayWord(STARTERS[lang].text.split(".")[0], lang, settings[voiceKey], Number(settings.tts_rate) || 0.95)}
              aria-label={t("Écouter la voix", "Listen to the voice")}
            >
              <Icon name="speaker" />
            </button>
          </div>
        </div>
      </Section>
    </>
  );
}

// ---------- Découvrir ----------

/** Réglages › Découvrir : la lecture quotidienne des sources, et le niveau de la langue active. */
function DiscoverPane() {
  const lang = useApp((s) => s.lang)();
  const auto = useApp((s) => s.settings.discover_auto) !== "0";
  const setSetting = useApp((s) => s.setSetting);
  const { level, auto: estimatedLevel, estimated, estimate, setLevel } = useLevel(lang);
  const forms = count(estimate.forms, "mot connu", "mots connus", "known word", "known words");
  return (
    <Section
      note={t(
        "Lumen regarde ce que publient des chaînes, des podcasts et des journaux choisis pour vos langues, et vous le propose dans Découvrir. Il lit seulement des listes publiques, sans rien envoyer de personnel ; une vidéo ou un épisode n'est téléchargé que si vous en faites une leçon.",
        "Lumen looks at what channels, podcasts and newspapers chosen for your languages publish, and suggests it in Discover. It only reads public lists and sends nothing personal; a video or an episode is downloaded only if you make it a lesson.",
      )}
    >
      <div className="set-card">
        <div className="set-row">
          <div className="grow">
            <strong>{t("Chercher de nouvelles leçons en arrière-plan", "Look for new lessons in the background")}</strong>
            <span>
              {t(
                "Les actualités toutes les trois heures, les chaînes et les podcasts deux fois par jour, les chansons chaque jour. Sinon, seulement quand vous touchez « Actualiser » dans Découvrir.",
                "News every three hours, channels and podcasts twice a day, songs once a day. Otherwise, only when you tap “Refresh” in Discover.",
              )}
            </span>
          </div>
          <Switch on={auto} onChange={(v) => setSetting("discover_auto", v ? "1" : "0")} label={t("Chercher en arrière-plan", "Look in the background")} />
        </div>
        <div className="set-row">
          <div className="grow">
            <strong>{t(`Votre niveau en ${langLower(lang)}`, `Your level in ${langLower(lang)}`)}</strong>
            <span>
              {estimatedLevel
                ? t(`Estimé d'après ${unitsLabel(estimate)}`, `Estimated from ${unitsLabel(estimate)}`)
                : t(`Choisi par vous (estimé : ${levelName(estimated)})`, `Chosen by you (estimated: ${levelName(estimated)})`)}
            </span>
          </div>
          <Segmented
            id="set-level"
            label={t("Niveau", "Level")}
            value={String(level)}
            onChange={(v) => setLevel(Number(v) === estimated ? null : Number(v))}
            options={LEVELS.map((name, i) => ({ value: String(i + 1), label: name }))}
          />
        </div>
        <LevelExplained lang={lang} forms={forms} />
      </div>
    </Section>
  );
}

/** Comment le niveau est estimé : les formes connues regroupées par mot de base, et le prochain palier. */
function LevelExplained({ lang, forms }: { lang: LangCode; forms: string }) {
  const { estimate: e } = useLevel(lang);
  const span = e.next ? e.next - e.floor : 0;
  const pct = span ? Math.min(100, Math.max(3, ((e.units - e.floor) / span) * 100)) : 100;
  let how: string;
  if (e.unit === "kanji") how = t("En japonais, Lumen compte les kanji que vous connaissez : c'est eux qui ouvrent les textes.", "In Japanese, Lumen counts the kanji you know: they are what opens up texts.");
  else if (e.unit === "syllables") how = t("En vietnamien, Lumen compte les syllabes que vous connaissez : un mot en compte une ou deux.", "In Vietnamese, Lumen counts the syllables you know: a word has one or two.");
  else if (e.exact)
    how = t(
      `Vos ${forms} sont regroupés par mot de base grâce au dictionnaire, comme « parle », « parlait » et « parlé » ne comptent qu'une fois. Les mots que le dictionnaire ignore (noms propres) comptent pour moitié.`,
      `Your ${forms} are grouped by base word thanks to the dictionary, the way “speak”, “spoke” and “spoken” count only once. Words the dictionary doesn't know (proper names) count for half.`,
    );
  else
    how = t(
      `Le dictionnaire ${inLang(lang)} n'est pas encore sur ce Mac : Lumen estime le nombre de mots de base d'après vos ${forms}, selon le nombre de formes qu'ont les mots dans cette langue.`,
      `The ${langLower(lang)} dictionary isn't on this Mac yet: Lumen estimates the number of base words from your ${forms}, based on how many forms words have in this language.`,
    );
  return (
    <div className="set-row level-explained">
      <div className="grow">
        <strong>
          {unitsLabel(e)} · {levelName(e.level)}
        </strong>
        <span>{how}</span>
        {e.next > 0 && (
          <>
            <div className="bar" style={{ marginTop: 10, maxWidth: 360 }}>
              <i style={{ width: `${pct}%` }} />
            </div>
            <span className="num" style={{ marginTop: 6 }}>
              {t(
                `${levelName(e.level + 1)} à partir de ${formatNumber(e.next)} : encore ≈ ${formatNumber(e.next - e.units)}`,
                `${levelName(e.level + 1)} from ${formatNumber(e.next)}: about ${formatNumber(e.next - e.units)} to go`,
              )}
            </span>
          </>
        )}
      </div>
    </div>
  );
}

// ---------- IA locale ----------

function ModelLine({ m }: { m: ModelRow }) {
  const dl = useApp((s) => s.downloads[m.id]);
  const download = useApp((s) => s.download);
  const cancel = useApp((s) => s.cancelDownload);
  const refresh = useApp((s) => s.refreshModels);
  const setSetting = useApp((s) => s.setSetting);
  const settings = useApp((s) => s.settings);
  // la voix n'a qu'un modèle : pas de choix « Utiliser »
  const activeKey = m.kind === "llm" ? "llm_model" : m.kind === "asr" ? "asr_model" : null;
  const isActive = !!activeKey && settings[activeKey] === m.id;

  const remove = async () => {
    const ok = await confirmAsk(
      t(`Supprimer ${m.name} (${formatBytes(m.size)}) de ce Mac ?`, `Delete ${m.name} (${formatBytes(m.size)}) from this Mac?`),
      t("Supprimer le modèle", "Delete the model"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return;
    await api().modelDelete(m.id);
    await refresh();
  };

  return (
    <div className="set-row model-row">
      <div className="grow">
        <strong>
          {m.name} {isActive && m.installed && <span className="chip light" style={{ marginLeft: 6, height: 22 }}>{t("Actif", "Active")}</span>}
        </strong>
        <span>
          {m.detail} · {formatBytes(m.size)}
        </span>
        {dl && !dl.error && (
          <>
            <div className="bar live">
              <i style={{ width: `${(dl.received / Math.max(1, dl.total)) * 100}%` }} />
            </div>
            <span className="num">
              {t(`${formatBytes(dl.received)} sur ${formatBytes(dl.total)}`, `${formatBytes(dl.received)} of ${formatBytes(dl.total)}`)}
              {dl.speed > 0 ? ` · ${formatBytes(dl.speed)}/s` : ""}
            </span>
          </>
        )}
        {dl?.error && <span style={{ color: "var(--danger)" }}>{dl.error}</span>}
      </div>
      {m.installed ? (
        <>
          {activeKey && !isActive && (
            <button className="btn sm soft" onClick={() => setSetting(activeKey, m.id)}>
              {t("Utiliser", "Use")}
            </button>
          )}
          <button className="icon-btn" onClick={remove} aria-label={t(`Supprimer ${m.name}`, `Delete ${m.name}`)}>
            <Icon name="trash" size={16} />
          </button>
        </>
      ) : dl && !dl.error ? (
        <button className="btn sm ghost" onClick={() => cancel(m.id)}>
          {t("Annuler", "Cancel")}
        </button>
      ) : (
        <button className="btn sm outline" onClick={() => (activeKey && setSetting(activeKey, m.id), download(m.id))}>
          <Icon name="download" size={14} /> {m.partial > 0 || dl?.error ? t("Reprendre", "Resume") : t("Télécharger", "Download")}
        </button>
      )}
    </div>
  );
}

function AiPane() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const models = useApp((s) => s.models);
  const download = useApp((s) => s.download);

  const pickProfile = (id: string) => {
    const p = PROFILES.find((x) => x.id === id)!;
    void setSetting("llm_model", p.llm);
    void setSetting("asr_model", p.asr);
    const m = models.find((x) => x.id === p.llm);
    if (m && !m.installed) void download(p.llm);
  };
  const activeProfile = PROFILES.find((p) => p.llm === settings.llm_model)?.id;
  const lines = (kind: ModelRow["kind"]) => models.filter((m) => m.kind === kind).map((m) => <ModelLine key={m.id} m={m} />);

  return (
    <>
      <Section title={t("Profil", "Profile")} note={t("Plus le modèle est grand, plus il saisit les nuances, mais plus il pèse et demande de mémoire.", "The bigger the model, the more nuance it catches, but the more space and memory it needs.")}>
        <div className="profile-grid">
          {PROFILES.map((p) => (
            <button key={p.id} className={`profile ${activeProfile === p.id ? "on" : ""}`} onClick={() => pickProfile(p.id)}>
              <span className="rays">
                {[1, 2, 3].map((r) => (
                  <i key={r} className={r <= p.rays ? "on" : ""} />
                ))}
              </span>
              <strong>{p.name}</strong>
              <span>{p.desc}</span>
              <span className="size">{formatBytes(p.size)}</span>
            </button>
          ))}
        </div>
      </Section>
      <Section title={t("Traduction et chat", "Translation and chat")} note={t("Le sens de chaque mot dans sa phrase, les réécritures et le chat.", "The meaning of each word in its sentence, rewriting and the chat.")}>
        <div className="set-card">{lines("llm")}</div>
      </Section>
      <Section title={t("Transcription audio et vidéo", "Audio and video transcription")} note={t("Whisper écoute vos fichiers et vos vidéos, et situe chaque mot dans le temps pour la lanterne.", "Whisper listens to your files and videos, and places each word in time for the lantern.")}>
        <div className="set-card">{lines("asr")}</div>
      </Section>
      <Section
        title={t("Texte des transcriptions", "Transcript text")}
        note={t(
          "Facultatif, conseillé : Qwen3-ASR écrit le texte, plus juste et sans phrase sautée, et Whisper repère chaque mot dans le temps pour la lanterne. Il couvre 23 langues ; l'estonien, le letton, le lituanien, le slovaque, le slovène, le croate, le bulgare et l'ukrainien restent transcrits par Whisper seul.",
          "Optional, recommended: Qwen3-ASR writes the text, more accurately and without skipped sentences, and Whisper times each word for the lantern. It covers 23 languages; Estonian, Latvian, Lithuanian, Slovak, Slovenian, Croatian, Bulgarian and Ukrainian are still transcribed by Whisper alone.",
        )}
      >
        <div className="set-card">{lines("asrtext")}</div>
      </Section>
    </>
  );
}

// ---------- Vidéos en ligne ----------

function VideosPane() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const info = useApp((s) => s.info);
  return (
    <Section note={t("Collez un lien dans « Importer » : Lumen en fait une leçon que vous regardez avec la transcription synchronisée.", "Paste a link in “Import”: Lumen turns it into a lesson you watch with the transcript in sync.")}>
      <div className="set-card">
        <div className="set-row">
          <div className="grow">
            <strong>{t("Navigateur connecté à YouTube", "Browser signed in to YouTube")}</strong>
            <span>{t("Utilisé seulement si YouTube demande de se connecter pour une vidéo.", "Used only if YouTube asks you to sign in for a video.")}</span>
          </div>
          <select className="select" value={settings.youtube_browser ?? ""} onChange={(e) => setSetting("youtube_browser", e.target.value)} aria-label={t("Navigateur", "Browser")}>
            <option value="">{t("Aucun", "None")}</option>
            <option value="safari">Safari</option>
            <option value="chrome">Chrome</option>
            <option value="firefox">Firefox</option>
            <option value="brave">Brave</option>
            <option value="edge">Edge</option>
            <option value="vivaldi">Vivaldi</option>
            <option value="opera">Opera</option>
          </select>
        </div>
        <div className="set-row">
          <div className="grow">
            <strong>{t("Composants vidéo", "Video components")}</strong>
            <span>
              {info?.ytdlp
                ? t("Installés et tenus à jour automatiquement.", "Installed and kept up to date automatically.")
                : t("Installés automatiquement au premier import (environ 40 Mo).", "Installed automatically on the first import (about 40 MB).")}
            </span>
          </div>
          <span className={`dot ${info?.ytdlp ? "ok" : ""}`} />
        </div>
      </div>
    </Section>
  );
}

// ---------- À propos ----------

function AboutPane() {
  const info = useApp((s) => s.info);
  const openGuide = useApp((s) => s.openGuide);
  const openNews = useApp((s) => s.openNews);
  const startTour = useApp((s) => s.startTour);
  const upd = useUpdate();
  const version = info?.version.match(/\d+\.\d+\.\d+/)?.[0] ?? "";

  const updateLine =
    upd.phase === "checking"
      ? t("Recherche en cours…", "Checking…")
      : upd.phase === "uptodate"
        ? t("Lumen est à jour.", "Lumen is up to date.")
        : upd.phase === "available"
          ? t(`La version ${upd.version} est disponible.`, `Version ${upd.version} is available.`)
          : upd.phase === "downloading"
            ? t(`Téléchargement… ${Math.round(upd.progress * 100)} %`, `Downloading… ${Math.round(upd.progress * 100)}%`)
            : upd.phase === "ready"
              ? t("Installée : redémarrez Lumen pour l'utiliser.", "Installed: restart Lumen to use it.")
              : upd.phase === "error"
                ? t(`Impossible de vérifier : ${upd.error}`, `Couldn't check: ${upd.error}`)
                : t("Lumen vérifie automatiquement au démarrage et toutes les six heures.", "Lumen checks automatically at launch and every six hours.");

  return (
    <>
      <div className="about-hero">
        <span className="about-orb">
          <Orb size={58} />
        </span>
        <div>
          <strong className="about-name">Lumen</strong>
          <span className="about-version num">{version ? t(`Version ${version}`, `Version ${version}`) : ""}</span>
          <span className="about-tag">
            {t("Apprendre une langue en lisant et en écoutant. Toute l'intelligence vit sur votre Mac.", "Learn a language by reading and listening. All the intelligence lives on your Mac.")}
          </span>
        </div>
      </div>

      <Section title={t("Découvrir Lumen", "Getting to know Lumen")}>
        <div className="set-card">
          <div className="set-row">
            <span className="set-row-icon">
              <Icon name="sparkle" size={16} />
            </span>
            <div className="grow">
              <strong>{t("Nouveautés", "What's new")}</strong>
              <span>{t("Ce qui a changé dans chaque version de Lumen.", "What changed in each version of Lumen.")}</span>
            </div>
            <button className="btn sm soft" onClick={() => openNews("all")}>
              {t("Voir les nouveautés", "See what's new")}
            </button>
          </div>
          <div className="set-row">
            <span className="set-row-icon">
              <Icon name="book" size={16} />
            </span>
            <div className="grow">
              <strong>{t("Visite guidée", "Guided tour")}</strong>
              <span>{t("Pas à pas, dans une vraie leçon : les couleurs, le panneau du mot, les modèles d'IA, la lanterne.", "Step by step, inside a real lesson: colors, the word panel, AI models, the lantern.")}</span>
            </div>
            <button className="btn sm primary glow" onClick={() => void startTour()}>
              {t("Lancer la visite", "Start the tour")}
            </button>
          </div>
          <div className="set-row">
            <span className="set-row-icon">
              <Icon name="bulb" size={16} />
            </span>
            <div className="grow">
              <strong>{t("Petit guide", "Short guide")}</strong>
              <span>{t("Le principe de Lumen et le rôle de chaque modèle d'IA, en quatre images.", "How Lumen works and what each AI model does, in four pictures.")}</span>
            </div>
            <button className="btn sm soft" onClick={() => openGuide()}>
              {t("Ouvrir le guide", "Open the guide")}
            </button>
          </div>
          <div className="set-row">
            <span className="set-row-icon">
              <Icon name="sun" size={16} />
            </span>
            <div className="grow">
              <strong>{t("Écran d'accueil", "Welcome screen")}</strong>
              <span>{t("Revoir l'aube de Lumen. Vos leçons et votre vocabulaire restent intacts.", "See Lumen's dawn again. Your lessons and vocabulary stay intact.")}</span>
            </div>
            <button className="btn sm soft" onClick={() => useApp.getState().setReplay(true)}>
              {t("Revoir l'accueil", "Replay the welcome")}
            </button>
          </div>
        </div>
      </Section>

      <Section title={t("Cette installation", "This installation")}>
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{t("Mises à jour", "Updates")}</strong>
              <span>{updateLine}</span>
            </div>
            {upd.phase === "available" ? (
              <button className="btn sm primary" onClick={upd.install}>
                {t("Mettre à jour", "Update")}
              </button>
            ) : upd.phase === "ready" ? (
              <button className="btn sm primary" onClick={upd.restart}>
                {t("Redémarrer", "Restart")}
              </button>
            ) : (
              <button className="btn sm soft" onClick={() => upd.check(true)} disabled={upd.phase === "checking" || upd.phase === "downloading"}>
                {t("Rechercher", "Check")}
              </button>
            )}
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Données", "Data")}</strong>
              <span className="set-path">{info?.data_dir ?? ""}</span>
            </div>
            {isTauri && info && (
              <button className="btn sm soft" onClick={() => import("@tauri-apps/plugin-opener").then((o) => o.revealItemInDir(info.data_dir + "/lumen.db"))}>
                {t("Ouvrir le dossier", "Open the folder")}
              </button>
            )}
          </div>
        </div>
      </Section>

      <Section title={t("Crédits", "Credits")}>
        <p className="set-credits">
          {t(
            "Dictionnaires : Wiktionnaire via kaikki.org ; JMdict et KANJIDIC2, selon la licence de l'Electronic Dictionary Research and Development Group ; corpus Universal Dependencies (tous CC BY-SA 4.0). Traduction : Qwen3.5 (Apache 2.0) par llama.cpp (MIT). Transcription : Qwen3-ASR (Apache 2.0) par llama.cpp et Whisper (MIT) par whisper.cpp. Voix : Supertonic 3 par sherpa-onnx. Polices : Literata, Newsreader, Geist (OFL).",
            "Dictionaries: Wiktionary via kaikki.org; JMdict and KANJIDIC2, used under the Electronic Dictionary Research and Development Group licence; Universal Dependencies treebanks (all CC BY-SA 4.0). Translation: Qwen3.5 (Apache 2.0) with llama.cpp (MIT). Transcription: Qwen3-ASR (Apache 2.0) with llama.cpp, and Whisper (MIT) with whisper.cpp. Voice: Supertonic 3 with sherpa-onnx. Fonts: Literata, Newsreader, Geist (OFL).",
          )}
        </p>
      </Section>
    </>
  );
}
