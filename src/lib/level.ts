// Niveau estimé d'après les mots connus (miroir de level.rs). Le natif regroupe
// les formes connues par lemme grâce aux dictionnaires ; ici, le repli approché
// (formes par lemme typiques de chaque langue) en attendant sa réponse.
import type { LangCode, LevelEstimate } from "./types";

/** Lemmes connus au début de A2, B1, B2, C1 (mêmes seuils pour toutes les langues). */
const LEMMAS = [1000, 2200, 4000, 7000];
/** Kanji connus (japonais). */
const KANJI = [120, 350, 700, 1100];
/** Syllabes connues (vietnamien). */
const SYLLABLES = [400, 900, 1600, 2500];

/** Formes par lemme, en moyenne, dans un vocabulaire d'apprenant (miroir de `level::forms_per_lemma`). */
export function formsPerLemma(lang: LangCode): number {
  if (["en", "id", "hi"].includes(lang)) return 1.25;
  if (["es", "it", "pt", "fr", "nl", "sv", "da"].includes(lang)) return 1.4;
  if (["de", "ro", "bg"].includes(lang)) return 1.5;
  if (["el", "ar", "ko"].includes(lang)) return 1.8;
  if (["pl", "cs", "sk", "sl", "hr", "uk", "ru", "lv", "lt"].includes(lang)) return 2.2;
  if (["fi", "et", "hu", "tr"].includes(lang)) return 2.5;
  return 1;
}

export function placeLevel(units: number, unit: LevelEstimate["unit"]): Pick<LevelEstimate, "level" | "floor" | "next"> {
  const steps = unit === "kanji" ? KANJI : unit === "syllables" ? SYLLABLES : LEMMAS;
  const reached = steps.filter((s) => units >= s).length;
  return { level: reached + 1, floor: reached ? steps[reached - 1] : 0, next: steps[reached] ?? 0 };
}

/** Estimation approchée, sans dictionnaire (japonais : on ne connaît pas les kanji ici). */
export function roughEstimate(lang: LangCode, forms: number): LevelEstimate {
  const unit: LevelEstimate["unit"] = lang === "ja" ? "kanji" : lang === "vi" ? "syllables" : "lemmas";
  const units = unit === "lemmas" ? Math.round(forms / formsPerLemma(lang)) : unit === "kanji" ? Math.round(forms * 0.35) : forms;
  return { forms, units, unit, exact: false, ...placeLevel(units, unit) };
}
