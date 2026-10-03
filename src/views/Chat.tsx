import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { frenchSpaces } from "../components/Markdown";
import { api } from "../lib/api";
import { useChat, type LessonRef } from "../lib/chat";
import { confirmAsk } from "../lib/dialogs";
import { locale, t } from "../lib/i18n";
import { langInfo } from "../lib/langs";
import { useApp } from "../lib/store";
import type { ChatSummary } from "../lib/types";
import { ChatThread } from "./chat/ChatThread";

function when(ts: number): string {
  const d = new Date(ts * 1000);
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return new Intl.DateTimeFormat(locale(), { hour: "2-digit", minute: "2-digit" }).format(d);
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return t("Hier", "Yesterday");
  return new Intl.DateTimeFormat(locale(), { day: "numeric", month: "short" }).format(d);
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
    const name = c.title || t("cette conversation", "this conversation");
    const ok = await confirmAsk(
      t(`Supprimer « ${name} » ? Ses messages seront effacés.`, `Delete “${name}”? Its messages will be erased.`),
      t("Supprimer la conversation", "Delete the conversation"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return;
    await remove(c.id);
  };

  const title = frenchSpaces(thread?.chat.title || t("Nouvelle conversation", "New conversation"));
  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="chat-view">
        <aside className="chat-side" aria-label={t("Conversations", "Conversations")}>
          <div className="chat-side-head">
            <h1>Chat</h1>
            <button className="icon-btn" onClick={() => fresh(null)} aria-label={t("Nouvelle conversation", "New conversation")} title={t("Nouvelle conversation", "New conversation")}>
              <Icon name="edit" size={17} />
            </button>
          </div>
          <p className="chat-side-sub">
            {t(`Un professeur de ${langInfo(lang).name.toLowerCase()} sur votre Mac`, `Your ${langInfo(lang).name} teacher, on your Mac`)}
          </p>
          <div className="chat-list">
            <button className={`chat-item new ${!thread ? "on" : ""}`} onClick={() => fresh(null)}>
              {!thread && <motion.span layoutId="chat-pill" className="chat-pill" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
              <span className="chat-item-icon">
                <Icon name="plus" size={14} stroke={2} />
              </span>
              <span className="chat-item-text">
                <strong>{t("Nouvelle conversation", "New conversation")}</strong>
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
                        <strong>{frenchSpaces(c.title || t("Nouvelle conversation", "New conversation"))}</strong>
                        <small>{c.lesson_title ?? (c.preview || t("Aucun message", "No messages"))}</small>
                      </span>
                      <span className="chat-item-when num">{when(c.updated_at)}</span>
                    </button>
                    <button className="chat-item-del" onClick={() => void del(c)} aria-label={t(`Supprimer « ${c.title || "Nouvelle conversation"} »`, `Delete “${c.title || "New conversation"}”`)}
                      title={t("Supprimer", "Delete")}
                    >
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
                aria-label={t("Titre de la conversation", "Conversation title")}
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
              <h2 onDoubleClick={() => thread && setEditing(true)} title={thread ? t("Double-cliquez pour renommer", "Double-click to rename") : undefined}>
                {title}
              </h2>
            )}
            {thread && (
              <button className="icon-btn" onClick={() => void del(thread.chat)} aria-label={t("Supprimer la conversation", "Delete the conversation")} title={t("Supprimer la conversation", "Delete the conversation")}>
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
