import { useId, useMemo, useState, type ReactNode } from "react";
import { api, errorText } from "../lib/api";
import { chooseCover, grainTexture, youtubeId, youtubeThumbs } from "../lib/covers";
import { useApp } from "../lib/store";
import type { LessonSummary } from "../lib/types";
import { Icon, type IconName } from "./Icon";
import { Menu } from "./ui";
import { t } from "../lib/i18n";

export const KIND: Record<string, { label: string; icon: IconName }> = {
  text: { get label() { return t("Texte", "Text"); }, icon: "text" },
  web: { label: "Article", icon: "globe" },
  book: { get label() { return t("Livre", "Book"); }, icon: "book" },
  pdf: { label: "PDF", icon: "file" },
  subtitles: { get label() { return t("Sous-titres", "Subtitles"); }, icon: "text" },
  audio: { label: "Audio", icon: "wave" },
  video: { get label() { return t("Vidéo", "Video"); }, icon: "video" },
  simplified: { get label() { return t("Simplifié", "Simplified"); }, icon: "sparkle" },
};

// ---------- œuvre générée ----------
// Chaque leçon reçoit toujours la même image (graine = son identifiant),
// dans les couleurs de sa teinte : aube, halo, aurore ou prisme.

const W = 320;
const H = 180;

/** Générateur pseudo-aléatoire reproductible (mulberry32). */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const hsl = (h: number, s: number, l: number) => `hsl(${(((h % 360) + 360) % 360).toFixed(0)}, ${s}%, ${l}%)`;

/** Ligne d'horizon ondulée, fermée vers le bas. */
function wave(y: number, amp: number, freq: number, phase: number): string {
  let d = `M0,${H} L0,${y.toFixed(1)}`;
  for (let x = 0; x <= W; x += 8) {
    const t = (x / W) * Math.PI * 2 * freq + phase;
    d += ` L${x},${(y + Math.sin(t) * amp + Math.sin(t * 2.3 + phase) * amp * 0.35).toFixed(1)}`;
  }
  return `${d} L${W},${H} Z`;
}

