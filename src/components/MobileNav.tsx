import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useChat } from "../lib/chat";
import { useDiscoverNews } from "../lib/discover";
import { count, formatNumber, t } from "../lib/i18n";
import { LANGS, langInfo } from "../lib/langs";
import { isIPad, isMobile } from "../lib/platform";
import { progressOf } from "../lib/playlists";
import { useApp, type View } from "../lib/store";
import type { LangCode, LessonSummary } from "../lib/types";
import { useUserName } from "../lib/user";
import { Avatar } from "./Avatar";
import { BackupCard } from "./BackupCard";
import { Cover } from "./Cover";
import { Icon, type IconName } from "./Icon";
import { ImportQueueCard } from "./ImportQueueCard";
import { LingqCard } from "./LingqCard";
import { CountUp } from "./ui";

/*
 * iPhone : la barre latérale du Mac devient une barre d'onglets en verre, posée
 * en bas de l'écran (à portée du pouce), avec l'import au centre, une lumière.
 * La leçon en cours flotte au-dessus dans une capsule, comme le mini-lecteur de
 * Musique ; « Plus » ouvre le reste (playlists, chat, vocabulaire, réglages, langue).
 * Dans une leçon, tout s'efface : la lecture prend l'écran.
 */

/** Écran étroit d'iPhone (ou iPad partagé en deux) : navigation en bas. */
export function useCompact(): boolean {
  const [w, setW] = useState(() => (typeof window === "undefined" ? 1200 : window.innerWidth));
  useEffect(() => {
    if (!isMobile) return;
    const f = () => setW(window.innerWidth);
    window.addEventListener("resize", f);
    return () => window.removeEventListener("resize", f);
  }, []);
  return isMobile && (!isIPad || w < 700);
}

const SPRING = { type: "spring", stiffness: 520, damping: 40 } as const;

type Tab = { view: View | "more"; label: string; icon: IconName };

const tabs = (): Tab[] => [
  { view: "library", label: t("Bibliothèque", "Library"), icon: "library" },
  { view: "discover", label: t("Découvrir", "Discover"), icon: "sparkle" },
  { view: "progress", label: t("Progrès", "Progress"), icon: "chart" },
  { view: "more", label: t("Plus", "More"), icon: "more" },
];

/** Vues rangées sous « Plus » : l'onglet s'allume quand on y est. */
const MORE: View[] = ["playlists", "chat", "vocab", "settings"];

