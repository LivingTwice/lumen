import { isLinux } from "./platform";

/**
 * L'image muette rejoint le son qui la mène. Sur Mac, un léger écart se corrige par la
 * vitesse ; sous Linux (WebKitGTK), changer la vitesse d'une vidéo en lecture la vide et
 * la relit : une demi-seconde d'arrêt à chaque fois. L'image y rattrape donc par un saut,
 * et pas plus d'une fois toutes les deux secondes. Renvoie vrai si elle a sauté.
 * `lastSeek` : `{ at: 0 }`, gardé d'une image d'animation à l'autre.
 */
export function followAudio(video: HTMLVideoElement, master: number, rate: number, lastSeek: { at: number }): boolean {
  const d = video.currentTime - master;
  if (Math.abs(d) > 0.35 && (!isLinux || performance.now() - lastSeek.at > 2000)) {
    video.currentTime = master;
    video.playbackRate = rate;
    lastSeek.at = performance.now();
    return true;
  }
  if (Math.abs(d) > 0.06 && !isLinux) video.playbackRate = rate * (d > 0 ? 0.96 : 1.04);
  else if (video.playbackRate !== rate) video.playbackRate = rate;
  return false;
}
