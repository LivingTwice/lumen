import data from "../changelog.json";
import type { IconName } from "../components/Icon";
import { locale, t } from "./i18n";
import { useApp, type ImportTab, type SettingsTab, type View } from "./store";

/**
 * Nouveautés de chaque version (src/changelog.json). Après une mise à jour,
 * celles des versions pas encore vues s'affichent d'elles-mêmes (réglage
 * `seen_version`). La version "next" est celle en préparation : le script de
 * publication lui donne son numéro ; d'ici là, elle compte comme la version installée.
 */

interface Text {
  fr: string;
  en: string;
}

export interface NewsItem {
  icon: IconName;
  title: Text;
  body: Text;
  action?: string;
}

export interface Release {
  version: string;
  date: string;
  title: Text;
  items: NewsItem[];
}

export const RELEASES = (data as { releases: Release[] }).releases;

/** Texte dans la langue de l'interface. */
export function tx(x: Text): string {
  return t(x.fr, x.en);
}

/** « 0.3.0 (aperçu navigateur) » → [0, 3, 0] */
function parts(v: string): number[] | null {
  const m = v.match(/(\d+)\.(\d+)\.(\d+)/);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** Comparaison de deux versions (négatif : a avant b). */
export function compareVersions(a: string, b: string): number {
  const x = parts(a) ?? [0, 0, 0];
  const y = parts(b) ?? [0, 0, 0];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}

/** Numéro d'une version du journal : "next" prend celui de l'app installée. */
export function versionOf(r: Release, current: string): string {
  return r.version === "next" ? (parts(current)?.join(".") ?? r.version) : r.version;
}

/**
 * Versions dont les nouveautés restent à montrer, la plus récente d'abord :
 * celles d'après la dernière vue, jusqu'à la version installée. Sans version vue
 * (Lumen d'avant les nouveautés), seulement celles de la version installée.
 */
export function unseenReleases(current: string, seen: string): Release[] {
  if (!parts(current)) return [];
  return RELEASES.filter((r) => {
    if (!r.items.length) return false;
    const v = versionOf(r, current);
    if (compareVersions(v, current) > 0) return false;
    return seen ? compareVersions(v, seen) > 0 : compareVersions(v, current) === 0;
  });
}

/** Ce que fait le bouton d'une nouveauté : ouvrir une page, la visite, le guide. */
export function runAction(action: string) {
  const app = useApp.getState();
  if (action === "tour") void app.startTour();
  else if (action === "guide") app.openGuide();
  else if (action.startsWith("settings:")) app.openSettings(action.slice(9) as SettingsTab);
  else if (action.startsWith("import:")) app.openImport(null, action.slice(7) as ImportTab);
  else if (action.startsWith("view:")) {
    const view = action.slice(5) as View;
    if (view === "playlists") app.openPlaylist(null);
    else app.go(view);
  }
}

/** Libellé du bouton d'une nouveauté. */
export function actionLabel(action: string): string {
  if (action === "tour") return t("Lancer la visite", "Start the tour");
  if (action === "guide") return t("Ouvrir le guide", "Open the guide");
  if (action.startsWith("settings:")) return t("Voir le réglage", "See the setting");
  return t("Essayer", "Try it");
}

/** Date d'une version, dans la langue de l'interface (« 4 octobre 2026 »). */
export function releaseDate(r: Release): string {
  if (!r.date) return "";
  const d = new Date(`${r.date}T12:00:00`);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString(locale(), { day: "numeric", month: "long", year: "numeric" });
}
