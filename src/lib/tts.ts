// Synthèse vocale par les voix du système (macOS : voix Siri et « améliorées »,
// gratuites et hors ligne). Les événements « boundary » donnent la position du
// mot prononcé ; s'ils manquent, on estime la progression au fil du temps.

import { langInfo } from "./langs";

export function ttsAvailable(): boolean {
  return typeof window !== "undefined" && "speechSynthesis" in window;
}

let voicesCache: SpeechSynthesisVoice[] = [];

export function loadVoices(): Promise<SpeechSynthesisVoice[]> {
  if (!ttsAvailable()) return Promise.resolve([]);
  const now = speechSynthesis.getVoices();
  if (now.length) {
    voicesCache = now;
    return Promise.resolve(now);
  }
  return new Promise((resolve) => {
    const done = () => {
      voicesCache = speechSynthesis.getVoices();
      resolve(voicesCache);
    };
    speechSynthesis.addEventListener("voiceschanged", done, { once: true });
    setTimeout(done, 1200);
  });
}

/** Voix disponibles pour une langue, les plus naturelles d'abord. */
export function voicesFor(lang: string, voices = voicesCache): SpeechSynthesisVoice[] {
  const prefs = langInfo(lang).tts;
  const score = (v: SpeechSynthesisVoice) => {
    let s = 0;
    const idx = prefs.findIndex((p) => v.lang.toLowerCase().startsWith(p.toLowerCase()));
    s += idx === -1 ? 0 : 100 - idx * 10;
    if (/premium|enhanced|améliorée|siri|neural|natural/i.test(v.name)) s += 30;
    if (v.localService) s += 5;
    return s;
  };
  return voices
    .filter((v) => v.lang.toLowerCase().startsWith(lang))
    .sort((a, b) => score(b) - score(a));
}

export interface SpeakHandle {
  stop(): void;
  pause(): void;
  resume(): void;
}

export interface SpeakOptions {
  lang: string;
  voiceURI?: string;
  rate: number;
  /** position (unités UTF-16 dans `text`) du mot en cours */
  onWord(charIndex: number): void;
  onEnd(completed: boolean): void;
}

export function speak(text: string, opts: SpeakOptions): SpeakHandle {
  const synth = speechSynthesis;
  synth.cancel();
  const u = new SpeechSynthesisUtterance(text);
  const voices = voicesFor(opts.lang);
  const voice = voices.find((v) => v.voiceURI === opts.voiceURI) ?? voices[0];
  if (voice) u.voice = voice;
  u.lang = voice?.lang ?? langInfo(opts.lang).tts[0];
  u.rate = opts.rate;

  let gotBoundary = false;
  let ended = false;
  let stopped = false;
  let fallbackTimer: number | undefined;
  let startedAt = 0;

  // estimation : ~14 caractères par seconde à vitesse 1
  const starts: number[] = [];
  text.replace(/\S+/g, (m, off: number) => {
    starts.push(off);
    return m;
  });
  const runFallback = () => {
    const elapsed = (performance.now() - startedAt) / 1000;
    const pos = Math.min(text.length - 1, Math.floor(elapsed * 14 * opts.rate));
    let w = 0;
    for (let i = 0; i < starts.length && starts[i] <= pos; i++) w = starts[i];
    opts.onWord(w);
    fallbackTimer = window.setTimeout(runFallback, 120);
  };

  u.onstart = () => {
    startedAt = performance.now();
    window.setTimeout(() => {
      if (!gotBoundary && !ended) runFallback();
    }, 700);
  };
  u.onboundary = (e) => {
    if (e.name && e.name !== "word") return;
    gotBoundary = true;
    if (fallbackTimer) window.clearTimeout(fallbackTimer);
    opts.onWord(e.charIndex);
  };
  const finish = () => {
    if (ended) return;
    ended = true;
    if (fallbackTimer) window.clearTimeout(fallbackTimer);
    opts.onEnd(!stopped);
  };
  u.onend = finish;
  u.onerror = finish;
  synth.speak(u);

  return {
    stop() {
      stopped = true;
      synth.cancel();
      finish();
    },
    pause() {
      synth.pause();
    },
    resume() {
      synth.resume();
    },
  };
}

/** Prononce un mot isolé. */
export function sayWord(word: string, lang: string, voiceURI?: string, rate = 0.9) {
  if (!ttsAvailable()) return;
  speechSynthesis.cancel();
  const u = new SpeechSynthesisUtterance(word);
  const voices = voicesFor(lang);
  const voice = voices.find((v) => v.voiceURI === voiceURI) ?? voices[0];
  if (voice) u.voice = voice;
  u.lang = voice?.lang ?? langInfo(lang).tts[0];
  u.rate = rate;
  speechSynthesis.speak(u);
}
