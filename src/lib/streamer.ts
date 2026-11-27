import type { MediaSegments, MediaTrack } from "./types";

/**
 * Linux : une vidéo lue par segments depuis l'interface, comme le lecteur de youtube.com.
 * L'interface prend chaque segment du flux (par Lumen, qui le demande au site) et le donne au
 * moteur (Media Source Extensions), en gardant de l'avance sur la lecture.
 *
 * Pourquoi : donné à l'élément vidéo, un fichier distant est téléchargé entier par le moteur
 * de WebKitGTK ; au démarrage, son tampon de téléchargement laisse parfois une lecture en
 * suspens jusqu'à la fin du fichier, et l'image ne vient jamais. Ici le moteur reçoit des
 * données déjà découpées : rien à télécharger de son côté, et il garde l'image et le son
 * ensemble dans un seul élément.
 */

/** Secondes de flux gardées devant la lecture. */
const AHEAD = 30;
/** Secondes gardées derrière elle ; au-delà, la mémoire est rendue au moteur. */
const BEHIND = 60;

/** Une lecture par segments ; arrêtée quand l'aperçu se ferme. */
export interface Streamer {
  stop(): void;
}

/** Ce que le lecteur veut savoir. */
export interface StreamerEvents {
  /** le flux n'a pas pu être lu */
  error(error: unknown): void;
}

/** Une piste (image ou son) et son tampon dans le moteur. */
interface Lane {
  track: MediaTrack;
  buffer: SourceBuffer;
  /** indice, dans la table, du prochain segment à donner */
  next: number;
  /** instant de début du dernier segment donné (-1 : aucun) */
  tail: number;
  /** tous les segments ont été donnés */
  done: boolean;
  /** un échange avec le moteur ou le site est en cours */
  busy: boolean;
}

const mime = (kind: "video" | "audio", codec: string) => `${kind}/mp4; codecs="${codec}"`;

/** Le moteur sait-il lire ces deux pistes par segments ? */
export function canStream(segments: MediaSegments): boolean {
  return (
    typeof MediaSource !== "undefined" &&
    !!segments.video.codec &&
    !!segments.audio.codec &&
    MediaSource.isTypeSupported(mime("video", segments.video.codec)) &&
    MediaSource.isTypeSupported(mime("audio", segments.audio.codec))
  );
}

/** L'erreur du moteur quand il n'a plus de place. */
const isQuota = (e: unknown) => e instanceof DOMException && e.name === "QuotaExceededError";

/** Le temps que la condition soit vraie. */
const untilTrue = async (cond: () => boolean) => {
  while (!cond()) await new Promise((r) => setTimeout(r, 20));
};

/**
 * Lit `segments` dans `video` (un seul élément : le moteur garde l'image et le son
 * ensemble). `duration` : la durée annoncée par le site, pour la barre de position.
 */
