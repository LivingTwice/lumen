import { motion } from "motion/react";
import { useEffect } from "react";
import { Icon } from "../../components/Icon";
import { useChat, type LessonRef } from "../../lib/chat";
import { t } from "../../lib/i18n";
import { useApp } from "../../lib/store";
import type { LangCode } from "../../lib/types";
import { ChatThread } from "../chat/ChatThread";

export type AsideTab = "word" | "chat";

/** Onglets du panneau de droite du lecteur : le mot touché, ou le chat sur la leçon. */
export function AsideTabs({ value, onChange }: { value: AsideTab; onChange(v: AsideTab): void }) {
  // une réponse s'écrit pendant qu'on regarde un mot : la lueur le signale
  const busy = useChat((s) => !!s.pending);
  const tabs: { v: AsideTab; label: string }[] = [
    { v: "word", label: t("Mot", "Word") },
    { v: "chat", label: "Chat" },
  ];
  return (
    <div className="aside-tabs no-drag" role="tablist" aria-label={t("Panneau", "Panel")}>
      {tabs.map((tab) => (
        <button
          key={tab.v}
          role="tab"
          aria-selected={value === tab.v}
          className={value === tab.v ? "on" : ""}
          onClick={() => onChange(tab.v)}
          data-tour={tab.v === "chat" ? "chat-tab" : undefined}
        >
          {value === tab.v && <motion.span layoutId="aside-tab" className="aside-tab-pill" transition={{ type: "spring", stiffness: 500, damping: 38 }} />}
          <Icon name={tab.v === "word" ? "text" : "chat"} size={14} />
          {tab.label}
          {tab.v === "chat" && busy && value !== "chat" && <span className="tab-live" />}
        </button>
      ))}
    </div>
  );
}

/** Le chat dans le lecteur : la conversation de la leçon ouverte (`onClose` : panneau flottant, à refermer). */
export function ReaderChat({ lesson, onClose }: { lesson: LessonRef & { lang: LangCode }; onClose?(): void }) {
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
          {thread?.chat.title || t("Nouvelle conversation", "New conversation")}
        </span>
        <button className="icon-btn" onClick={() => fresh(lesson)} aria-label={t("Nouvelle conversation sur cette leçon", "New conversation about this lesson")}
          title={t("Nouvelle conversation sur cette leçon", "New conversation about this lesson")}
        >
          <Icon name="edit" size={15} />
        </button>
        <button className="icon-btn" onClick={() => go("chat")} aria-label={t("Ouvrir dans la vue Chat", "Open in the Chat view")}
          title={t("Ouvrir dans la vue Chat (historique des conversations)", "Open in the Chat view (conversation history)")}
        >
          <Icon name="expand" size={15} />
        </button>
        {onClose && (
          <button className="icon-btn" onClick={onClose} aria-label={t("Fermer le chat", "Close the chat")} title={t("Fermer le chat (Échap)", "Close the chat (Esc)")}>
            <Icon name="close" size={14} />
          </button>
        )}
      </div>
      <ChatThread compact lessonNow={lesson} />
    </div>
  );
}
