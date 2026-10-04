// Icône de Lumen : l'astre se lève derrière un livre ouvert, un mot luit sous la lanterne.
// Écrit icon.svg et icon-small.svg (sans texte ni poussières, pour 16 et 32 px) à côté de ce fichier.
// Rendu en PNG et installation dans l'app : `node scripts/design.mjs`.
import { writeFileSync } from "node:fs";

const f = (n) => Math.round(n * 100) / 100;
const pt = (p) => `${f(p[0])} ${f(p[1])}`;

// Forme d'Apple : carré à coins continus (lissage 60 %), 824 dans 1024
function squircle(x, y, w, h, r, s = 0.6) {
  const rad = (d) => (d * Math.PI) / 180;
  const p = (1 + s) * r;
  const arc = 90 * (1 - s);
  const arcLen = Math.sin(rad(arc / 2)) * r * Math.SQRT2;
  const alpha = (90 - arc) / 2;
  const p34 = r * Math.tan(rad(alpha / 2));
  const beta = 45 * s;
  const c = p34 * Math.cos(rad(beta));
  const d = c * Math.tan(rad(beta));
  const b = (p - arcLen - c - d) / 3;
  const a = 2 * b;
  const A = f(a), B = f(a + b), C = f(a + b + c), D = f(d), L = f(arcLen), R = f(r), c_ = f(c), bc = f(b + c);
  return [
    `M${f(x + w - p)} ${y}`, `c${A} 0 ${B} 0 ${C} ${D}`, `a${R} ${R} 0 0 1 ${L} ${L}`, `c${D} ${c_} ${D} ${bc} ${D} ${C}`,
    `L${x + w} ${f(y + h - p)}`, `c0 ${A} 0 ${B} ${-D} ${C}`, `a${R} ${R} 0 0 1 ${-L} ${L}`, `c${-c_} ${D} ${-bc} ${D} ${-C} ${D}`,
    `L${f(x + p)} ${y + h}`, `c${-A} 0 ${-B} 0 ${-C} ${-D}`, `a${R} ${R} 0 0 1 ${-L} ${-L}`, `c${-D} ${-c_} ${-D} ${-bc} ${-D} ${-C}`,
    `L${x} ${f(y + p)}`, `c0 ${-A} 0 ${-B} ${D} ${-C}`, `a${R} ${R} 0 0 1 ${L} ${-L}`, `c${c_} ${-D} ${bc} ${-D} ${C} ${-D}`, "Z",
  ].join(" ");
}

const bez = (c, t) => {
  const u = 1 - t;
  return [0, 1].map((k) => u * u * u * c[0][k] + 3 * u * u * t * c[1][k] + 3 * u * t * t * c[2][k] + t * t * t * c[3][k]);
};
const lerp = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
const mir = (c) => c.map(([x, y]) => [1024 - x, y]);

// ---------- livre ouvert ----------
// page gauche : bord haut et bord bas, du bord extérieur vers la reliure
const GX = 512;
const TOP = [[186, 664], [258, 612], [452, 596], [GX, 676]];
const BOT = [[180, 786], [256, 740], [452, 726], [GX, 800]];

// décalage d'une feuille de la tranche (k = rang sous la page) : vers le bas et l'extérieur, presque rien à la reliure
const shift = (c, k, out = 0) => c.map(([x, y], i) => {
  const w = 1 - i / 3;
  return [x - (4.2 * k + out) * w, y + 6 * k * w + 2.2 * k * (1 - w) + out * 0.6 * w];
});

// contour d'une page : haut (ext → reliure), reliure, bas (reliure → ext), bord extérieur légèrement bombé
function page(top, bot) {
  const ctrl = [Math.min(top[0][0], bot[0][0]) - 7, (top[0][1] + bot[0][1]) / 2];
  return `M${pt(top[0])} C${pt(top[1])} ${pt(top[2])} ${pt(top[3])} L${pt(bot[3])} C${pt(bot[2])} ${pt(bot[1])} ${pt(bot[0])} Q${pt(ctrl)} ${pt(top[0])} Z`;
}
const both = (top, bot) => [page(top, bot), page(mir(top), mir(bot))];
const MTOP = mir(TOP);
const curve = (c) => `M${pt(c[0])} C${pt(c[1])} ${pt(c[2])} ${pt(c[3])}`;

