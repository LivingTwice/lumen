import { AnimatePresence, motion } from "motion/react";
import { useId, useMemo, type ReactNode } from "react";
import { avatarString, initialOf, useUser, type AvatarSpec } from "../lib/user";
import { hsl, rng } from "./Cover";

/* Avatar de l'apprenant : l'initiale de son nom, une lumière générée dans sa
   teinte (aube, halo, aurore, nuit : toujours la même pour une même graine)
   ou sa photo. Dessiné dans un carré de 100, découpé en disque par le CSS. */

const S = 100;

/** Silhouette douce, quand il n'y a ni nom ni avatar. */
function silhouette(color: string, opacity = 1): ReactNode {
  return (
    <g fill={color} opacity={opacity}>
      <circle cx="50" cy="40" r="15" />
      <path d="M22 84c3-15 14-24 28-24s25 9 28 24z" />
    </g>
  );
}

function art(a: AvatarSpec, name: string, id: string): ReactNode {
  const r = rng(Math.imul(a.seed + 7, 2654435761) ^ (a.hue * 7919));
  const h = a.hue;
  const u = (n: string) => `${id}-${n}`;
  const url = (n: string) => `url(#${id}-${n})`;
  const dust = (n: number, color: string, maxY = 60) =>
    Array.from({ length: n }, (_, i) => <circle key={`d${i}`} cx={6 + r() * 88} cy={6 + r() * maxY} r={0.5 + r() * 1.1} fill={color} opacity={0.35 + r() * 0.6} />);

  switch (a.style) {
    // aube : l'astre se lève derrière des collines
    case "dawn": {
      const sx = 38 + r() * 24;
      const hy = 62 + r() * 6;
      const hill = (y: number, amp: number, phase: number) => {
        let d = `M0,${S} L0,${y.toFixed(1)}`;
        for (let x = 0; x <= S; x += 5) d += ` L${x},${(y + Math.sin((x / S) * Math.PI * 2 * 0.9 + phase) * amp).toFixed(1)}`;
        return `${d} L${S},${S} Z`;
      };
      return (
        <>
          <defs>
            <linearGradient id={u("sky")} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor={hsl(h - 8, 52, 20)} />
              <stop offset="0.48" stopColor={hsl(h, 60, 42)} />
              <stop offset="0.8" stopColor={hsl(h + 22, 88, 68)} />
              <stop offset="1" stopColor={hsl(h + 32, 96, 86)} />
            </linearGradient>
            <radialGradient id={u("halo")}>
              <stop offset="0" stopColor={hsl(h + 38, 100, 96)} stopOpacity="0.95" />
              <stop offset="0.35" stopColor={hsl(h + 24, 96, 78)} stopOpacity="0.5" />
              <stop offset="1" stopColor={hsl(h + 20, 95, 70)} stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect width={S} height={S} fill={url("sky")} />
          {dust(5, hsl(h + 40, 100, 94), 34)}
          <circle cx={sx} cy={hy} r={44} fill={url("halo")} />
          <circle cx={sx} cy={hy} r={11 + r() * 3} fill={hsl(h + 40, 100, 96)} />
          <path d={hill(hy + 6, 3 + r() * 3, r() * 6.28)} fill={hsl(h - 6, 50, 32)} fillOpacity="0.75" />
          <path d={hill(hy + 17, 2.5 + r() * 3, r() * 6.28)} fill={hsl(h - 14, 46, 20)} />
        </>
      );
    }
    // halo : anneaux de lumière autour d'une source
    case "halo": {
      const cx = 42 + r() * 16;
      const cy = 42 + r() * 16;
      return (
        <>
          <defs>
            <radialGradient id={u("bg")} cx={cx / S} cy={cy / S} r="0.85">
              <stop offset="0" stopColor={hsl(h + 18, 78, 54)} />
              <stop offset="0.5" stopColor={hsl(h, 62, 26)} />
              <stop offset="1" stopColor={hsl(h - 10, 55, 11)} />
            </radialGradient>
            <radialGradient id={u("core")}>
              <stop offset="0" stopColor={hsl(h + 30, 100, 95)} stopOpacity="0.95" />
              <stop offset="1" stopColor={hsl(h + 20, 100, 80)} stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect width={S} height={S} fill={url("bg")} />
          {[7, 12, 18.5, 26, 35, 46, 59].map((rad, i) => (
            <circle key={rad} cx={cx} cy={cy} r={rad} fill="none" stroke={hsl(h + 30, 100, 88)} strokeOpacity={Math.max(0.07, 0.6 - i * 0.08)} strokeWidth={i % 3 === 0 ? 1.3 : 0.7} />
          ))}
          <circle cx={cx} cy={cy} r={24} fill={url("core")} />
          <circle cx={cx} cy={cy} r={3.6 + r() * 1.6} fill={hsl(h + 35, 100, 97)} />
          {dust(7, hsl(h + 35, 100, 92), 88)}
        </>
      );
    }
    // aurore : rubans de lumière dans la nuit
    case "aurora": {
      const side = r() < 0.5 ? 1 : -1;
      const hues = [h + 20 * side, h - 40 * side, h + 150, h + 60 * side];
      const ribbons = Array.from({ length: 3 }, (_, i) => {
        const y0 = 18 + r() * 64;
        const y3 = 18 + r() * 64;
        const d = `M-12,${y0.toFixed(0)} C${(22 + r() * 22).toFixed(0)},${(-10 + r() * 120).toFixed(0)} ${(56 + r() * 26).toFixed(0)},${(-10 + r() * 120).toFixed(0)} 112,${y3.toFixed(0)}`;
        return { d, a: hues[i % hues.length], b: hues[(i + 1) % hues.length], w: 9 + r() * 12 };
      });
      return (
        <>
          <defs>
            <linearGradient id={u("bg")} x1="0" y1="0" x2="1" y2="1">
              <stop offset="0" stopColor={hsl(h, 52, 12)} />
              <stop offset="1" stopColor={hsl(h + 150, 40, 18)} />
            </linearGradient>
            {ribbons.map((rb, i) => (
              <linearGradient key={i} id={u(`rb${i}`)} x1="0" y1="0" x2="1" y2="0">
                <stop offset="0" stopColor={hsl(rb.a, 90, 65)} stopOpacity="0" />
                <stop offset="0.3" stopColor={hsl(rb.a, 90, 66)} stopOpacity="0.95" />
                <stop offset="0.7" stopColor={hsl(rb.b, 85, 70)} stopOpacity="0.85" />
                <stop offset="1" stopColor={hsl(rb.b, 85, 70)} stopOpacity="0" />
              </linearGradient>
            ))}
          </defs>
          <rect width={S} height={S} fill={url("bg")} />
          {dust(9, hsl(h + 20, 90, 92), 88)}
          {ribbons.map((rb, i) => (
            <g key={i} style={{ mixBlendMode: "screen" }}>
              <path d={rb.d} fill="none" stroke={url(`rb${i}`)} strokeWidth={rb.w * 1.9} strokeLinecap="round" opacity={0.2} />
              <path d={rb.d} fill="none" stroke={url(`rb${i}`)} strokeWidth={rb.w} strokeLinecap="round" opacity={0.6} />
              <path d={rb.d} fill="none" stroke={hsl(rb.a + 15, 100, 92)} strokeWidth={0.6} opacity={0.6} />
            </g>
          ))}
        </>
      );
    }
    // nuit : un croissant de lune et ses étoiles, dans une nuit chaude
    case "night": {
      const mx = 50 + (r() - 0.5) * 14;
      const my = 40 + (r() - 0.5) * 10;
      const mr = 15 + r() * 4;
      const cut = 0.55 + r() * 0.2;
      const spark = (x: number, y: number, s: number, k: number) => (
        <path key={`s${k}`} d={`M${x},${y - s} Q${x},${y} ${x + s},${y} Q${x},${y} ${x},${y + s} Q${x},${y} ${x - s},${y} Q${x},${y} ${x},${y - s}Z`} fill={hsl(h + 35, 100, 92)} opacity={0.85} />
      );
      return (
        <>
          <defs>
            <radialGradient id={u("bg")} cx={mx / S} cy={my / S} r="0.9">
              <stop offset="0" stopColor={hsl(h, 48, 30)} />
              <stop offset="0.55" stopColor={hsl(h - 10, 46, 14)} />
              <stop offset="1" stopColor={hsl(h - 18, 44, 7)} />
            </radialGradient>
            <radialGradient id={u("glow")}>
              <stop offset="0" stopColor={hsl(h + 30, 100, 85)} stopOpacity="0.45" />
              <stop offset="1" stopColor={hsl(h + 30, 100, 80)} stopOpacity="0" />
            </radialGradient>
            <mask id={u("moon")}>
              <rect width={S} height={S} fill="white" />
              <circle cx={mx + mr * cut} cy={my - mr * 0.42} r={mr * 0.92} fill="black" />
            </mask>
          </defs>
          <rect width={S} height={S} fill={url("bg")} />
          {dust(12, hsl(h + 35, 100, 92), 88)}
          {spark(18 + r() * 18, 20 + r() * 14, 2.6, 1)}
          {spark(66 + r() * 18, 64 + r() * 16, 1.9, 2)}
          <circle cx={mx} cy={my} r={mr * 2.4} fill={url("glow")} />
          <circle cx={mx} cy={my} r={mr} fill={hsl(h + 34, 100, 90)} mask={url("moon")} />
          <path d={`M0,${S} L0,86 C24,${78 + r() * 6} 56,${84 + r() * 6} ${S},80 L${S},${S} Z`} fill={hsl(h - 16, 40, 6)} opacity={0.85} />
        </>
      );
    }
    // initiale : la première lettre du nom sur un dégradé lumineux
    default: {
      const letter = initialOf(name);
      return (
        <>
          <defs>
            <linearGradient id={u("bg")} x1="0.1" y1="0" x2="0.9" y2="1">
              <stop offset="0" stopColor={hsl(h + 16, 92, 74)} />
              <stop offset="0.55" stopColor={hsl(h + 4, 72, 56)} />
              <stop offset="1" stopColor={hsl(h - 10, 62, 40)} />
            </linearGradient>
            <radialGradient id={u("shine")} cx="0.3" cy="0.2" r="0.7">
              <stop offset="0" stopColor="#fff" stopOpacity="0.5" />
              <stop offset="1" stopColor="#fff" stopOpacity="0" />
            </radialGradient>
          </defs>
          <rect width={S} height={S} fill={url("bg")} />
          <rect width={S} height={S} fill={url("shine")} />
          {letter ? (
            <text
              x="50"
              y="52"
              textAnchor="middle"
              dominantBaseline="central"
              style={{ fontFamily: "var(--font-display)" }}
              fontWeight={500}
              fontSize={letter.length > 1 || /\p{Extended_Pictographic}/u.test(letter) ? 44 : 54}
              fill={hsl(h + 30, 100, 98)}
            >
              {letter}
            </text>
          ) : (
            silhouette(hsl(h + 30, 100, 97), 0.92)
          )}
        </>
      );
    }
  }
}

