import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { CoverArt } from "../../components/Cover";
import { Icon, type IconName } from "../../components/Icon";
import { prefetchStream } from "../../components/Preview";
import { Menu, Orb, useGlow } from "../../components/ui";
import { grainTexture, youtubeId } from "../../lib/covers";
import { ago, cleanTitle, levelRange } from "../../lib/discover";
import { count, formatNumber, t } from "../../lib/i18n";
import { extractArticle } from "../../lib/importers";
import { stageText, useJob } from "../../lib/imports";
import { useLessonOf } from "../../lib/known";
import { LANGS, inLang } from "../../lib/langs";
import { fromArticle, fromHit, fromLink, usePreview } from "../../lib/preview";
import {
  NO_FILTERS,
  PLATFORMS,
  URL_LIKE,
  activeFilters,
  forgetSearches,
  platformName,
  recentSearches,
  searchIdeas,
  tabSupports,
  useFilters,
  useSearch,
  windowText,
  type Ability,
  type Length,
  type SearchTab,
  type SortBy,
  type When,
} from "../../lib/search";
import { formatDuration, useApp } from "../../lib/store";
import type { DiscoverItem, LangCode, LinkInfo, SearchHit, SearchPlatform } from "../../lib/types";
import type { SearchFilters } from "../../lib/search";