// lignes de texte : des mots posés le long de courbes parallèles aux bords
let seed = 11;
const rnd = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
const curveAt = (t) => TOP.map((p, i) => lerp(p, BOT[i], t));
function lines(side) {
  const out = [];
  const rows = [0.2, 0.34, 0.48, 0.62, 0.76];
  rows.forEach((tv, row) => {
    const c = curveAt(tv);
    let t = 0.1 + (row === 0 ? 0.05 : 0);
    const end = row === rows.length - 1 ? 0.55 + rnd() * 0.15 : 0.88;
    let wi = 0;
    while (t < end - 0.02) {
      const t1 = Math.min(t + 0.035 + rnd() * 0.1, end);
      const pts = [];
      for (let k = 0; k <= 6; k++) pts.push(bez(c, t + ((t1 - t) * k) / 6));
      const P = side === "R" ? mir(pts) : pts;
      out.push({ d: "M" + P.map(pt).join(" L"), row, wi, side, mid: P[3] });
      t = t1 + 0.022;
      wi++;
    }
  });
  return out;
}
const words = [...lines("L"), ...lines("R")];
const lit = words.find((w) => w.side === "R" && w.row === 1 && w.wi === 2);

// poussières de lumière
const motes = [
  [330, 404, 3.4, 0.75], [706, 372, 2.6, 0.6], [748, 498, 3.8, 0.65], [282, 540, 2.3, 0.5],
  [618, 286, 2.1, 0.45], [398, 318, 1.9, 0.4], [818, 586, 2, 0.35], [222, 446, 1.7, 0.32], [560, 236, 1.5, 0.3],
];

const SQ = squircle(100, 100, 824, 824, 185.4);
const SUN = { x: 512, y: 532, r: 158 };

