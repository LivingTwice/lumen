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
import { langInfo } from "../lib/langs";
import { formatBytes, formatNumber, useApp } from "../lib/store";
import type { ImportEvent, NewLesson } from "../lib/types";

type Tab = "text" | "web" | "file" | "media" | "youtube";

const TABS: { id: Tab; label: string; hint: string; icon: IconName }[] = [
  { id: "text", label: "Texte", hint: "Coller", icon: "text" },
  { id: "web", label: "Page web", hint: "Article, blog", icon: "globe" },
  { id: "file", label: "Fichier", hint: "EPUB, PDF, SRT…", icon: "file" },
  { id: "media", label: "Audio, vidéo", hint: "Transcription", icon: "wave" },
  { id: "youtube", label: "YouTube", hint: "Lien vidéo", icon: "youtube" },
];

const STAGES: Record<string, string> = {
  copy: "Copie du fichier",
  tools: "Installation des composants vidéo",
  download: "Téléchargement du son",
  video: "Finalisation de la vidéo",
  decode: "Lecture du son",
  model: "Préparation du modèle",
  transcribe: "Transcription",
  // avec Qwen3-ASR : Whisper repère les mots, puis Qwen3-ASR écrit le texte
  timing: "Repérage des mots",
  text: "Écriture du texte",
};

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

  const li = langInfo(lang);
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
    run("Création de la leçon", async () => {
      const t = cleanText(text);
      const ids = await create([{ title: title.trim() || t.split(/\s+/).slice(0, 6).join(" "), text: t, kind: "text", source: "" }]);
      finish(ids, "Leçon créée");
    });

  // ---------- web ----------
  const fetchWeb = () =>
    run("Lecture de la page", async () => {
      let u = url.trim();
      if (!/^https?:\/\//.test(u)) u = "https://" + u;
      const html = await api().fetchUrl(u);
      const art = extractArticle(html, u);
      setPreview({ title: art.title, text: art.text, kind: "web", source: u });
    });

  // ---------- fichiers ----------
  async function loadDocs(paths: string[]) {
    await run("Lecture du fichier", async () => {
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
          pend.push({ title: r.title === "Document PDF" ? baseName(p) : r.title, text: r.text, kind: "pdf", source: p });
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
        finish(ids, `${ids.length} leçons créées`);
      }
    });
  }

  const chooseDocs = async () => {
    const paths = await pickFiles([{ name: "Textes et livres", extensions: ["epub", "pdf", "txt", "md", "srt", "vtt", "html", "htm"] }]);
    if (paths.length) await loadDocs(paths);
  };

  const createBook = () =>
    run("Création des chapitres", async () => {
      if (!book) return;
      const chosen = book.chapters.filter((c) => c.on);
      const ids = await create(chosen.map((c) => ({ title: c.title, text: c.text, kind: "book", source: "", collection: book.title })));
      finish(ids, `${chosen.length} chapitre${chosen.length > 1 ? "s" : ""} ajouté${chosen.length > 1 ? "s" : ""}`);
    });

  // ---------- audio et vidéo ----------
  const chooseMedia = async () => {
    const paths = await pickFiles([{ name: "Audio et vidéo", extensions: MEDIA_EXT }]);
    if (paths.length) setMediaFiles((m) => [...m, ...paths.filter((p) => !m.includes(p))]);
  };

  const onImportEvent = (stagePrefix: string) => (e: ImportEvent) => {
    if (e.type === "stage") {
      setBusy(`${stagePrefix}${STAGES[e.stage] ?? e.stage}`);
      setProgress(["transcribe", "timing", "text", "download", "tools"].includes(e.stage) ? 0 : null);
    } else setProgress(e.value);
  };

  const transcribeAll = () =>
    run("Préparation", async () => {
      const ids: number[] = [];
      for (let i = 0; i < mediaFiles.length; i++) {
        const prefix = mediaFiles.length > 1 ? `${i + 1}/${mediaFiles.length} · ` : "";
        ids.push(await api().importMedia(lang, mediaFiles[i], null, onImportEvent(prefix)));
      }
      finish(ids, ids.length > 1 ? `${ids.length} transcriptions prêtes` : "Transcription prête");
    });

  const importYt = () =>
    run("Téléchargement", async () => {
      const id = await api().importYoutube(lang, url.trim(), onImportEvent(""));
      finish([id], "Vidéo transcrite");
    });

  const AsrMissing = () => {
    const d = asrModel ? downloads[asrModel.id] : undefined;
    return (
      <div className="file-item" style={{ background: "var(--light-veil)", alignItems: "flex-start" }}>
        <Icon name="cpu" size={18} />
        <div className="grow" style={{ whiteSpace: "normal" }}>
          <strong>Un modèle de transcription est nécessaire</strong>
          <div className="import-hint">
            {asrModel ? `${asrModel.name}, ${formatBytes(asrModel.size)}, téléchargé une seule fois.` : ""}
          </div>
          {d && (
            <div className="bar live" style={{ marginTop: 8 }}>
              <i style={{ width: `${(d.received / d.total) * 100}%` }} />
            </div>
          )}
        </div>
        {asrModel && !d && (
          <button className="btn sm accent" onClick={() => download(asrModel.id)}>
            Télécharger
          </button>
        )}
      </div>
    );
  };

  let footer: React.ReactNode = null;
  if (!busy) {
    if (tab === "text") footer = <button className="btn primary" disabled={wordCount(text) < 3} onClick={createText}>Créer la leçon</button>;
    if (tab === "web" && !preview) footer = <button className="btn primary" disabled={url.trim().length < 4} onClick={fetchWeb}>Récupérer l'article</button>;
    if ((tab === "web" || tab === "file") && preview)
      footer = (
        <>
          <button className="btn ghost" onClick={() => setPreview(null)}>Retour</button>
          <button className="btn primary" onClick={() => run("Création", async () => finish(await create([preview]), "Leçon créée"))}>Créer la leçon</button>
        </>
      );
    if (tab === "file" && book)
      footer = (
        <>
          <span className="muted" style={{ marginRight: "auto" }}>
            {book.chapters.filter((c) => c.on).length} chapitres sélectionnés
          </span>
          <button className="btn primary" disabled={!book.chapters.some((c) => c.on)} onClick={createBook}>
            Ajouter à la bibliothèque
          </button>
        </>
      );
    if (tab === "media") footer = <button className="btn primary" disabled={!mediaFiles.length || !asrReady} onClick={transcribeAll}>Transcrire {mediaFiles.length > 1 ? `${mediaFiles.length} fichiers` : ""}</button>;
    if (tab === "youtube") footer = <button className="btn primary" disabled={!url.trim() || !asrReady} onClick={importYt}>Importer la vidéo</button>;
  }

  return (
    <Sheet open={open} onClose={() => !busy && close()} title={`Importer en ${li.name.toLowerCase()}`} footer={footer}>
      {!busy && (
        <div className="import-tabs" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              className={`import-tab ${tab === t.id ? "on" : ""}`}
              onClick={() => {
                setTab(t.id);
                setPreview(null);
                setBook(null);
                setError(null);
              }}
            >
              <Icon name={t.icon} size={20} />
              <div>
                {t.label}
                <br />
                <span>{t.hint}</span>
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
                <span className="muted num">{Math.round(progress)} %</span>
              </>
            )}
            <span className="import-hint">Tout se passe sur votre Mac. Rien n'est envoyé en ligne.</span>
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
                  <label htmlFor="pv-title">Titre</label>
                  <input id="pv-title" className="input" value={preview.title} onChange={(e) => setPreview({ ...preview, title: e.target.value })} />
                </div>
                <div className="file-item">
                  <Icon name="text" size={16} />
                  <span className="grow">
                    {formatNumber(wordCount(preview.text))} mots
                    {wordCount(preview.text) > 2600 ? ` · découpé en ${splitLong(preview.text).length} leçons` : ""}
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
                  <label htmlFor="imp-title">Titre (facultatif)</label>
                  <input id="imp-title" className="input" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Ex. : Chapitre 1" />
                </div>
                <div className="field">
                  <label htmlFor="imp-text">Texte en {li.name.toLowerCase()}</label>
                  <textarea id="imp-text" className="textarea" value={text} onChange={(e) => setText(e.target.value)} placeholder="Collez ici un article, un chapitre, des paroles…" autoFocus />
                </div>
                <span className="import-hint num">{formatNumber(wordCount(text))} mots</span>
              </>
            )}

            {!preview && tab === "web" && (
              <>
                <div className="field">
                  <label htmlFor="imp-url">Adresse de la page</label>
                  <input id="imp-url" className="input" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://…" onKeyDown={(e) => e.key === "Enter" && url.trim() && fetchWeb()} autoFocus />
                </div>
                <span className="import-hint">Lumen garde uniquement le texte de l'article, sans menus ni publicités.</span>
              </>
            )}

            {!preview && tab === "file" && !book && (
              <button className="file-drop" onClick={chooseDocs} disabled={!isTauri}>
                <Icon name="file" size={28} />
                <strong style={{ color: "var(--text)" }}>Choisir des fichiers</strong>
                <span>EPUB (un chapitre par leçon), PDF, TXT, Markdown, sous-titres SRT ou VTT, pages HTML</span>
                <span className="import-hint">Vous pouvez aussi les déposer n'importe où dans la fenêtre.</span>
              </button>
            )}

            {tab === "file" && book && (
              <>
                <div className="field">
                  <label htmlFor="book-title">Livre</label>
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
                      <span className="muted num">{formatNumber(c.words)} mots</span>
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
                  <strong style={{ color: "var(--text)" }}>Choisir des fichiers audio ou vidéo</strong>
                  <span>MP3, M4A, AAC, WAV, FLAC, OGG, MP4, MOV… Transcription mot à mot, synchronisée avec le son.</span>
                </button>
                {mediaFiles.length > 0 && (
                  <div className="file-list">
                    {mediaFiles.map((f) => (
                      <div key={f} className="file-item">
                        <Icon name={["mp4", "mov", "m4v", "mkv"].includes(extOf(f)) ? "video" : "wave"} size={16} />
                        <span className="grow">{baseName(f)}</span>
                        <button className="icon-btn" aria-label="Retirer" onClick={() => setMediaFiles((m) => m.filter((x) => x !== f))}>
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
                  <label htmlFor="imp-yt">Lien de la vidéo</label>
                  <input id="imp-yt" className="input" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://www.youtube.com/watch?v=…" />
                </div>
                <span className="import-hint">
                  YouTube et la plupart des sites vidéo. Le son est transcrit mot à mot sur votre Mac pendant que l'image se télécharge, pour regarder la vidéo avec la transcription synchronisée.
                  {info && !info.ytdlp ? " La première fois, Lumen installe ses composants vidéo (environ 40 Mo)." : ""}
                </span>
              </>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </Sheet>
  );
}