/** Graine reproductible d'une chaîne (FNV-1a) : l'œuvre générée ne change pas d'un affichage à l'autre. */
export function seedOf(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

function tabIcon(p: SearchTab): IconName {
  switch (p) {
    case "all":
      return "sparkle";
    case "youtube":
      return "youtube";
    case "music":
      return "music";
    case "podcast":
      return "podcast";
    case "wiki":
      return "text";
    case "dailymotion":
      return "video";
  }
}

/** Texte d'aide du champ : ce que la catégorie choisie permet de chercher. */
function placeholderOf(tab: SearchTab, lang: LangCode): string {
  switch (tab) {
    case "all":
      return t(`Une vidéo, une chanson, un podcast, un article ${inLang(lang)}… ou un lien`, `A video, a song, a podcast, an article ${inLang(lang)}… or a link`);
    case "youtube":
      return t(`Une vidéo YouTube ${inLang(lang)}… ou un lien`, `A YouTube video ${inLang(lang)}… or a link`);
    case "music":
      return t(`Une chanson ou un artiste ${inLang(lang)}`, `A song or an artist ${inLang(lang)}`);
    case "podcast":
      return t(`Un podcast ou une émission ${inLang(lang)}`, `A podcast or a show ${inLang(lang)}`);
    case "wiki":
      return t(`Un article ${inLang(lang)} : un sujet, une personne, un lieu…`, `An article ${inLang(lang)}: a topic, a person, a place…`);
    case "dailymotion":
      return t(`Une vidéo Dailymotion ${inLang(lang)}… ou un lien`, `A Dailymotion video ${inLang(lang)}… or a link`);
  }
}

/** Petite pastille : la leçon se prépare, attend, est prête, ou a échoué. */
export function JobBadge({ jobKey }: { jobKey: string }) {
  const job = useJob(jobKey);
  const openLesson = useApp((s) => s.openLesson);
  if (!job) return null;
  if (job.status === "done" && job.lessonId)
    return (
      <button
        className="job-badge done"
        onClick={(e) => {
          e.stopPropagation();
          openLesson(job.lessonId!);
        }}
      >
        <Icon name="check" size={12} stroke={2.4} /> {t("Leçon prête", "Lesson ready")}
      </button>
    );
  if (job.status === "error")
    return (
      <span className="job-badge error" title={job.error}>
        <Icon name="ban" size={12} /> {t("Échec", "Failed")}
      </span>
    );
  const pct = job.status === "running" && job.progress !== null ? Math.round(job.progress) : null;
  return (
    <span className="job-badge busy" title={job.status === "running" ? stageText(job.stage) : undefined}>
      <span className="job-ring" style={{ ["--p" as string]: `${pct ?? 30}%` }} data-spin={pct === null ? "" : undefined} />
      {job.status === "running" ? (pct !== null ? t(`${pct} %`, `${pct}%`) : t("En préparation", "Preparing")) : t("En attente", "Waiting")}
    </span>
  );
}

/** Image d'un résultat : la sienne, sinon l'œuvre générée ; carrée pour un son. */
function HitThumb({ hit, square }: { hit: SearchHit; square: boolean }) {
  const [shown, setShown] = useState(false);
  const [failed, setFailed] = useState(false);
  const yt = youtubeId(hit.url);
  return (
    <div className={`find-thumb ${square ? "square-cover" : ""}`}>
      <CoverArt seed={seedOf(hit.id)} hue={seedOf(hit.author || hit.platform) % 360} />
      <span className="cover-grain" style={{ backgroundImage: `url(${grainTexture()})` }} />
      {hit.image && !failed && (
        <img
          className={`find-img ${shown ? "on" : ""}`}
          src={hit.image}
          alt=""
          draggable={false}
          onLoad={(e) => (yt && e.currentTarget.naturalWidth < 200 ? setFailed(true) : setShown(true))}
          onError={() => setFailed(true)}
        />
      )}
      {hit.duration > 0 && <span className="find-duration num">{formatDuration(hit.duration)}</span>}
      {hit.in_lang === false && hit.other_lang && (
        <span className="hit-lang" title={t(`Semble être en ${langName(hit.other_lang).toLowerCase()}`, `Seems to be in ${langName(hit.other_lang)}`)}>
          {hit.other_lang.toUpperCase()}
        </span>
      )}
    </div>
  );
}

function langName(code: string): string {
  return LANGS.find((l) => l.code === code)?.name ?? code.toUpperCase();
}

/** Les paroles d'une chanson, dites en un mot. */
function lyricsBadge(hit: SearchHit): { icon: IconName; text: string; on: boolean } {
  if (!hit.lyrics) return { icon: "ban", text: t("Sans paroles", "No lyrics"), on: false };
  return { icon: "lyrics", text: hit.lyrics.synced ? t("Paroles minutées", "Timed lyrics") : t("Paroles", "Lyrics"), on: true };
}

/** `lesson` : la leçon déjà tirée de ce résultat (même vidéo, même page, même chanson). */
function HitCard({ hit, index, lesson }: { hit: SearchHit; index: number; lesson: number | null }) {
  const glow = useGlow<HTMLDivElement>();
  const open = usePreview((s) => s.open);
  const openImportLink = useApp((s) => s.openImportLink);
  const openLesson = useApp((s) => s.openLesson);
  const p = useMemo(() => ({ ...fromHit(hit), lesson }), [hit, lesson]);
  const hover = useRef<number | null>(null);
  const square = hit.kind === "song" || hit.kind === "show" || (hit.platform === "podcast" && hit.kind === "audio");
  const title = cleanTitle(hit.title);
  const show = hit.kind === "show";
  // déjà dans la bibliothèque : la leçon s'ouvre, comme depuis les cartes de Découvrir
  const onOpen = () => (lesson ? openLesson(lesson) : show ? openImportLink(hit.url) : open(p));
  // une vidéo survolée un instant se prépare : l'aperçu s'ouvrira aussitôt
  const warm = () => {
    if (lesson || hit.kind !== "video" || hit.platform === "podcast") return;
    hover.current = window.setTimeout(() => void prefetchStream(hit.url, false).catch(() => {}), 450);
  };
  const cool = () => hover.current !== null && window.clearTimeout(hover.current);
  const facts: string[] = [];
  if (show) facts.push(count(hit.count, "épisode", "épisodes", "episode", "episodes"));
  else if (hit.kind === "song") facts.push(hit.album);
  else if (hit.kind === "text") facts.push(hit.words > 0 ? t(`≈ ${formatNumber(Math.round(hit.words / 50) * 50)} mots`, `≈ ${formatNumber(Math.round(hit.words / 50) * 50)} words`) : "");
  else {
    if (hit.count > 0 && hit.platform !== "podcast") facts.push(t(`${formatNumber(hit.count)} vues`, `${formatNumber(hit.count)} views`));
    if (hit.published) facts.push(ago(hit.published));
  }
  const lyr = hit.kind === "song" ? lyricsBadge(hit) : null;
  return (
    <motion.div
      className={`find-cell hit-cell hit-${hit.kind}`}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: Math.min(index, 8) * 0.035, type: "spring", stiffness: 260, damping: 28 }}
    >
      <div
        className={`find-card glow-surface ${hit.in_lang === false ? "elsewhere" : ""} ${lesson ? "done" : ""}`}
        ref={glow.ref}
        onMouseMove={glow.onMouseMove}
        onMouseEnter={warm}
        onMouseLeave={cool}
      >
        <button
          className="lesson-hit"
          onClick={onOpen}
          aria-label={lesson ? t(`Ouvrir ${title}`, `Open ${title}`) : show ? t(`Voir les épisodes de ${title}`, `See the episodes of ${title}`) : t(`Aperçu de ${title}`, `Preview ${title}`)}
        />
        <HitThumb hit={hit} square={square} />
        {lesson ? (
          <span className="find-done">
            <Icon name="check" size={12} stroke={2.4} /> {t("Dans la bibliothèque", "In your library")}
          </span>
        ) : (
          <span className="hit-badges">
            <JobBadge jobKey={p.key} />
          </span>
        )}
        <div className="find-body">
          <div className="find-top">
            <span className="find-source">{hit.author}</span>
            {hit.lo > 0 && <span className={`lv-chip lv-${hit.hi}`}>{levelRange(hit)}</span>}
          </div>
          <h3 className="find-title" dir="auto">
            {title}
          </h3>
          {hit.kind === "text" && hit.summary && (
            <p className="hit-summary" dir="auto">
              {hit.summary}
            </p>
          )}
          <span className="find-when">
            {lyr && (
              <span className={`hit-lyrics ${lyr.on ? "on" : ""}`}>
                <Icon name={lyr.icon} size={12} /> {lyr.text}
              </span>
            )}
            {facts.filter(Boolean).join(" · ")}
          </span>
        </div>
      </div>
    </motion.div>
  );
}

