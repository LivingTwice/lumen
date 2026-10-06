import { motion } from "motion/react";
import { useCallback, useEffect, useId, useMemo, useState, type ReactNode } from "react";
import { Icon, type IconName } from "../components/Icon";
import { CountUp, Duration, Menu, Segmented } from "../components/ui";
import { api } from "../lib/api";
import { count, formatNumber, locale, pick, t } from "../lib/i18n";
import { MILESTONES, inLang } from "../lib/langs";
import { GOALS, studyTime } from "../lib/progress";
import { useApp } from "../lib/store";
import type { DayStat, Span, Stats, Streak } from "../lib/types";

type Period = "day" | "week" | "month" | "total";
type Metric = "learn" | "words" | "listen" | "known";

const EASE = [0.2, 0.8, 0.2, 1] as const;

const dayDate = (s: string) => new Date(s + "T12:00:00");
const active = (d: DayStat) => d.learn_secs > 0 || d.words_read > 0 || d.listen_secs > 0 || d.lingqs > 0 || d.known_added > 0;

/** Valeur d'une mesure sur un jour ou une période. */
function valueOf(x: DayStat | Span, m: Metric): number {
  return m === "learn" ? x.learn_secs : m === "words" ? x.words_read : m === "listen" ? x.listen_secs : x.known_added;
}

/** Valeur d'une mesure en texte (durée ou nombre de mots). */
function formatValue(v: number, m: Metric): string {
  if (m === "learn" || m === "listen") return studyTime(v);
  if (m === "known") return count(v, "mot connu", "mots connus", "known word", "known words");
  return count(v, "mot", "mots", "word", "words");
}

export function Progress() {
  const lang = useApp((s) => s.lang)();
  const why = useApp((s) => (s.settings.user_why ?? "").trim());
  const [st, setSt] = useState<Stats | null>(null);
  const [period, setPeriod] = useState<Period>("day");
  const [metric, setMetric] = useState<Metric>("learn");

  const load = useCallback(async () => {
    try {
      const s = await api().stats(lang);
      setSt(s);
      // la barre latérale suit (mots connus, série)
      useApp.setState({ knownCount: s.known, streak: s.streak });
    } catch {
      /* base occupée : la vue garde ce qu'elle montrait */
    }
  }, [lang]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view progress-view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>{t("Progrès", "Progress")}</h1>
              <p>
                {t(
                  `Votre progression ${inLang(lang)}, jour après jour : seul compte le temps passé dans les leçons.`,
                  `Your progress ${inLang(lang)}, day after day: only time spent in lessons counts.`,
                )}
              </p>
              {/* ce qui fait apprendre, choisi dans le profil : un rappel, pour soi */}
              {why && (
                <motion.p className="page-why" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.5, ease: EASE }}>
                  <span className="page-why-mark" aria-hidden="true" />
                  {t(`« ${why} »`, `“${why}”`)}
                </motion.p>
              )}
            </div>
          </header>

          <StreakHero st={st} onGoal={load} />

          <div className="period-head">
            <h2>{periodTitle(period)}</h2>
            <Segmented<Period>
              id="progress-period"
              label={t("Période", "Period")}
              value={period}
              onChange={setPeriod}
              options={[
                { value: "day", label: t("Jour", "Day") },
                { value: "week", label: t("Semaine", "Week") },
                { value: "month", label: t("Mois", "Month") },
                { value: "total", label: t("Total", "All time") },
              ]}
            />
          </div>

          <PeriodCards st={st} period={period} />

          <ActivityChart st={st} period={period} metric={metric} setMetric={setMetric} />

          <div className="progress-duo">
            <VocabCard st={st} />
            <LightCalendar st={st} />
          </div>
        </div>
      </div>
    </>
  );
}

function periodTitle(p: Period): string {
  return p === "day" ? t("Aujourd'hui", "Today") : p === "week" ? t("Cette semaine", "This week") : p === "month" ? t("Ce mois-ci", "This month") : t("Depuis le début", "All time");
}

// ---------- série et objectif du jour ----------

