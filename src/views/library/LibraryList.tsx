// Bibliothèque en liste, comme une fenêtre du Finder : colonnes triables d'un
// clic sur leur titre, livres qu'on déplie pour voir leurs chapitres.
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { BookCover, Cover, KIND } from "../../components/Cover";
import { Icon } from "../../components/Icon";
import { Orb } from "../../components/ui";
import { youtubeId } from "../../lib/covers";
import { count, t } from "../../lib/i18n";
import { progressOf, readBook, type BookView } from "../../lib/playlists";
import { useApp } from "../../lib/store";
import type { LessonSummary } from "../../lib/types";
import {
  EntryMenu,
  completedOf,
  dayText,
  lengthText,
  newOf,
  progressOfEntry,
  recentOf,
  useEntryMenu,
  type Entry,
  type EntryActions,
  type Sort,
  type SortKey,
} from "./entries";

const SLIDE = { type: "spring", stiffness: 420, damping: 40, mass: 0.8 } as const;

const columns = (): { key: SortKey; label: string; cls: string }[] => [
  { key: "title", label: t("Nom", "Name"), cls: "c-name" },
  { key: "kind", label: t("Genre", "Kind"), cls: "c-kind" },
  { key: "length", label: t("Longueur", "Length"), cls: "c-len" },
  { key: "new", label: t("Nouveaux", "New"), cls: "c-new" },
  { key: "progress", label: t("Avancement", "Progress"), cls: "c-prog" },
  { key: "date", label: t("Dernière lecture", "Last read"), cls: "c-date" },
];

