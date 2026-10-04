import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { CoverArt } from "../components/Cover";
import { Icon, type IconName } from "../components/Icon";
import { Menu, Orb, Segmented, useGlow } from "../components/ui";
import { formatWhen } from "../lib/backup";
import { grainTexture, youtubeId } from "../lib/covers";
import { LEVELS, ago, cleanTitle, fits, isNew, levelName, levelRange, markSeen, newAtLevel, openSource, seenAt, useDiscover, useLevel } from "../lib/discover";
import { count, t } from "../lib/i18n";
import { inLang } from "../lib/langs";
import { formatDuration, useApp } from "../lib/store";
import type { DiscoverItem, LangCode } from "../lib/types";

type Kind = "all" | "video" | "audio" | "text";

/** Éléments au plus par rayon. */
const SHELF_MAX = 18;

/** Graine reproductible d'une chaîne (FNV-1a) : l'œuvre générée ne change pas d'un affichage à l'autre. */
function seedOf(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

function kindIcon(it: DiscoverItem): IconName {
  if (it.kind === "video") return youtubeId(it.url) ? "youtube" : "video";
  return it.kind === "audio" ? "podcast" : "text";
}

function kindLabel(it: DiscoverItem): string {
  if (it.kind === "video") return t("Vidéo", "Video");
  return it.kind === "audio" ? "Podcast" : "Article";
}

/** « Voir sur YouTube », « Écouter sur le site », « Lire sur le site ». */
function sourceLabel(it: DiscoverItem): string {
  if (youtubeId(it.url)) return t("Voir sur YouTube", "Watch on YouTube");
  return it.kind === "text" ? t("Lire sur le site", "Read on the site") : t("Écouter sur le site", "Listen on the site");
}

/** Ce que devient l'élément une fois choisi, dit simplement. */
function whatHappens(it: DiscoverItem): string {
  if (it.lesson_id) return t("Déjà dans votre bibliothèque : reprenez là où vous en étiez.", "Already in your library: pick up where you left off.");
  if (it.kind === "text") return t("L'article devient une leçon de lecture. Trop riche ? Le bouton « Simplifier » le réécrit à votre niveau.", "The article becomes a reading lesson. Too rich? The “Simplify” button rewrites it at your level.");
  if (it.page_text) return t("Le son et le texte publié par la source : la lanterne suit la voix, mot à mot.", "The sound and the text published by the source: the lantern follows the voice, word by word.");
  return it.kind === "video"
    ? t("Lumen télécharge la vidéo, la transcrit sur votre Mac et cale chaque mot sur la voix.", "Lumen downloads the video, transcribes it on your Mac and syncs every word with the voice.")
    : t("Lumen télécharge l'épisode, le transcrit sur votre Mac et cale chaque mot sur la voix.", "Lumen downloads the episode, transcribes it on your Mac and syncs every word with the voice.");
}

/** Toucher un élément : la leçon s'il en est déjà une, sinon l'import. */
function useOpenItem() {
  const openLesson = useApp((s) => s.openLesson);
  const openImportItem = useApp((s) => s.openImportItem);
  return (it: DiscoverItem) => (it.lesson_id ? openLesson(it.lesson_id) : openImportItem(it));
}

/** Pastille de niveau : plus le niveau monte, plus la lumière est dense. */
function LevelChip({ item }: { item: Pick<DiscoverItem, "lo" | "hi"> }) {
  return (
    <span className={`lv-chip lv-${item.hi}`} title={t(`Niveau ${levelRange(item)}`, `Level ${levelRange(item)}`)}>
      {levelRange(item)}
    </span>
  );
}

/** Image de l'élément : la sienne, sinon l'œuvre générée. Un podcast (image carrée) flotte sur son propre reflet flou. */
function FindThumb({ item, big = false }: { item: DiscoverItem; big?: boolean }) {
  const yt = youtubeId(item.url);
  const sources = useMemo(() => [item.image, yt ? `https://i.ytimg.com/vi/${yt}/hqdefault.jpg` : ""].filter(Boolean), [item.image, yt]);
  const [stage, setStage] = useState(0);
  const [shown, setShown] = useState<string | null>(null);
  const src = sources[stage];
  const square = item.kind === "audio";
  return (
    <div className={`find-thumb ${big ? "big" : ""} ${square ? "square" : ""}`}>
      <CoverArt seed={seedOf(item.id)} hue={seedOf(item.source) % 360} />
      <span className="cover-grain" style={{ backgroundImage: `url(${grainTexture()})` }} />
      {src && square && <img className={`find-backdrop ${shown === src ? "on" : ""}`} src={src} alt="" aria-hidden="true" draggable={false} />}
      {src && (
        <img
          key={src}
          className={`find-img ${shown === src ? "on" : ""}`}
          src={src}
          alt=""
          draggable={false}
          onLoad={(e) => {
            // YouTube renvoie une vignette grise de 120 px quand l'image demandée n'existe pas
            if (yt && e.currentTarget.naturalWidth < 200) setStage((s) => s + 1);
            else setShown(src);
          }}
          onError={() => setStage((s) => s + 1)}
        />
      )}
      <span className="cover-kind">
        <Icon name={kindIcon(item)} size={13} />
        {kindLabel(item)}
      </span>
      {item.duration > 0 && <span className="find-duration num">{formatDuration(item.duration)}</span>}
    </div>
  );
}

function FindCard({ item, lang, index, intro, seen }: { item: DiscoverItem; lang: LangCode; index: number; intro: boolean; seen: number }) {
  const glow = useGlow<HTMLDivElement>();
  const [menu, setMenu] = useState(false);
  const open = useOpenItem();
  const hide = useDiscover((s) => s.hide);
  const title = cleanTitle(item.title);
  return (
    // l'enveloppe porte l'apparition (motion), la carte garde son survol en CSS
    <motion.div
      className="find-cell"
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: intro ? Math.min(index, 7) * 0.045 : 0, type: "spring", stiffness: 260, damping: 28 }}
    >
      <div className={`find-card glow-surface ${item.lesson_id ? "done" : ""}`} ref={glow.ref} onMouseMove={glow.onMouseMove}>
        <button className="lesson-hit" onClick={() => open(item)} aria-label={item.lesson_id ? t(`Ouvrir ${title}`, `Open ${title}`) : t(`Faire une leçon de ${title}`, `Make a lesson of ${title}`)} />
        <FindThumb item={item} />
        {isNew(item, seen) && <span className="find-new">{t("Nouveau", "New")}</span>}
        {item.lesson_id && (
          <span className="find-done">
            <Icon name="check" size={12} stroke={2.4} /> {t("Dans la bibliothèque", "In your library")}
          </span>
        )}
        <div className="find-body">
          <div className="find-top">
            <span className="find-source">{item.source_name}</span>
            <LevelChip item={item} />
          </div>
          <h3 className="find-title" dir="auto">
            {title}
          </h3>
          {item.published > 0 && <span className="find-when">{ago(item.published)}</span>}
        </div>
        <div className="lesson-menu">
          <Menu
            open={menu}
            onClose={() => setMenu(false)}
            align="right"
            anchor={
              <button className="icon-btn" onClick={() => setMenu((m) => !m)} aria-label={t("Options", "Options")}>
                <Icon name="more" size={18} stroke={2.6} />
              </button>
            }
          >
            <button className="menu-item" onClick={() => (setMenu(false), void openSource(item.page || item.url))}>
              <Icon name="external" size={16} /> {sourceLabel(item)}
            </button>
            <div className="menu-sep" />
            <button className="menu-item" onClick={() => (setMenu(false), void hide(lang, item))}>
              <Icon name="ban" size={16} /> {t("Ne plus proposer", "Don't suggest again")}
            </button>
          </Menu>
        </div>
      </div>
    </motion.div>
  );
}