function StreakHero({ st, onGoal }: { st: Stats | null; onGoal(): Promise<void> }) {
  const setSetting = useApp((s) => s.setSetting);
  const [menu, setMenu] = useState(false);
  const streak: Streak = st?.streak ?? { current: 0, best: 0, today_done: false, goal_min: 10, today_secs: 0 };
  const goalSecs = streak.goal_min * 60;
  const left = Math.max(60, goalSecs - streak.today_secs);
  const state = streak.today_done ? "lit" : streak.current > 0 ? "wait" : "out";

  const message =
    state === "lit"
      ? t("Objectif du jour atteint. La flamme brille jusqu'à demain.", "Daily goal reached. The flame burns until tomorrow.")
      : state === "wait"
        ? t(`Encore ${studyTime(left)} dans une leçon aujourd'hui pour garder votre série.`, `${studyTime(left)} more in a lesson today to keep your streak.`)
        : t(`Passez ${streak.goal_min} min dans une leçon aujourd'hui pour allumer la flamme.`, `Spend ${streak.goal_min} min in a lesson today to light the flame.`);

  const chooseGoal = async (m: number) => {
    setMenu(false);
    await setSetting("daily_goal", String(m));
    await onGoal();
  };

  return (
    <motion.section className={`streak-hero ${state}`} data-tour="streak" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.5, ease: EASE }}>
      <div className="streak-main">
        <Flame state={state} />
        <div className="streak-text">
          <span className="eyebrow">{t("Série", "Streak")}</span>
          <div className="streak-count">
            <strong className="num">
              <CountUp value={streak.current} duration={900} />
            </strong>
            <span>{pick(streak.current, "jour de suite", "jours de suite", "day in a row", "days in a row")}</span>
          </div>
          <p className="streak-msg">{message}</p>
          <WeekStrip days={st?.days ?? []} />
        </div>
      </div>

      <div className="goal">
        <GoalRing secs={streak.today_secs} goal={goalSecs} done={streak.today_done} />
        <Menu
          open={menu}
          onClose={() => setMenu(false)}
          align="right"
          anchor={
            <button className="goal-pick" onClick={() => setMenu((v) => !v)} aria-haspopup="menu" aria-expanded={menu}>
              {t(`Objectif : ${streak.goal_min} min par jour`, `Goal: ${streak.goal_min} min a day`)}
              <Icon name="chevron" size={14} />
            </button>
          }
        >
          <div className="menu-note">{t("Temps actif dans une leçon, chaque jour", "Active time in a lesson, every day")}</div>
          {GOALS.map((m) => (
            <button key={m} className="menu-item" role="menuitem" onClick={() => void chooseGoal(m)}>
              <span style={{ flex: 1 }}>{t(`${m} min par jour`, `${m} min a day`)}</span>
              {m === streak.goal_min && <Icon name="check" size={16} />}
            </button>
          ))}
        </Menu>
        {streak.best > 0 && (
          <span className="goal-best muted num">{t(`Record : ${count(streak.best, "jour", "jours", "", "")}`, `Best: ${count(streak.best, "", "", "day", "days")}`)}</span>
        )}
      </div>
    </motion.section>
  );
}