function art(seed: number, hue: number, id: string): ReactNode {
  const r = rng(Math.imul(seed + 1, 2654435761) ^ (hue * 7919));
  const side = r() < 0.5 ? 1 : -1;
  const h0 = hue;
  const h1 = hue + side * (30 + r() * 35);
  const h2 = hue - side * (150 + r() * 50);
  const u = (n: string) => `${id}-${n}`;
  const url = (n: string) => `url(#${id}-${n})`;
  const dust = (n: number, color: string) =>
    Array.from({ length: n }, (_, i) => <circle key={`d${i}`} cx={r() * W} cy={r() * H * 0.8} r={0.5 + r() * 1.3} fill={color} opacity={0.25 + r() * 0.6} />);

  switch (Math.floor(r() * 4)) {
    // aube : un astre se lève derrière des collines
    case 0: {
      const sx = 60 + r() * 200;
      const hy = 92 + r() * 22;
      return (
        <>
          <defs>
            <linearGradient id={u("sky")} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor={hsl(h0, 55, 17)} />
              <stop offset="0.45" stopColor={hsl(h0, 58, 40)} />
              <stop offset="0.78" stopColor={hsl(h1, 85, 66)} />
              <stop offset="1" stopColor={hsl(h1 + 10, 95, 85)} />
            </linearGradient>
            <radialGradient id={u("halo")}>
              <stop offset="0" stopColor={hsl(h1 + 15, 100, 95)} stopOpacity="0.95" />
              <stop offset="0.35" stopColor={hsl(h1, 95, 78)} stopOpacity="0.45" />
              <stop offset="1" stopColor={hsl(h1, 95, 70)} stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect width={W} height={H} fill={url("sky")} />
          {dust(10, hsl(h1 + 20, 100, 92))}
          <circle cx={sx} cy={hy} r={115} fill={url("halo")} />
          <circle cx={sx} cy={hy} r={14 + r() * 7} fill={hsl(h1 + 20, 100, 96)} />
          {[0, 1, 2].map((i) => (
            <path key={i} d={wave(hy + 4 + i * 23, 7 - i * 1.5 + r() * 4, 0.8 + r() * 1.4, r() * 6.28)} fill={hsl(h0 + i * 10 * side, 52 - i * 5, 36 - i * 10)} fillOpacity={0.6 + i * 0.2} />
          ))}
        </>
      );
    }
    // halo : anneaux de lumière autour d'une source
    case 1: {
      const cx = r() < 0.5 ? 40 + r() * 90 : 190 + r() * 90;
      const cy = 30 + r() * 120;
      return (
        <>
          <defs>
            <radialGradient id={u("bg")} cx={cx / W} cy={cy / H} r="1.1">
              <stop offset="0" stopColor={hsl(h1, 72, 48)} />
              <stop offset="0.45" stopColor={hsl(h0, 60, 25)} />
              <stop offset="1" stopColor={hsl(h0 - 12 * side, 55, 11)} />
            </radialGradient>
            <radialGradient id={u("core")}>
              <stop offset="0" stopColor={hsl(h1 + 20, 100, 94)} stopOpacity="0.9" />
              <stop offset="1" stopColor={hsl(h1, 100, 80)} stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect width={W} height={H} fill={url("bg")} />
          {[16, 30, 48, 70, 96, 126, 160, 200, 245].map((rad, i) => (
            <circle key={rad} cx={cx} cy={cy} r={rad} fill="none" stroke={hsl(h1 + 20, 100, 88)} strokeOpacity={Math.max(0.05, 0.55 - i * 0.06)} strokeWidth={i % 3 === 0 ? 1.6 : 0.8} />
          ))}
          <circle cx={cx} cy={cy} r={78} fill={url("core")} />
          <circle cx={cx} cy={cy} r={6 + r() * 4} fill={hsl(h1 + 25, 100, 97)} />
          {dust(16, hsl(h1 + 30, 100, 90))}
        </>
      );
    }
    // aurore : rubans de lumière dans la nuit
    case 2: {
      const ribbons = Array.from({ length: 3 + Math.floor(r() * 2) }, (_, i) => {
        const y0 = 15 + r() * 150;
        const y3 = 15 + r() * 150;
        const d = `M-40,${y0.toFixed(0)} C${(70 + r() * 70).toFixed(0)},${(-50 + r() * 280).toFixed(0)} ${(170 + r() * 90).toFixed(0)},${(-50 + r() * 280).toFixed(0)} 360,${y3.toFixed(0)}`;
        const hues = [h1, h0 + 20 * side, h2, h1 + 40 * side];
        return { d, h: hues[i % hues.length], hb: hues[(i + 1) % hues.length], width: 22 + r() * 40 };
      });
      return (
        <>
          <defs>
            <linearGradient id={u("bg")} x1="0" y1="0" x2="1" y2="1">
              <stop offset="0" stopColor={hsl(h0, 55, 12)} />
              <stop offset="1" stopColor={hsl(h2, 45, 20)} />
            </linearGradient>
            {ribbons.map((rb, i) => (
              <linearGradient key={i} id={u(`rb${i}`)} x1="0" y1="0" x2="1" y2="0">
                <stop offset="0" stopColor={hsl(rb.h, 90, 65)} stopOpacity="0" />
                <stop offset="0.3" stopColor={hsl(rb.h, 90, 66)} stopOpacity="0.95" />
                <stop offset="0.7" stopColor={hsl(rb.hb, 85, 70)} stopOpacity="0.85" />
                <stop offset="1" stopColor={hsl(rb.hb, 85, 70)} stopOpacity="0" />
              </linearGradient>
            ))}
          </defs>
          <rect width={W} height={H} fill={url("bg")} />
          {dust(18, hsl(h1, 90, 92))}
          {ribbons.map((rb, i) => (
            <g key={i} style={{ mixBlendMode: "screen" }}>
              <path d={rb.d} fill="none" stroke={url(`rb${i}`)} strokeWidth={rb.width * 1.8} strokeLinecap="round" opacity={0.18} />
              <path d={rb.d} fill="none" stroke={url(`rb${i}`)} strokeWidth={rb.width} strokeLinecap="round" opacity={0.55} />
              <path d={rb.d} fill="none" stroke={hsl(rb.h + 15, 100, 92)} strokeWidth={1.1} opacity={0.55} />
            </g>
          ))}
        </>
      );
    }
    // prisme : formes pleines qui se superposent dans une lumière claire
    default: {
      const ax = 50 + r() * 140;
      const ay = 40 + r() * 90;
      const ar = 55 + r() * 45;
      const bx = ax + (r() < 0.5 ? -1 : 1) * (40 + r() * 50);
      const by = ay + 20 + r() * 40;
      const half = 140 + r() * 140;
      const hr = 45 + r() * 35;
      return (
        <>
          <defs>
            <linearGradient id={u("bg")} x1="0" y1="0" x2="1" y2="1">
              <stop offset="0" stopColor={hsl(h1, 80, 91)} />
              <stop offset="1" stopColor={hsl(h0, 60, 80)} />
            </linearGradient>
          </defs>
          <rect width={W} height={H} fill={url("bg")} />
          <path d={`M${half - hr},${H} A${hr},${hr} 0 0 1 ${half + hr},${H} Z`} fill={hsl(h1, 85, 60)} />
          <circle cx={ax} cy={ay} r={ar} fill={hsl(h0, 68, 52)} opacity={0.92} />
          <circle cx={bx} cy={by} r={ar * (0.6 + r() * 0.3)} fill={hsl(h2, 80, 62)} opacity={0.8} style={{ mixBlendMode: "multiply" }} />
          <circle cx={W - 40 - r() * 80} cy={30 + r() * 40} r={18 + r() * 16} fill="none" stroke={hsl(h0, 50, 20)} strokeWidth={1.2} opacity={0.6} />
          <line x1={r() * 80} y1={H - 20 - r() * 60} x2={W - r() * 60} y2={10 + r() * 50} stroke={hsl(h0, 50, 20)} strokeWidth={1} opacity={0.4} />
          <circle cx={ax + ar * 0.7} cy={ay - ar * 0.6} r={4 + r() * 3} fill={hsl(h0, 55, 18)} />
        </>
      );
    }
  }
}

