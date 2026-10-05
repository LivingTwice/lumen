// IA en ligne, facultative : un fournisseur choisi par l'apprenant, avec sa clé,
// prend le relais de Qwen3.5 sur ce Mac pour la traduction et le chat
// (miroir des fournisseurs de `online.rs`). Whisper et la voix restent sur ce Mac.

import { t } from "./i18n";
import { useApp } from "./store";

export type OnlineId = "deepseek" | "gemini" | "mistral" | "openai" | "anthropic" | "openrouter" | "custom";

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
  /** réglage qui garde la clé (celle de Gemini sert aussi aux podcasts) */
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
      "Les modèles de ChatGPT, payés à l'usage sur la plateforme d'OpenAI. C'est un compte à part : l'abonnement ChatGPT n'en fait pas partie.",
      "ChatGPT's models, paid as you go on the OpenAI platform. It's a separate account: the ChatGPT subscription isn't included.",
    ),
    keyUrl: "https://platform.openai.com/api-keys",
    keySetting: "online_key_openai",
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