/** Flamme de la série : éteinte, en attente (série d'hier à prolonger) ou allumée. */
function Flame({ state }: { state: "lit" | "wait" | "out" }) {
  const id = useId().replace(/:/g, "");
  return (
    <div className={`flame ${state}`} aria-hidden="true">
      <span className="flame-halo" />
      <svg viewBox="0 0 64 64" width="96" height="96">
        <defs>
          <linearGradient id={`fo${id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" style={{ stopColor: "var(--flame-top)" }} />
            <stop offset="1" style={{ stopColor: "var(--flame-base)" }} />
          </linearGradient>
          <linearGradient id={`fi${id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" style={{ stopColor: "var(--flame-core-top)" }} />
            <stop offset="1" style={{ stopColor: "var(--flame-core)" }} />
          </linearGradient>
        </defs>
        <g className="flame-body">
          <path
            className="flame-outer"
            fill={`url(#fo${id})`}
            d="M32 4C36 14 48 22 48 38C48 50 41 58 32 58C23 58 16 50 16 39C16 30 21 25 24 20C25 26 27 29 30 30C28 21 29 12 32 4Z"
          />
          <path
            className="flame-inner"
            fill={`url(#fi${id})`}
            d="M32 30C35 36 40 40 40 46C40 51 36.5 54 32 54C27.5 54 24 51 24 46.5C24 42 27 39 29 36C29.6 39 30.6 40.5 32 41C31.2 37 31.2 33.5 32 30Z"
          />
        </g>
      </svg>
    </div>
  );
}

/** Les sept jours de la semaine en cours : allumés quand l'objectif est atteint. */
function WeekStrip({ days }: { days: DayStat[] }) {
  const week = days.length ? days.slice(Math.floor((days.length - 1) / 7) * 7) : [];
  const names = useMemo(() => {
    const f = new Intl.DateTimeFormat(locale(), { weekday: "narrow" });
    // 1er janvier 2024 : un lundi
    return Array.from({ length: 7 }, (_, i) => f.format(new Date(2024, 0, 1 + i, 12)));
  }, []);
  const long = new Intl.DateTimeFormat(locale(), { weekday: "long", day: "numeric", month: "long" });
  return (
    <div className="week-strip" role="list" aria-label={t("Cette semaine", "This week")}>
      {names.map((n, i) => {
        const d = week[i];
        const isToday = !!d && i === week.length - 1;
        const cls = !d ? "future" : d.goal_met ? "met" : active(d) ? "some" : "";
        const tip = d
          ? `${long.format(dayDate(d.day))} · ${d.goal_met ? t("objectif atteint", "goal reached") : active(d) ? studyTime(d.learn_secs) : t("rien", "nothing")}`
          : undefined;
        return (
          <div key={i} className={`wd ${cls} ${isToday ? "today" : ""}`} role="listitem" title={tip}>
            <motion.span
              className="wd-dot"
              initial={{ scale: 0.4, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              transition={{ delay: 0.15 + i * 0.05, type: "spring", stiffness: 380, damping: 22 }}
            >
              {d?.goal_met && <Icon name="check" size={12} stroke={2.4} />}
            </motion.span>
            <span className="wd-name">{n}</span>
          </div>
        );
      })}
    </div>
  );
}

/** Anneau de l'objectif du jour. */
function GoalRing({ secs, goal, done }: { secs: number; goal: number; done: boolean }) {
  const id = useId().replace(/:/g, "");
  const p = Math.min(1, secs / Math.max(1, goal));
  return (
    <div className={`goal-ring ${done ? "done" : ""}`}>
      <svg viewBox="0 0 120 120" width="148" height="148" aria-hidden="true">
        <defs>
          <linearGradient id={`gr${id}`} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" style={{ stopColor: "var(--light)" }} />
            <stop offset="1" style={{ stopColor: "var(--light-strong)" }} />
          </linearGradient>
        </defs>
        <circle cx="60" cy="60" r="51" className="goal-track" />
        {p > 0 && (
          <motion.circle
            cx="60"
            cy="60"
            r="51"
            className="goal-fill"
            stroke={`url(#gr${id})`}
            initial={{ pathLength: 0 }}
            animate={{ pathLength: p }}
            transition={{ duration: 1.1, ease: EASE, delay: 0.2 }}
          />
        )}
      </svg>
      <div className="goal-center">
        <strong>
          <Duration secs={secs} />
        </strong>
        <span>{done ? t("objectif atteint", "goal reached") : t(`sur ${Math.round(goal / 60)} min`, `of ${Math.round(goal / 60)} min`)}</span>
      </div>
      <span className="sr-only">
        {t(`Aujourd'hui : ${studyTime(secs)} sur ${Math.round(goal / 60)} min`, `Today: ${studyTime(secs)} of ${Math.round(goal / 60)} min`)}
      </span>
    </div>
  );
}

// ---------- chiffres de la période ----------

function PeriodCards({ st, period }: { st: Stats | null; period: Period }) {
  const p = st?.periods;
  const span = p ? (period === "day" ? p.today : period === "week" ? p.week : period === "month" ? p.month : p.total) : null;
  const prev = p ? (period === "day" ? p.yesterday : period === "week" ? p.last_week : period === "month" ? p.last_month : null) : null;
  const prevLabel = period === "day" ? t("Hier", "Yesterday") : period === "week" ? t("Semaine dernière", "Last week") : t("Mois dernier", "Last month");
  const days = Math.max(1, span?.active_days ?? 0);

  const sub = (m: Metric): string => {
    if (!span) return "";
    if (prev) return t(`${prevLabel} : ${formatValue(valueOf(prev, m), m)}`, `${prevLabel}: ${formatValue(valueOf(prev, m), m)}`);
    if (!span.active_days) return t("Rien pour l'instant", "Nothing yet");
    const avg = valueOf(span, m) / days;
    const v = m === "learn" || m === "listen" ? studyTime(avg) : formatNumber(Math.round(avg));
    return t(`≈ ${v} par jour actif`, `≈ ${v} per active day`);
  };

  const cards: { m: Metric; icon: IconName; label: string; value: ReactNode }[] = [
    { m: "learn", icon: "clock", label: t("Temps d'apprentissage", "Learning time"), value: <Duration secs={span?.learn_secs ?? 0} /> },
    { m: "words", icon: "book", label: t("Mots lus", "Words read"), value: <CountUp value={span?.words_read ?? 0} /> },
    { m: "listen", icon: "speaker", label: t("Temps d'écoute", "Listening time"), value: <Duration secs={span?.listen_secs ?? 0} /> },
    { m: "known", icon: "sparkle", label: t("Nouveaux mots connus", "New known words"), value: <CountUp value={span?.known_added ?? 0} /> },
  ];

  const notes: string[] = [];
  if (span) {
    notes.push(count(span.lingqs, "mot mis à l'étude", "mots mis à l'étude", "word saved for study", "words saved for study"));
    if (period !== "day") {
      notes.push(count(span.active_days, "jour actif", "jours actifs", "active day", "active days"));
      notes.push(t(`objectif atteint ${count(span.goal_days, "jour", "jours", "", "")}`, `goal reached on ${count(span.goal_days, "", "", "day", "days")}`));
    }
    if (period === "total" && st?.first_day) {
      const f = new Intl.DateTimeFormat(locale(), { day: "numeric", month: "long", year: "numeric" });
      notes.push(t(`depuis le ${f.format(dayDate(st.first_day))}`, `since ${f.format(dayDate(st.first_day))}`));
    }
  }

  return (
    <>
      <div className="period-grid">
        {cards.map((c, i) => (
          <motion.div
            key={c.m}
            className={`period-card m-${c.m}`}
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.05 + i * 0.05, duration: 0.4, ease: EASE }}
          >
            <span className="period-label">
              <Icon name={c.icon} size={15} />
              {c.label}
            </span>
            <strong className="period-value num">
              {c.value}
            </strong>
            <span className="period-sub">{sub(c.m)}</span>
          </motion.div>
        ))}
      </div>
      {notes.length > 0 && <p className="period-notes">{notes.join(" · ")}</p>}
    </>
  );
}

// ---------- graphique ----------

function ActivityChart({ st, period, metric, setMetric }: { st: Stats | null; period: Period; metric: Metric; setMetric(m: Metric): void }) {
  const DAY = new Intl.DateTimeFormat(locale(), { day: "numeric", month: "short" });
  const MONTH = new Intl.DateTimeFormat(locale(), { month: "short" });
  const MONTH_YEAR = new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" });

  const bars = useMemo(() => {
    if (!st) return [];
    if (period === "day")
      return st.days.slice(-30).map((d) => ({ key: d.day, value: valueOf(d, metric), tip: DAY.format(dayDate(d.day)), axis: DAY.format(dayDate(d.day)) }));
    if (period === "week")
      return st.weeks.map((w) => ({
        key: w.start,
        value: valueOf(w, metric),
        tip: t(`Semaine du ${DAY.format(dayDate(w.start))}`, `Week of ${DAY.format(dayDate(w.start))}`),
        axis: DAY.format(dayDate(w.start)),
      }));
    // depuis le début : à partir du premier mois actif (six barres au moins)
    const first = st.months.findIndex((m) => m.active_days > 0);
    const months = period === "month" ? st.months.slice(-12) : st.months.slice(Math.max(0, Math.min(first < 0 ? Infinity : first, st.months.length - 6)));
    return months.map((m) => ({ key: m.start, value: valueOf(m, metric), tip: MONTH_YEAR.format(dayDate(m.start)), axis: MONTH.format(dayDate(m.start)) }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [st, period, metric]);

  const goalLine = period === "day" && metric === "learn" && st ? st.streak.goal_min * 60 : 0;
  const floor = metric === "words" ? 50 : metric === "known" ? 10 : 600;
  const max = Math.max(floor, goalLine * 1.15, ...bars.map((b) => b.value));
  const total = bars.reduce((s, b) => s + b.value, 0);
  const title =
    period === "day"
      ? t("Les 30 derniers jours", "The last 30 days")
      : period === "week"
        ? t("Les 12 dernières semaines", "The last 12 weeks")
        : period === "month"
          ? t("Les 12 derniers mois", "The last 12 months")
          : t("Mois par mois, depuis le début", "Month by month, from the start");
  const lastAxis = period === "day" ? t("Aujourd'hui", "Today") : period === "week" ? t("Cette semaine", "This week") : t("Ce mois-ci", "This month");

  return (
    <section className="chart-card">
      <div className="chart-head">
        <div>
          <h3>{title}</h3>
          <span className="muted num">{t(`${formatValue(total, metric)} en tout`, `${formatValue(total, metric)} in total`)}</span>
        </div>
        <Segmented<Metric>
          id="progress-metric"
          label={t("Mesure", "Measure")}
          value={metric}
          onChange={setMetric}
          options={[
            { value: "learn", label: t("Apprentissage", "Learning") },
            { value: "words", label: t("Mots lus", "Words read") },
            { value: "listen", label: t("Écoute", "Listening") },
            { value: "known", label: t("Mots connus", "Known words") },
          ]}
        />
      </div>
      <div
        key={`${period}-${metric}`}
        className={`bars n${bars.length > 20 ? "many" : "few"}`}
        style={{ gridTemplateColumns: `repeat(${Math.max(1, bars.length)}, minmax(0, 1fr))` }}
        role="img"
        aria-label={`${title} · ${formatValue(total, metric)}`}
      >
        {goalLine > 0 && (
          <div className="goal-line" style={{ bottom: `${(goalLine / max) * 100}%` }}>
            <span>{t(`objectif ${Math.round(goalLine / 60)} min`, `goal ${Math.round(goalLine / 60)} min`)}</span>
          </div>
        )}
        {bars.map((b, i, arr) => (
          <div key={b.key} className={`col ${i === arr.length - 1 ? "now" : ""}`} data-tip={`${b.tip} · ${formatValue(b.value, metric)}`}>
            <motion.i
              className={`b ${b.value > 0 ? "has" : ""} ${goalLine && b.value >= goalLine ? "met" : ""}`}
              style={{ height: `${Math.max(2, (Math.max(0, b.value) / max) * 100)}%` }}
              initial={{ scaleY: 0 }}
              animate={{ scaleY: 1 }}
              transition={{ delay: Math.min(0.5, i * (0.45 / Math.max(1, arr.length))), type: "spring", stiffness: 200, damping: 24 }}
            />
          </div>
        ))}
      </div>
      {bars.length > 0 && (
        <div className="bars-axis">
          <span>{bars[0].axis}</span>
          <span>{lastAxis}</span>
        </div>
      )}
    </section>
  );
}

// ---------- vocabulaire ----------

function VocabCard({ st }: { st: Stats | null }) {
  const known = st?.known ?? 0;
  const next = MILESTONES.find((m) => m.words > known) ?? MILESTONES[MILESTONES.length - 1];
  const prevIdx = MILESTONES.indexOf(next) - 1;
  const prev = prevIdx >= 0 ? MILESTONES[prevIdx] : { words: 0, label: t("Départ", "Start") };
  const pct = Math.min(100, ((known - prev.words) / Math.max(1, next.words - prev.words)) * 100);
  return (
    <motion.section className="vocab-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.1, duration: 0.5, ease: EASE }}>
      <span className="eyebrow">{t("Mots connus", "Known words")}</span>
      <div className="big-number">
        <CountUp value={known} duration={1400} />
      </div>
      <div className="records vocab-facts">
        <div>
          <strong className="num">{formatNumber(st?.learning ?? 0)}</strong>
          <span>{t("en apprentissage", "being learned")}</span>
        </div>
        <div>
          <strong className="num">{formatNumber(st?.phrases ?? 0)}</strong>
          <span>{pick(st?.phrases ?? 0, "expression", "expressions", "phrase", "phrases")}</span>
        </div>
        <div>
          <strong className="num">{formatNumber(st?.lessons ?? 0)}</strong>
          <span>{pick(st?.lessons ?? 0, "leçon", "leçons", "lesson", "lessons")}</span>
        </div>
      </div>
      <div className="milestone">
        <div className="milestone-head">
          <span className="eyebrow">{t("Prochain palier", "Next milestone")}</span>
          <strong className="display">{next.label}</strong>
        </div>
        <div className="bar live">
          <motion.i initial={{ width: 0 }} animate={{ width: `${pct}%` }} transition={{ duration: 1.2, ease: EASE }} />
        </div>
        <span className="muted num">
          {t(
            `${formatNumber(known)} / ${formatNumber(next.words)} mots · encore ${formatNumber(Math.max(0, next.words - known))}`,
            `${formatNumber(known)} / ${formatNumber(next.words)} words · ${formatNumber(Math.max(0, next.words - known))} to go`,
          )}
        </span>
      </div>
    </motion.section>
  );
}

// ---------- calendrier ----------

/** Intensité d'un jour : 0 rien, 1-2 un peu, 3 objectif atteint, 4 le double de l'objectif. */
function level(d: DayStat, goal: number): number {
  if (!active(d)) return 0;
  if (d.goal_met) return d.learn_secs >= goal * 2 ? 4 : 3;
  return d.learn_secs >= goal / 2 ? 2 : 1;
}

/** Vingt-six semaines de lumière : chaque jour s'éclaire selon le temps passé. */
function LightCalendar({ st }: { st: Stats | null }) {
  const goal = (st?.streak.goal_min ?? 10) * 60;
  const days = st?.days ?? [];
  const weeks: DayStat[][] = [];
  for (let i = 0; i < days.length; i += 7) weeks.push(days.slice(i, i + 7));
  const MONTH = new Intl.DateTimeFormat(locale(), { month: "short" });
  const LONG = new Intl.DateTimeFormat(locale(), { weekday: "long", day: "numeric", month: "long" });
  const names = useMemo(() => {
    const f = new Intl.DateTimeFormat(locale(), { weekday: "narrow" });
    return Array.from({ length: 7 }, (_, i) => (i % 2 === 0 ? f.format(new Date(2024, 0, 1 + i, 12)) : ""));
  }, []);
  const activeDays = st?.periods.total.active_days ?? 0;
  const rec = st?.records;
  const DAY = new Intl.DateTimeFormat(locale(), { day: "numeric", month: "short" });

  return (
    <motion.section className="calendar-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.16, duration: 0.5, ease: EASE }}>
      <div className="calendar-head">
        <span className="eyebrow">{t("Calendrier", "Calendar")}</span>
        <span className="muted">{t("26 dernières semaines", "Last 26 weeks")}</span>
      </div>
      <div className="cal" style={{ ["--weeks" as string]: Math.max(1, weeks.length) }}>
        <span />
        <div className="cal-months">
          {weeks.map((w, i) => {
            const m = w[0] ? dayDate(w[0].day).getMonth() : -1;
            const before = i > 0 && weeks[i - 1][0] ? dayDate(weeks[i - 1][0].day).getMonth() : -2;
            return <span key={i}>{m !== before && i < weeks.length - 1 ? MONTH.format(dayDate(w[0].day)) : ""}</span>;
          })}
        </div>
        <div className="cal-names">
          {names.map((n, i) => (
            <span key={i}>{n}</span>
          ))}
        </div>
        <div className="cal-grid" role="img" aria-label={t("Activité jour par jour sur 26 semaines", "Day-by-day activity over 26 weeks")}>
          {weeks.map((w, wi) => (
            <div key={wi} className="cal-week">
              {Array.from({ length: 7 }, (_, di) => {
                const d = w[di];
                if (!d) return <span key={di} className="cal-day empty" />;
                const lv = level(d, goal);
                const isToday = wi === weeks.length - 1 && di === w.length - 1;
                const tip = `${LONG.format(dayDate(d.day))} · ${
                  active(d)
                    ? [d.learn_secs ? studyTime(d.learn_secs) : "", d.words_read ? count(d.words_read, "mot lu", "mots lus", "word read", "words read") : ""].filter(Boolean).join(", ") ||
                      t("un peu d'activité", "a little activity")
                    : t("rien", "nothing")
                }`;
                return <span key={di} className={`cal-day l${lv} ${isToday ? "today" : ""}`} data-tip={tip} />;
              })}
            </div>
          ))}
        </div>
      </div>
      <div className="cal-legend">
        <span>{t("Moins", "Less")}</span>
        {[0, 1, 2, 3, 4].map((l) => (
          <span key={l} className={`cal-day l${l}`} />
        ))}
        <span>{t("Plus", "More")}</span>
        <span className="cal-legend-note">{t("Lumineux : objectif atteint", "Bright: goal reached")}</span>
      </div>
      <div className="records">
        <div>
          <strong className="num">{formatNumber(activeDays)}</strong>
          <span>{pick(activeDays, "jour actif", "jours actifs", "active day", "active days")}</span>
        </div>
        <div>
          <strong className="num">{formatNumber(st?.streak.best ?? 0)}</strong>
          <span>{t("meilleure série", "best streak")}</span>
        </div>
        <div title={rec?.learn_day ? DAY.format(dayDate(rec.learn_day)) : undefined}>
          <strong className="num">
            <Duration secs={rec?.learn_secs ?? 0} />
          </strong>
          <span>{rec?.learn_day ? t(`meilleure journée, le ${DAY.format(dayDate(rec.learn_day))}`, `best day, ${DAY.format(dayDate(rec.learn_day))}`) : t("meilleure journée", "best day")}</span>
        </div>
      </div>
    </motion.section>
  );
}