/** Un rayon : une rangée qui défile, avec ses flèches et ses bords fondus. */
function Shelf({ title, hint, items, lang, seen, intro }: { title: string; hint: string; items: DiscoverItem[]; lang: LangCode; seen: number; intro: boolean }) {
  const track = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: true, end: true });
  const update = () => {
    const el = track.current;
    if (el) setEdges({ start: el.scrollLeft < 4, end: el.scrollLeft + el.clientWidth >= el.scrollWidth - 4 });
  };
  useEffect(() => {
    update();
    const el = track.current;
    if (!el) return;
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [items.length]);
  const page = (dir: 1 | -1) => {
    const el = track.current;
    el?.scrollBy({ left: dir * el.clientWidth * 0.86, behavior: "smooth" });
  };
  return (
    <section className="shelf">
      <header className="shelf-head">
        <div>
          <h3>{title}</h3>
          <p>{hint}</p>
        </div>
        {!(edges.start && edges.end) && (
          <div className="shelf-nav">
            <button className="icon-btn" disabled={edges.start} onClick={() => page(-1)} aria-label={t("Précédents", "Previous")}>
              <Icon name="left" size={17} />
            </button>
            <button className="icon-btn" disabled={edges.end} onClick={() => page(1)} aria-label={t("Suivants", "Next")}>
              <Icon name="right" size={17} />
            </button>
          </div>
        )}
      </header>
      <div className={`shelf-track ${edges.start ? "at-start" : ""} ${edges.end ? "at-end" : ""}`} ref={track} onScroll={update}>
        {items.map((it, i) => (
          <FindCard key={it.id} item={it} lang={lang} index={i} intro={intro} seen={seen} />
        ))}
      </div>
    </section>
  );
}

