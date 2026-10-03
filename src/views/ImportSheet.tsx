import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon, type IconName } from "../components/Icon";
import { Sheet } from "../components/ui";
import { api, errorText, isTauri } from "../lib/api";
import { pickFiles } from "../lib/dialogs";
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
  type Chapter,
} from "../lib/importers";
import { count, t } from "../lib/i18n";
import { inLang } from "../lib/langs";
import { formatBytes, useApp } from "../lib/store";
import type { ImportEvent, NewLesson } from "../lib/types";

type Tab = "text" | "web" | "file" | "media" | "youtube";

const tabs = (): { id: Tab; label: string; hint: string; icon: IconName }[] => [
  { id: "text", label: t("Texte", "Text"), hint: t("Coller", "Paste"), icon: "text" },
  { id: "web", label: t("Page web", "Web page"), hint: t("Article, blog", "Article, blog"), icon: "globe" },
  { id: "file", label: t("Fichier", "File"), hint: "EPUB, PDF, SRT…", icon: "file" },
  { id: "media", label: t("Audio, vidéo", "Audio, video"), hint: "Transcription", icon: "wave" },
  { id: "youtube", label: "YouTube", hint: t("Lien vidéo", "Video link"), icon: "youtube" },
];

const stages = (): Record<string, string> => ({
  copy: t("Copie du fichier", "Copying the file"),
  tools: t("Installation des composants vidéo", "Installing the video components"),
  download: t("Téléchargement du son", "Downloading the sound"),
  video: t("Finalisation de la vidéo", "Finishing the video"),
  decode: t("Lecture du son", "Reading the sound"),
  model: t("Préparation du modèle", "Preparing the model"),
  transcribe: "Transcription",
  // avec Qwen3-ASR : Whisper repère les mots, puis Qwen3-ASR écrit le texte
  timing: t("Repérage des mots", "Locating the words"),
  text: t("Écriture du texte", "Writing the text"),
});

function wordCount(t: string) {
  return t.split(/\s+/).filter(Boolean).length;
}

/** Coupe un long texte en parties d'environ `max` mots, aux paragraphes. */
function splitLong(text: string, max = 2600): string[] {
  const paras = text.split("\n\n");
  const parts: string[] = [];
  let cur: string[] = [];
  let n = 0;
  for (const p of paras) {
    const w = wordCount(p);
    if (n + w > max && cur.length) {
      parts.push(cur.join("\n\n"));
      cur = [];
      n = 0;
    }
    cur.push(p);
    n += w;
  }
  if (cur.length) parts.push(cur.join("\n\n"));
  return parts;
}

interface Pending {
  title: string;
  text: string;
  kind: string;
  source: string;
  collection?: string;
}

export function ImportSheet() {
  const open = useApp((s) => s.importOpen);
  const files = useApp((s) => s.importFiles);
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
  const [error, setError] = useState<string | null>(null);

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
    setError(null);
  };

  useEffect(() => {
    if (!open) return;
    reset();
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
  }, [open, files]);

  const finish = (ids: number[], label: string) => {
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

  // ---------- web ----------
  const fetchWeb = () =>
    run(t("Lecture de la page", "Reading the page"), async () => {
      let u = url.trim();
      if (!/^https?:\/\//.test(u)) u = "https://" + u;
      const html = await api().fetchUrl(u);
      const art = extractArticle(html, u);
      setPreview({ title: art.title, text: art.text, kind: "web", source: u });
    });

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
      setBusy(`${stagePrefix}${stages()[e.stage] ?? e.stage}`);
      setProgress(["transcribe", "timing", "text", "download", "tools"].includes(e.stage) ? 0 : null);
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

  const importYt = () =>
    run(t("Téléchargement", "Downloading"), async () => {
      const id = await api().importYoutube(lang, url.trim(), onImportEvent(""));
      finish([id], t("Vidéo transcrite", "Video transcribed"));
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
    if (tab === "web" && !preview)
      footer = (
        <button className="btn primary" disabled={url.trim().length < 4} onClick={fetchWeb}>
          {t("Récupérer l'article", "Get the article")}
        </button>
      );
    if ((tab === "web" || tab === "file") && preview)
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
    if (tab === "media")
      footer = (
        <button className="btn primary" disabled={!mediaFiles.length || !asrReady} onClick={transcribeAll}>
          {mediaFiles.length > 1 ? t(`Transcrire ${mediaFiles.length} fichiers`, `Transcribe ${mediaFiles.length} files`) : t("Transcrire", "Transcribe")}
        </button>
      );
    if (tab === "youtube")
      footer = (
        <button className="btn primary" disabled={!url.trim() || !asrReady} onClick={importYt}>
          {t("Importer la vidéo", "Import the video")}
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
          <motion.div key={tab + (preview ? "p" : "") + (book ? "b" : "")} className="import-pane" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
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
                <span className="import-hint num">{count(wordCount(text), "mot", "mots", "word", "words")}</span>
              </>
            )}

            {!preview && tab === "web" && (
              <>
                <div className="field">
                  <label htmlFor="imp-url">{t("Adresse de la page", "Page address")}</label>
                  <input id="imp-url" className="input" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://…" onKeyDown={(e) => e.key === "Enter" && url.trim() && fetchWeb()} autoFocus />
                </div>
                <span className="import-hint">{t("Lumen garde uniquement le texte de l'article, sans menus ni publicités.", "Lumen keeps only the text of the article, with no menus or ads.")}</span>
              </>
            )}

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

            {tab === "youtube" && (
              <>
                {!asrReady && <AsrMissing />}
                <div className="field">
                  <label htmlFor="imp-yt">{t("Lien de la vidéo", "Video link")}</label>
                  <input id="imp-yt" className="input" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://www.youtube.com/watch?v=…" />
                </div>
                <span className="import-hint">
                  {t(
                    "YouTube et la plupart des sites vidéo. Le son est transcrit mot à mot sur votre Mac pendant que l'image se télécharge, pour regarder la vidéo avec la transcription synchronisée.",
                    "YouTube and most video sites. The sound is transcribed word by word on your Mac while the picture downloads, so you can watch the video with the transcript in sync.",
                  )}
                  {info && !info.ytdlp ? t(" La première fois, Lumen installe ses composants vidéo (environ 40 Mo).", " The first time, Lumen installs its video components (about 40 MB).") : ""}
                </span>
              </>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </Sheet>
  );
}
