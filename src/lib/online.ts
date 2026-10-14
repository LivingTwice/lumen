// IA en ligne, facultative : un fournisseur choisi par l'apprenant, avec sa clé,
// prend le relais de Qwen3.5 sur ce Mac pour la traduction et le chat
// (miroir des fournisseurs de `online.rs`). Whisper et la voix restent sur ce Mac.

import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import { t } from "./i18n";
import { useApp } from "./store";
import type { ChatGptStatus } from "./types";

export type OnlineId = "deepseek" | "gemini" | "nvidia" | "mistral" | "openai" | "chatgpt" | "anthropic" | "openrouter" | "custom";

export interface OnlineProvider {
  id: OnlineId;
  name: string;
  /** lettre de la tuile (pas de logo de marque) */
  mark: string;
  /** une ligne sous le nom */
  note: string;
  /** prix, où vont les demandes */
  about: string;
  /** page où l'on crée sa clé ("" : serveur choisi) */
  keyUrl: string;
  /** réglage qui garde la clé (celle de Gemini sert aussi aux podcasts) ; pour ChatGPT,
   *  l'adresse du compte connecté (les jetons restent hors des réglages, `chatgpt.rs`) */
  keySetting: string;
}

export const onlineProviders = (): OnlineProvider[] => [
  {
    id: "deepseek",
    name: "DeepSeek",
    mark: "D",
    note: t("Le moins cher", "The cheapest"),
    about: t(
      "Très bon marché : moins de 20 centimes pour mille mots traduits, crédit prépayé. Les demandes sont traitées en Chine.",
      "Very cheap: under 20 cents for a thousand translated words, prepaid credit. Requests are processed in China.",
    ),
    keyUrl: "https://platform.deepseek.com/api_keys",
    keySetting: "online_key_deepseek",
  },
  {
    id: "gemini",
    name: "Gemini",
    mark: "G",
    note: t("Gratuit, avec un quota", "Free, within a quota"),
    about: t(
      "La clé gratuite de Google AI Studio, la même que pour les podcasts. Le quota gratuit se renouvelle chaque jour ; avec une clé gratuite, Google peut se servir des demandes pour améliorer ses modèles.",
      "The free Google AI Studio key, the same one as for podcasts. The free quota renews every day; with a free key, Google may use requests to improve its models.",
    ),
    keyUrl: "https://aistudio.google.com/apikey",
    keySetting: "gemini_key",
  },
  {
    id: "nvidia",
    name: "NVIDIA",
    mark: "N",
    note: t("Gratuit, modèles ouverts", "Free, open models"),
    about: t(
      "Gratuit avec un compte développeur NVIDIA, sans carte bancaire : des modèles ouverts (Nemotron, DeepSeek, Kimi, GLM…) servis par NVIDIA. Environ 40 demandes par minute, assez pour lire. Les modèles les plus demandés font parfois attendre longtemps : Lumen conseille un modèle qui répond tout de suite.",
      "Free with an NVIDIA developer account, no credit card: open models (Nemotron, DeepSeek, Kimi, GLM…) served by NVIDIA. About 40 requests a minute, plenty for reading. The most popular models sometimes keep you waiting a long time: Lumen suggests a model that answers right away.",
    ),
    keyUrl: "https://build.nvidia.com/settings/api-keys",
    keySetting: "online_key_nvidia",
  },
  {
    id: "mistral",
    name: "Mistral",
    mark: "M",
    note: t("Français, en Europe", "French, in Europe"),
    about: t(
      "Mistral AI, entreprise française : les demandes restent en Europe. Payé à l'usage, avec une offre d'essai gratuite.",
      "Mistral AI, a French company: requests stay in Europe. Pay as you go, with a free trial tier.",
    ),
    keyUrl: "https://console.mistral.ai/api-keys",
    keySetting: "online_key_mistral",
  },
  {
    id: "openai",
    name: "OpenAI",
    mark: "O",
    note: t("Les modèles de ChatGPT", "ChatGPT's models"),
    about: t(
      "Les modèles de ChatGPT, payés à l'usage sur la plateforme d'OpenAI. C'est un compte à part ; pour utiliser votre abonnement ChatGPT, choisissez plutôt la tuile ChatGPT.",
      "ChatGPT's models, paid as you go on the OpenAI platform. It's a separate account; to use your ChatGPT subscription, pick the ChatGPT tile instead.",
    ),
    keyUrl: "https://platform.openai.com/api-keys",
    keySetting: "online_key_openai",
  },
  {
    id: "chatgpt",
    name: "ChatGPT",
    mark: "◎",
    note: t("Votre abonnement Plus ou Pro", "Your Plus or Pro subscription"),
    about: t(
      "Sans clé : connectez votre compte ChatGPT, et Lumen puise dans votre forfait Plus ou Pro (le même que Codex), dans la limite que vous fixez dans ChatGPT. Les comptes gratuits peuvent se connecter, mais leur forfait ne sert pas aux autres apps. Aperçu d'OpenAI, ouvert aux applications open source comme Lumen.",
      "No key: connect your ChatGPT account, and Lumen draws on your Plus or Pro plan (the same one as Codex), within the limit you set in ChatGPT. Free accounts can sign in, but their plan can't be used by other apps. An OpenAI preview, open to open-source apps like Lumen.",
    ),
    keyUrl: "",
    keySetting: "online_key_chatgpt",
  },
  {
    id: "anthropic",
    name: "Claude",
    mark: "C",
    note: t("Anthropic", "Anthropic"),
    about: t(
      "Les modèles Claude, payés à l'usage sur la console d'Anthropic. C'est un compte à part : l'abonnement Claude n'en fait pas partie. Plus cher que DeepSeek.",
      "Claude models, paid as you go on Anthropic's console. It's a separate account: the Claude subscription isn't included. Pricier than DeepSeek.",
    ),
    keyUrl: "https://console.anthropic.com/settings/keys",
    keySetting: "online_key_anthropic",
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    mark: "R",
    note: t("Tous les modèles, une clé", "Every model, one key"),
    about: t(
      "Des centaines de modèles (DeepSeek, Claude, GPT, Gemini, Llama…) avec une seule clé et un seul crédit.",
      "Hundreds of models (DeepSeek, Claude, GPT, Gemini, Llama…) with a single key and a single credit.",
    ),
    keyUrl: "https://openrouter.ai/settings/keys",
    keySetting: "online_key_openrouter",
  },
  {
    id: "custom",
    name: t("Autre serveur", "Other server"),
    mark: "",
    note: t("Ollama, LM Studio…", "Ollama, LM Studio…"),
    about: t(
      "N'importe quel serveur qui parle comme l'API d'OpenAI : Ollama ou LM Studio sur ce Mac, ou un service en ligne. Son adresse, puis sa clé s'il en demande une.",
      "Any server that speaks like the OpenAI API: Ollama or LM Studio on this Mac, or an online service. Its address, then its key if it asks for one.",
    ),
    keyUrl: "",
    keySetting: "online_key_custom",
  },
];

