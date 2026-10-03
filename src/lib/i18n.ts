/**
 * Langue de l'interface : français ou anglais (réglage `ui_lang`). Elle décide
 * aussi de la langue des traductions de l'IA, du chat et des dictionnaires.
 *
 * Chaque texte visible s'écrit dans les deux langues, côte à côte :
 *   t("Terminer la page", "Finish page")
 * Le texte reste à sa place dans le code, et une traduction oubliée se voit.
 * Changer de langue remonte toute l'interface (clé de `App`) : les textes
 * calculés au rendu suffisent, pas besoin d'abonnement.
 */
export type UiLang = "fr" | "en";

let current: UiLang = typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("fr") ? "fr" : "en";

export function uiLang(): UiLang {
  return current;
}

export function isEn(): boolean {
  return current === "en";
}

export function setUiLang(l: UiLang) {
  current = l;
  document.documentElement.lang = l;
}

/** Texte de l'interface dans la langue choisie. */
export function t(fr: string, en: string): string {
  return current === "en" ? en : fr;
}

/** Mot accordé au nombre (en français, 0 et 1 restent au singulier). */
export function pick(n: number, frOne: string, frMany: string, enOne: string, enMany: string): string {
  return current === "en" ? (n === 1 ? enOne : enMany) : n > 1 ? frMany : frOne;
}

/** Nombre suivi de son nom accordé : count(3, "leçon", "leçons", "lesson", "lessons") → « 3 leçons ». */
export function count(n: number, frOne: string, frMany: string, enOne: string, enMany: string): string {
  return `${formatNumber(n)} ${pick(n, frOne, frMany, enOne, enMany)}`;
}

/** Locale des dates et des nombres (anglais : celle du Mac si elle est anglaise, « en-GB » par exemple). */
export function locale(): string {
  if (current === "fr") return "fr-FR";
  const nav = typeof navigator !== "undefined" ? navigator.language : "";
  return nav && nav.toLowerCase().startsWith("en") ? nav : "en-US";
}

export function formatNumber(n: number): string {
  return n.toLocaleString(locale());
}

/** Langue proposée au premier lancement : celle du Mac (français ou anglais, sinon anglais). */
export function systemUiLang(): UiLang {
  const prefs = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const p of prefs) {
    const c = (p || "").slice(0, 2).toLowerCase();
    if (c === "fr" || c === "en") return c;
  }
  return "en";
}
