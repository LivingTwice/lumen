import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { count, isEn, t } from "../lib/i18n";
import { LANGS } from "../lib/langs";
import { formatBytes, useApp } from "../lib/store";
import type { LangCode } from "../lib/types";
import { Icon } from "./Icon";

/* La visite guidée : après l'accueil, dans la première leçon. Un voile du soir couvre
   l'application, une lumière éclaire ce dont on parle (la page, un mot à toucher, le
   panneau du mot, « Terminer la page »…), une carte l'explique à côté. Deux scènes, au
   milieu, montrent ce qu'est un modèle d'IA et pourquoi il compte. */

const EASE = [0.16, 1, 0.3, 1] as const;

type Side = "left" | "right" | "top" | "bottom";

interface Step {
  id: "page" | "display" | "word" | "panel" | "model" | "why" | "chat" | "status" | "finish" | "player" | "import" | "library" | "progress" | "end";
  chapter: number;
  /** élément éclairé ("word" : un mot nouveau de la page) ; sans cible, une scène au milieu */
  target?: string;
  side?: Side[];
  pad?: number;
  radius?: number;
  /** il faut toucher l'élément éclairé pour continuer */
  wait?: boolean;
  /** l'élément éclairé reste utilisable */
  pass?: boolean;
  /** l'amener à l'écran (bas du panneau du mot) */
  scroll?: boolean;
  scene?: "model" | "why" | "end";
}

const STEPS: Step[] = [
  { id: "page", chapter: 0, target: '[data-tour="page"]', side: ["right", "left", "bottom", "top"], pad: 16, radius: 18 },
  { id: "display", chapter: 0, target: '[data-tour="display"]', side: ["bottom", "left"], pad: 6, radius: 12 },
  { id: "word", chapter: 0, target: "word", side: ["bottom", "top", "right", "left"], pad: 7, radius: 10, wait: true },
  { id: "panel", chapter: 0, target: '[data-tour="panel"]', side: ["left"], pad: -8, radius: 18 },
  { id: "model", chapter: 1, scene: "model" },
  { id: "why", chapter: 1, scene: "why" },
  { id: "chat", chapter: 1, target: '[data-tour="chat-tab"]', side: ["bottom", "left"], pad: 6, radius: 12 },
  { id: "status", chapter: 2, target: '[data-tour="status"]', side: ["left"], pad: 8, radius: 14, pass: true, scroll: true },
  { id: "finish", chapter: 2, target: '[data-tour="finish"]', side: ["top", "left", "right"], pad: 8, radius: 16 },
  { id: "player", chapter: 2, target: '[data-tour="player"]', side: ["top"], pad: 8, radius: 18, pass: true },
  { id: "import", chapter: 3, target: '[data-tour="import"]', side: ["right"], pad: 6, radius: 14 },
  { id: "library", chapter: 3, target: '[data-tour="nav-library"]', side: ["right"], pad: 4, radius: 12 },
  { id: "progress", chapter: 3, target: '[data-tour="nav-progress"]', side: ["right"], pad: 4, radius: 12 },
  { id: "end", chapter: 3, scene: "end" },
];

const chapters = () => [t("La page", "The page"), t("L'IA locale", "Local AI"), t("Apprendre", "Learning"), t("Ensuite", "What's next")];

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

function visible(el: Element): boolean {
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0 && r.bottom > 0 && r.right > 0 && r.top < window.innerHeight && r.left < window.innerWidth;
}

/** Un mot nouveau à toucher : de quatre lettres au moins, en minuscules (pas un nom propre), en haut de la page. */
function pickWord(page: Element): HTMLElement | null {
  const words = [...page.querySelectorAll<HTMLElement>(".w.s0")].filter(visible);
  const text = (w: HTMLElement) => w.textContent ?? "";
  const long = (w: HTMLElement) => text(w).length >= 4;
  const common = (w: HTMLElement) => long(w) && text(w)[0] === text(w)[0].toLowerCase();
  return words.find(common) ?? words.find(long) ?? words[0] ?? null;
}

const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));

