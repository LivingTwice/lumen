import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon, type IconName } from "../components/Icon";
import { Sheet } from "../components/ui";
import { api, errorText, isTauri } from "../lib/api";
import { youtubeId } from "../lib/covers";
import { pickFiles } from "../lib/dialogs";
import { cleanTitle, useDiscover } from "../lib/discover";
import { stageText } from "../lib/imports";
import {
  MEDIA_EXT,
  baseName,
  cleanText,
  decodeText,
  extOf,
  extractArticle,
  extractEpub,
  extractPdf,
  extractSubtitles,
  splitLong,
  wordCount,
  type Chapter,
} from "../lib/importers";
import { count, formatNumber, locale, pick, t } from "../lib/i18n";
import { inLang } from "../lib/langs";
import { formatBytes, formatDuration, useApp, type ImportTab } from "../lib/store";
import type { DiscoverItem, ImportEvent, LinkInfo, LinkMedia, NewLesson } from "../lib/types";
import { PodcastTab, usePodcastForm } from "./import/PodcastTab";

type Tab = ImportTab;

const tabs = (): { id: Tab; label: string; hint: string; icon: IconName }[] => [
  { id: "text", label: t("Texte", "Text"), hint: t("Coller", "Paste"), icon: "text" },
  { id: "link", label: t("Lien", "Link"), hint: t("Web, vidéo, podcast", "Web, video, podcast"), icon: "link" },
  { id: "file", label: t("Fichier", "File"), hint: "EPUB, PDF, SRT…", icon: "file" },
  { id: "media", label: t("Audio, vidéo", "Audio, video"), hint: "Transcription", icon: "wave" },
  { id: "podcast", label: "Podcast", hint: t("Écrit pour vous", "Made for you"), icon: "podcast" },
];

interface Pending {
  title: string;
  text: string;
  kind: string;
  source: string;
  collection?: string;
}

/** Ce qu'on importe d'un lien qui offre à la fois un texte et un son. */
type Choice = "both" | "media" | "article";

/** Lien analysé : ce qu'il contient et ce que l'utilisateur en garde. */
interface Found {
  info: LinkInfo;
  article: { title: string; text: string; words: number } | null;
  choice: Choice;
  /** éléments cochés d'une liste, dans l'ordre de la liste */
  picked: number[];
}

/** Un lien seul, collé dans le champ de texte. */
const LONE_LINK = /^(https?:\/\/|www\.)\S+$/i;

/** Nom des éléments d'une liste, accordé : « 3 épisodes », « 12 vidéos », « 10 morceaux ». */
function itemsLabel(info: LinkInfo, n: number) {
  if (info.via === "youtube") return count(n, "morceau", "morceaux", "song", "songs");
  if (info.media.length && info.media.every((m) => m.video)) return count(n, "vidéo", "vidéos", "video", "videos");
  return count(n, "épisode", "épisodes", "episode", "episodes");
}

function shortDate(d: string) {
  const date = new Date(`${d}T12:00:00`);
  if (Number.isNaN(date.getTime())) return "";
  const year = date.getFullYear() === new Date().getFullYear() ? undefined : "numeric";
  return date.toLocaleDateString(locale(), { day: "numeric", month: "short", year });
}

/** Le son ou la vidéo d'un élément de Découvrir, prêt pour `importLink`. */
function itemMedia(item: DiscoverItem): LinkMedia {
  const date = item.published ? new Date(item.published * 1000).toISOString().slice(0, 10) : "";
  return {
    url: item.url,
    title: cleanTitle(item.title),
    duration: item.duration,
    video: item.kind === "video",
    // une vidéo YouTube passe par yt-dlp ; un épisode de podcast se télécharge tel quel
    direct: !youtubeId(item.url),
    image: item.image,
    date,
    page: item.page || item.url,
    collection: item.source_name,
  };
}

/** Vignette du lien : l'image trouvée, sinon une icône dans un halo. */
function LinkThumb({ src, icon }: { src: string; icon: IconName }) {
  const [failed, setFailed] = useState(false);
  return (
    <span className="link-thumb">
      {src && !failed ? <img src={src} alt="" draggable={false} onError={() => setFailed(true)} /> : <Icon name={icon} size={26} stroke={1.6} />}
    </span>
  );
}