/** « À la une pour vous » : la plus belle trouvaille du jour, à votre niveau. */
function FindHero({ item }: { item: DiscoverItem }) {
  const glow = useGlow<HTMLDivElement>();
  const open = useOpenItem();
  const facts = [item.source_name, kindLabel(item), item.duration > 0 ? formatDuration(item.duration) : "", ago(item.published)].filter(Boolean);
  return (
    <motion.div
      className="find-hero glow-surface"
      ref={glow.ref}
      onMouseMove={glow.onMouseMove}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 220, damping: 26 }}
    >
      <button className="find-hero-media" onClick={() => open(item)} aria-label={cleanTitle(item.title)}>
        <FindThumb item={item} big />
      </button>
      <div className="find-hero-body">
        <span className="eyebrow find-eyebrow">
          <Icon name="sparkle" size={13} /> {t("À la une pour vous", "Picked for you")}
          <LevelChip item={item} />
        </span>
        <h2 className="display" dir="auto">
          {cleanTitle(item.title)}
        </h2>
        <p className="find-hero-facts num">{facts.join(" · ")}</p>
        {item.summary && (
          <p className="hero-excerpt" dir="auto">
            {item.summary}
          </p>
        )}
        <p className="find-hero-hint">{whatHappens(item)}</p>
        <div className="hero-foot">
          <button className="btn primary lg glow" onClick={() => open(item)}>
            <Icon name={item.lesson_id ? "book" : "sparkle"} size={17} /> {item.lesson_id ? t("Ouvrir la leçon", "Open the lesson") : t("En faire une leçon", "Make it a lesson")}
          </button>
          <button className="btn ghost" onClick={() => void openSource(item.page || item.url)}>
            <Icon name="external" size={15} /> {sourceLabel(item)}
          </button>
        </div>
      </div>
    </motion.div>
  );
}

/** La lumière cherche : cartes fantômes traversées d'un reflet, pendant la première lecture. */
function Searching({ lang, progress, tools }: { lang: LangCode; progress: number; tools: boolean }) {
  return (
    <div className="disc-searching">
      <div className="disc-searching-head">
        <Orb size={30} />
        <div>
          <strong>{tools ? t("Installation des composants vidéo, une seule fois…", "Installing the video components, just once…") : t(`Lumen parcourt les sources ${inLang(lang)}…`, `Lumen is reading the sources ${inLang(lang)}…`)}</strong>
          <span className="muted">{t("Chaînes, podcasts et journaux : quelques secondes.", "Channels, podcasts and newspapers: a few seconds.")}</span>
        </div>
      </div>
      <div className="bar live disc-searching-bar">
        <i style={{ width: `${Math.max(6, progress)}%` }} />
      </div>
      <div className="shelf-track ghost" aria-hidden="true">
        {[0, 1, 2, 3, 4].map((i) => (
          <div key={i} className="find-ghost" style={{ animationDelay: `${i * 0.14}s` }}>
            <div className="find-ghost-img" />
            <i />
            <i />
          </div>
        ))}
      </div>
    </div>
  );
}