/** Place la carte à côté de la lumière, du premier côté où elle tient. */
function placeCard(h: Box, vw: number, vh: number, cw: number, ch: number, sides: Side[]): { x: number; y: number } {
  const gap = 18;
  const m = 16;
  const fits: Record<Side, boolean> = {
    right: vw - (h.x + h.w) - gap - cw >= m,
    left: h.x - gap - cw >= m,
    bottom: vh - (h.y + h.h) - gap - ch >= m,
    top: h.y - gap - ch >= m,
  };
  const side = [...sides, "right", "left", "bottom", "top"].find((x) => fits[x as Side]) as Side | undefined;
  const cy = clamp(h.y + h.h / 2 - ch / 2, m, vh - ch - m);
  const cx = clamp(h.x + h.w / 2 - cw / 2, m, vw - cw - m);
  if (side === "right") return { x: h.x + h.w + gap, y: cy };
  if (side === "left") return { x: h.x - gap - cw, y: cy };
  if (side === "bottom") return { x: cx, y: h.y + h.h + gap };
  if (side === "top") return { x: cx, y: h.y - gap - ch };
  // nulle part : en bas à droite, par-dessus
  return { x: vw - cw - m * 2, y: vh - ch - m * 2 };
}

/** Le modèle de traduction choisi : prêt, en téléchargement, ou à installer. */
function useLlm() {
  const models = useApp((s) => s.models);
  const chosen = useApp((s) => s.settings.llm_model);
  const downloads = useApp((s) => s.downloads);
  const ready = models.find((m) => m.kind === "llm" && m.installed && m.id === chosen) ?? models.find((m) => m.kind === "llm" && m.installed);
  const m = ready ?? models.find((x) => x.id === chosen) ?? models.find((x) => x.kind === "llm");
  const dl = m && !ready ? downloads[m.id] : undefined;
  return { m, ready: !!ready, dl: dl && !dl.error ? dl : undefined };
}

export function Tour() {
  const step = useApp((s) => s.tour);
  return <AnimatePresence>{step !== null && STEPS[step] && <TourLayer key="tour" step={step} />}</AnimatePresence>;
}