export function CoverArt({ seed, hue }: { seed: number; hue: number }) {
  const id = `ca${useId().replace(/[^a-zA-Z0-9]/g, "")}`;
  const body = useMemo(() => art(seed, hue, id), [seed, hue, id]);
  return (
    <svg className="cover-art" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="xMidYMid slice" aria-hidden="true">
      {body}
    </svg>
  );
}

// ---------- couverture complète ----------

type CoverLesson = Pick<LessonSummary, "id" | "hue" | "kind" | "source" | "cover_path">;

/**
 * Couverture d'une leçon : l'image choisie, sinon la miniature YouTube,
 * sinon l'œuvre générée (qui sert aussi de fond pendant le chargement).
 */
export function Cover({
  lesson,
  big = false,
  progress = 0,
  editable = false,
  bare = false,
}: {
  lesson: CoverLesson;
  big?: boolean;
  progress?: number;
  editable?: boolean;
  /** image seule, sans étiquette ni avancement (vignettes, mosaïques) */
  bare?: boolean;
}) {
  const yt = useMemo(() => youtubeId(lesson.source), [lesson.source]);
  const [stage, setStage] = useState(0);
  const [shown, setShown] = useState<string | null>(null);
  const custom = lesson.cover_path ? api().mediaUrl(lesson.cover_path) : null;
  const src = custom ?? (yt ? youtubeThumbs(yt)[stage] : undefined) ?? null;
  const k = KIND[lesson.kind] ?? KIND.text;

  return (
    <div className={`cover-box ${big ? "big" : ""}`}>
      <div className="cover">
        <CoverArt seed={lesson.id} hue={lesson.hue} />
        <span className="cover-grain" style={{ backgroundImage: `url(${grainTexture()})` }} />
        {src && (
          <img
            key={src}
            className={`cover-img ${shown === src ? "on" : ""}`}
            src={src}
            alt=""
            draggable={false}
            onLoad={(e) => {
              // YouTube renvoie une vignette grise de 120 px quand la haute définition n'existe pas
              if (!custom && stage === 0 && e.currentTarget.naturalWidth < 200) setStage(1);
              else setShown(src);
            }}
            onError={() => !custom && setStage((s) => s + 1)}
          />
        )}
        {!bare && (
          <span className="cover-kind">
            <Icon name={yt ? "youtube" : k.icon} size={14} />
            {k.label}
          </span>
        )}
        {!bare && progress > 0.005 && progress < 0.995 && (
          <span className="cover-progress" aria-hidden="true">
            <i style={{ width: `${progress * 100}%` }} />
          </span>
        )}
      </div>
      {editable && <CoverButton lessonId={lesson.id} custom={!!lesson.cover_path} />}
    </div>
  );
}

