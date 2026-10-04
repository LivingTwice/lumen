import { AnimatePresence, motion } from "motion/react";
import { t } from "../lib/i18n";
import { stageText, useImports } from "../lib/imports";

/** Leçons en préparation, dans la barre latérale : on continue d'explorer pendant ce temps. */
export function ImportQueueCard() {
  const jobs = useImports((s) => s.jobs);
  const running = jobs.find((j) => j.status === "running");
  const waiting = jobs.filter((j) => j.status === "waiting" || j.status === "model").length;
  const visible = !!running || waiting > 0;
  const shown = running ?? jobs.find((j) => j.status === "waiting" || j.status === "model");
  const pct = running?.progress ?? null;
  return (
    <AnimatePresence>
      {visible && shown && (
        <motion.div
          className="update-card queue-card"
          role="status"
          initial={{ opacity: 0, y: 12, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ type: "spring", stiffness: 300, damping: 26 }}
        >
          <div className="update-head">
            <span className="update-dot" />
            <strong>{t("Leçon en préparation", "Lesson in the making")}</strong>
          </div>
          <span className="queue-title" title={shown.spec.title}>
            {shown.spec.title}
          </span>
          <span className="muted" style={{ fontSize: 11.5 }}>
            {running
              ? `${stageText(running.stage)}${pct !== null ? t(` · ${Math.round(pct)} %`, ` · ${Math.round(pct)}%`) : "…"}`
              : shown.status === "model"
                ? t("En attente du modèle de transcription", "Waiting for the transcription model")
                : t("En attente", "Waiting")}
            {waiting > (running ? 0 : 1) ? t(` · ${waiting - (running ? 0 : 1)} ensuite`, ` · ${waiting - (running ? 0 : 1)} next`) : ""}
          </span>
          <div className={`bar live ${pct === null ? "indeterminate" : ""}`}>
            <i style={{ width: `${pct ?? 30}%` }} />
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