function TourLayer({ step }: { step: number }) {
  const s = STEPS[step];
  const setStep = useApp((x) => x.setTourStep);
  const endTour = useApp((x) => x.endTour);
  const still = !!useReducedMotion();
  const [vp, setVp] = useState({ w: window.innerWidth, h: window.innerHeight });
  const [rect, setRect] = useState<Box | null>(null);
  // "wait" : on cherche encore l'élément ; "ok" : éclairé ; "none" : introuvable, carte au milieu.
  // L'état vaut pour une étape : à l'étape suivante, on recommence à chercher dès le premier rendu.
  const [found, setFound] = useState<{ step: number; status: "wait" | "ok" | "none" }>({ step, status: "wait" });
  const status = found.step === step ? found.status : s.target ? "wait" : "none";
  const [shown, setShown] = useState(false);
  const [ch, setCh] = useState(220);
  const elRef = useRef<HTMLElement | null>(null);
  const wordRef = useRef<string | null>(null);

  const find = useCallback((): HTMLElement | null => {
    if (!s.target) return null;
    if (s.target === "word") {
      const page = document.querySelector('[data-tour="page"]');
      if (!page) return null;
      if (wordRef.current) {
        const el = page.querySelector<HTMLElement>(`.w[data-i="${wordRef.current}"]`);
        if (el && visible(el)) return el;
      }
      const el = pickWord(page);
      wordRef.current = el?.dataset.i ?? null;
      return el;
    }
    // le premier visible (« Terminer la page » existe dans les deux mises en page)
    for (const el of document.querySelectorAll<HTMLElement>(s.target)) if (visible(el)) return el;
    return null;
  }, [s]);

  // suit l'élément éclairé à chaque image : la page se compose, le panneau glisse, la fenêtre change
  useEffect(() => {
    // une scène au milieu : la lumière se replie, et repartira du centre vers l'élément suivant
    if (!s.target) setRect(null);
    let raf = 0;
    let seen = performance.now();
    let scrolled = false;
    // la première leçon peut mettre un moment à s'ouvrir
    const grace = step === 0 ? 5000 : 1200;
    const tick = () => {
      const el = find();
      elRef.current = el;
      const now = performance.now();
      if (el) {
        if (s.scroll && !scrolled) {
          scrolled = true;
          el.scrollIntoView({ block: "nearest", behavior: still ? "auto" : "smooth" });
        }
        const r = el.getBoundingClientRect();
        const next = { x: r.left, y: r.top, w: r.width, h: r.height };
        setRect((p) => (p && Math.abs(p.x - next.x) < 0.5 && Math.abs(p.y - next.y) < 0.5 && Math.abs(p.w - next.w) < 0.5 && Math.abs(p.h - next.h) < 0.5 ? p : next));
        setFound((f) => (f.step === step && f.status === "ok" ? f : { step, status: "ok" }));
        seen = now;
      } else if (s.target && now - seen > grace) setFound((f) => (f.step === step && f.status === "none" ? f : { step, status: "none" }));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [step, s, find, still]);

  useEffect(() => {
    if (status !== "wait") setShown(true);
  }, [status]);

  useEffect(() => {
    const onResize = () => setVp({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // hauteur de la carte (une nouvelle à chaque étape), pour la placer à côté de la lumière
  const ro = useRef<ResizeObserver | null>(null);
  const cardRef = useCallback((el: HTMLDivElement | null) => {
    ro.current?.disconnect();
    ro.current = null;
    if (!el) return;
    setCh(el.offsetHeight);
    ro.current = new ResizeObserver(() => setCh(el.offsetHeight));
    ro.current.observe(el);
  }, []);

  const go = useCallback(
    (n: number) => {
      // la lecture à voix haute lancée pendant la visite s'arrête à l'étape suivante
      if (s.id === "player") window.dispatchEvent(new Event("lumen:pause"));
      if (n >= STEPS.length) endTour();
      else if (n >= 0) setStep(n);
    },
    [s, endTour, setStep],
  );
  const quit = useCallback(() => {
    window.dispatchEvent(new Event("lumen:pause"));
    endTour();
  }, [endTour]);

  // pendant qu'on cherche l'élément suivant, la lumière reste sur le précédent (pas de retour au centre)
  const lit = status !== "none" && !!rect && !s.scene;
  const waiting = !!s.wait && status === "ok" && lit;

  // le mot éclairé, une fois touché, mène à l'étape suivante
  useEffect(() => {
    if (!waiting) return;
    let timer = 0;
    const onUp = (e: PointerEvent) => {
      const el = elRef.current;
      if (el && e.target instanceof Node && el.contains(e.target)) timer = window.setTimeout(() => go(step + 1), 650);
    };
    window.addEventListener("pointerup", onUp, true);
    return () => {
      window.removeEventListener("pointerup", onUp, true);
      window.clearTimeout(timer);
    };
  }, [waiting, step, go]);

  // la visite est modale : le lecteur derrière n'entend pas le clavier
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) return;
      e.stopPropagation();
      const onButton = !!(e.target as HTMLElement).closest?.("button");
      if (e.key === "Escape") {
        e.preventDefault();
        quit();
      } else if ((e.key === "ArrowRight" || (e.key === "Enter" && !onButton)) && !waiting) {
        e.preventDefault();
        go(step + 1);
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        go(step - 1);
      } else if (e.key === " " && !onButton) e.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [go, quit, step, waiting]);

  const pad = s.pad ?? 8;
  const hole = lit && rect ? { x: rect.x - pad, y: rect.y - pad, w: rect.w + 2 * pad, h: rect.h + 2 * pad } : null;
  const centered = !hole;
  const cw = Math.min(centered && s.scene ? 600 : 352, vp.w - 32);
  const pos = hole ? placeCard(hole, vp.w, vp.h, cw, ch, s.side ?? ["right"]) : { x: (vp.w - cw) / 2, y: Math.max(16, (vp.h - ch) / 2) };
  const open = (s.pass || s.wait) && hole;
  const spring = still ? { duration: 0 } : { type: "spring" as const, stiffness: 190, damping: 28 };

  return (
    <motion.div className="tour" initial={{ opacity: 0 }} animate={{ opacity: shown ? 1 : 0 }} exit={{ opacity: 0 }} transition={{ duration: 0.5 }}>
      {/* la lumière : un trou dans le voile, qui glisse d'un élément à l'autre */}
      <motion.div
        className={`tour-hole ${waiting ? "wait" : ""}`}
        initial={false}
        animate={
          hole
            ? { x: hole.x, y: hole.y, width: hole.w, height: hole.h, borderRadius: s.radius ?? 14 }
            : { x: vp.w / 2, y: vp.h / 2, width: 0, height: 0, borderRadius: 40 }
        }
        transition={spring}
      >
        <span className={`tour-ring ${hole ? "on" : ""}`} />
      </motion.div>
      <motion.div className="tour-dim" animate={{ opacity: centered && shown ? 1 : 0 }} transition={{ duration: 0.5 }} />

      {/* ce qui n'est pas éclairé ne se touche pas */}
      {open ? (
        <>
          <div className="tour-block" style={{ left: 0, top: 0, width: vp.w, height: Math.max(0, hole.y) }} />
          <div className="tour-block" style={{ left: 0, top: hole.y + hole.h, width: vp.w, height: Math.max(0, vp.h - hole.y - hole.h) }} />
          <div className="tour-block" style={{ left: 0, top: hole.y, width: Math.max(0, hole.x), height: hole.h }} />
          <div className="tour-block" style={{ left: hole.x + hole.w, top: hole.y, width: Math.max(0, vp.w - hole.x - hole.w), height: hole.h }} />
        </>
      ) : (
        <div className="tour-block" style={{ inset: 0 }} />
      )}

      <AnimatePresence mode="wait">
        {status !== "wait" && (
          <motion.div
            key={step}
            ref={cardRef}
            className={`tour-card ${centered && s.scene ? "scene" : ""}`}
            role="dialog"
            aria-modal="true"
            aria-label={t("Visite guidée", "Guided tour")}
            style={{ left: pos.x, top: pos.y, width: cw }}
            initial={still ? { opacity: 0 } : { opacity: 0, y: 10, scale: 0.97, filter: "blur(6px)" }}
            animate={{ opacity: 1, y: 0, scale: 1, filter: "blur(0px)" }}
            exit={still ? { opacity: 0 } : { opacity: 0, scale: 0.98, filter: "blur(4px)" }}
            transition={{ duration: 0.38, ease: EASE, delay: step === 0 ? 0.5 : 0.14 }}
          >
            <Progress step={step} />
            {centered && s.scene === "model" && <SceneModel still={still} />}
            {centered && s.scene === "why" && <SceneWhy still={still} />}
            {centered && s.scene === "end" && <SceneEnd still={still} />}
            <StepText step={s} found={status === "ok"} />
            <div className="tour-foot">
              {s.id === "end" ? (
                <span />
              ) : (
                <button className="tour-skip" onClick={quit}>
                  {t("Passer la visite", "Skip the tour")}
                </button>
              )}
              <span className="tour-foot-actions">
                {step > 0 && (
                  <button className="btn sm ghost" onClick={() => go(step - 1)}>
                    {t("Retour", "Back")}
                  </button>
                )}
                {waiting ? (
                  <button className="btn sm soft" onClick={() => go(step + 1)}>
                    {t("Passer", "Skip")}
                  </button>
                ) : (
                  <button className="btn sm primary glow" onClick={() => go(step + 1)} autoFocus>
                    {s.id === "end" ? (
                      <>
                        {t("Commencer à lire", "Start reading")} <Icon name="book" size={14} />
                      </>
                    ) : (
                      <>
                        {t("Suivant", "Next")} <Icon name="forward" size={14} stroke={2} />
                      </>
                    )}
                  </button>
                )}
              </span>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}

/** Avancement : quatre chapitres, une graine de lumière par étape. */
function Progress({ step }: { step: number }) {
  const s = STEPS[step];
  return (
    <div className="tour-progress">
      <span className="tour-chapter">
        <span className="num">{s.chapter + 1}</span>
        {chapters()[s.chapter]}
      </span>
      <span className="tour-seeds" aria-label={t(`Étape ${step + 1} sur ${STEPS.length}`, `Step ${step + 1} of ${STEPS.length}`)}>
        {chapters().map((_, ci) => (
          <span key={ci} className="tour-chap">
            {STEPS.map((x, i) =>
              x.chapter === ci ? (
                <i key={i} className={i < step ? "done" : i === step ? "on" : ""}>
                  {i === step && <motion.b layoutId="tour-seed" transition={{ type: "spring", stiffness: 420, damping: 34 }} />}
                </i>
              ) : null,
            )}
          </span>
        ))}
      </span>
    </div>
  );
}

/** Titre et texte de chaque étape ; certains disent l'état réel de ce Mac. */
function StepText({ step, found }: { step: Step; found: boolean }) {
  const { m, ready, dl } = useLlm();
  const download = useApp((s) => s.download);
  const name = m?.name ?? "Qwen3.5";

  const modelState: ReactNode = ready ? (
    <span className="tour-state ok">
      <i className="tour-dot" /> {t(`${name} est prêt sur ce Mac.`, `${name} is ready on this Mac.`)}
    </span>
  ) : dl ? (
    <span className="tour-state busy">
      <span className="tour-state-line">
        <i className="tour-dot" /> {t(`${name} se télécharge`, `${name} is downloading`)}
        <span className="num">
          {formatBytes(dl.received)} / {formatBytes(dl.total)}
        </span>
      </span>
      <span className="bar live">
        <i style={{ width: `${(dl.received / Math.max(1, dl.total)) * 100}%` }} />
      </span>
    </span>
  ) : m ? (
    <span className="tour-state">
      <span className="tour-state-line">
        <i className="tour-dot" /> {t("Pas encore de modèle sur ce Mac.", "No model on this Mac yet.")}
      </span>
      <button className="btn sm primary glow" onClick={() => void download(m.id)}>
        <Icon name="download" size={13} /> {t(`Télécharger ${name} (${formatBytes(m.size)})`, `Download ${name} (${formatBytes(m.size)})`)}
      </button>
    </span>
  ) : null;

  let title = "";
  let body: ReactNode = null;
  switch (step.id) {
    case "page":
      title = t("Votre première leçon", "Your first lesson");
      body = (
        <>
          <p>
            {t(
              "Chaque mot porte une couleur. Plus vous lisez, plus la page s'éclaire : les mots bleus deviennent ambrés, puis sans couleur.",
              "Every word wears a color. The more you read, the brighter the page gets: blue words turn amber, then plain.",
            )}
          </p>
          <span className="tour-legend">
            <span>
              <i className="sw s0" /> {t("Nouveau", "New")}
            </span>
            <span>
              <i className="sw s1" /> {t("En apprentissage", "Learning")}
            </span>
            <span>
              <i className="sw s4" /> {t("Connu", "Known")}
            </span>
          </span>
        </>
      );
      break;
    case "display":
      title = t("La page à votre goût", "The page, your way");
      body = (
        <p>
          {t(
            "« Aa » règle la police, la taille, l'interligne et la couleur de la page, de Papier à Nuit. Le texte tient dans l'écran : on tourne la page avec les flèches ou deux doigts sur le trackpad.",
            "“Aa” sets the font, size, line spacing and page color, from Paper to Night. The text fits on the screen: turn the page with the arrows or two fingers on the trackpad.",
          )}
        </p>
      );
      break;
    case "word":
      title = t("Touchez ce mot", "Tap this word");
      body = found ? (
        <>
          <p>{t("Un mot bleu est un mot que vous n'avez jamais rencontré. Touchez celui qui brille.", "A blue word is one you've never met. Tap the one that's glowing.")}</p>
          <span className="tour-tap">
            <i className="tour-dot" /> {t("Touchez le mot éclairé", "Tap the glowing word")}
          </span>
        </>
      ) : (
        <p>{t("Dans une leçon, touchez n'importe quel mot bleu : son sens s'affiche à droite.", "In a lesson, tap any blue word: its meaning appears on the right.")}</p>
      );
      break;
    case "panel":
      title = t("Son sens, dans cette phrase", "Its meaning, in this sentence");
      body = (
        <>
          <p>
            {t(
              "Le dictionnaire hors ligne donne la forme du mot et tous ses sens. Puis l'IA lit la phrase entière et choisit le sens juste, ici, maintenant.",
              "The offline dictionary gives the word's form and all its meanings. Then the AI reads the whole sentence and picks the right meaning, here and now.",
            )}
          </p>
          {!ready && (
            <p className="tour-note">
              {dl
                ? t("Le sens en contexte apparaîtra dès que le modèle aura fini de se télécharger.", "The meaning in context will appear as soon as the model has finished downloading.")
                : t("Pour cela, il faut un modèle d'IA. Voyons ce que c'est.", "For that, you need an AI model. Let's see what that is.")}
            </p>
          )}
        </>
      );
      break;
    case "model":
      title = t("Qu'est-ce qu'un modèle ?", "What is a model?");
      body = (
        <>
          <p>
            {t(
              "C'est le « cerveau » de l'IA : un gros fichier qui a lu des milliards de phrases, dans des dizaines de langues, et en a tiré le sens des mots. Lumen le télécharge une fois ; ensuite, il travaille ici, même sans Internet. Ce que vous lisez ne quitte jamais votre Mac.",
              "It's the AI's “brain”: a big file that has read billions of sentences in dozens of languages and learned what words mean. Lumen downloads it once; after that, it works right here, even offline. What you read never leaves your Mac.",
            )}
          </p>
          {modelState}
        </>
      );
      break;
    case "why":
      title = t("Pourquoi c'est important", "Why it matters");
      body = (
        <p>
          {t(
            "Un mot a souvent plusieurs sens. Le dictionnaire les donne tous ; le modèle lit la phrase et choisit le bon. Plus il est grand (Léger, Équilibré, Maximum), plus il saisit les nuances, mais plus il pèse. Vous pouvez en changer à tout moment dans Réglages › IA locale.",
            "A word often has several meanings. The dictionary gives them all; the model reads the sentence and picks the right one. The bigger it is (Light, Balanced, Maximum), the more nuance it catches, but the more it weighs. You can switch at any time in Settings › Local AI.",
          )}
        </p>
      );
      break;
    case "chat":
      title = t("Un professeur, toujours là", "A teacher, always there");
      body = (
        <p>
          {t(
            "Le même modèle répond à vos questions dans l'onglet Chat : grammaire, exemples, nuances, sur la leçon que vous lisez. Raccourci : la touche C.",
            "The same model answers your questions in the Chat tab: grammar, examples, nuances, about the lesson you're reading. Shortcut: the C key.",
          )}
        </p>
      );
      break;
    case "status":
      title = t("Où en êtes-vous avec ce mot ?", "Where are you with this word?");
      body = (
        <p>
          {t(
            "1, 2, 3 : vous l'apprenez, sa couleur pâlit à mesure. ✓ : vous le connaissez. Essayez, ou laissez faire la page.",
            "1, 2, 3: you're learning it, and its color fades as you go. ✓: you know it. Give it a try, or let the page do it.",
          )}
        </p>
      );
      break;
    case "finish":
      title = t("Terminez la page", "Finish the page");
      body = (
        <p>
          {t(
            "Les mots bleus que vous n'avez pas touchés, vous les avez compris : ils rejoignent vos mots connus. C'est ainsi que votre vocabulaire grandit, page après page.",
            "The blue words you didn't tap, you understood: they join your known words. That's how your vocabulary grows, page after page.",
          )}
        </p>
      );
      break;
    case "player":
      title = t("Écoutez, la lanterne suit", "Listen, the lantern follows");
      body = (
        <p>
          {t(
            "Touchez ▶ : la page se lit à voix haute et un halo glisse sur chaque mot prononcé. Avec un podcast ou une vidéo, c'est leur propre voix que la lanterne suit.",
            "Tap ▶: the page is read aloud and a halo glides over each spoken word. With a podcast or a video, the lantern follows their own voice.",
          )}
        </p>
      );
      break;
    case "import":
      title = t("Lisez ce qui vous plaît", "Read what you love");
      body = (
        <p>
          {t(
            "Un livre, un article, un PDF, un podcast, une vidéo YouTube : Lumen en fait une leçon. Collez un lien, ou glissez un fichier n'importe où dans la fenêtre.",
            "A book, an article, a PDF, a podcast, a YouTube video: Lumen turns it into a lesson. Paste a link, or drop a file anywhere in the window.",
          )}
        </p>
      );
      break;
    case "library":
      title = t("Des leçons à votre niveau", "Lessons at your level");
      body = (
        <p>
          {t(
            "Dans la bibliothèque, l'onglet Découvrir propose chaque jour des vidéos, des podcasts et des articles récents, rangés de A1 à C1.",
            "In the library, the Discover tab suggests recent videos, podcasts and articles every day, sorted from A1 to C1.",
          )}
        </p>
      );
      break;
    case "progress":
      title = t("Un peu chaque jour", "A little every day");
      body = (
        <p>
          {t(
            "Dix minutes dans une leçon suffisent à tenir l'objectif du jour et à allumer la flamme. Vos mots connus, votre temps et vos records vous attendent ici.",
            "Ten minutes in a lesson are enough to reach the daily goal and light the flame. Your known words, your time and your records are waiting here.",
          )}
        </p>
      );
      break;
    case "end":
      title = t("À vous de lire", "Your turn to read");
      body = (
        <p>
          {t(
            "Touchez les mots inconnus, écoutez, terminez la page. La visite se rejoue dans Réglages › À propos ; « Comment marche Lumen ? », sous le panneau du mot, rouvre le petit guide.",
            "Tap the unknown words, listen, finish the page. You can replay the tour in Settings › About; “How does Lumen work?”, under the word panel, reopens the short guide.",
          )}
        </p>
      );
      break;
  }
  return (
    <div className="tour-text">
      <h3 className="display">{title}</h3>
      {body}
    </div>
  );
}

// ---------- scènes ----------

/** Des phrases de toutes les langues viennent nourrir une lumière, qui s'installe dans le Mac. */
function SceneModel({ still }: { still: boolean }) {
  const { m, ready } = useLlm();
  const words = LANGS.slice(0, 16).map((l) => ({ text: l.hello, code: l.code, rtl: l.rtl }));
  return (
    <div className="tour-stage">
      {!still &&
        words.map((w, i) => {
          const a = (i / words.length) * Math.PI * 2 + 0.4;
          return (
            <span
              key={w.code}
              className="tm-word"
              lang={w.code}
              dir={w.rtl ? "rtl" : undefined}
              style={
                {
                  "--dx": `${Math.cos(a) * 230}px`,
                  "--dy": `${Math.sin(a) * 92}px`,
                  animationDelay: `${(i * 0.37) % 5.2}s`,
                } as React.CSSProperties
              }
            >
              {w.text}
            </span>
          );
        })}
      <div className="tm-mac">
        <div className="tm-screen">
          <span className="tm-halo" />
          <span className={`tm-core ${ready ? "ready" : ""}`} />
          <span className="tm-offline">
            <i /> {t("hors ligne", "offline")}
          </span>
        </div>
        <div className="tm-base" />
      </div>
      <span className="tm-name">
        {m?.name ?? "Qwen3.5"}
        {m ? ` · ${formatBytes(m.size)}` : ""}
      </span>
    </div>
  );
}

interface Why {
  text: string;
  word: string;
  senses: { fr: string[]; en: string[] };
  pick: number;
  ctx: { fr: string; en: string };
}

/** Un mot à plusieurs sens, dans la langue étudiée si possible. */
const WHY: Partial<Record<LangCode, Why>> = {
  it: {
    text: "Il gatto entra piano nella stanza.",
    word: "piano",
    senses: { fr: ["piano (l'instrument)", "étage", "plan, projet", "doucement"], en: ["piano (the instrument)", "floor, storey", "plan, project", "softly, slowly"] },
    pick: 3,
    ctx: { fr: "doucement", en: "softly" },
  },
  es: {
    text: "Hoy hace un tiempo precioso.",
    word: "tiempo",
    senses: { fr: ["temps (qui passe)", "temps (qu'il fait)", "époque", "mi-temps"], en: ["time", "weather", "era", "half (of a match)"] },
    pick: 1,
    ctx: { fr: "le temps qu'il fait", en: "the weather" },
  },
  de: {
    text: "Die Bank ist heute geschlossen.",
    word: "Bank",
    senses: { fr: ["banc", "banque", "banc de sable"], en: ["bench", "bank", "sandbank"] },
    pick: 1,
    ctx: { fr: "la banque", en: "the bank" },
  },
  pt: {
    text: "O banco fecha às cinco.",
    word: "banco",
    senses: { fr: ["banc", "banque", "tabouret"], en: ["bench", "bank", "stool"] },
    pick: 1,
    ctx: { fr: "la banque", en: "the bank" },
  },
  ru: {
    text: "Ключ лежит на столе.",
    word: "Ключ",
    senses: { fr: ["clé", "source (d'eau)", "clé (en musique)"], en: ["key", "spring (of water)", "clef"] },
    pick: 0,
    ctx: { fr: "la clé", en: "the key" },
  },
  fr: {
    text: "Le temps passe si vite.",
    word: "temps",
    senses: { fr: ["temps (qui passe)", "temps (qu'il fait)", "temps (grammaire)"], en: ["time", "weather", "tense (grammar)"] },
    pick: 0,
    ctx: { fr: "le temps qui passe", en: "time" },
  },
  en: {
    text: "They sat on the bank of the river.",
    word: "bank",
    senses: { fr: ["banque", "rive", "talus", "réserve"], en: ["financial institution", "riverside", "slope", "store, supply"] },
    pick: 1,
    ctx: { fr: "la rive", en: "the riverside" },
  },
};

/** Le dictionnaire donne tous les sens ; le modèle lit la phrase et choisit le bon. */
function SceneWhy({ still }: { still: boolean }) {
  const lang = useApp((s) => s.lang)();
  const code: LangCode = WHY[lang] ? lang : "it";
  const w = WHY[code]!;
  const senses = isEn() ? w.senses.en : w.senses.fr;
  const ctx = t(w.ctx.fr, w.ctx.en);
  // 0 : la phrase ; 1 : le dictionnaire ; 2 : l'IA écrit ; 3 : le bon sens s'éclaire
  const [phase, setPhase] = useState(still ? 3 : 0);
  const [typed, setTyped] = useState(still ? ctx.length : 0);
  useEffect(() => {
    if (still) return;
    const timers = [window.setTimeout(() => setPhase(1), 450), window.setTimeout(() => setPhase(2), 1900), window.setTimeout(() => setPhase(3), 1900 + ctx.length * 55 + 450)];
    return () => timers.forEach((x) => window.clearTimeout(x));
  }, [still, ctx]);
  useEffect(() => {
    if (phase < 2 || typed >= ctx.length) return;
    const timer = window.setTimeout(() => setTyped((n) => n + 1), 55);
    return () => window.clearTimeout(timer);
  }, [phase, typed, ctx]);
  const [before, after] = [w.text.slice(0, w.text.indexOf(w.word)), w.text.slice(w.text.indexOf(w.word) + w.word.length)];

  return (
    <div className="tour-stage why">
      <p className="tw-line" lang={code}>
        {before}
        <span className="tw-word">{w.word}</span>
        {after}
      </p>
      <div className="tw-cols">
        <div className={`tw-col ${phase >= 1 ? "on" : ""}`}>
          <span className="eyebrow">{t("Le dictionnaire", "The dictionary")}</span>
          <ol>
            {senses.map((x, i) => (
              <motion.li
                key={i}
                initial={still ? false : { opacity: 0, x: -8 }}
                animate={phase >= 1 ? { opacity: 1, x: 0 } : { opacity: 0, x: -8 }}
                transition={{ delay: still ? 0 : i * 0.14, duration: 0.4, ease: EASE }}
              >
                <span className={`tw-sense ${phase >= 3 ? (i === w.pick ? "pick" : "dim") : ""}`}>
                  <span className="n">{i + 1}</span>
                  {x}
                </span>
              </motion.li>
            ))}
          </ol>
        </div>
        <div className={`tw-col ai ${phase >= 2 ? "on" : ""}`}>
          <span className="eyebrow">
            <Icon name="sparkle" size={11} /> {t("Le modèle, dans cette phrase", "The model, in this sentence")}
          </span>
          <span className={`tw-ctx ${phase === 2 && typed < ctx.length ? "typing" : ""}`}>{ctx.slice(0, typed) || " "}</span>
        </div>
      </div>
    </div>
  );
}

/** La lumière se lève : la visite est finie, la lecture commence. */
function SceneEnd({ still }: { still: boolean }) {
  const known = useApp((s) => s.knownCount);
  return (
    <div className="tour-stage end">
      <motion.div
        className="news-sun"
        initial={still ? false : { y: 80, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        transition={{ duration: 1.6, ease: EASE }}
      >
        <span className="news-rays" />
        <span className="news-halo" />
        <span className="news-core" />
      </motion.div>
      <div className="news-horizon" />
      {Array.from({ length: 16 }).map((_, i) => (
        <i key={i} className="news-mote" style={{ left: `${(i * 41 + 7) % 100}%`, animationDelay: `${(i * 0.6) % 6}s`, animationDuration: `${6 + (i % 5)}s` }} />
      ))}
      {known > 0 && <span className="te-known num">{count(known, "mot connu", "mots connus", "known word", "known words")}</span>}
    </div>
  );
}