const defs = `
    <clipPath id="sq"><path d="${SQ}"/></clipPath>
    <linearGradient id="night" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#26140f"/>
      <stop offset="0.55" stop-color="#1b0e0a"/>
      <stop offset="1" stop-color="#0f0806"/>
    </linearGradient>
    <radialGradient id="glow" cx="${SUN.x}" cy="${SUN.y + 10}" r="600" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffd9a2"/>
      <stop offset="0.2" stop-color="#ffb468" stop-opacity="0.95"/>
      <stop offset="0.36" stop-color="#e57a32" stop-opacity="0.68"/>
      <stop offset="0.56" stop-color="#983c19" stop-opacity="0.4"/>
      <stop offset="0.8" stop-color="#4a1a0d" stop-opacity="0.18"/>
      <stop offset="1" stop-color="#2a0f08" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="horizon" cx="512" cy="660" r="1" gradientUnits="userSpaceOnUse" gradientTransform="translate(512 660) scale(480 100) translate(-512 -660)">
      <stop offset="0" stop-color="#ffd7a0" stop-opacity="0.75"/>
      <stop offset="0.5" stop-color="#ff9e52" stop-opacity="0.26"/>
      <stop offset="1" stop-color="#ff9e52" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="orb" cx="${SUN.x}" cy="${SUN.y}" r="${SUN.r}" fx="${SUN.x - 26}" fy="${SUN.y - 38}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="0.42" stop-color="#fff7e8"/>
      <stop offset="0.74" stop-color="#ffe0ad"/>
      <stop offset="0.93" stop-color="#ffc47c"/>
      <stop offset="1" stop-color="#ffb466"/>
    </radialGradient>
    <linearGradient id="paper" x1="0" y1="600" x2="0" y2="800" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fffaf1"/>
      <stop offset="0.5" stop-color="#fbecd6"/>
      <stop offset="1" stop-color="#efd2ab"/>
    </linearGradient>
    <linearGradient id="gutterL" x1="420" y1="0" x2="512" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#8a4c22" stop-opacity="0"/>
      <stop offset="0.7" stop-color="#8a4c22" stop-opacity="0.12"/>
      <stop offset="1" stop-color="#8a4c22" stop-opacity="0.38"/>
    </linearGradient>
    <linearGradient id="gutterR" x1="604" y1="0" x2="512" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#8a4c22" stop-opacity="0"/>
      <stop offset="0.7" stop-color="#8a4c22" stop-opacity="0.1"/>
      <stop offset="1" stop-color="#8a4c22" stop-opacity="0.32"/>
    </linearGradient>
    <linearGradient id="outerL" x1="180" y1="0" x2="300" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#8a4c22" stop-opacity="0.16"/>
      <stop offset="1" stop-color="#8a4c22" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="outerR" x1="844" y1="0" x2="724" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#8a4c22" stop-opacity="0.16"/>
      <stop offset="1" stop-color="#8a4c22" stop-opacity="0"/>
    </linearGradient>
    <radialGradient id="backlight" cx="512" cy="600" r="290" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffc376" stop-opacity="0.55"/>
      <stop offset="1" stop-color="#ffc376" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="sheet" x1="0" y1="700" x2="0" y2="830" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#f3dcb8"/>
      <stop offset="1" stop-color="#d9b386"/>
    </linearGradient>
    <linearGradient id="cover" x1="0" y1="660" x2="0" y2="850" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#7a3518"/>
      <stop offset="1" stop-color="#3e170b"/>
    </linearGradient>
    <radialGradient id="lantern" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#ffb04f" stop-opacity="0.55"/>
      <stop offset="0.6" stop-color="#ffb04f" stop-opacity="0.2"/>
      <stop offset="1" stop-color="#ffb04f" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="rim" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity="0.2"/>
      <stop offset="0.3" stop-color="#fff" stop-opacity="0.03"/>
      <stop offset="0.8" stop-color="#fff" stop-opacity="0"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0.05"/>
    </linearGradient>
    <linearGradient id="rimL" x1="186" y1="0" x2="512" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff6e6" stop-opacity="0"/>
      <stop offset="0.3" stop-color="#fff6e6" stop-opacity="1"/>
    </linearGradient>
    <linearGradient id="rimR" x1="838" y1="0" x2="512" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff6e6" stop-opacity="0"/>
      <stop offset="0.3" stop-color="#fff6e6" stop-opacity="1"/>
    </linearGradient>
    <filter id="b3" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="3"/></filter>
    <filter id="b6" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="6"/></filter>
    <filter id="b10" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="10"/></filter>
    <filter id="b18" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="18"/></filter>
    <filter id="b36" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="36"/></filter>
    <filter id="grain" x="0" y="0" width="100%" height="100%">
      <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="3" stitchTiles="stitch"/>
      <feColorMatrix type="saturate" values="0"/>
      <feComponentTransfer><feFuncA type="linear" slope="0.5"/></feComponentTransfer>
    </filter>`;

