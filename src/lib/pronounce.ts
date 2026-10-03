// Prononciation d'un mot ou d'une expression : la voix naturelle locale
// (Supertonic, calculée sur le Mac) si elle est installée, sinon la voix du
// système. La voix naturelle ne tourne qu'à la demande ; chaque prononciation
// est gardée en cache côté natif, et préparée dès qu'un mot est touché.

import { api, isTauri } from "./api";
import { useApp } from "./store";
import { sayWord } from "./tts";
import type { LangCode } from "./types";

/** Les 10 voix de Supertonic (identifiant = numéro du locuteur), pour toutes les langues. */
export const NATURAL_VOICES = [
  ...[1, 2, 3, 4, 5].map((n) => ({ id: String(n - 1), name: `Féminine ${n}`, group: "Voix féminines" })),
  ...[1, 2, 3, 4, 5].map((n) => ({ id: String(n + 4), name: `Masculine ${n}`, group: "Voix masculines" })),
];

/** Voix naturelle choisie pour une langue (même logique que le natif). */
export function naturalVoiceFor(lang: string): string {
  const s = useApp.getState().settings;
  const v = s[`tts_voice_${lang}`] ?? s.tts_voice ?? "0";
  return v === "m" ? "5" : v === "f" ? "0" : v;
}

/** Au-delà, un passage n'est pas préparé d'avance (il ne l'est qu'au clic). */
const PREPARE_MAX_WORDS = 12;

const pending = new Map<string, Promise<string>>();
let player: HTMLAudioElement | null = null;

/** La voix naturelle est installée et utilisable. */
export function naturalVoiceReady(): boolean {
  return isTauri && useApp.getState().models.some((m) => m.kind === "tts" && m.installed);
}

function keyOf(lang: LangCode, text: string): string {
  return `${lang}|${naturalVoiceFor(lang)}|${text.replace(/\s+/g, " ").trim()}`;
}

function request(lang: LangCode, text: string, prefetch: boolean): Promise<string> {
  const k = keyOf(lang, text);
  const known = pending.get(k);
  if (known) return known;
  const p = api()
    .ttsSay(lang, text, prefetch)
    .catch((e) => {
      pending.delete(k);
      throw e;
    });
  pending.set(k, p);
  // garde la mémoire courte : les plus anciennes demandes s'oublient
  if (pending.size > 300) pending.delete(pending.keys().next().value!);
  return p;
}

/** Prépare la prononciation en arrière-plan, pour qu'elle soit immédiate au clic. */
export function preparePronunciation(text: string, lang: LangCode) {
  if (!naturalVoiceReady() || text.split(/\s+/).length > PREPARE_MAX_WORDS) return;
  request(lang, text, true).catch(() => {});
}

/** Prononce un mot ou une expression. */
export async function pronounce(text: string, lang: LangCode, voiceURI?: string) {
  if (naturalVoiceReady()) {
    try {
      let path: string;
      try {
        path = await request(lang, text, false);
      } catch {
        // une préparation abandonnée (nouveau mot touché entre-temps) : on la refait
        path = await request(lang, text, false);
      }
      if (typeof speechSynthesis !== "undefined") speechSynthesis.cancel();
      player?.pause();
      player = new Audio(api().mediaUrl(path));
      await player.play();
      return;
    } catch {
      // en cas d'échec, la voix du système prend le relais
    }
  }
  sayWord(text, lang, voiceURI);
}
