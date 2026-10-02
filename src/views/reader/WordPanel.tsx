import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { api, errorText, isNoModel } from "../../lib/api";
import { sayWord } from "../../lib/tts";
import { useApp } from "../../lib/store";
import type { DictResult, LangCode, Status, Term } from "../../lib/types";

export interface Selection {
  /** texte affiché (mot ou expression) */
  surface: string;
  /** clé du terme */
  key: string;
  /** phrase de contexte */
  sentence: string;
  /** bornes de la sélection dans la phrase (pour la surligner) */
  before: string;
  after: string;
  isPhrase: boolean;
  /** indice du premier jeton (pour lire depuis ce mot) */
  tokenIndex: number;
  /** expression enregistrée qui contient ce mot, s'il y en a une */
  phrase?: { key: string; a: number; b: number } | null;
}

const STATUS_OPTS: { s: Status; label: string; sw: string }[] = [
  { s: 1, label: "1", sw: "var(--w-l1)" },
  { s: 2, label: "2", sw: "var(--w-l2)" },
  { s: 3, label: "3", sw: "var(--w-l3)" },
  { s: 4, label: "Connu", sw: "var(--ok)" },
];

interface Props {
  lang: LangCode;
  sel: Selection | null;
  term: Term | undefined;
  onStatus(key: string, status: number): void;
  onTranslation(key: string, translation: string, note: string, sentence: string): void;
  onPlayFrom(i: number): void;
  onSelectPhrase(a: number, b: number): void;
  onClose(): void;
}

