// Profil de l'apprenant : un nom (prénom ou pseudo), un avatar, ce qui le
// motive et ses centres d'intérêt. Rien de communautaire : il vit dans les
// réglages de la base, sur ce Mac, et voyage avec la sauvegarde iCloud.
// Seul le chat local le lit (miroir des clés dans src-tauri/src/user.rs).
import { useMemo } from "react";
import { pickCoverImage } from "./covers";
import { t } from "./i18n";
import { useApp } from "./store";

/** Avatar : l'initiale du nom, une lumière générée (aube, halo, aurore, nuit) ou une photo. */
export type AvatarStyle = "initial" | "dawn" | "halo" | "aurora" | "night" | "photo";

export interface AvatarSpec {
  style: AvatarStyle;
  hue: number;
  /** graine de la lumière générée : « Une autre » en tire une nouvelle */
  seed: number;
}

export interface UserProfile {
  name: string;
  avatar: AvatarSpec;
  /** photo réduite, en data URL (vide : pas de photo) */
  photo: string;
  why: string;
  /** centres d'intérêt : « #cuisine » pour une suggestion, sinon le texte tapé */
  interests: string[];
  /** rien n'a encore été choisi : l'avatar reste une silhouette */
  empty: boolean;
}

export const NAME_MAX = 32;
export const WHY_MAX = 140;
export const INTEREST_MAX = 30;
export const INTERESTS_MAX = 8;
/** Côté de la photo enregistrée : nette jusqu'à 128 points sur écran Retina, légère dans la base. */
const PHOTO_SIDE = 256;

const STYLES: AvatarStyle[] = ["initial", "dawn", "halo", "aurora", "night", "photo"];

/** Teintes proposées, les chaudes d'abord. */
export const HUES = [34, 16, 350, 318, 268, 205, 165, 92];

/** Styles proposés, dans l'ordre du choix. */
export const avatarStyles = (): { id: AvatarStyle; label: string }[] => [
  { id: "initial", label: t("Initiale", "Initial") },
  { id: "dawn", label: t("Aube", "Dawn") },
  { id: "halo", label: "Halo" },
  { id: "aurora", label: t("Aurore", "Aurora") },
  { id: "night", label: t("Nuit", "Night") },
];

/** Suggestions de centres d'intérêt (miroir de `INTERESTS` dans user.rs). */
export const interestSuggestions = (): { id: string; label: string }[] => [
  { id: "#cooking", label: t("Cuisine", "Cooking") },
  { id: "#travel", label: t("Voyages", "Travel") },
  { id: "#history", label: t("Histoire", "History") },
  { id: "#film", label: t("Cinéma et séries", "Film and TV") },
  { id: "#music", label: t("Musique", "Music") },
  { id: "#books", label: t("Littérature", "Books") },
  { id: "#sport", label: t("Sport", "Sports") },
  { id: "#science", label: t("Sciences", "Science") },
  { id: "#nature", label: t("Nature", "Nature") },
  { id: "#art", label: t("Art et design", "Art and design") },
  { id: "#tech", label: t("Technologie", "Technology") },
  { id: "#news", label: t("Actualité", "Current affairs") },
  { id: "#games", label: t("Jeux vidéo", "Video games") },
  { id: "#work", label: t("Travail et économie", "Work and business") },
  { id: "#philosophy", label: t("Philosophie", "Philosophy") },
  { id: "#health", label: t("Santé et bien-être", "Health and wellbeing") },
];

/** Libellé d'un centre d'intérêt, dans la langue de l'interface. */
export function interestLabel(id: string): string {
  return interestSuggestions().find((x) => x.id === id)?.label ?? id;
}

/** Une seule ligne, sans espaces superflus, coupée à `max` caractères. */
export function oneLine(s: string, max: number): string {
  return Array.from(s.replace(/\s+/g, " ").trimStart()).slice(0, max).join("");
}

/** Première lettre du nom, en majuscule (un émoji ou un idéogramme reste entier). */
export function initialOf(name: string): string {
  const s = name.trim();
  if (!s) return "";
  const seg = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter().segment(s)[Symbol.iterator]().next().value?.segment : undefined;
  return (seg ?? Array.from(s)[0]).toLocaleUpperCase();
}

