import { t } from "./i18n";

export interface Profile {
  id: "light" | "balanced" | "max";
  name: string;
  llm: string;
  asr: string;
  rays: number;
  desc: string;
  size: number;
}

export const PROFILES: Profile[] = [
  {
    id: "light",
    get name() { return t("Léger", "Light"); },
    llm: "qwen3.5-0.8b",
    asr: "whisper-small",
    rays: 1,
    get desc() { return t("Très rapide sur tous les Mac. Traductions justes pour les textes simples.", "Very fast on every Mac. Accurate translations for simple texts."); },
    size: 533e6,
  },
  {
    id: "balanced",
    get name() { return t("Équilibré", "Balanced"); },
    llm: "qwen3.5-2b",
    asr: "whisper-turbo",
    rays: 2,
    get desc() { return t("Le meilleur compromis entre finesse et vitesse. Recommandé.", "The best trade-off between nuance and speed. Recommended."); },
    size: 1281e6,
  },
  {
    id: "max",
    name: "Maximum",
    llm: "qwen3.5-4b",
    asr: "whisper-turbo",
    rays: 3,
    get desc() { return t("Les nuances les plus fines. 16 Go de mémoire conseillés.", "The finest nuances. 16 GB of memory recommended."); },
    size: 2741e6,
  },
];

export function profileOf(llm: string): Profile["id"] | null {
  return PROFILES.find((p) => p.llm === llm)?.id ?? null;
}