/** Cartes fantômes traversées d'un reflet, le temps que la plateforme réponde. */
function Ghosts({ n, square }: { n: number; square?: boolean }) {
  return (
    <>
      {Array.from({ length: n }, (_, i) => (
        <div key={i} className={`find-ghost hit-ghost ${square ? "square" : ""}`} style={{ animationDelay: `${i * 0.09}s` }} aria-hidden="true">
          <div className="find-ghost-img" />
          <i />
          <i />
        </div>
      ))}
    </>
  );
}

/** Les résultats d'une plateforme : une rangée (onglet « Tout ») ou une grille. */
function PlatformResults({ lang, platform, compact }: { lang: LangCode; platform: SearchPlatform; compact: boolean }) {
  const bucket = useSearch((s) => s.buckets[platform]);
  const lessonOf = useLessonOf(lang);
  const filters = useFilters();
  // une émission (son flux) n'est pas une leçon : seuls ses épisodes le deviennent
  const lessonFor = (h: SearchHit) => (h.kind === "show" ? null : lessonOf({ url: h.url, page: h.page, ...(h.kind === "song" ? { artist: h.author, title: h.title } : {}) }));
  const setTab = useSearch((s) => s.setTab);
  const more = useSearch((s) => s.more);
  const installing = useSearch((s) => s.installing);
  if (!bucket) return null;
  const square = platform === "music" || platform === "podcast";
  const limit = compact ? (platform === "youtube" ? 8 : 6) : Infinity;
  // filtres de l'affichage : une autre langue, ce qui est déjà dans la bibliothèque
  const kept = bucket.hits.filter((h) => !(filters.onlyLang && h.in_lang === false) && !(filters.hideKnown && lessonFor(h) !== null));
  const masked = bucket.hits.length - kept.length;
  // podcasts : les émissions d'abord, puis les épisodes
  const shows = platform === "podcast" ? kept.filter((h) => h.kind === "show") : [];
  const hits = (platform === "podcast" ? kept.filter((h) => h.kind !== "show") : kept).slice(0, limit);
  if (compact && !bucket.loading && !kept.length) return null;
  const period = bucket.window ? windowText(bucket.window) : "";
  return (
    <section className={`hit-section hit-${platform}`}>
      <header className="shelf-head">
        <div>
          <h3>
            <Icon name={tabIcon(platform)} size={18} /> {platformName(platform)}
          </h3>
          {bucket.loading && !bucket.hits.length && (
            <p>{platform === "youtube" && installing ? t("Installation des composants vidéo, une seule fois…", "Installing the video components, just once…") : t("Recherche…", "Searching…")}</p>
          )}
          {bucket.error && <p className="hit-error">{bucket.error}</p>}
          {!bucket.error && (period || masked > 0) && !(bucket.loading && !bucket.hits.length) && (
            <p className="hit-note">
              {[period, masked > 0 ? count(masked, "résultat masqué par vos filtres", "résultats masqués par vos filtres", "result hidden by your filters", "results hidden by your filters") : ""]
                .filter(Boolean)
                .join(" · ")}
            </p>
          )}
        </div>
        {compact && kept.length > limit && (
          <button className="btn ghost sm" onClick={() => setTab(platform)}>
            {t("Tout voir", "See all")} <Icon name="right" size={14} />
          </button>
        )}
      </header>
      {shows.length > 0 && (
        <>
          <p className="hit-sub">{t("Émissions", "Shows")}</p>
          <div className={compact ? "shelf-track hit-row" : "hit-grid square"}>
            {shows.slice(0, compact ? 6 : 12).map((h, i) => (
              <HitCard key={h.id} hit={h} index={i} lesson={null} />
            ))}
          </div>
          {hits.length > 0 && <p className="hit-sub">{t("Épisodes", "Episodes")}</p>}
        </>
      )}
      <div className={compact && platform !== "youtube" ? "shelf-track hit-row" : `hit-grid ${square ? "square" : ""}`}>
        {hits.map((h, i) => (
          <HitCard key={h.id} hit={h} index={i} lesson={lessonFor(h)} />
        ))}
        {bucket.loading && <Ghosts n={bucket.hits.length ? 4 : compact ? 4 : 8} square={square} />}
      </div>
      {!compact && bucket.more && !bucket.loading && (
        <div className="hit-more">
          <button className="btn outline" onClick={() => more(platform)}>
            {t("Plus de résultats", "More results")}
          </button>
        </div>
      )}
      {!compact && !bucket.loading && !kept.length && !bucket.error && (
        <p className="muted hit-none">
          {bucket.hits.length
            ? t("Vos filtres masquent tous les résultats de cette plateforme.", "Your filters hide every result from this platform.")
            : activeFilters(filters) > 0 || filters.sort !== "relevance"
              ? t("Aucun résultat avec ces filtres : essayez une autre période ou une autre durée.", "No results with these filters: try another period or length.")
              : t("Aucun résultat sur cette plateforme.", "No results on this platform.")}
        </p>
      )}
    </section>
  );
}