export function LibraryList({
  entries,
  sort,
  onSort,
  query,
  selecting,
  picked,
  onPick,
  actions,
  rise,
}: {
  entries: Entry[];
  sort: Sort;
  onSort(key: SortKey): void;
  query: string;
  selecting: boolean;
  picked: Set<string>;
  onPick(key: string, range: boolean): void;
  actions(e: Entry): EntryActions;
  /** rang d'une ligne qui se lève avec les autres (première apparition, « Voir plus ») */
  rise(i: number): number | undefined;
}) {
  // livres dépliés ; une recherche qui trouve un chapitre déplie son livre
  const [unfold, setUnfold] = useState<Record<number, boolean>>({});
  const q = query.trim().toLowerCase();
  const found = (b: BookView) => !!q && !b.p.name.toLowerCase().includes(q) && b.ls.some((l) => l.title.toLowerCase().includes(q));
  const isOpen = (b: BookView) => !selecting && (unfold[b.p.id] ?? found(b));
  // rayures du Finder : une ligne sur deux, chapitres dépliés compris
  let stripe = 0;

  return (
    <div className={`lib-list ${selecting ? "selecting" : ""}`} role="table" aria-label={t("Bibliothèque", "Library")}>
      <div className="lib-head" role="row">
        <span className="c-lead" />
        {columns().map((c) => {
          const on = sort.key === c.key;
          return (
            <span key={c.key} className={c.cls} role="columnheader" aria-sort={on ? (sort.desc ? "descending" : "ascending") : "none"}>
              <button className={`lib-sort ${on ? "on" : ""}`} onClick={() => onSort(c.key)} title={t(`Trier par ${c.label.toLowerCase()}`, `Sort by ${c.label.toLowerCase()}`)}>
                {c.label}
                <Icon name="chevron" size={13} stroke={2.2} className={`icon lib-arrow ${on && !sort.desc ? "up" : ""}`} />
              </button>
            </span>
          );
        })}
        <span className="c-menu" />
      </div>
      <AnimatePresence mode="popLayout" initial={false}>
        {entries.map((e, i) => {
          const odd = stripe++ % 2 === 1;
          const open = !!e.b && isOpen(e.b);
          const chapterStripes = open ? e.b!.ls.map(() => stripe++ % 2 === 1) : [];
          return (
            <motion.div
              key={e.key}
              layout="position"
              className="lib-group"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, transition: { duration: 0.14 } }}
              transition={{ default: rise(i) !== undefined ? { delay: Math.min(rise(i)!, 16) * 0.022, type: "spring", stiffness: 300, damping: 30 } : { duration: 0.2 }, layout: SLIDE }}
            >
              <Row
                e={e}
                odd={odd}
                open={open}
                onToggle={e.b ? () => setUnfold((u) => ({ ...u, [e.b!.p.id]: !open })) : undefined}
                selecting={selecting}
                picked={picked.has(e.key)}
                onPick={(ev) => onPick(e.key, ev.shiftKey)}
                actions={actions(e)}
              />
              <AnimatePresence initial={false}>
                {open && (
                  <motion.div
                    className="lib-chapters"
                    role="rowgroup"
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: "auto", opacity: 1 }}
                    exit={{ height: 0, opacity: 0 }}
                    transition={{ duration: 0.26, ease: [0.2, 0.8, 0.2, 1] }}
                  >
                    {e.b!.ls.map((l, j) => (
                      <Chapter key={l.id} b={e.b!} l={l} index={j} odd={chapterStripes[j]} hit={!!q && l.title.toLowerCase().includes(q)} />
                    ))}
                  </motion.div>
                )}
              </AnimatePresence>
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
}

function Row({
  e,
  odd,
  open,
  onToggle,
  selecting,
  picked,
  onPick,
  actions,
}: {
  e: Entry;
  odd: boolean;
  open: boolean;
  onToggle?(): void;
  selecting: boolean;
  picked: boolean;
  onPick(ev: React.MouseEvent): void;
  actions: EntryActions;
}) {
  const openLesson = useApp((s) => s.openLesson);
  const { menu, setMenu, onContextMenu } = useEntryMenu();
  const title = e.b ? e.b.p.name : e.l.title;
  const progress = progressOfEntry(e);
  const completed = completedOf(e);
  const opened = e.b ? e.b.started : !!e.l.opened_at;
  const k = e.l ? (KIND[e.l.kind] ?? KIND.text) : null;
  return (
    <div className={`lib-row ${odd ? "odd" : ""} ${picked ? "picked" : ""} ${menu ? "menu-open" : ""}`} role="row" onContextMenu={selecting ? undefined : onContextMenu}>
      {selecting ? (
        <button className="lesson-hit" onClick={onPick} aria-pressed={picked} aria-label={t(`Sélectionner ${title}`, `Select ${title}`)} />
      ) : (
        <button
          className="lesson-hit"
          onClick={() => (e.b ? readBook(e.b) : openLesson(e.l.id))}
          aria-label={e.b ? t(`Lire ${title}`, `Read ${title}`) : t(`Ouvrir ${title}`, `Open ${title}`)}
        />
      )}
      <span className="c-lead" role="cell">
        {selecting ? (
          <span className={`lib-check ${picked ? "on" : ""}`} aria-hidden="true">
            <Icon name="check" size={11} stroke={3} />
          </span>
        ) : (
          onToggle && (
            <button
              className={`lib-disclose ${open ? "open" : ""}`}
              onClick={onToggle}
              aria-expanded={open}
              aria-label={open ? t("Replier les chapitres", "Hide the chapters") : t("Déplier les chapitres", "Show the chapters")}
            >
              <Icon name="right" size={14} stroke={2.4} />
            </button>
          )
        )}
      </span>
      <span className="c-name" role="cell">
        <span className="lib-thumb">{e.b ? <BookCover lesson={e.b.ls[0]} title={e.b.p.name} bare /> : <Cover lesson={e.l} bare />}</span>
        <span className="lib-name">
          <strong>{title}</strong>
          {e.b ? (
            <span>
              {count(e.b.ls.length, "chapitre", "chapitres", "chapter", "chapters")}
              {e.b.started && !e.b.completed ? t(` · reprise à « ${e.b.at.title} »`, ` · resumes at “${e.b.at.title}”`) : ""}
            </span>
          ) : (
            e.l.collection && <span>{e.l.collection}</span>
          )}
          {/* écran étroit (iPhone) : la longueur et l'avancement, sous le titre */}
          <span className="lib-mini num">
            {lengthText(e)}
            {completedOf(e) ? t(" · Lu", " · Read") : progress > 0.005 ? t(` · ${Math.round(progress * 100)} %`, ` · ${Math.round(progress * 100)}%`) : ""}
          </span>
        </span>
      </span>
      <span className="c-kind" role="cell">
        <Icon name={e.b ? "book" : e.l && youtubeId(e.l.source) ? "youtube" : k!.icon} size={14} />
        {e.b ? t("Livre", "Book") : k!.label}
      </span>
      <span className="c-len num" role="cell">
        {lengthText(e)}
      </span>
      <span className="c-new" role="cell">
        <span
          className="new-pct num"
          title={
            e.b
              ? t("Part des mots nouveaux, en moyenne sur les chapitres", "Share of new words, averaged over the chapters")
              : t(`${e.l.new_pct} % des mots différents de la leçon sont nouveaux pour vous`, `${e.l.new_pct}% of the lesson's distinct words are new to you`)
          }
        >
          {t(`${newOf(e)} %`, `${newOf(e)}%`)}
        </span>
      </span>
      <span className="c-prog" role="cell">
        <Progress value={progress} completed={completed} />
      </span>
      <span className={`c-date num ${opened ? "" : "faint"}`} role="cell" title={opened ? undefined : t("Ajoutée, pas encore ouverte", "Added, not opened yet")}>
        {dayText(recentOf(e))}
      </span>
      <span className="c-menu lesson-menu" role="cell">
        {!selecting && <EntryMenu e={e} actions={actions} open={menu} setOpen={setMenu} />}
      </span>
    </div>
  );
}

/** Un chapitre d'un livre déplié : son numéro, son titre, son avancement. */
function Chapter({ b, l, index, odd, hit }: { b: BookView; l: LessonSummary; index: number; odd: boolean; hit: boolean }) {
  const here = l.id === b.at.id && b.started && !b.completed;
  return (
    <div className={`lib-row chapter ${odd ? "odd" : ""} ${hit ? "hit" : ""}`} role="row">
      <button className="lesson-hit" onClick={() => readBook(b, l.id)} aria-label={t(`Lire ${l.title}`, `Read ${l.title}`)} />
      <span className="c-lead" role="cell" />
      <span className="c-name" role="cell">
        <span className="lib-index num">{here ? <Orb size={9} /> : index + 1}</span>
        <span className="lib-name">
          <strong>{l.title}</strong>
          {here && <span>{t("Reprise ici", "Resumes here")}</span>}
        </span>
      </span>
      <span className="c-kind" role="cell" />
      <span className="c-len num" role="cell">
        {lengthText({ key: "", l })}
      </span>
      <span className="c-new" role="cell">
        <span className="new-pct num">{t(`${l.new_pct} %`, `${l.new_pct}%`)}</span>
      </span>
      <span className="c-prog" role="cell">
        <Progress value={progressOf(l)} completed={l.completed} />
      </span>
      <span className={`c-date num ${l.opened_at ? "" : "faint"}`} role="cell">
        {l.opened_at ? dayText(l.opened_at) : ""}
      </span>
      <span className="c-menu" role="cell" />
    </div>
  );
}

function Progress({ value, completed }: { value: number; completed: boolean }) {
  if (completed)
    return (
      <span className="done-pill">
        <Icon name="check" size={12} stroke={2.4} /> {t("Lu", "Read")}
      </span>
    );
  if (value <= 0.005) return null;
  const pct = Math.round(value * 100);
  return (
    <span className="lib-progress">
      <span className="pl-row-progress">
        <i style={{ width: `${value * 100}%` }} />
      </span>
      <span className="num">{t(`${pct} %`, `${pct}%`)}</span>
    </span>
  );
}