/** Dessin d'un avatar donné (aperçus du choix, sauvegardes d'un autre Mac). */
export function AvatarArt({ spec, name, photo }: { spec: AvatarSpec; name: string; photo?: string }) {
  const id = `av${useId().replace(/[^a-zA-Z0-9]/g, "")}`;
  const body = useMemo(() => (spec.style === "photo" ? null : art(spec, name, id)), [spec, name, id]);
  if (spec.style === "photo" && photo) return <img className="avatar-img" src={photo} alt="" draggable={false} />;
  return (
    <svg className="avatar-svg" viewBox={`0 0 ${S} ${S}`} aria-hidden="true">
      {body}
    </svg>
  );
}

/**
 * Avatar dans un disque. Sans propriétés : celui de l'apprenant (silhouette
 * tant qu'il n'a rien choisi). Un changement d'avatar se fait en fondu.
 */
export function Avatar({
  size = 32,
  spec,
  name,
  photo,
  empty,
  className = "",
}: {
  size?: number;
  spec?: AvatarSpec;
  name?: string;
  photo?: string;
  /** silhouette, même si un avatar est donné */
  empty?: boolean;
  className?: string;
}) {
  const me = useUser();
  const own = !spec;
  const a = spec ?? me.avatar;
  const n = name ?? me.name;
  const p = photo ?? me.photo;
  const blank = empty ?? (own && me.empty);
  const key = blank ? "empty" : a.style === "photo" ? `photo:${p.length}:${p.slice(-24)}` : `${avatarString(a)}:${a.style === "initial" ? initialOf(n) : ""}`;
  return (
    <span className={`avatar ${blank ? "avatar-blank" : ""} ${className}`} style={{ width: size, height: size }}>
      <AnimatePresence initial={false}>
        <motion.span key={key} className="avatar-face" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.35 }}>
          {blank ? (
            <svg className="avatar-svg" viewBox={`0 0 ${S} ${S}`} aria-hidden="true">
              {silhouette("currentColor", 0.55)}
            </svg>
          ) : (
            <AvatarArt spec={a} name={n} photo={p} />
          )}
        </motion.span>
      </AnimatePresence>
    </span>
  );
}