/** Une adresse collée : ce que Lumen y a trouvé (TikTok, Instagram, un article…). */
function LinkResult({ lang }: { lang: LangCode }) {
  const link = useSearch((s) => s.link);
  const open = usePreview((s) => s.open);
  const openImportLink = useApp((s) => s.openImportLink);
  if (!link) return null;
  if (link.loading)
    return (
      <div className="hit-link">
        <Orb size={26} /> <span>{t("Lumen regarde ce qu'il y a derrière ce lien…", "Lumen is looking at what's behind this link…")}</span>
      </div>
    );
  if (link.error || !link.info)
    return (
      <div className="hit-link error">
        <Icon name="ban" size={18} /> <span>{link.error}</span>
      </div>
    );
  const info: LinkInfo = link.info;
  let article: { title: string; words: number } | null = null;
  try {
    if (info.html) {
      const a = extractArticle(info.html, info.url);
      const words = a.text.split(/\s+/).filter(Boolean).length;
      if (words >= 40) article = { title: a.title || info.title, words };
    }
  } catch {
    /* pas d'article lisible */
  }
  return (
    <div className="hit-link-list">
      {info.list ? (
        <button className="hit-link found" onClick={() => openImportLink(info.url)}>
          <Icon name="podcast" size={20} />
          <span>
            <strong>{info.title || info.site}</strong>
            <em>{t(`${info.media.length} éléments à choisir`, `${info.media.length} items to choose from`)}</em>
          </span>
          <Icon name="right" size={16} />
        </button>
      ) : (
        info.media.slice(0, 1).map((m) => (
          <button key={m.url} className="hit-link found" onClick={() => open(fromLink(info, m))}>
            <Icon name={m.video ? "video" : "wave"} size={20} />
            <span>
              <strong>{m.title || info.title}</strong>
              <em>{[info.site, m.duration ? formatDuration(m.duration) : ""].filter(Boolean).join(" · ")}</em>
            </span>
            <span className="hit-link-go">
              {t("Aperçu", "Preview")} <Icon name="play" size={12} />
            </span>
          </button>
        ))
      )}
      {article && (
        <button className="hit-link found" onClick={() => open(fromArticle(info, article!.title))}>
          <Icon name="text" size={20} />
          <span>
            <strong>{article.title}</strong>
            <em>{count(article.words, "mot", "mots", "word", "words")}</em>
          </span>
          <span className="hit-link-go">
            {t("Lire", "Read")} <Icon name="right" size={12} />
          </span>
        </button>
      )}
      {!info.media.length && !article && <p className="muted">{info.note || t(`Rien à importer à cette adresse ${inLang(lang)}.`, `Nothing to import at this address ${inLang(lang)}.`)}</p>}
    </div>
  );
}

