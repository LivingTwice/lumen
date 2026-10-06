import { AnimatePresence, motion } from "motion/react";
import { useEffect } from "react";
import { Icon } from "../components/Icon";
import { Segmented } from "../components/ui";
import { count, t } from "../lib/i18n";
import { isWindows } from "../lib/platform";
import { mediaQuality, useStorage, type MediaQuality } from "../lib/storage";
import { formatBytes, useApp } from "../lib/store";
import type { MediaUsage } from "../lib/types";

const enter = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: 6 }, transition: { type: "spring" as const, stiffness: 320, damping: 30 } };

function qualityOptions(): { value: MediaQuality; label: string }[] {
  // sous Windows, rien n'est réencodé : seule la définition des vidéos en ligne change
  if (isWindows)
    return [
      { value: "balanced", label: t("Haute définition", "High definition") },
      { value: "compact", label: t("Compacte", "Compact") },
    ];
  return [
    { value: "original", label: t("D'origine", "Original") },
    { value: "balanced", label: t("Allégée", "Lighter") },
    { value: "compact", label: t("Compacte", "Compact") },
  ];
}

function qualityNote(q: MediaQuality): string {
  if (isWindows)
    return q === "compact"
      ? t("Les vidéos en ligne arrivent en 720p : deux à trois fois moins de place. Un peu moins net en plein écran.", "Online videos arrive in 720p: two to three times less space. A little less sharp in full screen.")
      : t(
          "Les vidéos en ligne arrivent jusqu'en 1080p, en AV1 ou en VP9 quand c'est plus léger que le H.264 : souvent moitié moins de place, sans rien perdre.",
          "Online videos arrive in up to 1080p, in AV1 or VP9 when that's lighter than H.264: often half the space, losing nothing.",
        );
  switch (q) {
    case "original":
      return t(
        "Rien n'est réencodé. Les vidéos en ligne arrivent quand même dans le format le plus léger que Lumen sait lire, sans rien perdre.",
        "Nothing is re-encoded. Online videos still arrive in the lightest format Lumen can play, losing nothing.",
      );
    case "compact":
      return t(
        "Vidéos en 720p et sons plus légers : deux à trois fois moins de place que l'original. Nettement moins net en plein écran : pour un Mac à l'étroit.",
        "Videos in 720p and lighter sounds: two to three times less space than the original. Clearly less sharp in full screen: for a Mac short on space.",
      );
    default:
      return t(
        "Recommandé. Les vidéos prennent environ 40 % de place en moins, les voix jusqu'à deux fois moins, à l'œil et à l'oreille presque identiques. La lanterne reste calée au mot près.",
        "Recommended. Videos take about 40% less space, voices up to half as much, nearly identical to the eye and ear. The lantern stays in sync word by word.",
      );
  }
}

