import { useEffect, useState } from "react";
import { api } from "./api";
import { t } from "./i18n";
import { isIOS } from "./platform";

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
    get desc() {
      return isIOS
        ? t("Rapide sur tous les iPhone, et léger pour la batterie. Traductions justes pour les textes simples.", "Fast on every iPhone, and easy on the battery. Accurate translations for simple texts.")
        : t("Très rapide sur tous les Mac. Traductions justes pour les textes simples.", "Very fast on every Mac. Accurate translations for simple texts.");
    },
    size: 533e6,
  },
  {
    id: "balanced",
    get name() { return t("Équilibré", "Balanced"); },
    llm: "qwen3.5-2b",
    asr: "whisper-turbo",
    rays: 2,
    get desc() {
      return isIOS
        ? t("Le meilleur compromis entre finesse et vitesse, pour les iPhone de 8 Go de mémoire et plus.", "The best trade-off between nuance and speed, for iPhones with 8 GB of memory or more.")
        : t("Le meilleur compromis entre finesse et vitesse. Recommandé.", "The best trade-off between nuance and speed. Recommended.");
    },
    size: 1281e6,
  },
  {
    id: "max",
    name: "Maximum",
    llm: "qwen3.5-4b",
    asr: "whisper-turbo",
    rays: 3,
    get desc() {
      return isIOS
        ? t("Les nuances les plus fines, pour les iPhone de 12 Go de mémoire et les iPad à puce M.", "The finest nuances, for iPhones with 12 GB of memory and iPads with an M chip.")
        : t("Les nuances les plus fines. 16 Go de mémoire conseillés.", "The finest nuances. 16 GB of memory recommended.");
    },
    size: 2741e6,
  },
];

export function profileOf(llm: string): Profile["id"] | null {
  return PROFILES.find((p) => p.llm === llm)?.id ?? null;
}

/**
 * Le profil conseillé pour cet appareil. Sur iPhone et iPad, d'après sa mémoire : iOS ne
 * laisse à une app qu'une partie de celle-ci, et un modèle trop grand se ferait fermer.
 * Sur Mac et PC : « Équilibré ».
 */
export function recommendedProfile(memory: number): Profile["id"] {
  if (!isIOS || !memory) return "balanced";
  const gb = memory / 1024 ** 3;
  if (gb >= 11) return "max";
  if (gb >= 7) return "balanced";
  return "light";
}

/** Le profil conseillé pour cet appareil (lu une fois). */
export function useRecommendedProfile(): Profile["id"] {
  const [id, setId] = useState<Profile["id"]>(isIOS ? "light" : "balanced");
  useEffect(() => {
    if (!isIOS) return;
    void api()
      .deviceInfo()
      .then((d) => setId(recommendedProfile(d.memory)))
      .catch(() => {});
  }, []);
  return id;
}
