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
  { id: "light", name: "Léger", llm: "qwen3.5-0.8b", asr: "whisper-small", rays: 1, desc: "Très rapide sur tous les Mac. Traductions justes pour les textes simples.", size: 533e6 },
  { id: "balanced", name: "Équilibré", llm: "qwen3.5-2b", asr: "whisper-turbo", rays: 2, desc: "Le meilleur compromis entre finesse et vitesse. Recommandé.", size: 1281e6 },
  { id: "max", name: "Maximum", llm: "qwen3.5-4b", asr: "whisper-turbo", rays: 3, desc: "Les nuances les plus fines. 16 Go de mémoire conseillés.", size: 2741e6 },
];

export function profileOf(llm: string): Profile["id"] | null {
  return PROFILES.find((p) => p.llm === llm)?.id ?? null;
}