export function WordPanel({ lang, sel, term, onStatus, onTranslation, onPlayFrom, onSelectPhrase, onClose }: Props) {
  const settings = useApp((s) => s.settings);
  const go = useApp((s) => s.go);
  const llmReady = useApp((s) => s.models.some((m) => m.kind === "llm" && m.installed));
  const llmDownloading = useApp((s) => s.models.some((m) => m.kind === "llm" && !!s.downloads[m.id] && !s.downloads[m.id].error));
  const [dict, setDict] = useState<DictResult | null>(null);
  const [ai, setAi] = useState("");
  const [aiNote, setAiNote] = useState("");
  const [aiState, setAiState] = useState<"idle" | "loading" | "stream" | "done" | "nomodel" | "error">("idle");
  const [aiError, setAiError] = useState("");
  const [sentenceTr, setSentenceTr] = useState("");
  const [sentState, setSentState] = useState<"idle" | "loading" | "done" | "error">("idle");
  const [mine, setMine] = useState("");
  const [slow, setSlow] = useState(false);
  const reqId = useRef(0);
  const termRef = useRef(term);
  termRef.current = term;

  const key = sel?.key ?? "";
  const sentence = sel?.sentence ?? "";

  // dictionnaire + IA à chaque nouvelle sélection
  useEffect(() => {
    if (!sel) return;
    const id = ++reqId.current;
    setDict(null);
    setAi("");
    setAiNote("");
    setAiError("");
    setSentenceTr("");
    setSentState("idle");
    setMine(termRef.current?.translation ?? "");
    setAiState("loading");
    setSlow(false);
    const slowTimer = window.setTimeout(() => id === reqId.current && setSlow(true), 2500);

    if (!sel.isPhrase) {
      api()
        .dictLookup(lang, sel.surface)
        .then((d) => id === reqId.current && setDict(d))
        .catch(() => {});
    }

    const t = window.setTimeout(async () => {
      let raw = "";
      try {
        const res = await api().aiWord(lang, sel.surface, sel.sentence, (piece) => {
          if (id !== reqId.current) return;
          raw += piece;
          const line = raw.split("\n").find((l) => /^\s*sens/i.test(l)) ?? raw.split("\n")[0];
          setAi(line.replace(/^\s*sens\s*[:：]\s*/i, ""));
          setAiState("stream");
        });
        if (id !== reqId.current) return;
        setAi(res.translation);
        setAiNote(res.note);
        setAiState("done");
        if (res.translation && !termRef.current?.translation) {
          onTranslation(sel.key, res.translation, res.note, sel.sentence);
          setMine(res.translation);
        }
        if (settings.auto_sentence !== "0") translateSentence(id);
      } catch (e) {
        if (id !== reqId.current) return;
        if (isNoModel(e)) setAiState("nomodel");
        else if (String(e).includes("interrompu")) return;
        else {
          setAiState("error");
          setAiError(errorText(e));
        }
      }
    }, 90);
    return () => {
      window.clearTimeout(t);
      window.clearTimeout(slowTimer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, sentence, lang, llmReady]);

  useEffect(() => {
    setMine(term?.translation ?? "");
  }, [term?.translation]);

  const translateSentence = async (id = reqId.current) => {
    if (!sel) return;
    setSentState("loading");
    let acc = "";
    try {
      const res = await api().aiSentence(lang, sel.sentence, (p) => {
        if (id !== reqId.current) return;
        acc += p;
        setSentenceTr(acc);
      });
      if (id !== reqId.current) return;
      setSentenceTr(res);
      setSentState("done");
    } catch (e) {
      if (id !== reqId.current) return;
      setSentState(String(e).includes("interrompu") ? "idle" : "error");
    }
  };

  const saveMine = () => {
    if (!sel) return;
    const v = mine.trim();
    if (v !== (term?.translation ?? "")) onTranslation(sel.key, v, term?.note ?? aiNote, sel.sentence);
  };

  if (!sel) {
    return (
      <aside className="word-panel" aria-label="Détail du mot">
        <div className="wp-top drag" data-tauri-drag-region />
        <div className="wp-scroll">
          <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} style={{ display: "flex", flexDirection: "column", gap: 10, paddingTop: 6 }}>
            <h2 className="display" style={{ fontSize: 28 }}>
              Touchez un mot
            </h2>
            <p className="muted" style={{ lineHeight: 1.55 }}>
              Sa traduction dans cette phrase précise s'affiche ici, calculée sur votre Mac. Glissez sur plusieurs mots pour traduire une expression.
            </p>
          </motion.div>
          <div className="wp-block" style={{ marginTop: "auto" }}>
            <span className="eyebrow">Légende</span>
            <div className="wp-legend">
              <div>
                <span className="sw" style={{ background: "var(--w-new)" }} />
                Nouveau
              </div>
              <div>
                <span className="sw" style={{ background: "linear-gradient(90deg, var(--w-l1), var(--w-l3))" }} />
                En apprentissage (1 à 3)
              </div>
              <div>
                <span className="sw" style={{ boxShadow: "inset 0 0 0 1px var(--border-strong)" }} />
                Connu
              </div>
            </div>
          </div>
          <div className="wp-block">
            <span className="eyebrow">Raccourcis</span>
            <div className="shortcuts">
              <span>
                <span className="kbd">←</span>
                <span className="kbd">→</span>
              </span>
              <span>Mot précédent, suivant</span>
              <span>
                <span className="kbd">1</span>
                <span className="kbd">2</span>
                <span className="kbd">3</span>
                <span className="kbd">K</span>
              </span>
              <span>Statut du mot (K : connu)</span>
              <span>
                <span className="kbd">X</span>
              </span>
              <span>Ignorer (nom propre…)</span>
              <span>
                <span className="kbd">Espace</span>
              </span>
              <span>Lecture, pause</span>
              <span>
                <span className="kbd">↵</span>
              </span>
              <span>Terminer la page</span>
            </div>
          </div>
        </div>
      </aside>
    );
  }

  const status = term?.status ?? 0;
  const entries = dict?.entries ?? [];
  const lemma = dict?.lemma;
  const firstIpa = entries.find((e) => e.ipa)?.ipa;
  const firstPos = entries[0]?.pos;
  const statusLabel =
    status === 0 ? "Nouveau" : status === 4 ? "Connu" : status === 5 ? "Ignoré" : `En apprentissage · niveau ${status}`;

  return (
    <aside className="word-panel" aria-label="Détail du mot">
      <div className="wp-top drag" data-tauri-drag-region />
      <AnimatePresence mode="wait">
        <motion.div
          key={key}
          className="wp-scroll"
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -6 }}
          transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
        >
          <div>
            <div className="wp-word">
              <h2>{sel.surface}</h2>
              <div style={{ display: "flex", gap: 2 }}>
                <button className="icon-btn" onClick={() => sayWord(sel.surface, lang, settings[`voice_${lang}`])} aria-label="Prononcer">
                  <Icon name="speaker" size={18} />
                </button>
                <button className="icon-btn" onClick={onClose} aria-label="Fermer">
                  <Icon name="close" size={16} />
                </button>
              </div>
            </div>
            <div className="wp-sub">
              {sel.isPhrase && <span className="chip">Expression</span>}
              {firstIpa && <span className="ipa">/{firstIpa}/</span>}
              {firstPos && <span>{firstPos}</span>}
              {lemma && (
                <span>
                  forme de <b>{lemma}</b>
                </span>
              )}
            </div>
            {dict?.form_note && <div className="wp-sub">{dict.form_note}</div>}
          </div>

          <div className="wp-block">
            <span className="eyebrow">Dans cette phrase</span>
            {aiState === "nomodel" ? (
              <div className="wp-note" style={{ display: "flex", flexDirection: "column", gap: 10, alignItems: "flex-start" }}>
                {llmDownloading ? "Le modèle de traduction se télécharge. La traduction en contexte apparaîtra ici dès qu'il sera prêt." : "La traduction en contexte demande un modèle d'IA locale."}
                {!llmDownloading && (
                  <button className="btn sm soft" onClick={() => go("settings")}>
                    Installer un modèle
                  </button>
                )}
              </div>
            ) : aiState === "error" ? (
              <div className="wp-note">{aiError}</div>
            ) : aiState === "loading" ? (
              <>
                <div className="skeleton" style={{ height: 28, width: "60%" }} />
                {slow && <span className="muted" style={{ fontSize: 12.5 }}>L'IA locale s'éveille… La toute première fois, cela prend quelques secondes.</span>}
              </>
            ) : (
              <div className={`wp-ai ${aiState === "stream" ? "stream" : ""}`}>{ai || "—"}</div>
            )}
            {aiNote && aiState === "done" && <div className="wp-note">{aiNote}</div>}
          </div>

          {entries.length > 0 && (
            <div className="wp-block">
              <span className="eyebrow">Dictionnaire</span>
              <div className="wp-gloss">
                {entries.slice(0, 2).map((e, i) => (
                  <div key={i}>
                    {(entries.length > 1 || e.word !== sel.surface.toLowerCase()) && (
                      <div className="pos">
                        {e.word} · {e.pos}
                      </div>
                    )}
                    {e.glosses.slice(0, 3).map((g, j) => (
                      <button key={j} onClick={() => (setMine(g), onTranslation(sel.key, g, term?.note ?? aiNote, sel.sentence))} title="Utiliser comme ma traduction">
                        <span className="n">{j + 1}</span>
                        <span>{g}</span>
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            </div>
          )}

          <div className="wp-block">
            <label className="eyebrow" htmlFor="wp-mine">
              Ma traduction
            </label>
            <input
              id="wp-mine"
              className="input"
              value={mine}
              placeholder="Ajoutez votre propre sens"
              onChange={(e) => setMine(e.target.value)}
              onBlur={saveMine}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                e.stopPropagation();
              }}
            />
          </div>

          <div className="wp-block">
            <span className="eyebrow">Phrase</span>
            <p className="wp-sentence">
              {sel.before}
              <mark>{sel.surface}</mark>
              {sel.after}
            </p>
            {sentState === "done" || sentState === "loading" ? (
              <p className="wp-sentence-tr">{sentenceTr || <span className="skeleton" style={{ display: "block", height: 16, width: "80%" }} />}</p>
            ) : (
              aiState !== "nomodel" && (
                <button className="btn sm soft" style={{ alignSelf: "flex-start" }} onClick={() => translateSentence()}>
                  <Icon name="sparkle" size={14} /> Traduire la phrase
                </button>
              )
            )}
            <div className="wp-actions">
              <button className="btn sm ghost" onClick={() => onPlayFrom(sel.tokenIndex)}>
                <Icon name="play" size={12} /> Écouter depuis ici
              </button>
              {sel.phrase && (
                <button className="btn sm ghost" onClick={() => onSelectPhrase(sel.phrase!.a, sel.phrase!.b)}>
                  <Icon name="layers" size={14} /> Voir l'expression
                </button>
              )}
            </div>
          </div>

          <div className="wp-block">
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
              <span className="eyebrow">Statut</span>
              <span className="muted" style={{ fontSize: 12 }}>
                {statusLabel}
              </span>
            </div>
            <div className="status-bar" role="radiogroup" aria-label="Statut du mot">
              {STATUS_OPTS.map((o) => (
                <button key={o.s} role="radio" aria-checked={status === o.s} className={status === o.s ? "on" : ""} onClick={() => onStatus(sel.key, o.s)}>
                  {status === o.s && <motion.span layoutId="st-pill" className="st-pill" transition={{ type: "spring", stiffness: 500, damping: 36 }} />}
                  <span className="sw" style={{ background: o.sw }} />
                  {o.label}
                </button>
              ))}
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <button className="btn sm ghost" onClick={() => onStatus(sel.key, 5)}>
                <Icon name="ban" size={14} /> Ignorer
              </button>
              {status !== 0 && (
                <button className="btn sm ghost" onClick={() => onStatus(sel.key, 0)}>
                  Redevenir nouveau
                </button>
              )}
            </div>
          </div>
        </motion.div>
      </AnimatePresence>
      <div className="wp-foot">
        <span className="dot ok" />
        {aiState === "nomodel" ? "Dictionnaire hors ligne" : "Traduit sur votre Mac · aucune donnée envoyée"}
      </div>
    </aside>
  );
}
