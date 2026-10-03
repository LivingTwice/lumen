import { motion } from "motion/react";
import { useEffect } from "react";
import { Icon } from "../../components/Icon";
import { useChat, type LessonRef } from "../../lib/chat";
import { useApp } from "../../lib/store";
import type { LangCode } from "../../lib/types";
import { ChatThread } from "../chat/ChatThread";

export type AsideTab = "word" | "chat";

/** Onglets du panneau de droite du lecteur : le mot touché, ou le chat sur la leçon. */
export function AsideTabs({ value, onChange }: { value: AsideTab; onChange(v: AsideTab): void }) {
  // une réponse s'écrit pendant qu'on regarde un mot : la lueur le signale
  const busy = useChat((s) => !!s.pending);
  const tabs: { v: AsideTab; label: string }[] = [
    { v: "word", label: "Mot" },
    { v: "chat", label: "Chat" },
  ];
  return (
    <div className="aside-tabs no-drag" role="tablist" aria-label="Panneau">
      {tabs.map((t) => (
        <button key={t.v} role="tab" aria-selected={value === t.v} className={value === t.v ? "on" : ""} onClick={() => onChange(t.v)}>
          {value === t.v && <motion.span layoutId="aside-tab" className="aside-tab-pill" transition={{ type: "spring", stiffness: 500, damping: 38 }} />}
          <Icon name={t.v === "word" ? "text" : "chat"} size={14} />
          {t.label}
          {t.v === "chat" && busy && value !== "chat" && <span className="tab-live" />}
        </button>
      ))}
    </div>
  );
}

/** Le chat dans le lecteur : la conversation de la leçon ouverte. */
export function ReaderChat({ lesson }: { lesson: LessonRef & { lang: LangCode } }) {
  const forLesson = useChat((s) => s.forLesson);
  const thread = useChat((s) => s.thread);
  const fresh = useChat((s) => s.fresh);
  const go = useApp((s) => s.go);

  useEffect(() => {
    void forLesson(lesson);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lesson.id]);

  return (
    <div className="reader-chat">
      <div className="reader-chat-head">
        <span className="reader-chat-title" title={thread?.chat.title || undefined}>
          {thread?.chat.title || "Nouvelle conversation"}
        </span>
        <button className="icon-btn" onClick={() => fresh(lesson)} aria-label="Nouvelle conversation sur cette leçon" title="Nouvelle conversation sur cette leçon">
          <Icon name="edit" size={15} />
        </button>
        <button className="icon-btn" onClick={() => go("chat")} aria-label="Ouvrir dans la vue Chat" title="Ouvrir dans la vue Chat (historique des conversations)">
          <Icon name="expand" size={15} />
        </button>
      </div>
      <ChatThread compact lessonNow={lesson} />
    </div>
  );
}