/** Teinte tirée du nom, parmi les chaudes (jamais de bleu d'office) : la même à chaque fois, tant qu'on n'en a pas choisi. */
export function hueOf(name: string): number {
  let h = 0;
  for (const c of name.trim().toLowerCase()) h = (Math.imul(h, 31) + c.codePointAt(0)!) | 0;
  return HUES[Math.abs(h) % 4];
}

/** « dawn:34:1234 » → avatar ; vide ou illisible : l'initiale, teinte tirée du nom. */
export function parseAvatar(s: string, name: string): AvatarSpec {
  const [style, hue, seed] = (s || "").split(":");
  return {
    style: STYLES.includes(style as AvatarStyle) ? (style as AvatarStyle) : "initial",
    hue: Number.isFinite(Number(hue)) && hue !== "" ? Number(hue) : hueOf(name),
    seed: Number(seed) || 0,
  };
}

export function avatarString(a: AvatarSpec): string {
  return `${a.style}:${Math.round(a.hue)}:${a.seed}`;
}

export function newSeed(): number {
  return 1 + Math.floor(Math.random() * 1e9);
}

function parseInterests(s: string): string[] {
  try {
    const v = JSON.parse(s || "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && !!x.trim()).slice(0, INTERESTS_MAX) : [];
  } catch {
    return [];
  }
}

export function interestsString(list: string[]): string {
  return list.length ? JSON.stringify(list.slice(0, INTERESTS_MAX)) : "";
}

/** Profil lu dans les réglages (ceux d'une sauvegarde aussi). */
export function userFrom(name: string, avatar: string, photo: string, why = "", interests = ""): UserProfile {
  const spec = parseAvatar(avatar, name);
  // la photo retirée ou perdue : l'initiale reprend sa place
  if (spec.style === "photo" && !photo) spec.style = "initial";
  return {
    name: name.trim(),
    avatar: spec,
    photo: spec.style === "photo" ? photo : "",
    why: why.trim(),
    interests: parseInterests(interests),
    empty: !name.trim() && !avatar,
  };
}

/** Le profil de l'apprenant, à jour à chaque changement. */
export function useUser(): UserProfile {
  const name = useApp((s) => s.settings.user_name ?? "");
  const avatar = useApp((s) => s.settings.user_avatar ?? "");
  const photo = useApp((s) => s.settings.user_photo ?? "");
  const why = useApp((s) => s.settings.user_why ?? "");
  const interests = useApp((s) => s.settings.user_interests ?? "");
  return useMemo(() => userFrom(name, avatar, photo, why, interests), [name, avatar, photo, why, interests]);
}

/** Le nom seul (« Bonsoir, Ulysse. »). */
export function useUserName(): string {
  return useApp((s) => (s.settings.user_name ?? "").trim());
}

/**
 * Choisit une photo, la recadre au carré (au centre) et la réduit. Renvoie
 * une data URL JPEG, ou null si l'on annule.
 */
export async function choosePhoto(): Promise<string | null> {
  const blob = await pickCoverImage();
  if (!blob) return null;
  const url = URL.createObjectURL(blob);
  try {
    const img = new Image();
    img.src = url;
    await img.decode();
    const side = Math.min(img.naturalWidth, img.naturalHeight);
    if (!side) throw new Error("vide");
    // un portrait garde le haut du cadre : c'est là qu'est le visage
    const sx = (img.naturalWidth - side) / 2;
    const sy = img.naturalHeight > img.naturalWidth ? Math.min((img.naturalHeight - side) / 2, img.naturalHeight * 0.12) : 0;
    const out = Math.min(PHOTO_SIDE, side);
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = out;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas");
    ctx.fillStyle = "#f3ede3";
    ctx.fillRect(0, 0, out, out);
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(img, sx, sy, side, side, 0, 0, out, out);
    return canvas.toDataURL("image/jpeg", 0.88);
  } catch {
    throw t("Cette image ne peut pas être lue. Essayez un fichier JPEG ou PNG.", "This image can't be read. Try a JPEG or PNG file.");
  } finally {
    URL.revokeObjectURL(url);
  }
}
