import type { ModelRow } from "./types";

/** Langues où Qwen3-ASR écrit le texte (miroir de `asr::writes`). */
const QWEN = new Set(["en", "it", "de", "pt", "ru", "es", "fr", "nl", "tr", "ar", "hi", "id", "vi", "ko", "ja"]);

/** Langues où Parakeet écrit le texte (miroir de `parakeet::knows`). */
const PARAKEET = new Set(["en", "it", "de", "pt", "ru", "es", "fr", "nl", "pl", "cs", "hu", "et", "lt", "sk", "sl", "hr", "bg", "uk"]);

/** Langues où Parakeet passe avant Qwen3-ASR (miroir de `parakeet::before_qwen`). */
const PARAKEET_FIRST = new Set(["it", "nl"]);

/**
 * Le modèle qui écrit le texte des transcriptions dans cette langue, s'il est
 * installé (miroir de `text_model`, commands.rs) ; aucun : Whisper écrit seul.
 */
export function writerFor(models: ModelRow[], lang: string): ModelRow | undefined {
  const ready = (id: string) => models.find((m) => m.id === id && m.installed);
  const qwen = QWEN.has(lang) ? ready("qwen3-asr-1.7b") : undefined;
  const parakeet = PARAKEET.has(lang) ? ready("parakeet-v3") : undefined;
  return PARAKEET_FIRST.has(lang) ? (parakeet ?? qwen) : (qwen ?? parakeet);
}

/** Nom court d'un modèle de transcription : « Qwen3-ASR », « Parakeet ». */
export const shortName = (m: ModelRow) => m.name.split(" ")[0];
