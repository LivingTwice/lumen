import { motion } from "motion/react";
import { Fragment } from "react";
import { t } from "../lib/i18n";
import { closeShortcuts, useMenu } from "../lib/menu";
import { isWindows, modKey as M } from "../lib/platform";
import { Sheet } from "./ui";

/** Une ligne : les touches (plusieurs combinaisons possibles) et ce qu'elles font. */
type Row = { keys: string[][]; label: string };

function groups(): { title: string; rows: Row[] }[] {
  // les touches telles qu'elles sont écrites sur le clavier : celui du Mac ou celui d'un PC
  const shift = isWindows ? t("Maj", "Shift") : "⇧";
  const enter = isWindows ? t("Entrée", "Enter") : "↩";
  const pages = isWindows ? [[t("Pg préc.", "Page Up")], [t("Pg suiv.", "Page Down")]] : [["fn", "↑"], ["fn", "↓"]];
  return [
    {
      title: t("Dans une leçon", "In a lesson"),
      rows: [
        { keys: [["←", "→"]], label: t("Mot précédent, mot suivant", "Previous word, next word") },
        { keys: [[shift, "←"], [shift, "→"]], label: t("Étendre la sélection", "Extend the selection") },
        { keys: [["1", "2", "3"]], label: t("Mot en apprentissage, niveau 1 à 3", "Word being learned, level 1 to 3") },
        { keys: [["K"], ["4"]], label: t("Mot connu", "Known word") },
        { keys: [["X"]], label: t("Ignorer le mot", "Ignore the word") },
        { keys: [["0"]], label: t("Remettre le mot à nouveau", "Make the word new again") },
        { keys: [[t("Espace", "Space")]], label: t("Lire ou mettre en pause", "Play or pause") },
        { keys: [[enter]], label: t("Terminer la page", "Finish the page") },
        { keys: pages, label: t("Page précédente ou suivante", "Previous or next page") },
        { keys: [["C"]], label: t("Discuter de la leçon", "Chat about the lesson") },
        { keys: [[t("Échap", "Esc")]], label: t("Effacer la sélection, quitter le plein écran", "Clear the selection, leave full screen") },
      ],
    },
    {
      title: t("Partout", "Anywhere"),
      rows: [
        { keys: [[M, "N"]], label: t("Nouvelle leçon", "New lesson") },
        { keys: [[M, "O"]], label: t("Ouvrir un fichier", "Open a file") },
        { keys: [[M, "L"]], label: t("Importer un lien", "Import a link") },
        { keys: [isWindows ? [M, shift, "N"] : [shift, M, "N"]], label: t("Nouvelle conversation", "New conversation") },
        { keys: [[M, "F"]], label: t("Rechercher", "Search") },
        { keys: [[M, "R"]], label: t("Reprendre la lecture", "Resume reading") },
        { keys: [[M, "1…6"]], label: t("Bibliothèque, Découvrir… jusqu'à Progrès", "Library, Discover… up to Progress") },
        { keys: [[M, "S"]], label: t("Sauvegarder la progression", "Back up progress") },
        { keys: [[M, ","]], label: t("Réglages", "Settings") },
        ...(isWindows ? [{ keys: [[M, "/"]], label: t("Raccourcis clavier", "Keyboard shortcuts") }] : []),
      ],
    },
    {
      title: t("Affichage", "Display"),
      rows: [
        { keys: [[M, "="], [M, "-"]], label: t("Agrandir ou réduire le texte", "Bigger or smaller text") },
        { keys: [[M, "0"]], label: t("Taille d'origine", "Default size") },
        { keys: [isWindows ? [M, "B"] : ["⌃", "⌘", "S"]], label: t("Barre latérale, dans une leçon", "Sidebar, in a lesson") },
        { keys: [isWindows ? ["F11"] : ["⌃", "⌘", "F"]], label: t("Plein écran", "Full screen") },
      ],
    },
    {
      title: t("Aperçu de Découvrir", "Discover preview"),
      rows: [
        { keys: [[t("Espace", "Space")]], label: t("Lire ou mettre en pause", "Play or pause") },
        { keys: [["←"], ["→"]], label: t("Reculer ou avancer de 5 secondes", "Back or forward 5 seconds") },
        { keys: [[t("Échap", "Esc")]], label: t("Fermer l'aperçu", "Close the preview") },
      ],
    },
  ];
}

/** Raccourcis clavier (menu Aide, ⌘/ ; sous Windows, Ctrl+/) : ceux du lecteur n'étaient écrits nulle part. */
export function Shortcuts() {
  const open = useMenu((s) => s.shortcuts);
  return (
    <Sheet open={open} onClose={closeShortcuts} title={t("Raccourcis clavier", "Keyboard shortcuts")} width={780}>
      <div className="keys-grid">
        {groups().map((g, gi) => (
          <motion.section
            key={g.title}
            className="keys-group"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.06 + gi * 0.05, duration: 0.35, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <span className="eyebrow">{g.title}</span>
            <ul>
              {g.rows.map((r) => (
                <li key={r.label}>
                  <span className="keys-label">{r.label}</span>
                  <span className="keys">
                    {r.keys.map((combo, i) => (
                      <Fragment key={i}>
                        {i > 0 && <span className="keys-or">{t("ou", "or")}</span>}
                        {combo.map((k) => (
                          <kbd key={k} className="kbd">
                            {k}
                          </kbd>
                        ))}
                      </Fragment>
                    ))}
                  </span>
                </li>
              ))}
            </ul>
          </motion.section>
        ))}
      </div>
    </Sheet>
  );
}
