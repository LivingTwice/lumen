import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { frenchSpaces } from "../components/Markdown";
import { api } from "../lib/api";
import { useChat, type LessonRef } from "../lib/chat";
import { confirmAsk } from "../lib/dialogs";
import { langInfo } from "../lib/langs";
import { useApp } from "../lib/store";
import type { ChatSummary } from "../lib/types";
import { ChatThread } from "./chat/ChatThread";

const DAY = new Intl.DateTimeFormat("fr-FR", { day: "numeric", month: "short" });
const TIME = new Intl.DateTimeFormat("fr-FR", { hour: "2-digit", minute: "2-digit" });

function when(ts: number): string {
  const d = new Date(ts * 1000);
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return TIME.format(d);
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return "Hier";
  return DAY.format(d);
}

export function Chat() {
  const lang = useApp((s) => s.lang)();
  const lessonId = useApp((s) => s.lessonId);
  const list = useChat((s) => s.list);
  const thread = useChat((s) => s.thread);
  const pending = useChat((s) => s.pending);
  const load = useChat((s) => s.load);
  const open = useChat((s) => s.open);
  const fresh = useChat((s) => s.fresh);
  const remove = useChat((s) => s.remove);
  const rename = useChat((s) => s.rename);
  const [lessonNow, setLessonNow] = useState<LessonRef | null>(null);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    void load(lang);
  }, [lang, load]);

  // la leçon en cours de lecture, que « /leçon » joint d'un coup
  useEffect(() => {
    if (!lessonId) return setLessonNow(null);
    api()
      .lessonsList(lang)
      .then((ls) => {
        const l = ls.find((x) => x.id === lessonId);
        setLessonNow(l ? { id: l.id, title: l.title } : null);
      })
      .catch(() => {});
  }, [lessonId, lang]);

  useEffect(() => setEditing(false), [thread?.chat.id]);

  const del = async (c: ChatSummary) => {
    const name = c.title || "cette conversation";
    if (!(await confirmAsk(`Supprimer « ${name} » ? Ses messages seront effacés.`, "Supprimer la conversation", "Supprimer"))) return;
    await remove(c.id);
  };

  const title = frenchSpaces(thread?.chat.title || "Nouvelle conversation");
  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="chat-view">
        <aside className="chat-side" aria-label="Conversations">
          <div className="chat-side-head">
            <h1>Chat</h1>
            <button className="icon-btn" onClick={() => fresh(null)} aria-label="Nouvelle conversation" title="Nouvelle conversation">
              <Icon name="edit" size={17} />
            </button>
          </div>
          <p className="chat-side-sub">
            Un professeur de {langInfo(lang).name.toLowerCase()} sur votre Mac
          </p>
          <div className="chat-list">
            <button className={`chat-item new ${!thread ? "on" : ""}`} onClick={() => fresh(null)}>
              {!thread && <motion.span layoutId="chat-pill" className="chat-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
              <span className="chat-item-icon">
                <Icon name="plus" size={14} stroke={2} />
              </span>
              <span className="chat-item-text">
                <strong>Nouvelle conversation</strong>
              </span>
            </button>
            <AnimatePresence initial={false}>
              {list.map((c) => {
                const on = thread?.chat.id === c.id;
                return (
                  <motion.div key={c.id} layout="position" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, height: 0 }} className="chat-item-row">
                    <button className={`chat-item ${on ? "on" : ""}`} onClick={() => !on && void open(c.id)}>
                      {on && <motion.span layoutId="chat-pill" className="chat-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
                      <span className="chat-item-icon">{pending?.chatId === c.id ? <span className="ember" /> : <Icon name={c.lesson_id ? "book" : "chat"} size={14} />}</span>
                      <span className="chat-item-text">
                        <strong>{frenchSpaces(c.title || "Nouvelle conversation")}</strong>
                        <small>{c.lesson_title ?? (c.preview || "Aucun message")}</small>
                      </span>
                      <span className="chat-item-when num">{when(c.updated_at)}</span>
                    </button>
                    <button className="chat-item-del" onClick={() => void del(c)} aria-label={`Supprimer « ${c.title || "Nouvelle conversation"} »`} title="Supprimer">
                      <Icon name="trash" size={14} />
                    </button>
                  </motion.div>
                );
              })}
            </AnimatePresence>
          </div>
        </aside>

        <section className="chat-main">
          <header className="chat-head">
            {editing && thread ? (
              <input
                className="chat-title-input"
                defaultValue={thread.chat.title}
                autoFocus
                aria-label="Titre de la conversation"
                onBlur={(e) => {
                  setEditing(false);
                  const v = e.target.value.trim();
                  if (v && v !== thread.chat.title) void rename(thread.chat.id, v);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                  if (e.key === "Escape") setEditing(false);
                }}
              />
            ) : (
              <h2 onDoubleClick={() => thread && setEditing(true)} title={thread ? "Double-cliquez pour renommer" : undefined}>
                {title}
              </h2>
            )}
            {thread && (
              <button className="icon-btn" onClick={() => void del(thread.chat)} aria-label="Supprimer la conversation" title="Supprimer la conversation">
                <Icon name="trash" size={16} />
              </button>
            )}
          </header>
          <ChatThread lessonNow={lessonNow} />
        </section>
      </div>
    </>
  );
}