/**
 * Couverture d'une playlist : la couverture de sa première leçon, ou une
 * mosaïque des quatre premières ; vide, une œuvre générée pour elle.
 */
export function PlaylistCover({ lessons, seed, big = false }: { lessons: CoverLesson[]; seed: number; big?: boolean }) {
  const tiles = lessons.length >= 4 ? lessons.slice(0, 4) : lessons.slice(0, 1);
  return (
    <div className={`cover-box pl-cover ${big ? "pl-big" : ""}`}>
      <div className={`cover pl-mosaic n${tiles.length}`}>
        {tiles.length === 0 ? (
          <>
            <CoverArt seed={seed * 7919 + 13} hue={(seed * 47 + 28) % 360} />
            <span className="cover-grain" style={{ backgroundImage: `url(${grainTexture()})` }} />
            <span className="pl-empty-icon">
              <Icon name="playlist" size={big ? 34 : 24} stroke={1.6} />
            </span>
          </>
        ) : (
          tiles.map((l) => <Cover key={l.id} lesson={l} bare />)
        )}
      </div>
    </div>
  );
}

/** Petit bouton discret (au survol) pour choisir ou retirer une couverture. */
function CoverButton({ lessonId, custom }: { lessonId: number; custom: boolean }) {
  const [menu, setMenu] = useState(false);
  const [busy, setBusy] = useState(false);
  const toast = useApp((s) => s.toast);
  const bump = useApp((s) => s.bumpLibrary);

  const choose = async () => {
    setMenu(false);
    setBusy(true);
    try {
      if (await chooseCover(lessonId)) {
        bump();
        toast(t("Nouvelle couverture", "New cover"), "light");
      }
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const remove = async () => {
    setMenu(false);
    try {
      await api().lessonSetCover(lessonId, null, null);
      bump();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const button = (
    <button
      className={`cover-edit ${busy ? "busy" : ""}`}
      onClick={() => (custom ? setMenu((m) => !m) : void choose())}
      disabled={busy}
      aria-label={t("Changer la couverture", "Change the cover")}
      title={t("Changer la couverture", "Change the cover")}
    >
      <Icon name="image" size={13} stroke={1.9} />
    </button>
  );
  return (
    <div className={`cover-edit-wrap ${menu ? "open" : ""}`}>
      {custom ? (
        <Menu open={menu} onClose={() => setMenu(false)} align="right" anchor={button}>
          <button className="menu-item" onClick={choose}>
            <Icon name="image" size={16} /> {t("Choisir une autre image", "Choose another image")}
          </button>
          <div className="menu-sep" />
          <button className="menu-item danger" onClick={remove}>
            <Icon name="close" size={16} /> {t("Retirer la couverture", "Remove the cover")}
          </button>
        </Menu>
      ) : (
        button
      )}
    </div>
  );
}
