import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../lib/api";
import { lessonsOf, plural } from "../lib/playlists";
import { useApp } from "../lib/store";
import type { LessonSummary, Playlist } from "../lib/types";
import { PlaylistCover } from "./Cover";
import { Icon } from "./Icon";
import { Sheet } from "./ui";
import { t } from "../lib/i18n";

/** « Ajouter à une playlist » : une leçon entre dans une ou plusieurs playlists, ou en crée une. */
export function AddToPlaylist({ lesson, all, onClose }: { lesson: LessonSummary | null; all: LessonSummary[]; onClose(): void }) {
  const toast = useApp((s) => s.toast);
  const [lists, setLists] = useState<Playlist[] | null>(null);
  const [name, setName] = useState("");
  // la feuille garde son contenu pendant qu'elle se referme
  const [shown, setShown] = useState<LessonSummary | null>(lesson);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!lesson) return;
    setShown(lesson);
    setName("");
    setLists(null);
    api()
      .playlistsList(lesson.lang)
      .then((ls) => {
        setLists(ls);
        // sans playlist, on commence par en nommer une
        if (!ls.length) window.setTimeout(() => input.current?.focus(), 120);
      })
      .catch((e) => toast(errorText(e), "error"));
  }, [lesson, toast]);

  if (!shown) return null;

  const toggle = async (p: Playlist) => {
    const has = p.lessons.includes(shown.id);
    const lessons = has ? p.lessons.filter((x) => x !== shown.id) : [...p.lessons, shown.id];
    setLists((ls) => ls?.map((x) => (x.id === p.id ? { ...x, lessons } : x)) ?? ls);
    try {
      await api().playlistUpdate(p.id, { lessons });
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const create = async () => {
    try {
      const n = name.trim();
      const id = await api().playlistCreate(shown.lang, n, [shown.id]);
      setName("");
      const ls = await api().playlistsList(shown.lang);
      setLists(ls);
      const made = ls.find((p) => p.id === id)?.name ?? n;
      toast(t(`Playlist « ${made} » créée`, `Playlist “${made}” created`), "light");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  // la leçon vient d'être ajoutée : elle apparaît dans la mosaïque
  const withShown = all.some((l) => l.id === shown.id) ? all : [...all, shown];

  return (
    <Sheet open={!!lesson} onClose={onClose} title={t("Ajouter à une playlist", "Add to a playlist")} width={520} footer={<button className="btn primary" onClick={onClose}>{t("Terminé", "Done")}</button>}>
      <p className="muted atp-lesson">{t(`« ${shown.title} »`, `“${shown.title}”`)}</p>
      {lists && lists.length > 0 && (
        <div className="pick-list atp-list">
          {lists.map((p) => {
            const on = p.lessons.includes(shown.id);
            return (
              <button key={p.id} className={`pick-row ${on ? "on" : ""}`} onClick={() => toggle(p)} aria-pressed={on}>
                <span className="pick-thumb">
                  <PlaylistCover lessons={lessonsOf(p, withShown)} seed={p.id} />
                </span>
                <span className="pick-body">
                  <strong>{p.name}</strong>
                  <span className="num">{plural(p.lessons.length, "leçon", "lesson")}</span>
                </span>
                <span className="pick-check" aria-hidden="true">
                  <Icon name={on ? "check" : "plus"} size={15} stroke={2.2} />
                </span>
              </button>
            );
          })}
        </div>
      )}
      <div className="atp-new">
        <input
          ref={input}
          className="input"
          placeholder={lists && !lists.length ? t("Nom de votre première playlist", "Name of your first playlist") : t("Nouvelle playlist", "New playlist")}
          value={name}
          maxLength={120}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && name.trim() && create()}
          aria-label={t("Nom de la nouvelle playlist", "Name of the new playlist")}
        />
        <button className="btn soft" onClick={create} disabled={!name.trim()}>
          <Icon name="plus" size={15} /> {t("Créer", "Create")}
        </button>
      </div>
    </Sheet>
  );
}
