// Leçons en préparation. Ce que l'apprenant a regardé, écouté ou lu dans
// Découvrir (ou trouvé par la recherche) devient une leçon en arrière-plan,
// l'une après l'autre : la transcription occupe la puce. On continue
// d'explorer pendant ce temps ; une notification ouvre la leçon prête.
import { create } from "zustand";
import { api, errorText } from "./api";
import { useDiscover } from "./discover";
import { count, t } from "./i18n";
import { extractArticle, splitLong, wordCount } from "./importers";
import { useApp } from "./store";
import type { ImportEvent, LangCode, LinkMedia, PodcastRequest, SongItem } from "./types";

export type JobStatus = "waiting" | "model" | "running" | "done" | "error";

/** Ce qu'il faut pour préparer une leçon. */
export interface JobSpec {
  /** identifiant : l'adresse de ce qu'on importe (deux demandes ne font qu'une leçon) */
  key: string;
  lang: LangCode;
  title: string;
  image: string;
  /** vidéo ou son à transcrire ; `text` : texte de la page, gardé si le son le suit */
  media?: LinkMedia;
  text?: string | null;
  /** page dont lire le texte d'abord (épisode publié avec son texte : DW, RFI…) */
  textFrom?: string;
  /** chanson : paroles pour texte */
  song?: SongItem;
  /** article : la leçon se crée aussitôt */
  article?: { title: string; text: string; source: string; collection: string };
  /** article à lire d'abord sur sa page */
  articleFrom?: string;
  /** élément de Découvrir à relier à la leçon */
  discoverId?: string;
  /** podcast sur mesure : Gemini l'écrit et le dit */
  podcast?: PodcastRequest;
}

export interface Job {
  spec: JobSpec;
  status: JobStatus;
  /** étape en cours (voir `stageText`) et avancement (0 à 100, null : indéterminé) */
  stage: string;
  progress: number | null;
  error?: string;
  lessonId?: number;
  /** leçons créées (un long article en fait plusieurs) */
  lessons?: number;
}

/** Étapes d'un import, dites simplement. */
export function stageText(stage: string): string {
  const map: Record<string, string> = {
    probe: t("Lecture du lien", "Reading the link"),
    search: t("Recherche", "Searching"),
    page: t("Lecture de la page", "Reading the page"),
    lyrics: t("Recherche des paroles", "Finding the lyrics"),
    copy: t("Copie du fichier", "Copying the file"),
    tools: t("Installation des composants vidéo", "Installing the video components"),
    download: t("Téléchargement du son", "Downloading the sound"),
    file: t("Téléchargement du fichier", "Downloading the file"),
    video: t("Finalisation de la vidéo", "Finishing the video"),
    lighten: t("Allègement de la vidéo", "Making the video lighter"),
    decode: t("Lecture du son", "Reading the sound"),
    model: t("Préparation du modèle", "Preparing the model"),
    transcribe: "Transcription",
    timing: t("Repérage des mots", "Locating the words"),
    text: t("Écriture du texte", "Writing the text"),
    lesson: t("Création de la leçon", "Creating the lesson"),
    script: t("Gemini écrit le podcast", "Gemini is writing the podcast"),
    studio: t("Enregistrement des voix", "Recording the voices"),
  };
  return map[stage] ?? (stage ? stage : t("En attente", "Waiting"));
}

/** Étapes dont l'avancement se mesure dès leur début (les autres restent indéterminées). */
export const MEASURED = ["transcribe", "timing", "text", "download", "file", "tools", "studio"];

/** Un modèle de transcription est-il sur ce Mac ? */
function transcriber(): boolean {
  return useApp.getState().models.some((m) => m.kind === "asr" && m.installed);
}

/** Le son ou la vidéo se transcrit (une chanson garde ses paroles, un article son texte). */
function needsTranscriber(spec: JobSpec): boolean {
  return !!spec.media;
}

interface ImportStore {
  jobs: Job[];
  enqueue(spec: JobSpec): void;
  retry(key: string): void;
  dismiss(key: string): void;
}

export const useImports = create<ImportStore>((set, get) => ({
  jobs: [],

  enqueue(spec) {
    const known = get().jobs.find((j) => j.spec.key === spec.key);
    if (known && known.status !== "error") {
      if (known.status === "done" && known.lessonId) useApp.getState().openLesson(known.lessonId);
      return;
    }
    const job: Job = { spec, status: "waiting", stage: "", progress: null };
    set((s) => ({ jobs: [...s.jobs.filter((j) => j.spec.key !== spec.key), job] }));
    void pump();
  },

  retry(key) {
    const job = get().jobs.find((j) => j.spec.key === key);
    if (job) get().enqueue(job.spec);
  },

  dismiss(key) {
    set((s) => ({ jobs: s.jobs.filter((j) => j.spec.key !== key || j.status === "running") }));
  },
}));

