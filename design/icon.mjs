// Icône de Lumen : l'illustration icon-source.png (l'astre au-dessus d'un livre ouvert), posée dans la forme d'Apple.
// Écrit icon.svg et icon-small.svg (même image : elle se lit aussi en 16 et 32 px) à côté de ce fichier.
// Rendu en PNG et installation dans l'app : `node scripts/design.mjs`.
import { writeFileSync } from "node:fs";

const f = (n) => Math.round(n * 100) / 100;

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

const SQ = squircle(100, 100, 824, 824, 185.4);

// image carrée qui remplit la forme ; lue à côté du SVG par le Chromium de scripts/design.mjs
const doc = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs><clipPath id="sq"><path d="${SQ}"/></clipPath></defs>
  <image href="icon-source.png" x="100" y="100" width="824" height="824" preserveAspectRatio="xMidYMid slice" clip-path="url(#sq)"/>
</svg>
`;
writeFileSync(new URL("./icon.svg", import.meta.url), doc);
writeFileSync(new URL("./icon-small.svg", import.meta.url), doc);
