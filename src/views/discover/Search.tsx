import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { CoverArt } from "../../components/Cover";
import { Icon, type IconName } from "../../components/Icon";
import { prefetchStream } from "../../components/Preview";
import { Orb, useGlow } from "../../components/ui";
import { grainTexture, youtubeId } from "../../lib/covers";
import { ago, cleanTitle, levelRange } from "../../lib/discover";
import { count, formatNumber, t } from "../../lib/i18n";
import { extractArticle } from "../../lib/importers";
import { stageText, useJob } from "../../lib/imports";
import { LANGS, inLang } from "../../lib/langs";
import { fromArticle, fromHit, fromLink, usePreview } from "../../lib/preview";
import { PLATFORMS, URL_LIKE, forgetSearches, platformName, recentSearches, searchIdeas, useSearch, type Length, type SearchTab } from "../../lib/search";
import { formatDuration, useApp } from "../../lib/store";
import type { DiscoverItem, LangCode, LinkInfo, SearchHit, SearchPlatform } from "../../lib/types";

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

function HitCard({ hit, index }: { hit: SearchHit; index: number }) {
  const glow = useGlow<HTMLDivElement>();
  const open = usePreview((s) => s.open);
  const openImportLink = useApp((s) => s.openImportLink);
  const p = useMemo(() => fromHit(hit), [hit]);
  const hover = useRef<number | null>(null);
  const square = hit.kind === "song" || hit.kind === "show" || (hit.platform === "podcast" && hit.kind === "audio");
  const title = cleanTitle(hit.title);
  const show = hit.kind === "show";
  const onOpen = () => (show ? openImportLink(hit.url) : open(p));
  // une vidéo survolée un instant se prépare : l'aperçu s'ouvrira aussitôt
  const warm = () => {
    if (hit.kind !== "video" || hit.platform === "podcast") return;
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
      <div className={`find-card glow-surface ${hit.in_lang === false ? "elsewhere" : ""}`} ref={glow.ref} onMouseMove={glow.onMouseMove} onMouseEnter={warm} onMouseLeave={cool}>
        <button className="lesson-hit" onClick={onOpen} aria-label={show ? t(`Voir les épisodes de ${title}`, `See the episodes of ${title}`) : t(`Aperçu de ${title}`, `Preview ${title}`)} />
        <HitThumb hit={hit} square={square} />
        <span className="hit-badges">
          <JobBadge jobKey={p.key} />
        </span>
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
function PlatformResults({ platform, compact }: { platform: SearchPlatform; compact: boolean }) {
  const bucket = useSearch((s) => s.buckets[platform]);
  const setTab = useSearch((s) => s.setTab);
  const more = useSearch((s) => s.more);
  const installing = useSearch((s) => s.installing);
  if (!bucket) return null;
  const square = platform === "music" || platform === "podcast";
  const limit = compact ? (platform === "youtube" ? 8 : 6) : Infinity;
  // podcasts : les émissions d'abord, puis les épisodes
  const shows = platform === "podcast" ? bucket.hits.filter((h) => h.kind === "show") : [];
  const hits = (platform === "podcast" ? bucket.hits.filter((h) => h.kind !== "show") : bucket.hits).slice(0, limit);
  if (compact && !bucket.loading && !bucket.hits.length) return null;
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
        </div>
        {compact && bucket.hits.length > limit && (
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
              <HitCard key={h.id} hit={h} index={i} />
            ))}
          </div>
          {hits.length > 0 && <p className="hit-sub">{t("Épisodes", "Episodes")}</p>}
        </>
      )}
      <div className={compact && platform !== "youtube" ? "shelf-track hit-row" : `hit-grid ${square ? "square" : ""}`}>
        {hits.map((h, i) => (
          <HitCard key={h.id} hit={h} index={i} />
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
      {!compact && !bucket.loading && !bucket.hits.length && !bucket.error && <p className="muted hit-none">{t("Aucun résultat sur cette plateforme.", "No results on this platform.")}</p>}
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

/** Le champ de recherche de Découvrir, ses catégories (choisies avant ou après la recherche), et ses idées quand il est vide. */
export function SearchBar({ lang }: { lang: LangCode }) {
  const query = useSearch((s) => s.query);
  const run = useSearch((s) => s.run);
  const tab = useSearch((s) => s.tab);
  const setTab = useSearch((s) => s.setTab);
  const [text, setText] = useState(query);
  const [focus, setFocus] = useState(false);
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
    <div className={`disc-search ${focus ? "focus" : ""} ${query ? "active" : ""}`}>
      <form
        className="disc-search-field"
        onSubmit={(e) => {
          e.preventDefault();
          go(text);
        }}
      >
        <Icon name={link ? "link" : "search"} size={19} />
        <input
          ref={input}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => setFocus(true)}
          onBlur={() => window.setTimeout(() => setFocus(false), 160)}
          placeholder={placeholderOf(tab, lang)}
          aria-label={t("Chercher en ligne", "Search online")}
          dir="auto"
          spellCheck={false}
        />
        {text && (
          <button type="button" className="icon-btn" onClick={() => go("")} aria-label={t("Effacer", "Clear")}>
            <Icon name="close" size={15} />
          </button>
        )}
        <button type="submit" className="btn primary sm" disabled={!text.trim()}>
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
      )}
    </div>
  );
}

/** Les résultats de la recherche en ligne (et, d'abord, ce que Découvrir a déjà trouvé). */
export function SearchResults({ lang, local, onBack, renderLocal }: { lang: LangCode; local: DiscoverItem[]; onBack(): void; renderLocal(items: DiscoverItem[]): React.ReactNode }) {
  const query = useSearch((s) => s.query);
  const tab = useSearch((s) => s.tab);
  const length = useSearch((s) => s.length);
  const setLength = useSearch((s) => s.setLength);
  const link = useSearch((s) => s.link);
  const video = tab === "youtube" || tab === "dailymotion" || tab === "all";
  return (
    <motion.div className="search-results" key={`${query}-${tab}`} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.26, ease: [0.2, 0.8, 0.2, 1] }}>
      <div className="search-head">
        <button className="btn ghost sm" onClick={onBack}>
          <Icon name="left" size={15} /> {t("Découvertes du jour", "Today's finds")}
        </button>
        {video && !link && (
          <div className="length-chips" role="radiogroup" aria-label={t("Durée des vidéos", "Video length")}>
            {(
              [
                ["", t("Toutes durées", "Any length")],
                ["short", t("Moins de 4 min", "Under 4 min")],
                ["medium", "4–20 min"],
                ["long", t("Plus de 20 min", "Over 20 min")],
              ] as [Length, string][]
            ).map(([v, label]) => (
              <button key={v} role="radio" aria-checked={length === v} className={`idea ${length === v ? "on" : ""}`} onClick={() => setLength(v)}>
                {label}
              </button>
            ))}
          </div>
        )}
      </div>
      {link ? (
        <LinkResult lang={lang} />
      ) : (
        <>
          {tab === "all" && local.length > 0 && (
            <section className="hit-section">
              <header className="shelf-head">
                <div>
                  <h3>
                    <Icon name="sparkle" size={18} /> {t("Dans vos découvertes", "In your finds")}
                  </h3>
                  <p>{t("Déjà trouvé par Lumen, rangé par niveau", "Already found by Lumen, sorted by level")}</p>
                </div>
              </header>
              {renderLocal(local)}
            </section>
          )}
          {(tab === "all" ? PLATFORMS : [tab]).map((p) => (
            <PlatformResults key={p} platform={p as SearchPlatform} compact={tab === "all"} />
          ))}
        </>
      )}
    </motion.div>
  );
}
