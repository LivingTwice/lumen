import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useChat } from "../lib/chat";
import { useDictStatus } from "../lib/dicts";
import { useDiscoverNews } from "../lib/discover";
import { LANGS, langInfo } from "../lib/langs";
import { useApp, type View } from "../lib/store";
import { useOnline } from "../lib/online";
import type { LangCode, LessonSummary } from "../lib/types";
import { Avatar } from "./Avatar";
import { Icon, type IconName } from "./Icon";
import { CountUp, Menu, Orb } from "./ui";
import { BackupCard } from "./BackupCard";
import { ImportQueueCard } from "./ImportQueueCard";
import { LingqCard } from "./LingqCard";
import { UpdateCard } from "./UpdateCard";
import { count, formatNumber, t } from "../lib/i18n";
import { studyTime } from "../lib/progress";
import { useUserName } from "../lib/user";

const nav = (): { view: View; label: string; icon: IconName }[] => [
  { view: "library", label: t("Bibliothèque", "Library"), icon: "library" },
  { view: "discover", label: t("Découvrir", "Discover"), icon: "sparkle" },
  { view: "playlists", label: "Playlists", icon: "playlist" },
  { view: "reader", label: t("Lecture en cours", "Now reading"), icon: "book" },
  { view: "chat", label: "Chat", icon: "chat" },
  { view: "vocab", label: t("Vocabulaire", "Vocabulary"), icon: "cards" },
  { view: "progress", label: t("Progrès", "Progress"), icon: "chart" },
];