/**
 * Connexion à ChatGPT de cet appareil, relue à l'affichage. `online_key_chatgpt`
 * (l'adresse du compte) suit la connexion pour le reste de l'interface : jamais un
 * jeton, seulement de quoi dire « connecté ».
 */
export function useChatGpt(): { status: ChatGptStatus | null; refresh(): Promise<ChatGptStatus | null>; set(s: ChatGptStatus): void } {
  const [status, setStatus] = useState<ChatGptStatus | null>(null);
  const setSetting = useApp((s) => s.setSetting);
  const marker = useApp((s) => s.settings.online_key_chatgpt ?? "");
  const set = useCallback(
    (s: ChatGptStatus) => {
      setStatus(s);
      const want = s.signed_in && s.plan && !s.expired ? s.email || "1" : "";
      if (want !== marker) void setSetting("online_key_chatgpt", want);
    },
    [marker, setSetting],
  );
  const refresh = useCallback(async () => {
    try {
      const s = await api().chatgptStatus();
      set(s);
      return s;
    } catch {
      return null;
    }
  }, [set]);
  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return { status, refresh, set };
}

export const onlineProvider = (id: string | undefined): OnlineProvider => onlineProviders().find((p) => p.id === id) ?? onlineProviders()[0];

export interface OnlineState {
  /** l'apprenant a choisi l'IA en ligne */
  on: boolean;
  provider: OnlineProvider;
  /** clé (ou adresse du serveur) présente */
  ready: boolean;
  /** traduction des mots et des phrases en ligne */
  words: boolean;
  /** chat et Simplifier en ligne */
  chat: boolean;
  model: string;
}

/** Ce qui part en ligne, d'après les réglages (miroir de `online::config_from`). */
export function onlineState(s: Record<string, string>): OnlineState {
  const on = s.online_on === "1";
  const provider = onlineProvider(s.online_provider);
  const ready = provider.id === "custom" ? !!s.online_url?.trim() : !!s[provider.keySetting]?.trim();
  return {
    on,
    provider,
    ready,
    words: on && s.online_words !== "0",
    chat: on && s.online_chat !== "0",
    model: s[`online_model_${provider.id}`] ?? "",
  };
}

export function useOnline(): OnlineState {
  const settings = useApp((s) => s.settings);
  return onlineState(settings);
}

/** « 1,2 s », « 640 ms » */
export function latency(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const s = (ms / 1000).toFixed(1);
  return t(`${s.replace(".", ",")} s`, `${s} s`);
}
