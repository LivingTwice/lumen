import { useEffect, useState } from "react";
import { api } from "./api";
import type { DictStatus, LangCode } from "./types";

/**
 * État du dictionnaire d'une langue, dans la langue de l'interface : livré avec l'app,
 * prêt, ou en téléchargement (il se télécharge dès qu'on le demande). Relu quand un
 * dictionnaire arrive.
 */
export function useDictStatus(lang: LangCode | null | undefined): DictStatus | null {
  const [status, setStatus] = useState<DictStatus | null>(null);
  useEffect(() => {
    if (!lang) return;
    let alive = true;
    let unlisten: (() => void) | undefined;
    const read = () =>
      api()
        .dictStatus(lang)
        .then((s) => alive && setStatus(s))
        .catch(() => {});
    void read();
    void api()
      .dictListen((l) => l === lang && void read())
      .then((u) => (alive ? (unlisten = u) : u()));
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [lang]);
  return status;
}