/** Les trouvailles de Découvrir selon les filtres : période, durée, bibliothèque, puis l'ordre des dates. */
export function filterFinds(items: DiscoverItem[], f: SearchFilters): DiscoverItem[] {
  const now = Date.now() / 1000;
  const span: Record<Exclude<When, "">, number> = { day: 86400, week: 7 * 86400, month: 31 * 86400, year: 366 * 86400, older: 366 * 86400 };
  const out = items.filter((it) => {
    if (f.hideKnown && it.lesson_id) return false;
    if (f.when) {
      if (!it.published) return false;
      const age = now - it.published;
      if (f.when === "older" ? age < span.older : age > span[f.when]) return false;
    }
    if (f.length && it.duration > 0) {
      if (f.length === "short" && it.duration >= 240) return false;
      if (f.length === "medium" && (it.duration < 240 || it.duration > 1200)) return false;
      if (f.length === "long" && it.duration <= 1200) return false;
    }
    // un article n'a pas de durée : il ne répond pas à un filtre de durée
    if (f.length && it.duration <= 0) return false;
    return true;
  });
  if (f.sort === "recent") out.sort((a, b) => b.published - a.published);
  if (f.sort === "old") out.sort((a, b) => (a.published || Infinity) - (b.published || Infinity));
  return out;
}