export function startStreamer(video: HTMLVideoElement, segments: MediaSegments, duration: number, on: StreamerEvents): Streamer {
  let media: MediaSource | null = null;
  let lanes: Lane[] = [];
  let objectUrl = "";
  let stopped = false;
  // chaque saut, chaque redémarrage change de génération : ce qui était en vol s'ignore
  let generation = 0;
  let abort = new AbortController();
  // le montage change : rien ne reprend tant que les pistes ne sont pas libres
  let switching = false;
  const total = duration > 0 ? duration : Infinity;

  /** Les octets d'une plage du fichier, demandés par Lumen. */
  const bytes = async (track: MediaTrack, start: number, end: number, signal: AbortSignal): Promise<Uint8Array<ArrayBuffer>> => {
    const r = await fetch(track.url, { headers: { Range: `bytes=${start}-${end}` }, signal });
    if (r.status !== 206 && r.status !== 200) throw new Error(`HTTP ${r.status}`);
    return new Uint8Array(await r.arrayBuffer());
  };

  /** Donne des octets au tampon d'une piste ; résolu quand le moteur les a pris en compte. */
  const append = (lane: Lane, data: Uint8Array<ArrayBuffer>) =>
    new Promise<void>((resolve, reject) => {
      const b = lane.buffer;
      // le code 5 de MediaError : plus de place
      const fail = () => reject(video.error?.code === 5 ? new DOMException("plein", "QuotaExceededError") : new Error("tampon refusé par le moteur"));
      b.addEventListener("updateend", () => {
        b.removeEventListener("error", fail);
        resolve();
      }, { once: true });
      b.addEventListener("error", fail, { once: true });
      try {
        b.appendBuffer(data);
      } catch (e) {
        b.removeEventListener("error", fail);
        reject(e);
      }
    });

  /** Rend au moteur ce qui est reçu avant `keepFrom`. */
  const release = (lane: Lane, keepFrom: number) =>
    new Promise<void>((resolve) => {
      const b = lane.buffer;
      if (b.updating || !b.buffered.length || b.buffered.start(0) >= keepFrom) return resolve();
      b.addEventListener("updateend", () => resolve(), { once: true });
      b.remove(0, keepFrom);
    });

  /** La fin de la plage reçue qui contient `at` ; `at` lui-même si rien n'y est reçu. */
  const receivedUntil = (lane: Lane, at: number): number => {
    const ranges = lane.buffer.buffered;
    for (let i = 0; i < ranges.length; i++) if (ranges.start(i) <= at && at < ranges.end(i)) return ranges.end(i);
    return at;
  };

  /** L'indice du segment qui contient `at` (ou du premier qui suit). */
  const indexAt = (track: MediaTrack, at: number): number => {
    const table = track.segments;
    let i = 0;
    while (i < table.length && table[i].time + table[i].duration <= at) i++;
    return i;
  };

  /** L'indice du premier segment qui commence à `from` ou après. */
  const indexAfter = (track: MediaTrack, from: number): number => {
    const table = track.segments;
    let i = 0;
    while (i < table.length && table[i].time < from - 0.01) i++;
    return i;
  };

  /** Plus rien : le montage est abandonné. */
  const teardown = () => {
    generation++;
    abort.abort();
    if (objectUrl) URL.revokeObjectURL(objectUrl);
    objectUrl = "";
    media = null;
    lanes = [];
  };

  const fail = (e: unknown) => {
    stopped = true;
    unlisten();
    teardown();
    on.error(e);
  };

  /** Donne à une piste tous les segments qui manquent jusqu'à `until`. */
  const feed = async (lane: Lane, until: number, gen: number) => {
    if (lane.busy) return;
    lane.busy = true;
    try {
      const table = lane.track.segments;
      while (!stopped && gen === generation && lane.next < table.length) {
        const seg = table[lane.next];
        if (seg.time >= until) break;
        const data = await bytes(lane.track, seg.start, seg.end - 1, abort.signal);
        if (stopped || gen !== generation) break;
        try {
          await append(lane, data);
        } catch (e) {
          // le moteur n'a plus de place : on lui rend le lointain, et on retente une fois
          if (!isQuota(e)) throw e;
          await release(lane, video.currentTime);
          await append(lane, data);
        }
        lane.next++;
        lane.tail = seg.time;
      }
      lane.done = lane.next === table.length;
      if (!stopped && gen === generation) await release(lane, video.currentTime - BEHIND);
    } catch (e) {
      if (gen === generation && !stopped) fail(e);
    } finally {
      lane.busy = false;
    }
    if (stopped || gen !== generation) return;
    // tout a été donné : la fin arrive comme à la fin d'un fichier
    if (lanes.every((l) => l.done) && media?.readyState === "open") media.endOfStream();
    pump();
  };

  /** Comble les pistes qui n'ont pas assez d'avance sur la lecture. */
  const pump = () => {
    if (stopped || switching || !lanes.length) return;
    const pos = video.currentTime;
    const until = Math.min(pos + AHEAD, total);
    for (const lane of lanes) {
      if (lane.busy || lane.done) continue;
      const have = receivedUntil(lane, pos);
      if (have >= until) continue;
      // rien n'est reçu à l'instant lu : on reprend là ; sinon la suite d'après la fin reçue
      lane.next = have <= pos ? indexAt(lane.track, pos) : Math.max(lane.next, indexAfter(lane.track, have));
      // ce qu'il faudrait donner est plus ancien que ce qui l'a été : on repart de là
      if (lane.tail >= 0 && lane.next < lane.track.segments.length && lane.track.segments[lane.next].time < lane.tail - 0.01) return void restart(pos);
      void feed(lane, until, generation);
    }
  };

  /** Un saut vers une place où rien n'est encore reçu (en avant) : les pistes reprennent là. */
  const jump = async (target: number) => {
    switching = true;
    generation++;
    abort.abort();
    abort = new AbortController();
    // le temps que les demandes en vol s'arrêtent et que les tampons soient libres
    await untilTrue(() => stopped || lanes.every((l) => !l.busy && !l.buffer.updating));
    if (stopped) return;
    for (const lane of lanes) if (receivedUntil(lane, target) <= target) lane.next = indexAt(lane.track, target);
    switching = false;
    pump();
  };

  /** Commence (ou recommence) la lecture par segments à `at` : les en-têtes, puis les premiers
   * segments. Ce qui avait été donné avant est abandonné. */
  const boot = async (at: number) => {
    const gen = ++generation;
    abort.abort();
    abort = new AbortController();
    switching = true;
    await untilTrue(() => stopped || lanes.every((l) => !l.busy && !l.buffer.updating));
    if (stopped) return;
    if (objectUrl) URL.revokeObjectURL(objectUrl);
    const ms = new MediaSource();
    media = ms;
    objectUrl = URL.createObjectURL(ms);
    try {
      video.src = objectUrl;
      // le moteur ouvre le montage dès que l'élément le prend ; sans nouvelle au bout de huit secondes : en panne
      await new Promise<void>((resolve) => {
        ms.onsourceopen = () => resolve();
        setTimeout(resolve, 8000);
      });
      if (ms.readyState !== "open") throw new Error("le moteur n'a pas ouvert le montage");
      if (stopped || gen !== generation) return;
      if (duration > 0) ms.duration = duration;
      const lane = (kind: "video" | "audio", track: MediaTrack): Lane => ({ track, buffer: ms.addSourceBuffer(mime(kind, track.codec)), next: indexAt(track, at), tail: -1, done: false, busy: false });
      lanes = [lane("video", segments.video), lane("audio", segments.audio)];
      await Promise.all(
        lanes.map(async (l) => append(l, await bytes(l.track, 0, l.track.init_end - 1, abort.signal))),
      );
      if (stopped || gen !== generation) return;
      if (at > 0) video.currentTime = at;
      switching = false;
      await Promise.all(lanes.map((l) => feed(l, Math.min(at + AHEAD, total), gen)));
    } catch (e) {
      if (gen === generation && !stopped) fail(e);
      switching = false;
    }
  };

  /** Repartir plus loin en arrière que ce qui a été donné : un nouveau montage. On ne donne
   * jamais au moteur des données plus anciennes que celles qu'il tient déjà (il les refuse,
   * ou détruit ce qui les suit) : on repart de l'instant demandé, d'un propre départ. */
  const restart = (at: number) => void boot(at);

  const onSeeking = () => {
    if (stopped || switching || !lanes.length) return;
    const target = video.currentTime;
    if (lanes.every((lane) => receivedUntil(lane, target) > target)) return;
    if (lanes.some((lane) => lane.tail >= 0 && target < lane.tail - 0.01)) return void restart(target);
    void jump(target);
  };

  const unlisten = () => {
    video.removeEventListener("seeking", onSeeking);
    video.removeEventListener("timeupdate", pump);
  };
  video.addEventListener("seeking", onSeeking);
  video.addEventListener("timeupdate", pump);
  void boot(0);

  return {
    stop() {
      stopped = true;
      unlisten();
      teardown();
    },
  };
}
