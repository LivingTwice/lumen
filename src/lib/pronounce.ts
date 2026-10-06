// Prononciation d'un mot ou d'une expression : la voix naturelle locale
// (Supertonic, calculée sur le Mac) si elle est installée, sinon la voix du
// système. La voix naturelle ne tourne qu'à la demande ; chaque prononciation
// est gardée en cache côté natif, et préparée dès qu'un mot est touché.

import { api, isTauri } from "./api";
import { isLinux } from "./platform";
import { t } from "./i18n";
import { useApp } from "./store";
import { sayWord } from "./tts";
import type { LangCode } from "./types";

/** Les 10 voix de Supertonic (identifiant = numéro du locuteur), pour toutes les langues. */
export function naturalVoices() {
  return [
    ...[1, 2, 3, 4, 5].map((n) => ({ id: String(n - 1), name: t(`Féminine ${n}`, `Female ${n}`), group: t("Voix féminines", "Female voices") })),
    ...[1, 2, 3, 4, 5].map((n) => ({ id: String(n + 4), name: t(`Masculine ${n}`, `Male ${n}`), group: t("Voix masculines", "Male voices") })),
  ];
}

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

/** Tour de parole : un mot touché ensuite, ou la leçon qui reprend, fait taire le précédent. */
let turn = 0;

/** Fait taire la prononciation en cours et oublie celles qui se préparent : la leçon reprend la parole. */
export function stopPronunciation() {
  turn++;
  player?.pause();
  if (typeof speechSynthesis !== "undefined") speechSynthesis.cancel();
}

/** Linux : une panne de lecture de la voix naturelle ne s'annonce qu'une fois par séance. */
let voice_failure = false;

/**
 * Prononce un mot ou une expression. `touched` : au toucher d'un mot (et non
 * au clic sur le haut-parleur) ; la voix naturelle abandonne alors sa
 * préparation si un autre mot est touché entre-temps.
 */
export async function pronounce(text: string, lang: LangCode, voiceURI?: string, touched = false) {
  const my = ++turn;
  if (naturalVoiceReady()) {
    try {
      let path: string;
      try {
        path = await request(lang, text, touched);
      } catch (e) {
        if (my !== turn) return;
        // une préparation abandonnée (nouveau mot touché entre-temps) : on la refait
        if (!String(e).includes("interrompu")) throw e;
        path = await request(lang, text, false);
      }
      // un autre mot a été touché, ou la leçon a repris : ce son n'a plus lieu d'être
      if (my !== turn) return;
      if (typeof speechSynthesis !== "undefined") speechSynthesis.cancel();
      player?.pause();
      // Linux (WebKitGTK) : l'élément audio refuse le fichier servi par le
      // protocole des ressources (il y attend des réponses aux requêtes de
      // plage, que ce protocole ne donne pas) ; les octets, eux, se lisent
      // bien : le son passe par un blob
      let url = api().mediaUrl(path);
      if (isLinux) {
        const res = await fetch(url);
        url = URL.createObjectURL(await res.blob());
      }
      player = new Audio(url);
      if (isLinux) player.addEventListener("ended", () => URL.revokeObjectURL(url), { once: true });
      await player.play();
      voice_failure = false;
      return;
    } catch (e) {
      // en cas d'échec, la voix du système prend le relais. Sous Linux, les
      // piles audio varient (speech-dispatcher, PipeWire…) : la raison s'y
      // montre une fois par séance, avec la classe de l'erreur — c'est elle
      // qui dit ce qui a coincé (NotSupportedError, NotAllowedError…)
      if (my !== turn) return;
      if (isLinux && !voice_failure) {
        voice_failure = true;
        const why = e instanceof Error ? e.name : String(e).slice(0, 40);
        useApp.getState().toast(
          t(`La voix naturelle n'a pas pu être lue (${why}) : la voix du système prend le relais.`, `The natural voice couldn't be played (${why}): the system voice takes over.`),
          "error",
        );
      }
    }
  }
  sayWord(text, lang, voiceURI);
}