export function Discover({ lang, query }: { lang: LangCode; query: string }) {
  const feed = useDiscover((s) => s.feeds[lang]);
  const busy = useDiscover((s) => s.busy[lang]);
  const refresh = useDiscover((s) => s.refresh);
  const ensure = useDiscover((s) => s.ensure);
  const { level, auto, estimated, known, setLevel } = useLevel(lang);
  const [kind, setKind] = useState<Kind>("all");
  // dernière visite : les « Nouveau » restent visibles pendant toute celle-ci
  const [seen] = useState(() => seenAt(lang));
  const [intro, setIntro] = useState(true);

  useEffect(() => {
    void ensure(lang);
  }, [lang, ensure]);
  // vu : ce qui est à l'écran ne compte plus comme nouveau, ni après une lecture faite sous les yeux, ni en partant
  const refreshedAt = feed?.refreshed_at ?? 0;
  useEffect(() => {
    const timer = window.setTimeout(() => markSeen(lang), 1500);
    return () => {
      window.clearTimeout(timer);
      markSeen(lang);
    };
  }, [lang, refreshedAt]);
  useEffect(() => {
    if (!feed?.items.length || !intro) return;
    const timer = window.setTimeout(() => setIntro(false), 1200);
    return () => window.clearTimeout(timer);
  }, [feed, intro]);

  const working = !!busy || !!feed?.refreshing;
  const autoOn = useApp((s) => s.settings.discover_auto) !== "0";
  const knownWords = count(known, "mot connu", "mots connus", "known word", "known words");
  const q = query.trim().toLowerCase();
  const shown = useMemo(
    () =>
      (feed?.items ?? []).filter(
        (it) => (kind === "all" || it.kind === kind) && (!q || it.title.toLowerCase().includes(q) || it.source_name.toLowerCase().includes(q)),
      ),
    [feed, kind, q],
  );
  const atLevel = useMemo(() => shown.filter((it) => fits(it, level)), [shown, level]);

  // à la une : une nouveauté pour apprenants, en vidéo de préférence, pas encore importée
  const hero = useMemo(() => {
    const fresh = atLevel.filter((it) => !it.lesson_id && Date.now() / 1000 - it.published < 14 * 86400);
    const score = (it: DiscoverItem) => (it.shelf === "learn" ? 2 : 0) + (it.kind === "video" ? 1 : 0) + (it.image ? 1 : 0) + it.published / 1e10;
    return [...(fresh.length ? fresh : atLevel.filter((it) => !it.lesson_id))].sort((a, b) => score(b) - score(a))[0] ?? null;
  }, [atLevel]);

  const shelves = useMemo(() => {
    const rest = atLevel.filter((it) => it.id !== hero?.id);
    const of = (shelf: DiscoverItem["shelf"]) => rest.filter((it) => it.shelf === shelf).slice(0, SHELF_MAX);
    const next = level < 5 ? shown.filter((it) => it.lo === level + 1).slice(0, SHELF_MAX) : [];
    return [
      { id: "learn", title: t("Pour apprendre", "For learners"), hint: t("Vidéos et podcasts pensés pour les apprenants", "Videos and podcasts made for learners"), items: of("learn") },
      {
        id: "news",
        title: t("Actualités", "News"),
        hint: level <= 3 ? t("L'actualité dans une langue plus simple", "The news in simpler language") : t("Ce qui se passe, comme le lisent les natifs", "What's happening, as natives read it"),
        items: of("news"),
      },
      { id: "culture", title: t("Culture et curiosités", "Culture and curiosity"), hint: t("Sciences, histoire, récits", "Science, history, stories"), items: of("culture") },
      {
        id: "next",
        title: t("Un pas plus loin", "One step further"),
        hint: t(`Niveau ${levelName(level + 1)}, pour progresser en douceur`, `Level ${levelName(level + 1)}, to grow gently`),
        items: next,
      },
    ].filter((s) => s.items.length > 0);
  }, [atLevel, shown, hero, level]);

  const sourceNames = useMemo(() => [...new Set((feed?.items ?? []).map((it) => it.source_name))], [feed]);
  const first = !feed || (feed.refreshed_at === 0 && feed.items.length === 0);

  return (
    <div className="discover">
      <div className="disc-band">
        <div className="disc-band-text">
          <span className="eyebrow">
            <Icon name="sparkle" size={13} /> {t("Leçons venues d'ailleurs", "Lessons from out there")}
          </span>
          <p>
            {autoOn
              ? t(
                  `Chaque jour, Lumen parcourt des chaînes, des podcasts et des journaux ${inLang(lang)}, puis range ce qu'il trouve par niveau. Rien n'est téléchargé avant que vous choisissiez.`,
                  `Every day, Lumen looks through channels, podcasts and newspapers ${inLang(lang)}, then sorts what it finds by level. Nothing is downloaded until you choose.`,
                )
              : t(
                  `Quand vous le lui demandez, Lumen parcourt des chaînes, des podcasts et des journaux ${inLang(lang)}, puis range ce qu'il trouve par niveau. Rien n'est téléchargé avant que vous choisissiez.`,
                  `When you ask, Lumen looks through channels, podcasts and newspapers ${inLang(lang)}, then sorts what it finds by level. Nothing is downloaded until you choose.`,
                )}
          </p>
        </div>
        <div className="disc-refresh">
          <span className="muted num">
            {working
              ? busy?.stage === "tools"
                ? t("Installation des composants vidéo…", "Installing the video components…")
                : t("Recherche de nouveautés…", "Looking for new finds…")
              : feed?.refreshed_at
                ? t(`Mis à jour ${formatWhen(feed.refreshed_at)}`, `Updated ${formatWhen(feed.refreshed_at)}`)
                : ""}
          </span>
          <button className="btn soft sm" disabled={working} onClick={() => void refresh(lang)}>
            <span className={`disc-refresh-icon ${working ? "turning" : ""}`}>
              <Icon name="refresh" size={14} />
            </span>
            {t("Actualiser", "Refresh")}
          </button>
        </div>
        {working && !first && (
          <div className="bar live disc-band-bar" aria-hidden="true">
            <i style={{ width: `${Math.max(8, busy?.progress ?? 35)}%` }} />
          </div>
        )}
      </div>

      <div className="disc-controls">
        <div className="disc-level">
          <span className="label">{t("Votre niveau", "Your level")}</span>
          <Segmented
            id="disc-level"
            label={t("Votre niveau", "Your level")}
            value={String(level)}
            onChange={(v) => setLevel(Number(v) === estimated ? null : Number(v))}
            options={LEVELS.map((name, i) => ({ value: String(i + 1), label: name }))}
          />
          <span className="disc-level-hint">
            {auto ? (
              t(`estimé d'après vos ${knownWords}`, `estimated from your ${knownWords}`)
            ) : (
              <button className="disc-link" onClick={() => setLevel(null)}>
                {t(`revenir au niveau estimé (${levelName(estimated)})`, `back to the estimated level (${levelName(estimated)})`)}
              </button>
            )}
          </span>
        </div>
        <Segmented
          id="disc-kind"
          label={t("Type de contenu", "Kind of content")}
          value={kind}
          onChange={(v) => setKind(v as Kind)}
          options={[
            { value: "all", label: t("Tout", "All") },
            { value: "video", label: t("Vidéos", "Videos") },
            { value: "audio", label: "Audio" },
            { value: "text", label: t("Textes", "Texts") },
          ]}
        />
      </div>

      {first ? (
        working ? (
          <Searching lang={lang} progress={busy?.progress ?? 0} tools={busy?.stage === "tools"} />
        ) : (
          <div className="empty disc-empty">
            <Orb size={44} />
            {autoOn ? (
              <>
                <h3>{t("Les sources n'ont pas répondu", "The sources didn't answer")}</h3>
                <p>{t("Lumen a besoin d'Internet pour trouver de nouvelles leçons. Vos leçons, elles, restent toutes là.", "Lumen needs the Internet to find new lessons. Your own lessons all stay right here.")}</p>
              </>
            ) : (
              <>
                <h3>{t(`Des leçons ${inLang(lang)}, venues d'ailleurs`, `Lessons ${inLang(lang)}, from out there`)}</h3>
                <p>{t("La recherche quotidienne est coupée dans les Réglages : Lumen ne cherche que lorsque vous le lui demandez.", "The daily search is turned off in Settings: Lumen only looks when you ask it to.")}</p>
              </>
            )}
            <button className="btn primary lg glow" onClick={() => void refresh(lang)}>
              <Icon name="refresh" size={16} /> {autoOn ? t("Réessayer", "Try again") : t("Chercher maintenant", "Look now")}
            </button>
          </div>
        )
      ) : (
        <AnimatePresence mode="wait">
          <motion.div
            key={`${level}-${kind}-${q}`}
            className="disc-results"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, transition: { duration: 0.12 } }}
            transition={{ duration: 0.28, ease: [0.2, 0.8, 0.2, 1] }}
          >
            {hero && <FindHero item={hero} />}
            {shelves.map((s) => (
              <Shelf key={s.id} title={s.title} hint={s.hint} items={s.items} lang={lang} seen={seen} intro={intro} />
            ))}
            {!hero && shelves.length === 0 && (
              <div className="empty disc-empty">
                <Orb size={40} />
                <h3>{q ? t("Rien ne correspond", "Nothing matches") : t(`Rien au niveau ${levelName(level)} pour l'instant`, `Nothing at level ${levelName(level)} for now`)}</h3>
                <p>
                  {q
                    ? t("Essayez un autre mot, ou un autre type de contenu.", "Try another word, or another kind of content.")
                    : t(
                        `Les sources ${inLang(lang)} proposent surtout d'autres niveaux aujourd'hui. Essayez le niveau voisin : un peu plus facile ou un peu plus riche, c'est encore de la lecture utile.`,
                        `The sources ${inLang(lang)} mostly offer other levels today. Try the next level: a little easier or a little richer is still useful reading.`,
                      )}
                </p>
                {!q && (
                  <div style={{ display: "flex", gap: 10 }}>
                    {level > 1 && (
                      <button className="btn outline" onClick={() => setLevel(level - 1)}>
                        {levelName(level - 1)}
                      </button>
                    )}
                    {level < 5 && (
                      <button className="btn outline" onClick={() => setLevel(level + 1)}>
                        {levelName(level + 1)}
                      </button>
                    )}
                  </div>
                )}
              </div>
            )}
          </motion.div>
        </AnimatePresence>
      )}

      {sourceNames.length > 0 && (
        <p className="disc-credits">
          {t(`Sources ${inLang(lang)} : `, `Sources ${inLang(lang)}: `)}
          {sourceNames.join(" · ")}
        </p>
      )}
    </div>
  );
}

/** L'onglet « Découvrir » du filtre de la bibliothèque : une lumière qui passe, et le nombre de nouveautés. */
export function DiscoverTab({ lang }: { lang: LangCode }) {
  const feed = useDiscover((s) => s.feeds[lang]);
  const load = useDiscover((s) => s.load);
  const seen = Number(useApp((s) => s.settings[`discover_seen_${lang}`])) || 0;
  const { level } = useLevel(lang);
  useEffect(() => {
    void load(lang);
  }, [lang, load]);
  const fresh = newAtLevel(feed, seen, level);
  return (
    <span className="seg-discover">
      <span className="seg-discover-star" aria-hidden="true">
        <Icon name="sparkle" size={13} stroke={1.9} />
      </span>
      <span className="seg-discover-text">{t("Découvrir", "Discover")}</span>
      {fresh > 0 && (
        <span className="seg-discover-count num" aria-label={count(fresh, "nouveauté", "nouveautés", "new find", "new finds")}>
          {fresh > 99 ? "99+" : fresh}
        </span>
      )}
      {!seen && !fresh && <span className="seg-discover-dot" aria-hidden="true" />}
    </span>
  );
}
