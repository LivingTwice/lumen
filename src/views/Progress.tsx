import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { CountUp } from "../components/ui";
import { api } from "../lib/api";
import { count, locale, t } from "../lib/i18n";
import { MILESTONES, inLang } from "../lib/langs";
import { formatNumber, useApp } from "../lib/store";
import type { Stats } from "../lib/types";

export function Progress() {
  const lang = useApp((s) => s.lang)();
  const [st, setSt] = useState<Stats | null>(null);
  const DAY_FMT = new Intl.DateTimeFormat(locale(), { day: "numeric", month: "short" });

  useEffect(() => {
    api().stats(lang).then(setSt).catch(() => {});
  }, [lang]);

  const known = st?.known ?? 0;
  const next = MILESTONES.find((m) => m.words > known) ?? MILESTONES[MILESTONES.length - 1];
  const prevIdx = MILESTONES.indexOf(next) - 1;
  const prev = prevIdx >= 0 ? MILESTONES[prevIdx] : { words: 0, label: t("Départ", "Start") };
  const pct = Math.min(100, ((known - prev.words) / Math.max(1, next.words - prev.words)) * 100);
  const max = Math.max(50, ...(st?.days.map((d) => d.words_read) ?? [0]));
  const listenMin = Math.round((st?.listen_secs_total ?? 0) / 60);

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>{t("Progrès", "Progress")}</h1>
              <p>{t(`Votre vocabulaire ${inLang(lang)} grandit à chaque page lue.`, `Your vocabulary ${inLang(lang)} grows with every page you read.`)}</p>
            </div>
          </header>

          <motion.section className="stats-hero" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }}>
            <div>
              <span className="eyebrow">{t("Mots connus", "Known words")}</span>
              <div className="big-number">
                <CountUp value={known} duration={1400} />
              </div>
              <p className="muted" style={{ marginTop: 10 }}>
                {t(
                  `et ${formatNumber(st?.learning ?? 0)} en cours d'apprentissage, ${count(st?.phrases ?? 0, "expression", "expressions", "", "")}`,
                  `and ${formatNumber(st?.learning ?? 0)} being learned, ${count(st?.phrases ?? 0, "", "", "phrase", "phrases")}`,
                )}
              </p>
            </div>
            <div className="milestone">
              <span className="eyebrow">{t("Prochain palier", "Next milestone")}</span>
              <strong className="display" style={{ fontSize: 28 }}>
                {next.label}
              </strong>
              <div className="bar live">
                <motion.i initial={{ width: 0 }} animate={{ width: `${pct}%` }} transition={{ duration: 1.2, ease: [0.2, 0.8, 0.2, 1] }} />
              </div>
              <span className="muted num">
                {t(
                  `${formatNumber(known)} / ${formatNumber(next.words)} mots · encore ${formatNumber(Math.max(0, next.words - known))}`,
                  `${formatNumber(known)} / ${formatNumber(next.words)} words · ${formatNumber(Math.max(0, next.words - known))} to go`,
                )}
              </span>
            </div>
          </motion.section>

          <div className="stat-grid">
            <div className="stat">
              <strong>{formatNumber(st?.today.words_read ?? 0)}</strong>
              <span>{t("mots lus aujourd'hui", "words read today")}</span>
            </div>
            <div className="stat">
              <strong>{formatNumber(st?.today.known_added ?? 0)}</strong>
              <span>{t("nouveaux mots connus aujourd'hui", "new known words today")}</span>
            </div>
            <div className="stat">
              <strong>{formatNumber(st?.words_read_total ?? 0)}</strong>
              <span>{t("mots lus au total", "words read in total")}</span>
            </div>
            <div className="stat">
              <strong>{formatNumber(listenMin)}</strong>
              <span>{t("minutes d'écoute", "minutes of listening")}</span>
            </div>
          </div>

          <section className="chart-card">
            <div className="chart-head">
              <h3>{t("Lecture des 30 derniers jours", "Reading over the last 30 days")}</h3>
              <span className="muted num">{count(st?.days.reduce((s, d) => s + d.words_read, 0) ?? 0, "mot", "mots", "word", "words")}</span>
            </div>
            <div className="bars" role="img" aria-label={t("Mots lus par jour sur 30 jours", "Words read per day over 30 days")}>
              {(st?.days ?? []).map((d, i, arr) => (
                <motion.div
                  key={d.day}
                  className={`b ${d.words_read ? "has" : ""} ${i === arr.length - 1 ? "today" : ""}`}
                  data-tip={`${DAY_FMT.format(new Date(d.day + "T12:00:00"))} · ${count(d.words_read, "mot", "mots", "word", "words")}`}
                  style={{ height: `${Math.max(2, (d.words_read / max) * 100)}%` }}
                  initial={{ scaleY: 0 }}
                  animate={{ scaleY: 1 }}
                  transition={{ delay: i * 0.015, type: "spring", stiffness: 200, damping: 24 }}
                />
              ))}
            </div>
            {st && (
              <div className="bars-axis">
                <span>{DAY_FMT.format(new Date(st.days[0].day + "T12:00:00"))}</span>
                <span>{t("Aujourd'hui", "Today")}</span>
              </div>
            )}
          </section>
        </div>
      </div>
    </>
  );
}