/** Le champ de recherche de Découvrir, ses catégories (choisies avant ou après la recherche), et ses idées quand il est vide. */
export function SearchBar({ lang }: { lang: LangCode }) {
  const query = useSearch((s) => s.query);
  const run = useSearch((s) => s.run);
  const tab = useSearch((s) => s.tab);
  const setTab = useSearch((s) => s.setTab);
  const [text, setText] = useState(query);
  const [focus, setFocus] = useState(false);
  const [panel, setPanel] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const recent = useApp((s) => s.settings[`search_recent_${lang}`]) ? recentSearches(lang) : [];
  useEffect(() => setText(query), [query]);
  // ⌘F ou / : le champ de recherche
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLElement && e.target.closest("input, textarea")) return;
      // l'aperçu ouvert garde le clavier
      if (usePreview.getState().item) return;
      if ((e.key === "f" && (e.metaKey || e.ctrlKey)) || e.key === "/") {
        e.preventDefault();
        input.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const go = (q: string, category?: SearchTab) => {
    setText(q);
    run(lang, q, category);
    input.current?.blur();
  };
  // une catégorie touchée : le texte tapé mais pas encore cherché part avec elle ;
  // champ vide, le curseur y revient pour taper la recherche
  const pick = (p: SearchTab) => {
    const q = text.trim();
    if (q && q !== query) go(q, p);
    else {
      setTab(p);
      if (!q) input.current?.focus();
    }
  };
  const ideas = searchIdeas(lang);
  const link = URL_LIKE.test(text.trim());
  return (
    <div className={`disc-search ${focus ? "focus" : ""} ${query ? "active" : ""}`} data-tour="search">
      <form
        className="disc-search-field"
        onSubmit={(e) => {
          e.preventDefault();
          go(text);
        }}
      >
        <Icon name={link ? "link" : "search"} size={19} />
        <input
          data-find
          ref={input}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => setFocus(true)}
          onBlur={() => window.setTimeout(() => setFocus(false), 160)}
          placeholder={placeholderOf(tab, lang)}
          aria-label={t("Chercher en ligne", "Search online")}
          dir="auto"
          spellCheck={false}
          enterKeyHint="search"
        />
        {text && (
          <button type="button" className="icon-btn" onClick={() => go("")} aria-label={t("Effacer", "Clear")}>
            <Icon name="close" size={15} />
          </button>
        )}
        <button type="submit" className="btn primary sm disc-search-go" disabled={!text.trim()}>
          {link ? t("Ouvrir", "Open") : t("Chercher", "Search")}
        </button>
      </form>
      <AnimatePresence>
        {focus && !text.trim() && (
          <motion.div className="disc-search-ideas" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -4 }} transition={{ duration: 0.18 }}>
            {recent.length > 0 && (
              <div className="ideas-row">
                <span className="ideas-label">{t("Récentes", "Recent")}</span>
                {recent.map((q) => (
                  <button key={q} className="idea recent" onMouseDown={(e) => e.preventDefault()} onClick={() => go(q)}>
                    <Icon name="clock" size={12} /> {q}
                  </button>
                ))}
                <button className="disc-link" onMouseDown={(e) => e.preventDefault()} onClick={() => forgetSearches(lang)}>
                  {t("Effacer", "Clear")}
                </button>
              </div>
            )}
            <div className="ideas-row">
              <span className="ideas-label">{t("Idées", "Ideas")}</span>
              {ideas.map((q) => (
                <button key={q} className="idea" dir="auto" onMouseDown={(e) => e.preventDefault()} onClick={() => go(q)}>
                  {q}
                </button>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
      {!(query && URL_LIKE.test(query)) && (
        <>
          <div className="disc-search-row">
            <div className="disc-search-tabs" role="tablist" aria-label={t("Où chercher", "Where to search")}>
              {(["all", ...PLATFORMS] as SearchTab[]).map((p) => (
                <button
                  key={p}
                  role="tab"
                  aria-selected={tab === p}
                  className={`search-tab ${tab === p ? "on" : ""}`}
                  // le champ garde le curseur : on choisit la catégorie, puis on tape
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => pick(p)}
                >
                  {tab === p && <motion.span layoutId="search-tab-pill" className="search-tab-pill" transition={{ type: "spring", stiffness: 500, damping: 38 }} />}
                  <Icon name={tabIcon(p)} size={14} /> {platformName(p)}
                </button>
              ))}
            </div>
            <FilterTools scope={query ? tab : "feed"} open={panel} onToggle={() => setPanel((v) => !v)} />
          </div>
          <FilterPanel scope={query ? tab : "feed"} lang={lang} open={panel} />
        </>
      )}
    </div>
  );
}

/** Les tris proposés, avec leur icône et ce qu'ils font. */
function sortChoices(scope: SearchTab | "feed"): { value: SortBy; icon: IconName; label: string; hint: string }[] {
  return [
    {
      value: "relevance",
      icon: "sparkle",
      label: scope === "feed" ? t("Pour vous", "For you") : t("Pertinence", "Relevance"),
      hint: scope === "feed" ? t("Rangé par rayon, à votre niveau", "Sorted by shelf, at your level") : t("Ce qui répond le mieux", "What matches best"),
    },
    { value: "recent", icon: "clock", label: t("Les plus récents", "Most recent"), hint: t("Publiés ces derniers jours", "Published in the last few days") },
    { value: "popular", icon: "flame", label: t("Les plus populaires", "Most popular"), hint: t("Les plus vus et les plus écoutés", "The most watched and listened to") },
    { value: "old", icon: "book", label: t("Les plus anciens", "Oldest"), hint: t("Des contenus d'archives", "Archive content") },
  ];
}

/** Pourquoi un tri ou un filtre n'est pas proposé ici. */
function unsupported(scope: SearchTab | "feed", what: Ability): string {
  if (scope === "feed")
    return what === "popular"
      ? t("La popularité se lit dans une recherche en ligne : tapez ce qui vous intéresse.", "Popularity comes with an online search: type what interests you.")
      : t("Seulement pour une recherche en ligne.", "Only for an online search.");
  if (scope === "music") return t("Les chansons se classent seulement par popularité.", "Songs can only be sorted by popularity.");
  if (scope === "wiki") return t("Les articles se classent par date de création ou par popularité, sans autre filtre.", "Articles sort by creation date or popularity, with no other filter.");
  if (scope === "podcast") return t("L'annuaire des podcasts ne dit pas leur popularité.", "The podcast directory doesn't share popularity.");
  return t("Seulement pour les vidéos YouTube.", "YouTube videos only.");
}

/** Tri (menu) et bouton « Filtres », à droite des catégories. */
function FilterTools({ scope, open, onToggle }: { scope: SearchTab | "feed"; open: boolean; onToggle(): void }) {
  const f = useFilters();
  const setFilters = useSearch((s) => s.setFilters);
  const [menu, setMenu] = useState(false);
  const choices = sortChoices(scope);
  const current = choices.find((c) => c.value === f.sort) ?? choices[0];
  // un tri que l'onglet ne connaît pas s'affiche tel quel, mais il est signalé
  const off = f.sort !== "relevance" && !tabSupports(scope, f.sort);
  const offText = f.sort !== "relevance" ? unsupported(scope, f.sort) : undefined;
  const n = activeFilters(f);
  return (
    <div className="filter-tools">
      <Menu
        open={menu}
        onClose={() => setMenu(false)}
        align="right"
        anchor={
          <button
            className={`filter-btn ${f.sort !== "relevance" ? "on" : ""} ${off ? "off" : ""}`}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => setMenu((v) => !v)}
            aria-haspopup="menu"
            aria-expanded={menu}
            title={off ? offText : undefined}
          >
            <Icon name={current.icon} size={14} />
            <span className="filter-btn-label">{current.label}</span>
            <Icon name="chevron" size={13} />
          </button>
        }
      >
        <div className="menu-note">{t("Trier les résultats", "Sort the results")}</div>
        {choices.map((c) => {
          const ok = c.value === "relevance" || tabSupports(scope, c.value);
          return (
            <button
              key={c.value}
              className={`menu-item sort-item ${ok ? "" : "disabled"}`}
              role="menuitemradio"
              aria-checked={f.sort === c.value}
              aria-disabled={!ok}
              title={ok ? undefined : unsupported(scope, c.value as Ability)}
              onClick={() => {
                if (!ok) return;
                setMenu(false);
                setFilters({ sort: c.value });
              }}
            >
              <Icon name={c.icon} size={16} />
              <span className="sort-text">
                <strong>{c.label}</strong>
                <em>{ok ? c.hint : unsupported(scope, c.value as Ability)}</em>
              </span>
              {f.sort === c.value && <Icon name="check" size={16} />}
            </button>
          );
        })}
      </Menu>
      <button className={`filter-btn ${open || n ? "on" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={onToggle} aria-expanded={open}>
        <Icon name="layers" size={14} />
        <span className="filter-btn-label">{t("Filtres", "Filters")}</span>
        {n > 0 && <span className="filter-count num">{n}</span>}
      </button>
    </div>
  );
}

/** Une rangée de pastilles à choix unique. */
function ChipRow<T extends string>({ label, value, options, disabled, onChange }: { label: string; value: T; options: [T, string][]; disabled?: string; onChange(v: T): void }) {
  return (
    <div className={`filter-row ${disabled ? "disabled" : ""}`} title={disabled}>
      <span className="ideas-label">{label}</span>
      <div className="filter-chips" role="radiogroup" aria-label={label}>
        {options.map(([v, text]) => (
          <button
            key={v}
            role="radio"
            aria-checked={value === v}
            disabled={!!disabled}
            className={`idea ${value === v ? "on" : ""}`}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onChange(v)}
          >
            {text}
          </button>
        ))}
      </div>
    </div>
  );
}

/** Le panneau des filtres : date, durée, sous-titres, ce qu'on cache. */
function FilterPanel({ scope, lang, open }: { scope: SearchTab | "feed"; lang: LangCode; open: boolean }) {
  const f = useFilters();
  const setFilters = useSearch((s) => s.setFilters);
  const feed = scope === "feed";
  const toggle = (on: boolean, label: string, icon: IconName, onChange: (v: boolean) => void, disabled?: string) => (
    <button role="switch" aria-checked={on} disabled={!!disabled} title={disabled} className={`idea toggle ${on ? "on" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={() => onChange(!on)}>
      <Icon name={on ? "check" : icon} size={13} stroke={on ? 2.4 : undefined} /> {label}
    </button>
  );
  const changed = activeFilters(f) > 0 || f.sort !== "relevance";
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          className="filter-panel"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ type: "spring", stiffness: 380, damping: 36, opacity: { duration: 0.18 } }}
        >
          <div className="filter-panel-inner">
            <ChipRow<When>
              label={t("Publié", "Published")}
              value={f.when}
              disabled={tabSupports(scope, "when") ? undefined : unsupported(scope, "when")}
              onChange={(when) => setFilters({ when })}
              options={[
                ["", t("N'importe quand", "Any time")],
                ["day", t("Aujourd'hui", "Today")],
                ["week", t("Cette semaine", "This week")],
                ["month", t("Ce mois-ci", "This month")],
                ["year", t("Cette année", "This year")],
                ["older", t("Il y a plus d'un an", "Over a year ago")],
              ]}
            />
            <ChipRow<Length>
              label={t("Durée", "Length")}
              value={f.length}
              disabled={tabSupports(scope, "length") ? undefined : unsupported(scope, "length")}
              onChange={(length) => setFilters({ length })}
              options={[
                ["", t("Toutes", "Any")],
                ["short", t("Moins de 4 min", "Under 4 min")],
                ["medium", t("4 à 20 min", "4–20 min")],
                ["long", t("Plus de 20 min", "Over 20 min")],
              ]}
            />
            <div className="filter-row">
              <span className="ideas-label">{t("Afficher", "Show")}</span>
              <div className="filter-chips">
                {!feed &&
                  toggle(
                    f.subtitles,
                    t("Vidéos sous-titrées", "Videos with subtitles"),
                    "type",
                    (subtitles) => setFilters({ subtitles }),
                    tabSupports(scope, "subtitles") ? undefined : unsupported(scope, "subtitles"),
                  )}
                {!feed && toggle(f.onlyLang, t(`Seulement ${inLang(lang)}`, `Only ${inLang(lang)}`), "globe", (onlyLang) => setFilters({ onlyLang }))}
                {toggle(f.hideKnown, t("Masquer ma bibliothèque", "Hide my library"), "eye", (hideKnown) => setFilters({ hideKnown }))}
              </div>
            </div>
            <div className="filter-foot">
              <span className="muted">
                {feed
                  ? t("Sans recherche, les filtres s'appliquent aux trouvailles de Lumen.", "Without a search, the filters apply to Lumen's finds.")
                  : scope === "all"
                    ? t("Chaque plateforme applique ce qu'elle sait faire.", "Each platform applies what it can.")
                    : ""}
              </span>
              {changed && (
                <button className="disc-link" onMouseDown={(e) => e.preventDefault()} onClick={() => setFilters(NO_FILTERS)}>
                  {t("Tout réinitialiser", "Reset all")}
                </button>
              )}
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Les résultats de la recherche en ligne (et, d'abord, ce que Découvrir a déjà trouvé). */
export function SearchResults({ lang, local, onBack, renderLocal }: { lang: LangCode; local: DiscoverItem[]; onBack(): void; renderLocal(items: DiscoverItem[]): React.ReactNode }) {
  const query = useSearch((s) => s.query);
  const tab = useSearch((s) => s.tab);
  const link = useSearch((s) => s.link);
  const f = useFilters();
  // les trouvailles de Lumen suivent aussi les filtres qu'elles connaissent
  const shownLocal = useMemo(() => filterFinds(local, f), [local, f]);
  return (
    <motion.div className="search-results" key={`${query}-${tab}`} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.26, ease: [0.2, 0.8, 0.2, 1] }}>
      <div className="search-head">
        <button className="btn ghost sm" onClick={onBack}>
          <Icon name="left" size={15} /> {t("Découvertes du jour", "Today's finds")}
        </button>
      </div>
      {link ? (
        <LinkResult lang={lang} />
      ) : (
        <>
          {tab === "all" && shownLocal.length > 0 && (
            <section className="hit-section">
              <header className="shelf-head">
                <div>
                  <h3>
                    <Icon name="sparkle" size={18} /> {t("Dans vos découvertes", "In your finds")}
                  </h3>
                  <p>{t("Déjà trouvé par Lumen, rangé par niveau", "Already found by Lumen, sorted by level")}</p>
                </div>
              </header>
              {renderLocal(shownLocal)}
            </section>
          )}
          {(tab === "all" ? PLATFORMS : [tab]).map((p) => (
            <PlatformResults key={p} lang={lang} platform={p as SearchPlatform} compact={tab === "all"} />
          ))}
        </>
      )}
    </motion.div>
  );
}
