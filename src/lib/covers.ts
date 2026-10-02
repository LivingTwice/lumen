// Couvertures des leçons : miniature YouTube, image choisie, ou œuvre
// générée (voir components/Cover.tsx).
import { api, isTauri } from "./api";
import { pickFiles } from "./dialogs";

/** Identifiant d'une vidéo YouTube dans une adresse (watch, youtu.be, shorts, embed, live). */
export function youtubeId(url: string): string | null {
  const m = url.match(/(?:youtu\.be\/|youtube(?:-nocookie)?\.com\/(?:watch\?(?:.*&)?v=|embed\/|shorts\/|live\/|v\/))([\w-]{11})/);
  return m ? m[1] : null;
}

/** Miniatures YouTube, de la meilleure à la plus sûre. */
export function youtubeThumbs(id: string): string[] {
  return [`https://i.ytimg.com/vi/${id}/maxresdefault.jpg`, `https://i.ytimg.com/vi/${id}/hqdefault.jpg`];
}

const IMAGE_EXT = ["jpg", "jpeg", "png", "webp", "gif", "heic", "heif", "avif", "bmp", "tif", "tiff"];
const MIME: Record<string, string> = {
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  png: "image/png",
  webp: "image/webp",
  gif: "image/gif",
  heic: "image/heic",
  heif: "image/heif",
  avif: "image/avif",
  bmp: "image/bmp",
  tif: "image/tiff",
  tiff: "image/tiff",
};

/** Ouvre le sélecteur d'images (natif dans l'app, navigateur sinon). */
export async function pickCoverImage(): Promise<Blob | null> {
  if (isTauri) {
    const [path] = await pickFiles([{ name: "Images", extensions: IMAGE_EXT }], false);
    if (!path) return null;
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    const buf = await api().readFile(path);
    return new Blob([buf], { type: MIME[ext] ?? "" });
  }
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.style.display = "none";
    input.onchange = () => {
      resolve(input.files?.[0] ?? null);
      input.remove();
    };
    document.body.appendChild(input);
    input.click();
  });
}

/** Côté le plus long d'une couverture : net sur écran Retina, léger sur le disque. */
const MAX_SIDE = 1280;

/** Réduit l'image et la convertit en JPEG (un fond crème remplace la transparence). */
export async function prepareCover(blob: Blob): Promise<Uint8Array> {
  const url = URL.createObjectURL(blob);
  try {
    const img = new Image();
    img.src = url;
    await img.decode();
    const scale = Math.min(1, MAX_SIDE / Math.max(img.naturalWidth, img.naturalHeight));
    const w = Math.max(1, Math.round(img.naturalWidth * scale));
    const h = Math.max(1, Math.round(img.naturalHeight * scale));
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas");
    ctx.fillStyle = "#f3ede3";
    ctx.fillRect(0, 0, w, h);
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(img, 0, 0, w, h);
    const out = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/jpeg", 0.88));
    if (!out) throw new Error("jpeg");
    return new Uint8Array(await out.arrayBuffer());
  } catch {
    throw "Cette image ne peut pas être lue. Essayez un fichier JPEG ou PNG.";
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Choisit une image et l'enregistre comme couverture. Renvoie faux si annulé. */
export async function chooseCover(lessonId: number): Promise<boolean> {
  const blob = await pickCoverImage();
  if (!blob) return false;
  const bytes = await prepareCover(blob);
  await api().lessonSetCover(lessonId, bytes, "jpg");
  return true;
}

// grain photographique partagé par toutes les couvertures générées
let grainUrl: string | null = null;
export function grainTexture(): string {
  if (grainUrl) return grainUrl;
  const c = document.createElement("canvas");
  c.width = c.height = 140;
  const ctx = c.getContext("2d");
  if (!ctx) return "";
  const data = ctx.createImageData(140, 140);
  for (let i = 0; i < data.data.length; i += 4) {
    const v = Math.random() * 255;
    data.data[i] = data.data[i + 1] = data.data[i + 2] = v;
    data.data[i + 3] = 255;
  }
  ctx.putImageData(data, 0, 0);
  grainUrl = c.toDataURL("image/png");
  return grainUrl;
}