/** Ce qui occupe la place : une barre de lumière découpée par catégorie, puis la légende. */
function DiskCard({ usage }: { usage: MediaUsage | null }) {
  const parts = usage
    ? [
        { id: "models", label: t("Modèles d'IA", "AI models"), bytes: usage.models, note: "" },
        { id: "videos", label: t("Vidéos", "Videos"), bytes: usage.videos, note: count(usage.video_count, "leçon", "leçons", "lesson", "lessons") },
        { id: "sounds", label: t("Sons", "Sounds"), bytes: usage.sounds, note: count(usage.sound_count, "leçon", "leçons", "lesson", "lessons") },
        { id: "dicts", label: t("Dictionnaires", "Dictionaries"), bytes: usage.dicts, note: "" },
        { id: "other", label: t("Le reste", "Everything else"), bytes: usage.other, note: t("progression, outils, voix", "progress, tools, voices") },
      ]
    : [];
  const total = parts.reduce((a, p) => a + p.bytes, 0);
  const filled = parts.filter((p) => p.bytes > 0);
  return (
    <div className="set-card disk-card">
      <div className="disk-head">
        <strong className="disk-total num">{usage ? formatBytes(total) : "…"}</strong>
        <span>{t("utilisés par Lumen sur ce Mac", "used by Lumen on this Mac")}</span>
      </div>
      <div className="disk-bar" aria-hidden="true">
        {filled.map((p) => (
          <motion.i
            key={p.id}
            className={`disk-seg ${p.id}`}
            initial={false}
            animate={{ width: `${total ? (p.bytes / total) * 100 : 0}%` }}
            transition={{ type: "spring", stiffness: 120, damping: 22 }}
          />
        ))}
      </div>
      <ul className="disk-legend">
        {parts.map((p) => (
          <li key={p.id}>
            <span className={`disk-dot ${p.id}`} />
            <span className="disk-name">
              {p.label}
              {p.note && <small>{p.note}</small>}
            </span>
            <b className="num">{formatBytes(p.bytes)}</b>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Alléger les leçons déjà importées : estimation, puis avancement fichier par fichier. */
function LightenCard({ usage, quality }: { usage: MediaUsage | null; quality: MediaQuality }) {
  const { run, busy, lighten, stop } = useStorage();
  const overall = run && run.total ? ((run.done + run.value / 100) / run.total) * 100 : 0;
  const gain = usage ? usage.candidate_bytes - usage.candidate_after : 0;
  return (
    <div className="set-card">
      <AnimatePresence mode="popLayout" initial={false}>
        {busy ? (
          <motion.div key="busy" className="set-row lighten-row" {...enter}>
            <span className="set-row-icon">
              <Icon name="sparkle" size={16} />
            </span>
            <div className="grow">
              <strong>{run?.title ? t(`Allègement · ${run.title}`, `Lightening · ${run.title}`) : t("Préparation…", "Getting ready…")}</strong>
              <span className="num">
                {run && run.total
                  ? t(`${Math.min(run.done + 1, run.total)} sur ${run.total} · ${formatBytes(run.saved)} gagnés`, `${Math.min(run.done + 1, run.total)} of ${run.total} · ${formatBytes(run.saved)} saved`)
                  : t("Lumen regarde ce qui peut s'alléger.", "Lumen is looking at what can be lightened.")}
              </span>
              <div className="bar live">
                <i style={{ width: `${Math.max(3, overall)}%` }} />
              </div>
            </div>
            <button className="btn sm soft" onClick={stop}>
              {t("Arrêter", "Stop")}
            </button>
          </motion.div>
        ) : quality === "original" ? (
          <motion.div key="original" className="set-row" {...enter}>
            <div className="grow">
              <strong>{t("Les fichiers restent tels quels", "Files stay as they are")}</strong>
              <span>{t("Choisissez « Allégée » ou « Compacte » pour alléger aussi les leçons déjà importées.", "Choose “Lighter” or “Compact” to lighten the lessons already imported too.")}</span>
            </div>
          </motion.div>
        ) : !usage ? (
          <motion.div key="wait" className="set-row" {...enter}>
            <div className="grow">
              <strong>{t("Calcul…", "Measuring…")}</strong>
              <span>{t("Lumen regarde chaque vidéo et chaque son.", "Lumen is looking at each video and sound.")}</span>
            </div>
          </motion.div>
        ) : usage.candidates > 0 ? (
          <motion.div key="todo" className="set-row" {...enter}>
            <span className="set-row-icon">
              <Icon name="sparkle" size={16} />
            </span>
            <div className="grow">
              <strong>
                {t(
                  `Environ ${formatBytes(gain)} à gagner`,
                  `About ${formatBytes(gain)} to gain`,
                )}
              </strong>
              <span>
                {t(
                  `${count(usage.candidates, "fichier peut", "fichiers peuvent", "", "")} encore être allégé${usage.candidates > 1 ? "s" : ""}, sans toucher aux leçons ni à la lanterne. Vous pouvez continuer à lire pendant ce temps.`,
                  `${count(usage.candidates, "", "", "file", "files")} can still be lightened, without changing the lessons or the lantern. You can keep reading meanwhile.`,
                )}
              </span>
            </div>
            <button className="btn sm primary glow" onClick={() => void lighten()}>
              {t("Alléger", "Lighten")}
            </button>
          </motion.div>
        ) : (
          <motion.div key="done" className="set-row" {...enter}>
            <span className="set-row-icon ok">
              <Icon name="check" size={16} />
            </span>
            <div className="grow">
              <strong>{t("Tout est déjà léger", "Everything is already light")}</strong>
              <span>{t("Les prochaines vidéos et les prochains sons s'allègeront d'eux-mêmes à l'import.", "Upcoming videos and sounds will be lightened on their own when imported.")}</span>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Réglages › Stockage : ce qui occupe la place, et des vidéos et des sons bien plus légers. */
export function StorageSection() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const usage = useStorage((s) => s.usage);
  const busy = useStorage((s) => s.busy);
  const refresh = useStorage((s) => s.refresh);
  const quality = mediaQuality(settings.media_quality);
  // sous Windows, « d'origine » et « allégée » font la même chose
  const shown = isWindows && quality === "original" ? "balanced" : quality;

  useEffect(() => {
    if (!busy) void refresh();
  }, [quality, busy, refresh]);

  return (
    <>
      <section className="set-section">
        <DiskCard usage={usage} />
      </section>

      <section className="set-section">
        <h3>{t("Vidéos et sons importés", "Imported videos and sounds")}</h3>
        <p>
          {isWindows
            ? t(
                "Lumen choisit pour chaque vidéo en ligne le format le plus léger que ce PC lit. Les fichiers du PC restent tels quels.",
                "For each online video, Lumen picks the lightest format this PC can play. Files from the PC stay as they are.",
              )
            : t(
                "À l'import, l'image est réencodée en HEVC par la puce vidéo du Mac, pendant la transcription, et le son en AAC, en mono quand il n'y a qu'une voix. Les vidéos en ligne arrivent d'abord dans le format le plus léger que Lumen sait lire.",
                "On import, the picture is re-encoded to HEVC by the Mac's video chip, during the transcription, and the sound to AAC, in mono when there's a single voice. Online videos first arrive in the lightest format Lumen can play.",
              )}
        </p>
        <div className="set-card">
          <div className="set-row storage-quality">
            <div className="grow">
              <strong>{t("Qualité", "Quality")}</strong>
              <AnimatePresence mode="wait" initial={false}>
                <motion.span key={shown} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.18 }}>
                  {qualityNote(shown)}
                </motion.span>
              </AnimatePresence>
            </div>
            <Segmented id="media-quality" label={t("Qualité des vidéos et des sons", "Video and sound quality")} value={shown} onChange={(v) => setSetting("media_quality", v)} options={qualityOptions()} />
          </div>
        </div>
      </section>

      {(usage?.supported ?? !isWindows) && (
        <section className="set-section">
          <h3>{t("Leçons déjà importées", "Lessons already imported")}</h3>
          <LightenCard usage={usage} quality={quality} />
        </section>
      )}
    </>
  );
}