function scene({ small = false } = {}) {
  const sheets = small ? [] : [4, 3, 2, 1];
  const coverK = small ? 3.2 : 5;
  const cover = both(shift(TOP, coverK, 2), shift(BOT, coverK, 2));
  return `
  <g clip-path="url(#sq)">
    <rect x="100" y="100" width="824" height="824" fill="url(#night)"/>
    <rect x="100" y="100" width="824" height="824" fill="url(#glow)"/>
    <rect x="100" y="100" width="824" height="824" fill="url(#horizon)"/>
${small ? "" : `
    <!-- poussières de lumière -->
    <g fill="#ffe8c2">
      ${motes.map(([x, y, r, o]) => `<circle cx="${x}" cy="${y}" r="${f(r * 2.8)}" opacity="${f(o * 0.32)}" filter="url(#b3)"/><circle cx="${x}" cy="${y}" r="${r}" opacity="${o}"/>`).join("\n      ")}
    </g>`}

    <!-- l'astre -->
    <circle cx="${SUN.x}" cy="${SUN.y}" r="${SUN.r + 60}" fill="#ffcf92" opacity="0.5" filter="url(#b36)"/>
    <circle cx="${SUN.x}" cy="${SUN.y}" r="${SUN.r + 14}" fill="#fff0d8" opacity="0.85" filter="url(#b10)"/>
    <circle cx="${SUN.x}" cy="${SUN.y}" r="${SUN.r}" fill="url(#orb)"/>

    <!-- ombre du livre, portée vers nous -->
    <ellipse cx="512" cy="846" rx="400" ry="44" fill="#070302" opacity="0.42" filter="url(#b36)"/>

    <!-- couverture -->
    ${cover.map((d) => `<path d="${d}" fill="url(#cover)"/>`).join("\n    ")}
    ${cover.map((d) => `<path d="${d}" fill="none" stroke="#ffb46a" stroke-opacity="0.28" stroke-width="2"/>`).join("\n    ")}

    <!-- tranche : feuilles empilées -->
    ${sheets.map((k) => both(shift(TOP, k), shift(BOT, k)).map((d) => `<path d="${d}" fill="url(#sheet)" stroke="#9a6438" stroke-opacity="0.32" stroke-width="1.4"/>`).join("")).join("\n    ")}

    <!-- les pages -->
    ${both(TOP, BOT).map((d, i) => `<path d="${d}" fill="url(#paper)"/><path d="${d}" fill="url(#backlight)"/><path d="${d}" fill="url(#gutter${"LR"[i]})"/><path d="${d}" fill="url(#outer${"LR"[i]})"/>`).join("\n    ")}
${small ? "" : `
    <!-- le mot éclairé : la lanterne -->
    <ellipse cx="0" cy="0" rx="1" ry="1" fill="url(#lantern)" transform="translate(${pt(lit.mid)}) rotate(-4) scale(70 34)"/>
    <path d="${lit.d}" stroke="#ffb35c" stroke-opacity="0.7" stroke-width="16" stroke-linecap="round" fill="none" filter="url(#b6)"/>

    <!-- lignes de texte -->
    <g stroke="#7a4626" stroke-opacity="0.18" stroke-width="5.5" stroke-linecap="round" fill="none">
      ${words.map((w) => `<path d="${w.d}"${w === lit ? ' stroke="#e88a2e" stroke-opacity="1" stroke-width="6.5"' : ""}/>`).join("\n      ")}
    </g>`}

    <!-- reliure -->
    <path d="M512 676 L512 800" stroke="#6b3a18" stroke-opacity="0.42" stroke-width="2.5"/>

    <!-- lumière rasante sur le bord des pages -->
    ${[TOP, MTOP].map((c) => `<path d="${curve(c)}" fill="none" stroke="#ffcf8a" stroke-width="14" stroke-opacity="0.5" filter="url(#b6)"/><path d="${curve(c)}" fill="none" stroke="url(#rim${c === TOP ? "L" : "R"})" stroke-width="3.5" stroke-linecap="round"/>`).join("\n    ")}

    <!-- grain -->
    <rect x="100" y="100" width="824" height="824" filter="url(#grain)" opacity="0.1" style="mix-blend-mode:overlay"/>
  </g>
  <path d="${SQ}" fill="none" stroke="url(#rim)" stroke-width="3" clip-path="url(#sq)"/>`;
}

const doc = (body) => `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>${defs}
  </defs>${body}
</svg>
`;
writeFileSync(new URL("./icon.svg", import.meta.url), doc(scene()));
writeFileSync(new URL("./icon-small.svg", import.meta.url), doc(scene({ small: true })));
