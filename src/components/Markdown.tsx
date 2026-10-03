import { Fragment, type ReactNode } from "react";

/**
 * Mise en forme légère des réponses du chat : paragraphes, titres, listes,
 * citations, tableaux (conjugaisons), code, **gras**, *italique* et `code`.
 * Pensée pour un texte qui s'écrit au fil de l'eau : une balise encore
 * ouverte reste simplement du texte. Aucun HTML n'est interprété.
 */
export function Markdown({ text }: { text: string }) {
  return <div className="md">{blocks(text)}</div>;
}

type Block =
  | { kind: "p"; lines: string[] }
  | { kind: "h"; text: string }
  | { kind: "ul" | "ol"; items: { text: string; depth: number }[] }
  | { kind: "quote"; lines: string[] }
  | { kind: "table"; rows: string[][]; head: boolean }
  | { kind: "code"; text: string }
  | { kind: "hr" };

const BULLET = /^(\s*)[-*•+]\s+(.*)$/;
const NUMBER = /^(\s*)\d+[.)]\s+(.*)$/;
const TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function parse(src: string): Block[] {
  const out: Block[] = [];
  const lines = src.replace(/\r/g, "").split("\n");
  let i = 0;
  const last = () => out[out.length - 1];
  while (i < lines.length) {
    const line = lines[i];
    const t = line.trim();
    if (!t) {
      // une ligne vide clôt le paragraphe en cours
      if (last()?.kind === "p") out.push({ kind: "p", lines: [] });
      i++;
      continue;
    }
    if (t.startsWith("```")) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) body.push(lines[i++]);
      i++;
      out.push({ kind: "code", text: body.join("\n") });
      continue;
    }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(t)) {
      out.push({ kind: "hr" });
      i++;
      continue;
    }
    const h = /^#{1,6}\s+(.*)$/.exec(t);
    if (h) {
      out.push({ kind: "h", text: h[1].replace(/\s*#+$/, "") });
      i++;
      continue;
    }
    if (t.startsWith("|") && t.length > 1) {
      const rows: string[][] = [];
      let head = false;
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        const r = lines[i].trim();
        if (TABLE_SEP.test(r)) head = rows.length === 1;
        else rows.push(r.replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim()));
        i++;
      }
      out.push({ kind: "table", rows, head });
      continue;
    }
    if (t.startsWith(">")) {
      const body: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) body.push(lines[i++].trim().replace(/^>\s?/, ""));
      out.push({ kind: "quote", lines: body });
      continue;
    }
    const b = BULLET.exec(line) ?? NUMBER.exec(line);
    if (b) {
      const kind = BULLET.test(line) ? "ul" : "ol";
      const items: { text: string; depth: number }[] = [];
      while (i < lines.length) {
        const m = BULLET.exec(lines[i]) ?? NUMBER.exec(lines[i]);
        if (m) items.push({ text: m[2], depth: Math.min(2, Math.floor(m[1].replace(/\t/g, "  ").length / 2)) });
        // suite d'un élément sur la ligne suivante, en retrait
        else if (lines[i].trim() && /^\s{2,}/.test(lines[i]) && items.length) items[items.length - 1].text += " " + lines[i].trim();
        else break;
        i++;
      }
      out.push({ kind, items });
      continue;
    }
    const p = last();
    if (p?.kind === "p") p.lines.push(t);
    else out.push({ kind: "p", lines: [t] });
    i++;
  }
  return out.filter((b) => b.kind !== "p" || b.lines.length > 0);
}

function blocks(src: string): ReactNode[] {
  return parse(src).map((b, k) => {
    switch (b.kind) {
      case "p":
        return (
          <p key={k} dir="auto">
            {b.lines.map((l, j) => (
              <Fragment key={j}>
                {j > 0 && <br />}
                {inline(l)}
              </Fragment>
            ))}
          </p>
        );
      case "h":
        return (
          <p key={k} className="md-h" dir="auto">
            {inline(b.text)}
          </p>
        );
      case "ul":
      case "ol": {
        const Tag = b.kind;
        return (
          <Tag key={k}>
            {b.items.map((it, j) => (
              <li key={j} className={it.depth ? `d${it.depth}` : undefined} dir="auto">
                {inline(it.text)}
              </li>
            ))}
          </Tag>
        );
      }
      case "quote":
        return (
          <blockquote key={k} dir="auto">
            {b.lines.map((l, j) => (
              <Fragment key={j}>
                {j > 0 && <br />}
                {inline(l)}
              </Fragment>
            ))}
          </blockquote>
        );
      case "table":
        return (
          <div key={k} className="md-table">
            <table>
              <tbody>
                {b.rows.map((r, j) => (
                  <tr key={j} className={b.head && j === 0 ? "head" : undefined}>
                    {r.map((c, n) => (b.head && j === 0 ? <th key={n}>{inline(c)}</th> : <td key={n}>{inline(c)}</td>))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        );
      case "code":
        return <pre key={k}>{b.text}</pre>;
      case "hr":
        return <hr key={k} />;
    }
  });
}

/**
 * Typographie française : l'espace avant « : ; ? ! » et à l'intérieur des
 * guillemets devient insécable, pour qu'aucun signe ne parte seul à la ligne.
 */
export function frenchSpaces(s: string): string {
  return s.replace(/ ([:;?!»])/g, "\u00a0$1").replace(/« /g, "«\u00a0");
}

/** **gras**, *italique*, `code` ; le reste tel quel. */
function inline(s: string): ReactNode[] {
  s = frenchSpaces(s);
  const out: ReactNode[] = [];
  const re = /(\*\*([^*]+?)\*\*|__([^_]+?)__|`([^`]+?)`|\*([^*\s][^*]*?)\*)/g;
  let at = 0;
  let k = 0;
  for (const m of s.matchAll(re)) {
    if (m.index > at) out.push(s.slice(at, m.index));
    if (m[2] !== undefined || m[3] !== undefined) out.push(<strong key={k++}>{inline(m[2] ?? m[3])}</strong>);
    else if (m[4] !== undefined) out.push(<code key={k++}>{m[4]}</code>);
    else out.push(<em key={k++}>{inline(m[5])}</em>);
    at = m.index + m[0].length;
  }
  if (at < s.length) out.push(s.slice(at));
  return out;
}

/** Texte brut d'une réponse (copie dans le presse-papiers). */
export function plainText(src: string): string {
  return src
    .replace(/```[^\n]*\n?/g, "")
    .replace(/\*\*([^*]+?)\*\*/g, "$1")
    .replace(/__([^_]+?)__/g, "$1")
    .replace(/`([^`]+?)`/g, "$1")
    .replace(/(^|[^*])\*([^*\s][^*]*?)\*/g, "$1$2")
    .replace(/^#{1,6}\s+/gm, "")
    .trim();
}