export function ImportSheet() {
  const open = useApp((s) => s.importOpen);
  const files = useApp((s) => s.importFiles);
  const item = useApp((s) => s.importItem);
  const linkToOpen = useApp((s) => s.importUrl);
  const tabToOpen = useApp((s) => s.importTab);
  const close = useApp((s) => s.closeImport);
  const lang = useApp((s) => s.lang)();
  const info = useApp((s) => s.info);
  const models = useApp((s) => s.models);
  const downloads = useApp((s) => s.downloads);
  const download = useApp((s) => s.download);
  const toast = useApp((s) => s.toast);
  const bump = useApp((s) => s.bumpLibrary);
  const openLesson = useApp((s) => s.openLesson);
  const go = useApp((s) => s.go);

  const [tab, setTab] = useState<Tab>("text");
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [preview, setPreview] = useState<Pending | null>(null);
  const [book, setBook] = useState<{ title: string; chapters: (Chapter & { on: boolean })[] } | null>(null);
  const [mediaFiles, setMediaFiles] = useState<string[]>([]);
  const [found, setFound] = useState<Found | null>(null);
  const [error, setError] = useState<string | null>(null);
  const podcast = usePodcastForm(lang);

  const asrReady = models.some((m) => m.kind === "asr" && m.installed);
  const asrModel = models.find((m) => m.kind === "asr" && m.id === "whisper-turbo") ?? models.find((m) => m.kind === "asr");

  const reset = () => {
    setTitle("");
    setText("");
    setUrl("");
    setBusy(null);
    setProgress(null);
    setPreview(null);
    setBook(null);
    setMediaFiles([]);
    setFound(null);
    setError(null);
    podcast.reset();
  };

  useEffect(() => {
    if (!open) return;
    reset();
    if (tabToOpen) {
      setTab(tabToOpen);
      return;
    }
    if (item) {
      // depuis Découvrir : la vidéo ou l'épisode est déjà connu ; un article (ou un
      // épisode dont la page porte le texte) se lit d'abord sur sa page
      const address = item.page || item.url;
      setTab("link");
      setUrl(address);
      if (item.kind === "text" || item.page_text) void probeLink(address, item);
      else
        setFound({
          info: { url: address, title: cleanTitle(item.title), site: item.source_name, image: item.image, html: "", media: [itemMedia(item)], list: false, via: "", note: "" },
          article: null,
          choice: "media",
          picked: [0],
        });
      return;
    }
    if (linkToOpen) {
      setTab("link");
      setUrl(linkToOpen);
      void probeLink(linkToOpen);
      return;
    }
    if (files && files.length) {
      const media = files.filter((f) => MEDIA_EXT.includes(extOf(f)));
      const docs = files.filter((f) => !MEDIA_EXT.includes(extOf(f)));
      if (media.length && !docs.length) {
        setTab("media");
        setMediaFiles(media);
      } else {
        setTab("file");
        void loadDocs(docs);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, files, item, linkToOpen, tabToOpen]);

  const finish = (ids: number[], label: string) => {
    // la carte de Découvrir mène désormais à la leçon
    if (item && ids.length) void useDiscover.getState().mark(lang, item.id, ids[0]);
    bump();
    toast(label, "light");
    close();
    if (ids.length === 1) openLesson(ids[0]);
    else go("library");
  };

  const create = async (items: Pending[]) => {
    const ids: number[] = [];
    for (const it of items) {
      const parts = splitLong(it.text);
      for (let i = 0; i < parts.length; i++) {
        const l: NewLesson = {
          lang,
          title: parts.length > 1 ? `${it.title} · ${i + 1}/${parts.length}` : it.title,
          text: parts[i],
          kind: it.kind,
          source: it.source,
          collection: it.collection ?? (parts.length > 1 ? it.title : ""),
        };
        ids.push(await api().lessonCreate(l));
      }
    }
    return ids;
  };

  const run = async (label: string, fn: () => Promise<void>) => {
    setError(null);
    setBusy(label);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  // ---------- texte ----------
  const createText = () =>
    run(t("Création de la leçon", "Creating the lesson"), async () => {
      const body = cleanText(text);
      const ids = await create([{ title: title.trim() || body.split(/\s+/).slice(0, 6).join(" "), text: body, kind: "text", source: "" }]);
      finish(ids, t("Leçon créée", "Lesson created"));
    });

  // ---------- lien : article, vidéo, podcast, Spotify ----------
  /** `hint` : l'élément de Découvrir derrière ce lien (son, collection, texte de la page) */
  const probeLink = (address = url, hint?: DiscoverItem) =>
    run(t("Lecture du lien", "Reading the link"), async () => {
      const info = await api().linkProbe(address.trim(), onImportEvent(""));
      // la page d'un épisode ne montre pas toujours son lecteur : le son du flux le remplace
      if (hint && hint.kind !== "text" && !info.media.length) info.media = [itemMedia(hint)];
      if (hint && !info.site) info.site = hint.source_name;
      let article: Found["article"] = null;
      if (info.html) {
        try {
          const a = extractArticle(info.html, info.url);
          const words = wordCount(a.text);
          if (words >= 40) article = { title: a.title || info.title, text: a.text, words };
        } catch {
          // pas d'article lisible : le son ou la vidéo seulement
        }
      }
      if (!info.media.length || hint?.kind === "text") {
        if (!article) throw info.note || t("Lumen n'a trouvé ni texte ni son à importer à cette adresse.", "Lumen found no text or sound to import at this address.");
        // le titre donné par la source est plus sûr que celui que Readability devine (« … – ANSA.it »)
        setPreview({ title: hint ? cleanTitle(hint.title) : article.title || info.title, text: article.text, kind: "web", source: info.url, collection: hint?.source_name });
        return;
      }
      // un long article accompagné d'un son : le texte d'abord ; un texte court : le son, recalé sur le texte s'il le suit.
      // Un épisode dont la page porte le texte (DW, RFI…) : le son avec ce texte, la lanterne calée dessus.
      const choice: Choice = !article ? "media" : hint?.page_text && !info.list ? "both" : article.words >= 150 ? "article" : info.list ? "media" : "both";
      setFound({ info, article, choice, picked: [0] });
    });

  const importFound = () => {
    if (!found) return;
    const { info, article, choice } = found;
    if (choice === "article" && article) {
      setPreview({ title: article.title, text: article.text, kind: "web", source: info.url });
      return;
    }
    const items: LinkMedia[] = info.list ? found.picked.map((i) => info.media[i]) : info.media.slice(0, 1);
    const text = choice === "both" && article ? article.text : null;
    void run(t("Préparation", "Preparing"), async () => {
      const ids: number[] = [];
      let failed = 0;
      let firstError: unknown = null;
      for (let i = 0; i < items.length; i++) {
        const prefix = items.length > 1 ? `${i + 1}/${items.length} · ` : "";
        try {
          ids.push(await api().importLink(lang, items[i], text, onImportEvent(prefix)));
        } catch (e) {
          // un élément d'une liste qui échoue n'arrête pas les suivants
          if (items.length === 1) throw e;
          failed++;
          firstError ??= e;
        }
      }
      if (!ids.length) throw firstError;
      finish(
        ids,
        ids.length > 1
          ? t(`${ids.length} leçons créées`, `${ids.length} lessons created`)
          : items[0].video
            ? t("Vidéo transcrite", "Video transcribed")
            : t("Transcription prête", "Transcript ready"),
      );
      if (failed)
        toast(
          `${pick(failed, `${failed} import n'a pas abouti`, `${failed} imports n'ont pas abouti`, `${failed} import failed`, `${failed} imports failed`)}${t(" : ", ": ")}${errorText(firstError)}`,
          "error",
        );
    });
  };

  // ---------- fichiers ----------
  async function loadDocs(paths: string[]) {
    await run(t("Lecture du fichier", "Reading the file"), async () => {
      const pend: Pending[] = [];
      for (const p of paths) {
        const ext = extOf(p);
        const data = await api().readFile(p);
        if (ext === "epub") {
          const { book, chapters } = await extractEpub(data);
          setBook({ title: book, chapters: chapters.map((c) => ({ ...c, on: true })) });
          return;
        }
        if (ext === "pdf") {
          const r = await extractPdf(data);
          pend.push({ title: r.title === t("Document PDF", "PDF document") ? baseName(p) : r.title, text: r.text, kind: "pdf", source: p });
        } else if (ext === "srt" || ext === "vtt") {
          pend.push({ title: baseName(p), text: extractSubtitles(decodeText(data)), kind: "subtitles", source: p });
        } else if (ext === "html" || ext === "htm") {
          const a = extractArticle(decodeText(data), "file:///" + p);
          pend.push({ title: a.title || baseName(p), text: a.text, kind: "web", source: p });
        } else {
          pend.push({ title: baseName(p), text: cleanText(decodeText(data)), kind: "text", source: p });
        }
      }
      if (pend.length === 1) setPreview(pend[0]);
      else if (pend.length > 1) {
        const ids = await create(pend);
        finish(ids, t(`${ids.length} leçons créées`, `${ids.length} lessons created`));
      }
    });
  }

  const chooseDocs = async () => {
    const paths = await pickFiles([{ name: t("Textes et livres", "Texts and books"), extensions: ["epub", "pdf", "txt", "md", "srt", "vtt", "html", "htm"] }]);
    if (paths.length) await loadDocs(paths);
  };

  const createBook = () =>
    run(t("Création des chapitres", "Creating the chapters"), async () => {
      if (!book) return;
      const chosen = book.chapters.filter((c) => c.on);
      const ids = await create(chosen.map((c) => ({ title: c.title, text: c.text, kind: "book", source: "", collection: book.title })));
      finish(ids, t(`${chosen.length} chapitre${chosen.length > 1 ? "s" : ""} ajouté${chosen.length > 1 ? "s" : ""}`, `${count(chosen.length, "", "", "chapter", "chapters")} added`));
    });

  // ---------- audio et vidéo ----------
  const chooseMedia = async () => {
    const paths = await pickFiles([{ name: t("Audio et vidéo", "Audio and video"), extensions: MEDIA_EXT }]);
    if (paths.length) setMediaFiles((m) => [...m, ...paths.filter((p) => !m.includes(p))]);
  };

  const onImportEvent = (stagePrefix: string) => (e: ImportEvent) => {
    if (e.type === "stage") {
      setBusy(`${stagePrefix}${stageText(e.stage)}`);
      setProgress(["transcribe", "timing", "text", "download", "file", "tools"].includes(e.stage) ? 0 : null);
    } else setProgress(e.value);
  };

  const transcribeAll = () =>
    run(t("Préparation", "Preparing"), async () => {
      const ids: number[] = [];
      for (let i = 0; i < mediaFiles.length; i++) {
        const prefix = mediaFiles.length > 1 ? `${i + 1}/${mediaFiles.length} · ` : "";
        ids.push(await api().importMedia(lang, mediaFiles[i], null, onImportEvent(prefix)));
      }
      finish(ids, ids.length > 1 ? t(`${ids.length} transcriptions prêtes`, `${ids.length} transcripts ready`) : t("Transcription prête", "Transcript ready"));
    });

  const AsrMissing = () => {
    const d = asrModel ? downloads[asrModel.id] : undefined;
    return (
      <div className="file-item" style={{ background: "var(--light-veil)", alignItems: "flex-start" }}>
        <Icon name="cpu" size={18} />
        <div className="grow" style={{ whiteSpace: "normal" }}>
          <strong>{t("Un modèle de transcription est nécessaire", "A transcription model is needed")}</strong>
          <div className="import-hint">
            {asrModel ? t(`${asrModel.name}, ${formatBytes(asrModel.size)}, téléchargé une seule fois.`, `${asrModel.name}, ${formatBytes(asrModel.size)}, downloaded only once.`) : ""}
          </div>
          {d && (
            <div className="bar live" style={{ marginTop: 8 }}>
              <i style={{ width: `${(d.received / d.total) * 100}%` }} />
            </div>
          )}
        </div>
        {asrModel && !d && (
          <button className="btn sm accent" onClick={() => download(asrModel.id)}>
            {t("Télécharger", "Download")}
          </button>
        )}
      </div>
    );
  };

  let footer: React.ReactNode = null;
  if (!busy) {
    if (tab === "text")
      footer = (
        <button className="btn primary" disabled={wordCount(text) < 3} onClick={createText}>
          {t("Créer la leçon", "Create the lesson")}
        </button>
      );
    if (tab === "link" && !preview && !found)
      footer = (
        <button className="btn primary" disabled={url.trim().length < 4} onClick={() => probeLink()}>
          {t("Ouvrir le lien", "Open the link")}
        </button>
      );
    if (tab === "link" && !preview && found) {
      const n = found.info.list ? found.picked.length : 1;
      const wantsMedia = found.choice !== "article";
      footer = (
        <>
          <button className="btn ghost" onClick={() => setFound(null)}>
            {t("Retour", "Back")}
          </button>
          <button className="btn primary" disabled={wantsMedia && (!asrReady || n === 0)} onClick={importFound}>
            {!wantsMedia
              ? t("Voir le texte", "See the text")
              : found.info.list
                ? `${t("Importer", "Import")} ${itemsLabel(found.info, n)}`
                : found.info.media[0].video
                  ? t("Importer la vidéo", "Import the video")
                  : t("Importer le son", "Import the sound")}
          </button>
        </>
      );
    }
    if ((tab === "link" || tab === "file") && preview)
      footer = (
        <>
          <button className="btn ghost" onClick={() => setPreview(null)}>
            {t("Retour", "Back")}
          </button>
          <button className="btn primary" onClick={() => run(t("Création", "Creating"), async () => finish(await create([preview]), t("Leçon créée", "Lesson created")))}>
            {t("Créer la leçon", "Create the lesson")}
          </button>
        </>
      );
    if (tab === "file" && book)
      footer = (
        <>
          <span className="muted" style={{ marginRight: "auto" }}>
            {t(`${book.chapters.filter((c) => c.on).length} chapitres sélectionnés`, `${book.chapters.filter((c) => c.on).length} chapters selected`)}
          </span>
          <button className="btn primary" disabled={!book.chapters.some((c) => c.on)} onClick={createBook}>
            {t("Ajouter à la bibliothèque", "Add to the library")}
          </button>
        </>
      );
    if (tab === "podcast" && podcast.hasKey)
      footer = (
        <button
          className="btn primary"
          disabled={!podcast.ready}
          onClick={() => {
            podcast.create();
            close();
            toast(t("Gemini écrit votre podcast. Il vous attendra dans la bibliothèque.", "Gemini is writing your podcast. It will be waiting in your library."), "light");
          }}
        >
          <Icon name="sparkle" size={15} /> {t("Créer le podcast", "Create the podcast")}
        </button>
      );
    if (tab === "media")
      footer = (
        <button className="btn primary" disabled={!mediaFiles.length || !asrReady} onClick={transcribeAll}>
          {mediaFiles.length > 1 ? t(`Transcrire ${mediaFiles.length} fichiers`, `Transcribe ${mediaFiles.length} files`) : t("Transcrire", "Transcribe")}
        </button>
      );
  }

  return (
    <Sheet open={open} onClose={() => !busy && close()} title={t(`Importer ${inLang(lang)}`, `Import ${inLang(lang)}`)} footer={footer}>
      {!busy && (
        <div className="import-tabs" role="tablist">
          {tabs().map((tb) => (
            <button
              key={tb.id}
              role="tab"
              aria-selected={tab === tb.id}
              className={`import-tab ${tab === tb.id ? "on" : ""}`}
              onClick={() => {
                setTab(tb.id);
                setPreview(null);
                setBook(null);
                setFound(null);
                setError(null);
              }}
            >
              <Icon name={tb.icon} size={20} />
              <div>
                {tb.label}
                <br />
                <span>{tb.hint}</span>
              </div>
            </button>
          ))}
        </div>
      )}

      <AnimatePresence mode="wait">
        {busy ? (
          <motion.div key="busy" className="working" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
            <div className="beam">
              <span className="orb" style={{ width: 46, height: 46, ["--s" as string]: "46px" }} />
            </div>
            <h3>{busy}…</h3>
            {progress !== null && (
              <>
                <div className="bar live">
                  <i style={{ width: `${progress}%` }} />
                </div>
                <span className="muted num">{t(`${Math.round(progress)} %`, `${Math.round(progress)}%`)}</span>
              </>
            )}
            <span className="import-hint">{t("Tout se passe sur votre Mac. Rien n'est envoyé en ligne.", "Everything happens on your Mac. Nothing is sent online.")}</span>
          </motion.div>
        ) : (
          <motion.div key={tab + (preview ? "p" : "") + (book ? "b" : "") + (found ? "f" : "")} className="import-pane" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
            {error && (
              <div className="file-item" style={{ background: "color-mix(in srgb, var(--danger) 12%, transparent)", color: "var(--danger)" }}>
                <Icon name="ban" size={16} /> <span className="grow" style={{ whiteSpace: "normal" }}>{error}</span>
              </div>
            )}

            {preview && (
              <>
                <div className="field">
                  <label htmlFor="pv-title">{t("Titre", "Title")}</label>
                  <input id="pv-title" className="input" value={preview.title} onChange={(e) => setPreview({ ...preview, title: e.target.value })} />
                </div>
                <div className="file-item">
                  <Icon name="text" size={16} />
                  <span className="grow">
                    {count(wordCount(preview.text), "mot", "mots", "word", "words")}
                    {wordCount(preview.text) > 2600 ? t(` · découpé en ${splitLong(preview.text).length} leçons`, ` · split into ${splitLong(preview.text).length} lessons`) : ""}
                  </span>
                </div>
                <div className="textarea" style={{ maxHeight: 260, overflow: "auto", whiteSpace: "pre-wrap", minHeight: 0 }}>
                  {preview.text.slice(0, 1600)}
                  {preview.text.length > 1600 ? "…" : ""}
                </div>
              </>
            )}

            {!preview && tab === "text" && (
              <>
                <div className="field">
                  <label htmlFor="imp-title">{t("Titre (facultatif)", "Title (optional)")}</label>
                  <input id="imp-title" className="input" value={title} onChange={(e) => setTitle(e.target.value)} placeholder={t("Ex. : Chapitre 1", "E.g. Chapter 1")} />
                </div>
                <div className="field">
                  <label htmlFor="imp-text">{t(`Texte ${inLang(lang)}`, `Text ${inLang(lang)}`)}</label>
                  <textarea
                    id="imp-text"
                    className="textarea"
                    value={text}
                    onChange={(e) => setText(e.target.value)}
                    placeholder={t("Collez ici un article, un chapitre, des paroles…", "Paste an article, a chapter, song lyrics… here")}
                    autoFocus
                  />
                </div>
                {LONE_LINK.test(text.trim()) ? (
                  <div className="file-item link-pasted">
                    <Icon name="link" size={16} />
                    <span className="grow">{t("Vous avez collé un lien.", "You pasted a link.")}</span>
                    <button
                      className="btn sm accent"
                      onClick={() => {
                        setUrl(text.trim());
                        setText("");
                        setTab("link");
                      }}
                    >
                      {t("L'ouvrir comme lien", "Open it as a link")}
                    </button>
                  </div>
                ) : (
                  <span className="import-hint num">{count(wordCount(text), "mot", "mots", "word", "words")}</span>
                )}
              </>
            )}

            {!preview && tab === "link" && !found && (
              <>
                <div className="field">
                  <label htmlFor="imp-url">{t("Adresse", "Address")}</label>
                  <input
                    id="imp-url"
                    className="input"
                    value={url}
                    onChange={(e) => setUrl(e.target.value)}
                    placeholder="https://…"
                    onKeyDown={(e) => e.key === "Enter" && url.trim().length >= 4 && probeLink()}
                    autoFocus
                  />
                </div>
                <div className="link-sources" aria-hidden="true">
                  {(
                    [
                      ["globe", t("Articles et blogs", "Articles and blogs")],
                      ["youtube", t("YouTube et mille sites vidéo", "YouTube and a thousand video sites")],
                      ["podcast", t("Podcasts et radios", "Podcasts and radio shows")],
                      ["music", "Spotify"],
                    ] as [IconName, string][]
                  ).map(([icon, label]) => (
                    <span key={icon} className="link-source">
                      <Icon name={icon} size={14} /> {label}
                    </span>
                  ))}
                </div>
                <span className="import-hint">
                  {t(
                    "Collez n'importe quel lien : Lumen trouve l'article, la vidéo ou les épisodes qui s'y cachent, et vous choisissez quoi importer. Le son est transcrit mot à mot sur votre Mac.",
                    "Paste any link: Lumen finds the article, video or episodes behind it, and you choose what to import. The sound is transcribed word by word on your Mac.",
                  )}
                  {info && !info.ytdlp ? t(" Pour les vidéos, Lumen installe la première fois ses composants (environ 40 Mo).", " For videos, Lumen installs its components the first time (about 40 MB).") : ""}
                </span>
              </>
            )}

            {!preview && tab === "link" && found && <LinkFound found={found} onChange={setFound} asrMissing={!asrReady && found.choice !== "article" ? <AsrMissing /> : null} />}

            {!preview && tab === "file" && !book && (
              <button className="file-drop" onClick={chooseDocs} disabled={!isTauri}>
                <Icon name="file" size={28} />
                <strong style={{ color: "var(--text)" }}>{t("Choisir des fichiers", "Choose files")}</strong>
                <span>{t("EPUB (un chapitre par leçon), PDF, TXT, Markdown, sous-titres SRT ou VTT, pages HTML", "EPUB (one chapter per lesson), PDF, TXT, Markdown, SRT or VTT subtitles, HTML pages")}</span>
                <span className="import-hint">{t("Vous pouvez aussi les déposer n'importe où dans la fenêtre.", "You can also drop them anywhere in the window.")}</span>
              </button>
            )}

            {tab === "file" && book && (
              <>
                <div className="field">
                  <label htmlFor="book-title">{t("Livre", "Book")}</label>
                  <input id="book-title" className="input" value={book.title} onChange={(e) => setBook({ ...book, title: e.target.value })} />
                </div>
                <div className="chapter-list">
                  {book.chapters.map((c, i) => (
                    <label key={i} className="chapter-item">
                      <input
                        type="checkbox"
                        checked={c.on}
                        onChange={(e) => setBook({ ...book, chapters: book.chapters.map((x, j) => (j === i ? { ...x, on: e.target.checked } : x)) })}
                      />
                      <span style={{ flex: 1 }}>{c.title}</span>
                      <span className="muted num">{count(c.words, "mot", "mots", "word", "words")}</span>
                    </label>
                  ))}
                </div>
              </>
            )}

            {tab === "podcast" && <PodcastTab form={podcast} />}

            {tab === "media" && (
              <>
                {!asrReady && <AsrMissing />}
                <button className="file-drop" onClick={chooseMedia} disabled={!isTauri}>
                  <Icon name="wave" size={28} />
                  <strong style={{ color: "var(--text)" }}>{t("Choisir des fichiers audio ou vidéo", "Choose audio or video files")}</strong>
                  <span>{t("MP3, M4A, AAC, WAV, FLAC, OGG, MP4, MOV… Transcription mot à mot, synchronisée avec le son.", "MP3, M4A, AAC, WAV, FLAC, OGG, MP4, MOV… Word-by-word transcript, in sync with the sound.")}</span>
                </button>
                {mediaFiles.length > 0 && (
                  <div className="file-list">
                    {mediaFiles.map((f) => (
                      <div key={f} className="file-item">
                        <Icon name={["mp4", "mov", "m4v", "mkv"].includes(extOf(f)) ? "video" : "wave"} size={16} />
                        <span className="grow">{baseName(f)}</span>
                        <button className="icon-btn" aria-label={t("Retirer", "Remove")} onClick={() => setMediaFiles((m) => m.filter((x) => x !== f))}>
                          <Icon name="close" size={14} />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </>
            )}

          </motion.div>
        )}
      </AnimatePresence>
    </Sheet>
  );
}

/** Ce qu'un lien contient, et le choix de ce qu'on en importe. */
function LinkFound({ found, onChange, asrMissing }: { found: Found; onChange(f: Found): void; asrMissing: React.ReactNode }) {
  const { info, article, choice, picked } = found;
  const first = info.media[0];
  const n = info.media.length;
  const video = info.list ? info.media.every((m) => m.video) : first.video;

  const facts: string[] = [];
  if (info.list) facts.push(itemsLabel(info, n));
  else {
    facts.push(info.via === "youtube" ? t("Morceau", "Song") : video ? t("Vidéo", "Video") : first.collection ? t("Épisode", "Episode") : t("Son", "Audio"));
    if (first.duration > 0) facts.push(formatDuration(first.duration));
    if (first.date) facts.push(shortDate(first.date));
  }
  if (article) facts.push(t(`article de ${count(article.words, "mot", "mots", "word", "words")}`, `${formatNumber(article.words)}-word article`));

  const options: { id: Choice; icon: IconName; label: string; hint: string }[] = [];
  if (article) {
    if (!info.list)
      options.push({
        id: "both",
        icon: "sparkle",
        label: video ? t("La vidéo, avec le texte de la page", "The video, with the page's text") : t("Le son, avec le texte de la page", "The sound, with the page's text"),
        hint: t(
          "Le texte de la page devient la leçon et la lanterne suit la voix. S'il ne correspond pas à ce qu'on entend, Lumen transcrit le son.",
          "The page's text becomes the lesson and the lantern follows the voice. If it doesn't match what is said, Lumen transcribes the sound.",
        ),
      });
    options.push({
      id: "media",
      icon: video ? "video" : "wave",
      label: info.list
        ? t(`Les ${itemsLabel(info, n)} de la page`, `The page's ${itemsLabel(info, n)}`)
        : video
          ? t("La vidéo, transcrite", "The video, transcribed")
          : t("Le son, transcrit", "The sound, transcribed"),
      hint: t("Transcription mot à mot sur votre Mac, synchronisée avec le son.", "Word-by-word transcript on your Mac, in sync with the sound."),
    });
    options.push({
      id: "article",
      icon: "text",
      label: t("Le texte de l'article", "The article's text"),
      hint: `${count(article.words, "mot", "mots", "word", "words")}${t(", sans le son", ", without the sound")}`,
    });
  }

  const title = info.title || article?.title || first.title;
  // le nom du site seulement s'il apporte quelque chose (un podcast porte souvent le nom de son émission)
  const site = info.site && info.site.toLowerCase() !== title.toLowerCase() ? info.site : "";
  const all = picked.length === n;
  const toggle = (i: number, on: boolean) =>
    onChange({ ...found, picked: on ? [...picked, i].sort((a, b) => a - b) : picked.filter((x) => x !== i) });

  return (
    <>
      <div className="link-card">
        <LinkThumb src={info.image || first.image} icon={info.via === "youtube" ? "music" : info.list && !video ? "podcast" : video ? "video" : article ? "globe" : "wave"} />
        <div className="link-meta">
          {site && <span className="link-site">{site}</span>}
          <strong className="link-title">{title}</strong>
          <span className="link-facts num">{facts.join(" · ")}</span>
        </div>
      </div>

      {info.via && (
        <p className="link-note">
          <Icon name="sparkle" size={15} />
          <span>
            {info.via === "rss"
              ? info.list
                ? t("Spotify protège ses fichiers : Lumen a retrouvé ce podcast dans son flux public.", "Spotify protects its files: Lumen found this podcast in its public feed.")
                : t("Spotify protège ses fichiers : Lumen a retrouvé cet épisode dans le flux public du podcast.", "Spotify protects its files: Lumen found this episode in the podcast's public feed.")
              : t("Spotify protège ses fichiers : Lumen prend le son du même morceau sur YouTube.", "Spotify protects its files: Lumen takes the sound of the same song from YouTube.")}
          </span>
        </p>
      )}

      {options.length > 0 && (
        <div className="link-choices" role="radiogroup" aria-label={t("Quoi importer", "What to import")}>
          {options.map((o) => (
            <button
              key={o.id}
              type="button"
              role="radio"
              aria-checked={choice === o.id}
              className={`link-choice ${choice === o.id ? "on" : ""}`}
              onClick={() => onChange({ ...found, choice: o.id })}
            >
              <Icon name={o.icon} size={18} />
              <span className="link-choice-text">
                <strong>{o.label}</strong>
                <span>{o.hint}</span>
              </span>
              <span className="link-radio" aria-hidden="true" />
            </button>
          ))}
        </div>
      )}

      {info.list && choice !== "article" && (
        <>
          <div className="link-list-head">
            <span className="muted num">{t(`${picked.length} sur ${n}`, `${picked.length} of ${n}`)}</span>
            <button className="btn ghost sm" onClick={() => onChange({ ...found, picked: all ? [] : info.media.map((_, i) => i) })}>
              {all ? t("Tout décocher", "Uncheck all") : t("Tout cocher", "Check all")}
            </button>
          </div>
          <div className="chapter-list link-list">
            {info.media.map((m, i) => (
              <label key={`${i}-${m.url}`} className="chapter-item">
                <input type="checkbox" checked={picked.includes(i)} onChange={(e) => toggle(i, e.target.checked)} />
                <span className="link-item-title">{m.title || t("Sans titre", "Untitled")}</span>
                {m.date && <span className="muted num link-item-meta">{shortDate(m.date)}</span>}
                {m.duration > 0 && <span className="muted num link-item-meta">{formatDuration(m.duration)}</span>}
              </label>
            ))}
          </div>
        </>
      )}

      {asrMissing}
    </>
  );
}
