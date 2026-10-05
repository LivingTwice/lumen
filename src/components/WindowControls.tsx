import { useEffect, useState } from "react";
import { isTauri } from "../lib/api";
import { t } from "../lib/i18n";
import { isWindows } from "../lib/platform";

type Win = import("@tauri-apps/api/window").Window;

/** La fenêtre de Lumen (rien dans le navigateur). */
async function current(): Promise<Win | null> {
  if (!isTauri) return null;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

/**
 * Boutons de la fenêtre sous Windows : réduire, agrandir ou restaurer, fermer.
 * La fenêtre n'a pas la barre de titre de Windows (elle garderait sa bande
 * blanche au-dessus de Lumen) : ces boutons en tiennent lieu, en haut à droite,
 * à la manière de Windows 11, et la barre du haut de chaque vue sert à déplacer
 * la fenêtre (double-clic : agrandir). Rien sur Mac.
 */
export function WindowControls() {
  const [max, setMax] = useState(false);
  const [full, setFull] = useState(false);

  // agrandie ou non : l'icône du bouton du milieu suit (aussi après un double-clic sur la
  // barre) ; en plein écran (F11), les boutons s'effacent
  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    void current().then(async (w) => {
      if (!w) return;
      const sync = async () => {
        setMax(await w.isMaximized());
        setFull(await w.isFullscreen());
      };
      await sync();
      const unlisten = await w.onResized(() => void sync());
      if (gone) unlisten();
      else stop = unlisten;
    });
    return () => {
      gone = true;
      stop?.();
    };
  }, []);

  if (!isWindows || full) return null;
  const run = (act: (w: Win) => Promise<void>) => () => void current().then((w) => w && act(w));

  return (
    <div className="win-controls" role="group" aria-label={t("Fenêtre", "Window")}>
      <button className="win-btn" onClick={run((w) => w.minimize())} aria-label={t("Réduire", "Minimize")} title={t("Réduire", "Minimize")}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5.5h10" />
        </svg>
      </button>
      <button
        className="win-btn"
        onClick={run((w) => w.toggleMaximize())}
        aria-label={max ? t("Niveau inférieur", "Restore down") : t("Agrandir", "Maximize")}
        title={max ? t("Niveau inférieur", "Restore down") : t("Agrandir", "Maximize")}
      >
        {max ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
            <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
          </svg>
        )}
      </button>
      <button className="win-btn close" onClick={run((w) => w.close())} aria-label={t("Fermer", "Close")} title={t("Fermer", "Close")}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
        </svg>
      </button>
    </div>
  );
}