function patch(key: string, p: Partial<Job>) {
  useImports.setState((s) => ({ jobs: s.jobs.map((j) => (j.spec.key === key ? { ...j, ...p } : j)) }));
}

/** L'état d'une leçon en préparation (ou rien). */
export function useJob(key: string | undefined): Job | undefined {
  return useImports((s) => (key ? s.jobs.find((j) => j.spec.key === key) : undefined));
}

/** Le texte publié sur la page d'un épisode (à défaut, rien : le son sera transcrit). */
async function pageText(url: string): Promise<string | null> {
  try {
    const info = await api().linkProbe(url, () => {});
    if (!info.html) return null;
    const a = extractArticle(info.html, info.url);
    return wordCount(a.text) >= 40 ? a.text : null;
  } catch {
    return null;
  }
}

async function run(job: Job): Promise<number[]> {
  const { spec } = job;
  const onEvent = (e: ImportEvent) =>
    e.type === "stage"
      ? patch(spec.key, { stage: e.stage, progress: MEASURED.includes(e.stage) ? 0 : null })
      : patch(spec.key, { progress: e.value });
  let article = spec.article;
  if (!article && spec.articleFrom) {
    patch(spec.key, { stage: "page" });
    const info = await api().linkProbe(spec.articleFrom, () => {});
    if (!info.html) throw new Error(t("Aucun article lisible n'a été trouvé sur cette page.", "No readable article was found on this page."));
    const a = extractArticle(info.html, info.url);
    article = { title: spec.title || a.title, text: a.text, source: info.url, collection: "" };
  }
  if (article) {
    patch(spec.key, { stage: "lesson" });
    const { title, text, source, collection } = article;
    const parts = splitLong(text);
    const ids: number[] = [];
    for (let i = 0; i < parts.length; i++) {
      ids.push(
        await api().lessonCreate({
          lang: spec.lang,
          title: parts.length > 1 ? `${title} · ${i + 1}/${parts.length}` : title,
          text: parts[i],
          kind: "web",
          source,
          collection: collection || (parts.length > 1 ? title : ""),
        }),
      );
    }
    return ids;
  }
  if (spec.podcast) return [await api().podcastCreate(spec.lang, spec.podcast, onEvent)];
  if (spec.song) return [await api().importSong(spec.lang, spec.song, onEvent)];
  if (spec.media) {
    let text = spec.text ?? null;
    if (text === null && spec.textFrom) {
      patch(spec.key, { stage: "page" });
      text = await pageText(spec.textFrom);
    }
    return [await api().importLink(spec.lang, spec.media, text, onEvent)];
  }
  throw new Error(t("Rien à importer.", "Nothing to import."));
}

let pumping = false;

/** Prépare les leçons demandées, l'une après l'autre. */
async function pump() {
  if (pumping) return;
  pumping = true;
  try {
    for (;;) {
      const jobs = useImports.getState().jobs;
      // sans modèle de transcription, un son attend son arrivée ; les autres passent devant
      for (const j of jobs) {
        if (j.status === "waiting" && needsTranscriber(j.spec) && !transcriber()) patch(j.spec.key, { status: "model" });
        if (j.status === "model" && transcriber()) patch(j.spec.key, { status: "waiting" });
      }
      const job = useImports.getState().jobs.find((j) => j.status === "waiting");
      if (!job) break;
      patch(job.spec.key, { status: "running", stage: "", progress: null, error: undefined });
      const app = useApp.getState();
      try {
        const ids = await run(job);
        patch(job.spec.key, { status: "done", lessonId: ids[0], lessons: ids.length, stage: "", progress: null });
        if (job.spec.discoverId && ids.length) void useDiscover.getState().mark(job.spec.lang, job.spec.discoverId, ids[0]);
        app.bumpLibrary();
        void app.refreshKnown();
        const title = job.spec.title;
        app.toast(
          job.spec.podcast
            ? t(`Votre podcast « ${title} » est prêt`, `Your podcast “${title}” is ready`)
            : ids.length > 1
            ? t(`« ${title} » est prête (${count(ids.length, "leçon", "leçons", "", "")})`, `“${title}” is ready (${count(ids.length, "", "", "lesson", "lessons")})`)
            : t(`« ${title} » est prête`, `“${title}” is ready`),
          "light",
          { label: t("Ouvrir", "Open"), run: () => useApp.getState().openLesson(ids[0]) },
        );
      } catch (e) {
        const msg = errorText(e);
        patch(job.spec.key, { status: "error", error: msg, stage: "", progress: null });
        app.toast(t(`« ${job.spec.title} » : ${msg}`, `“${job.spec.title}”: ${msg}`), "error");
      }
    }
  } finally {
    pumping = false;
  }
}

// un modèle de transcription vient d'arriver : les sons qui l'attendaient se préparent
useApp.subscribe((s, prev) => {
  if (s.models !== prev.models && useImports.getState().jobs.some((j) => j.status === "model")) void pump();
});