export function Sidebar() {
  const view = useApp((s) => s.view);
  const go = useApp((s) => s.go);
  const lessonId = useApp((s) => s.lessonId);
  const openLesson = useApp((s) => s.openLesson);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const openImport = useApp((s) => s.openImport);
  const openSettings = useApp((s) => s.openSettings);
  const settingsTab = useApp((s) => s.settingsTab);
  const known = useApp((s) => s.knownCount);
  const streak = useApp((s) => s.streak);
  const models = useApp((s) => s.models);
  const downloads = useApp((s) => s.downloads);
  const libraryVersion = useApp((s) => s.libraryVersion);
  const settings = useApp((s) => s.settings);
  const lang = useApp((s) => s.lang)();
  const langs = useApp((s) => s.langs)();
  const dict = useDictStatus(lang);
  const setSetting = useApp((s) => s.setSetting);
  const refreshKnown = useApp((s) => s.refreshKnown);
  // une réponse du chat s'écrit pendant qu'on est ailleurs : la lueur le signale
  const chatBusy = useChat((s) => !!s.pending);
  // Découvrir : nouveautés à votre niveau depuis la dernière visite, ou jamais ouvert
  const news = useDiscoverNews(lang);
  const fresh = view === "discover" ? 0 : news.fresh;
  const unseen = !news.seen && view !== "discover";
  // l'étoile ne scintille que s'il y a du nouveau, et jamais pendant la lecture
  const calling = (fresh > 0 || unseen) && view !== "reader";
  const [menu, setMenu] = useState(false);
  const [recent, setRecent] = useState<LessonSummary[]>([]);
  const name = useUserName();

  useEffect(() => {
    api()
      .lessonsList(lang)
      .then((l) => setRecent(l.filter((x) => x.opened_at).slice(0, 4)))
      .catch(() => {});
  }, [lang, libraryVersion, lessonId]);

  const li = langInfo(lang);
  const online = useOnline();
  const llm = models.find((m) => m.kind === "llm" && m.installed && m.id === settings.llm_model) ?? models.find((m) => m.kind === "llm" && m.installed);
  const asr = models.find((m) => m.kind === "asr" && m.installed && m.id === settings.asr_model) ?? models.find((m) => m.kind === "asr" && m.installed);
  const asrText = models.find((m) => m.kind === "asrtext" && m.installed);
  const busy = Object.keys(downloads).length > 0;
  const voice = models.find((m) => m.kind === "tts");

  const switchLang = async (code: LangCode) => {
    setMenu(false);
    await setSetting("lang", code);
    if (!langs.includes(code)) await setSetting("langs", [...langs, code].join(","));
    await refreshKnown();
    go(view === "discover" ? "discover" : "library");
  };

  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-top drag" data-tauri-drag-region />
      <div className="brand drag" data-tauri-drag-region>
        <Orb size={20} />
        <span className="brand-name">Lumen</span>
        {/* le profil de l'apprenant : son avatar, qui mène à Réglages › Profil */}
        <button
          className={`side-me no-drag ${view === "settings" && settingsTab === "profile" ? "on" : ""}`}
          onClick={() => openSettings("profile")}
          title={name ? t(`${name} · votre profil`, `${name} · your profile`) : t("Votre profil", "Your profile")}
          aria-label={name ? t(`${name} · votre profil`, `${name} · your profile`) : t("Votre profil", "Your profile")}
        >
          <Avatar size={30} />
        </button>
      </div>

      <div className="lang-switch">
        <Menu
          open={menu}
          onClose={() => setMenu(false)}
          anchor={
            <button className="lang-button" onClick={() => setMenu((m) => !m)} aria-haspopup="menu" aria-expanded={menu}>
              <span className="lang-badge" style={{ background: li.color }}>
                {li.badge}
              </span>
              <span className="lang-meta">
                <strong>{li.name}</strong>
                <span>
                  <CountUp value={known} /> {t("mots connus", "known words")}
                </span>
              </span>
              <Icon name="chevron" size={16} />
            </button>
          }
        >
          {/* seulement les langues étudiées : les autres s'ajoutent dans les Réglages */}
          {LANGS.filter((l) => langs.includes(l.code) || l.code === lang).map((l) => (
            <button key={l.code} className="menu-item" role="menuitem" onClick={() => switchLang(l.code)}>
              <span className="lang-badge" style={{ background: l.color, width: 24, height: 24, fontSize: 10, borderRadius: 7 }}>
                {l.badge}
              </span>
              <span style={{ flex: 1 }}>
                {l.name} <span className="muted">· {l.native}</span>
              </span>
              {l.code === lang && <Icon name="check" size={16} />}
            </button>
          ))}
          <div className="menu-sep" />
          <button
            className="menu-item"
            role="menuitem"
            onClick={() => {
              setMenu(false);
              openSettings("langs");
            }}
          >
            <span className="lang-badge add" style={{ width: 24, height: 24, borderRadius: 7 }}>
              <Icon name="plus" size={13} stroke={2.2} />
            </span>
            <span style={{ flex: 1 }}>{t("Ajouter une langue…", "Add a language…")}</span>
          </button>
        </Menu>
      </div>

      <nav className="nav">
        {nav().map((n) => {
          const disabled = n.view === "reader" && !lessonId;
          const active = view === n.view;
          return (
            <button
              key={n.view}
              className={`nav-item ${active ? "active" : ""} ${n.view === "discover" ? `nav-discover ${calling ? "calling" : ""}` : ""}`}
              onClick={() => (n.view === "reader" && lessonId ? openLesson(lessonId) : n.view === "playlists" ? openPlaylist(null) : go(n.view))}
              disabled={disabled}
              style={disabled ? { opacity: 0.45, cursor: "default" } : undefined}
              aria-current={active ? "page" : undefined}
              data-tour={`nav-${n.view}`}
            >
              {active && <motion.span layoutId="nav-pill" className="nav-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
              <Icon name={n.icon} />
              <span>{n.label}</span>
              {n.view === "chat" && chatBusy && view !== "chat" && <span className="nav-live" aria-label={t("Réponse en cours", "Answer in progress")} />}
              {n.view === "discover" && fresh > 0 && (
                <span className="nav-discover-count num" aria-label={count(fresh, "nouveauté à votre niveau", "nouveautés à votre niveau", "new find at your level", "new finds at your level")}>
                  {fresh > 99 ? "99+" : fresh}
                </span>
              )}
              {n.view === "discover" && unseen && !fresh && <span className="nav-discover-dot" aria-hidden="true" />}
              {n.view === "progress" && streak && streak.current > 0 && (
                <span
                  className={`nav-streak num ${streak.today_done ? "lit" : ""}`}
                  title={
                    streak.today_done
                      ? t(`Série de ${count(streak.current, "jour", "jours", "", "")} · objectif du jour atteint`, `${formatNumber(streak.current)}-day streak · daily goal reached`)
                      : t(
                          `Série de ${count(streak.current, "jour", "jours", "", "")} · encore ${studyTime(Math.max(60, streak.goal_min * 60 - streak.today_secs))} dans une leçon aujourd'hui pour la garder`,
                          `${formatNumber(streak.current)}-day streak · ${studyTime(Math.max(60, streak.goal_min * 60 - streak.today_secs))} more in a lesson today to keep it`,
                        )
                  }
                >
                  <Icon name="flame" size={12} stroke={2} />
                  {formatNumber(streak.current)}
                </span>
              )}
            </button>
          );
        })}
        <button className={`nav-item ${view === "settings" ? "active" : ""}`} onClick={() => go("settings")} aria-current={view === "settings" ? "page" : undefined}>
          {view === "settings" && <motion.span layoutId="nav-pill" className="nav-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
          <Icon name="settings" />
          <span>{t("Réglages", "Settings")}</span>
        </button>
      </nav>

      <div style={{ padding: "16px 12px 0" }}>
        <button className="btn primary glow" style={{ width: "100%" }} onClick={() => openImport()} data-tour="import">
          <Icon name="plus" size={16} stroke={2} />
          {t("Importer", "Import")}
        </button>
      </div>

      {recent.length > 0 && (
        <>
          <div className="side-section">{t("Récemment ouvert", "Recently opened")}</div>
          <div className="side-recent">
            {recent.map((r) => (
              <button key={r.id} onClick={() => openLesson(r.id)} title={r.title}>
                {r.title}
              </button>
            ))}
          </div>
        </>
      )}

      <div className="side-bottom">
        <UpdateCard />
        <ImportQueueCard />
        <LingqCard />
        <BackupCard />
        <button className="ai-card" style={{ textAlign: "left", cursor: "pointer" }} onClick={() => openSettings("ai")}>
          <span className="eyebrow">{online.on ? t("IA", "AI") : t("IA locale", "Local AI")}</span>
          {online.on && (
            <span className="ai-row">
              <span className={`dot ${online.ready ? "online" : ""}`} />
              {online.ready
                ? t(`${online.provider.name} · en ligne`, `${online.provider.name} · online`)
                : online.provider.id === "custom"
                  ? t("Serveur : adresse à indiquer", "Server: address needed")
                  : t(`${online.provider.name} : clé à ajouter`, `${online.provider.name}: key needed`)}
            </span>
          )}
          {/* le modèle de ce Mac, tant qu'il sert à quelque chose */}
          {!(online.on && online.words && online.chat) && (
            <span className="ai-row">
              <span className={`dot ${llm ? "ok" : busy ? "busy" : ""}`} />
              {llm ? t(`${llm.name} · prêt`, `${llm.name} · ready`) : busy ? t("Téléchargement…", "Downloading…") : t("Traduction : à installer", "Translation: to install")}
            </span>
          )}
          <span className="ai-row">
            <span className={`dot ${asr ? "ok" : ""}`} />
            {asr
              ? asrText
                ? t("Qwen3-ASR et Whisper · prêts", "Qwen3-ASR and Whisper · ready")
                : t(`${asr.name.replace("Large v3 ", "")} · prêt`, `${asr.name.replace("Large v3 ", "")} · ready`)
              : t("Transcription : à installer", "Transcription: to install")}
          </span>
          <span className="ai-row">
            <span className={`dot ${voice?.installed ? "ok" : voice && downloads[voice.id] ? "busy" : ""}`} />
            {voice?.installed
              ? t("Voix naturelle · prête", "Natural voice · ready")
              : voice && downloads[voice.id]
                ? t("Voix : téléchargement…", "Voice: downloading…")
                : t("Voix naturelle : à installer", "Natural voice: to install")}
          </span>
          {dict?.exists !== false && (
            <span className="ai-row">
              <span className={`dot ${!dict || dict.ready ? "ok" : dict.downloading ? "busy" : ""}`} />
              {!dict || dict.ready
                ? t("Dictionnaire hors ligne", "Offline dictionary")
                : dict.downloading
                  ? t("Dictionnaire : téléchargement…", "Dictionary: downloading…")
                  : t("Dictionnaire : à télécharger", "Dictionary: to download")}
            </span>
          )}
        </button>
      </div>
    </aside>
  );
}
