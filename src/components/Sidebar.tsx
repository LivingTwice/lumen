import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { LANGS, langInfo } from "../lib/langs";
import { useApp, type View } from "../lib/store";
import type { LangCode, LessonSummary } from "../lib/types";
import { Icon, type IconName } from "./Icon";
import { CountUp, Menu, Orb } from "./ui";
import { LingqCard } from "./LingqCard";
import { UpdateCard } from "./UpdateCard";

const NAV: { view: View; label: string; icon: IconName }[] = [
  { view: "library", label: "Bibliothèque", icon: "library" },
  { view: "reader", label: "Lecture en cours", icon: "book" },
  { view: "vocab", label: "Vocabulaire", icon: "cards" },
  { view: "progress", label: "Progrès", icon: "chart" },
];

export function Sidebar() {
  const view = useApp((s) => s.view);
  const go = useApp((s) => s.go);
  const lessonId = useApp((s) => s.lessonId);
  const openLesson = useApp((s) => s.openLesson);
  const openImport = useApp((s) => s.openImport);
  const known = useApp((s) => s.knownCount);
  const models = useApp((s) => s.models);
  const downloads = useApp((s) => s.downloads);
  const libraryVersion = useApp((s) => s.libraryVersion);
  const settings = useApp((s) => s.settings);
  const lang = useApp((s) => s.lang)();
  const langs = useApp((s) => s.langs)();
  const setSetting = useApp((s) => s.setSetting);
  const refreshKnown = useApp((s) => s.refreshKnown);
  const [menu, setMenu] = useState(false);
  const [recent, setRecent] = useState<LessonSummary[]>([]);

  useEffect(() => {
    api()
      .lessonsList(lang)
      .then((l) => setRecent(l.filter((x) => x.opened_at).slice(0, 4)))
      .catch(() => {});
  }, [lang, libraryVersion, lessonId]);

  const li = langInfo(lang);
  const llm = models.find((m) => m.kind === "llm" && m.installed && m.id === settings.llm_model) ?? models.find((m) => m.kind === "llm" && m.installed);
  const asr = models.find((m) => m.kind === "asr" && m.installed && m.id === settings.asr_model) ?? models.find((m) => m.kind === "asr" && m.installed);
  const busy = Object.keys(downloads).length > 0;

  const switchLang = async (code: LangCode) => {
    setMenu(false);
    await setSetting("lang", code);
    if (!langs.includes(code)) await setSetting("langs", [...langs, code].join(","));
    await refreshKnown();
    go("library");
  };

  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-top drag" data-tauri-drag-region />
      <div className="brand drag" data-tauri-drag-region>
        <Orb size={20} />
        <span className="brand-name">Lumen</span>
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
                  <CountUp value={known} /> mots connus
                </span>
              </span>
              <Icon name="chevron" size={16} />
            </button>
          }
        >
          {LANGS.map((l) => (
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
        </Menu>
      </div>

      <nav className="nav">
        {NAV.map((n) => {
          const disabled = n.view === "reader" && !lessonId;
          const active = view === n.view;
          return (
            <button
              key={n.view}
              className={`nav-item ${active ? "active" : ""}`}
              onClick={() => (n.view === "reader" && lessonId ? openLesson(lessonId) : go(n.view))}
              disabled={disabled}
              style={disabled ? { opacity: 0.45, cursor: "default" } : undefined}
              aria-current={active ? "page" : undefined}
            >
              {active && <motion.span layoutId="nav-pill" className="nav-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
              <Icon name={n.icon} />
              <span>{n.label}</span>
            </button>
          );
        })}
        <button className={`nav-item ${view === "settings" ? "active" : ""}`} onClick={() => go("settings")} aria-current={view === "settings" ? "page" : undefined}>
          {view === "settings" && <motion.span layoutId="nav-pill" className="nav-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
          <Icon name="settings" />
          <span>Réglages</span>
        </button>
      </nav>

      <div style={{ padding: "16px 12px 0" }}>
        <button className="btn primary glow" style={{ width: "100%" }} onClick={() => openImport()}>
          <Icon name="plus" size={16} stroke={2} />
          Importer
        </button>
      </div>

      {recent.length > 0 && (
        <>
          <div className="side-section">Récemment ouvert</div>
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
        <LingqCard />
        <button className="ai-card" style={{ textAlign: "left", cursor: "pointer" }} onClick={() => go("settings")}>
          <span className="eyebrow">IA locale</span>
          <span className="ai-row">
            <span className={`dot ${llm ? "ok" : busy ? "busy" : ""}`} />
            {llm ? `${llm.name} · prêt` : busy ? "Téléchargement…" : "Traduction : à installer"}
          </span>
          <span className="ai-row">
            <span className={`dot ${asr ? "ok" : ""}`} />
            {asr ? `${asr.name.replace("Large v3 ", "")} · prêt` : "Transcription : à installer"}
          </span>
          <span className="ai-row">
            <span className="dot ok" />
            Dictionnaire hors ligne
          </span>
        </button>
      </div>
    </aside>
  );
}