export function MobileNav() {
  const view = useApp((s) => s.view);
  const go = useApp((s) => s.go);
  const openImport = useApp((s) => s.openImport);
  const lang = useApp((s) => s.lang)();
  const streak = useApp((s) => s.streak);
  const chatBusy = useChat((s) => !!s.pending);
  const news = useDiscoverNews(lang);
  const fresh = view === "discover" ? 0 : news.fresh;
  const [more, setMore] = useState(false);
  const reading = view === "reader";

  const items = tabs();
  const left = items.slice(0, 2);
  const right = items.slice(2);
  const activeOf = (tab: Tab) => (tab.view === "more" ? more || MORE.includes(view) : !more && view === tab.view);

  const tabButton = (tab: Tab) => {
    const on = activeOf(tab);
    return (
      <button
        key={tab.view}
        className={`m-tab ${on ? "on" : ""}`}
        onClick={() => {
          if (tab.view === "more") setMore((m) => !m);
          else {
            setMore(false);
            go(tab.view);
          }
        }}
        aria-current={on ? "page" : undefined}
        aria-label={tab.label}
      >
        {on && <motion.span layoutId="m-tab-pill" className="m-tab-pill" transition={SPRING} />}
        <span className="m-tab-icon">
          <Icon name={tab.icon} size={21} />
          {tab.view === "discover" && fresh > 0 && <span className="m-badge num">{fresh > 99 ? "99+" : fresh}</span>}
          {tab.view === "progress" && streak?.today_done && <span className="m-flame" aria-hidden="true" />}
          {tab.view === "more" && chatBusy && view !== "chat" && <span className="m-live" aria-label={t("Réponse en cours", "Answer in progress")} />}
        </span>
        <span className="m-tab-label">{tab.label}</span>
      </button>
    );
  };

  return (
    <>
      <AnimatePresence>
        {!reading && (
          <motion.div
            key="dock"
            className="m-dock"
            initial={{ y: 40, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            exit={{ y: 40, opacity: 0 }}
            transition={{ type: "spring", stiffness: 380, damping: 34 }}
          >
            <NowReading />
            <nav className="m-tabs" aria-label="Navigation">
              {left.map(tabButton)}
              <button className="m-import" onClick={() => openImport()} aria-label={t("Importer", "Import")} data-tour="import">
                <span className="m-import-glow" aria-hidden="true" />
                <Icon name="plus" size={22} stroke={2.2} />
              </button>
              {right.map(tabButton)}
            </nav>
          </motion.div>
        )}
      </AnimatePresence>
      <MoreSheet open={more && !reading} onClose={() => setMore(false)} />
    </>
  );
}

/** La leçon en cours, à un toucher, au-dessus des onglets. */
function NowReading() {
  const lessonId = useApp((s) => s.lessonId);
  const openLesson = useApp((s) => s.openLesson);
  const lang = useApp((s) => s.lang)();
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [lesson, setLesson] = useState<LessonSummary | null>(null);

  useEffect(() => {
    if (!lessonId) {
      setLesson(null);
      return;
    }
    api()
      .lessonsList(lang)
      .then((l) => setLesson(l.find((x) => x.id === lessonId) ?? null))
      .catch(() => {});
  }, [lessonId, lang, libraryVersion]);

  // la capsule prend de la place au-dessus des onglets : le contenu s'arrête plus haut
  useEffect(() => {
    document.documentElement.classList.toggle("m-has-now", !!lesson);
    return () => document.documentElement.classList.remove("m-has-now");
  }, [lesson]);

  return (
    <AnimatePresence initial={false}>
      {lesson && (
        <motion.button
          key={lesson.id}
          className="m-now"
          onClick={() => openLesson(lesson.id)}
          initial={{ opacity: 0, y: 12, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8, scale: 0.98 }}
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
          aria-label={t(`Reprendre ${lesson.title}`, `Resume ${lesson.title}`)}
        >
          <span className="m-now-cover">
            <Cover lesson={lesson} bare />
          </span>
          <span className="m-now-text">
            <span className="m-now-eyebrow">{t("Lecture en cours", "Now reading")}</span>
            <strong>{lesson.kind === "book" && lesson.collection ? lesson.collection : lesson.title}</strong>
          </span>
          <span className="m-now-go">
            <Icon name="play" size={15} />
          </span>
          {progressOf(lesson) > 0 && <span className="m-now-bar" style={{ width: `${Math.round(progressOf(lesson) * 100)}%` }} />}
        </motion.button>
      )}
    </AnimatePresence>
  );
}

/** « Plus » : le reste de Lumen, et la langue étudiée. */
function MoreSheet({ open, onClose }: { open: boolean; onClose(): void }) {
  const view = useApp((s) => s.view);
  const go = useApp((s) => s.go);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const openSettings = useApp((s) => s.openSettings);
  const known = useApp((s) => s.knownCount);
  const streak = useApp((s) => s.streak);
  const lang = useApp((s) => s.lang)();
  const langs = useApp((s) => s.langs)();
  const setSetting = useApp((s) => s.setSetting);
  const refreshKnown = useApp((s) => s.refreshKnown);
  const chatBusy = useChat((s) => !!s.pending);
  const name = useUserName();
  const li = langInfo(lang);

  const entries: { view: View; label: string; icon: IconName; run(): void }[] = [
    { view: "playlists", label: "Playlists", icon: "playlist", run: () => openPlaylist(null) },
    { view: "chat", label: "Chat", icon: "chat", run: () => go("chat") },
    { view: "vocab", label: t("Vocabulaire", "Vocabulary"), icon: "cards", run: () => go("vocab") },
    { view: "settings", label: t("Réglages", "Settings"), icon: "settings", run: () => openSettings() },
  ];

  const switchLang = async (code: LangCode) => {
    if (code === lang) return;
    onClose();
    go(view === "discover" ? "discover" : "library");
    await setSetting("lang", code);
    await refreshKnown();
  };

  return (
    <AnimatePresence>
      {open && (
        <>
          <motion.div key="scrim" className="m-scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose} />
          <motion.div
            key="sheet"
            className="m-more"
            role="dialog"
            aria-label={t("Plus", "More")}
            initial={{ y: "100%" }}
            animate={{ y: 0 }}
            exit={{ y: "100%" }}
            transition={{ type: "spring", stiffness: 380, damping: 38 }}
            drag="y"
            dragConstraints={{ top: 0, bottom: 0 }}
            dragElastic={{ top: 0, bottom: 0.6 }}
            onDragEnd={(_, i) => (i.offset.y > 90 || i.velocity.y > 500) && onClose()}
          >
            <span className="m-grabber" aria-hidden="true" />
            <button
              className="m-me"
              onClick={() => {
                onClose();
                openSettings("profile");
              }}
            >
              <Avatar size={44} />
              <span className="m-me-text">
                <strong>{name || t("Votre profil", "Your profile")}</strong>
                <span className="muted">
                  <CountUp value={known} /> {t(`mots connus en ${li.name.toLowerCase()}`, `known words in ${li.name}`)}
                  {streak && streak.current > 0 && (
                    <>
                      {" · "}
                      <Icon name="flame" size={12} stroke={2} /> {formatNumber(streak.current)}
                    </>
                  )}
                </span>
              </span>
              <Icon name="right" size={16} />
            </button>

            <div className="m-langs" role="radiogroup" aria-label={t("Langue étudiée", "Language you're learning")}>
              {LANGS.filter((l) => langs.includes(l.code) || l.code === lang).map((l) => (
                <button key={l.code} role="radio" aria-checked={l.code === lang} className={`m-lang ${l.code === lang ? "on" : ""}`} onClick={() => void switchLang(l.code)}>
                  <span className="lang-badge" style={{ background: l.color }}>
                    {l.badge}
                  </span>
                  {l.name}
                </button>
              ))}
              <button
                className="m-lang add"
                onClick={() => {
                  onClose();
                  openSettings("langs");
                }}
              >
                <span className="lang-badge add">
                  <Icon name="plus" size={13} stroke={2.2} />
                </span>
                {t("Ajouter", "Add")}
              </button>
            </div>

            <div className="m-grid">
              {entries.map((e) => (
                <button
                  key={e.view}
                  className={`m-entry ${view === e.view ? "on" : ""}`}
                  onClick={() => {
                    onClose();
                    e.run();
                  }}
                >
                  <span className="m-entry-icon">
                    <Icon name={e.icon} size={20} />
                    {e.view === "chat" && chatBusy && <span className="m-live" />}
                  </span>
                  {e.label}
                </button>
              ))}
            </div>

            <div className="m-cards">
              <ImportQueueCard />
              <LingqCard />
              <BackupCard />
            </div>
            <span className="m-foot muted">{count(langs.length, "langue étudiée", "langues étudiées", "language studied", "languages studied")}</span>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
