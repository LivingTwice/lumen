import { AnimatePresence, motion, useMotionValue, useSpring, useTransform } from "motion/react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { Orb, Switch } from "../components/ui";
import { api, errorText } from "../lib/api";
import { RESTORE_STAGE, formatWhen, pickBackupFolder, reloadProgress, useBackup } from "../lib/backup";
import { CORE_LANGS, LANGS, STARTERS, langInfo, type LangInfo } from "../lib/langs";
import { PROFILES } from "../lib/profiles";
import { formatBytes, formatNumber, useApp } from "../lib/store";
import type { BackupInfo, BackupRestored, LangCode } from "../lib/types";

const EASE = [0.16, 1, 0.3, 1] as const;
// marge au-dessus de la scène pour que les rayons de l'astre ne soient pas coupés
const CLIP_PAD = 200;

const stepAnim = {
  initial: { opacity: 0, y: 16, filter: "blur(10px)" },
  animate: { opacity: 1, y: 0, filter: "blur(0px)" },
  exit: { opacity: 0, y: -10, filter: "blur(10px)" },
  transition: { duration: 0.6, ease: EASE },
};

function glow(e: React.MouseEvent<HTMLElement>) {
  const el = e.currentTarget;
  const r = el.getBoundingClientRect();
  el.style.setProperty("--mx", `${e.clientX - r.left}px`);
  el.style.setProperty("--my", `${e.clientY - r.top}px`);
}

export function Onboarding() {
  const setSetting = useApp((s) => s.setSetting);
  const download = useApp((s) => s.download);
  const models = useApp((s) => s.models);
  const refreshKnown = useApp((s) => s.refreshKnown);
  const openLesson = useApp((s) => s.openLesson);
  const replay = useApp((s) => s.replay);
  const setReplay = useApp((s) => s.setReplay);
  const [n, setN] = useState(0);
  // en relecture, les langues déjà choisies restent cochées
  const [langs, setLangs] = useState<LangCode[]>(() => (replay ? useApp.getState().langs() : []));
  const [profile, setProfile] = useState("balanced");
  const [bloom, setBloom] = useState<{ x: number; y: number } | null>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const orbRef = useRef<HTMLDivElement>(null);
  const [H, setH] = useState(800);
  // sauvegarde : retrouver une progression (étape 4), ou sauvegarder la nouvelle
  const backup = useBackup((s) => s.status);
  const restoring = useBackup((s) => s.restoring);
  const restoreError = useBackup((s) => s.restoreError);
  const [found, setFound] = useState<BackupInfo[] | null>(null);
  const [searchError, setSearchError] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [restored, setRestored] = useState<BackupRestored | null>(null);
  const [saveCloud, setSaveCloud] = useState(true);
  // proposée tant que l'utilisateur n'a pas choisi (pas en relecture d'une sauvegarde déjà réglée)
  const offerBackup = !!backup?.dir && !backup.decided;

  useEffect(() => {
    void useBackup.getState().refresh();
  }, []);

  // parallaxe douce : le ciel suit légèrement le pointeur
  const mx = useMotionValue(0);
  const my = useMotionValue(0);
  const sx = useSpring(mx, { stiffness: 40, damping: 20 });
  const sy = useSpring(my, { stiffness: 40, damping: 20 });
  const skyX = useTransform(sx, (v) => v * -36);
  const skyY = useTransform(sy, (v) => v * -24);
  const orbX = useTransform(sx, (v) => v * 10);

  useLayoutEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setH(el.clientHeight));
    ro.observe(el);
    setH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  const motes = useMemo(
    () =>
      Array.from({ length: 34 }).map((_, i) => ({
        left: `${(i * 41 + 7) % 100}%`,
        top: `${48 + ((i * 29) % 50)}%`,
        delay: `${(i * 0.83) % 12}s`,
        dur: `${10 + (i % 6)}s`,
      })),
    [],
  );

  // géométrie de l'astre selon l'étape
  const orbCenter = n === 0 ? H * 0.32 : n === 3 ? H * 0.3 : 58;
  const orbSize = n === 0 ? 132 : n === 3 ? 104 : 46;
  const horizonY = H * 0.32 + 92;
  const showHorizon = n === 0 || n === 3;

  const toggle = (c: LangCode) => setLangs((l) => (l.includes(c) ? l.filter((x) => x !== c) : [...l, c]));

  const langCard = (l: LangInfo, delay: number) => {
    const on = langs.includes(l.code);
    return (
      <motion.button
        key={l.code}
        className={`ob-card ${on ? "on" : ""}`}
        onClick={() => toggle(l.code)}
        onMouseMove={glow}
        aria-pressed={on}
        title={`${l.name} · ${l.native}`}
        initial={{ opacity: 0, y: 18 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay, duration: 0.6, ease: EASE }}
      >
        <span className="hello" lang={l.code} dir={l.rtl ? "rtl" : undefined}>
          {l.hello}
        </span>
        <span className="meta">
          <span className="dot-lang" style={{ background: l.color }} />
          {CORE_LANGS.includes(l.code) ? `${l.name} · ${l.native}` : l.name}
        </span>
        <AnimatePresence>
          {on && (
            <motion.span className="tick" initial={{ scale: 0 }} animate={{ scale: 1 }} exit={{ scale: 0 }} transition={{ type: "spring", stiffness: 500, damping: 22 }}>
              <Icon name="check" size={14} stroke={2.6} />
            </motion.span>
          )}
        </AnimatePresence>
      </motion.button>
    );
  };

  const startDownload = () => {
    const p = PROFILES.find((x) => x.id === profile)!;
    void setSetting("llm_model", p.llm);
    void setSetting("asr_model", p.asr);
    if (!models.find((m) => m.id === p.llm)?.installed) void download(p.llm);
    setN(3);
  };

  const startBloom = () => {
    const r = orbRef.current?.getBoundingClientRect();
    setBloom({ x: r ? r.left + r.width / 2 : window.innerWidth / 2, y: r ? r.top + r.height / 2 : window.innerHeight / 3 });
  };

  // ---------- retrouver une progression ----------

  const search = async () => {
    setFound(null);
    setSearchError("");
    try {
      const list = (await api().backupList()).filter((b) => !b.newer);
      setFound(list);
      setPicked(list[0]?.key ?? null);
    } catch (e) {
      setFound([]);
      setSearchError(errorText(e));
    }
  };

  const openRestore = () => {
    useBackup.setState({ restoreError: "" });
    setN(4);
    void search();
  };

  const chooseFolder = async () => {
    const dir = await pickBackupFolder();
    if (!dir) return;
    await setSetting("backup_dir", dir);
    void search();
  };

  const doRestore = async () => {
    const info = found?.find((b) => b.key === picked);
    if (!info) return;
    const r = await useBackup.getState().restore(info, null);
    if (!r) return;
    setRestored(r);
    // les modèles d'IA ne voyagent pas avec la sauvegarde : on propose de les télécharger
    setN(models.some((m) => m.kind === "llm" && m.installed) ? 3 : 2);
  };

  const finishRestored = async () => {
    startBloom();
    await new Promise((res) => setTimeout(res, 950));
    setReplay(false);
    // la base restaurée est relue : l'accueil laisse place à la bibliothèque
    await reloadProgress();
  };

  const finish = async () => {
    startBloom();
    const started = performance.now();
    await setSetting("langs", langs.join(","));
    await setSetting("lang", langs[0]);
    let first = 0;
    for (const l of langs) {
      const s = STARTERS[l];
      // pas de doublon si la leçon d'accueil existe déjà (relecture de l'accueil)
      const existing = (await api().lessonsList(l)).find((x) => x.title === s.title);
      const id = existing ? existing.id : await api().lessonCreate({ lang: l, title: s.title, text: s.text, collection: "Pour commencer", kind: "text" });
      if (!first) first = id;
    }
    await refreshKnown();
    const wait = 950 - (performance.now() - started);
    if (wait > 0) await new Promise((res) => setTimeout(res, wait));
    if (offerBackup) await setSetting("backup_on", saveCloud ? "1" : "0");
    await setSetting("onboarded", "1");
    setReplay(false);
    openLesson(first);
    // macOS demande l'accès à iCloud Drive maintenant, juste après le choix
    if (offerBackup && saveCloud) void useBackup.getState().save();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter" && n === 0) setN(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [n]);

  return (
    <div
      className="ob"
      onMouseMove={(e) => {
        mx.set(e.clientX / window.innerWidth - 0.5);
        my.set(e.clientY / window.innerHeight - 0.5);
      }}
    >
      <motion.div className="ob-sky" style={{ x: skyX, y: skyY }} initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 1.8 }}>
        <div className="ob-blob b1" />
        <div className="ob-blob b2" />
        <div className="ob-blob b3" />
        <div className="ob-blob b4" />
        <div className="ob-blob b5" />
      </motion.div>
      <div className="ob-grain" />
      <div className="ob-motes">
        {motes.map((d, i) => (
          <i key={i} style={{ left: d.left, top: d.top, animationDelay: d.delay, animationDuration: d.dur }} />
        ))}
      </div>

      <div className="ob-top drag" data-tauri-drag-region />

      <div className="ob-stage" ref={stageRef}>
        {/* l'astre se lève derrière l'horizon : on le découpe à la ligne d'horizon */}
        <div style={{ position: "absolute", left: 0, right: 0, top: -CLIP_PAD, height: horizonY + CLIP_PAD, overflow: "hidden", pointerEvents: "none", zIndex: 2 }}>
          <motion.div
            ref={orbRef}
            className="ob-orb"
            style={{ x: orbX, translateX: "-50%", translateY: "-50%" }}
            initial={false}
            animate={{ top: orbCenter + CLIP_PAD, width: orbSize, height: orbSize }}
            transition={{ type: "spring", stiffness: 70, damping: 18 }}
          >
            <motion.div
              style={{ position: "absolute", inset: 0 }}
              initial={{ y: 230, opacity: 0.4, scale: 0.85 }}
              animate={{ y: 0, opacity: 1, scale: 1 }}
              transition={{ duration: 2.4, delay: 0.5, ease: EASE }}
            >
              <div className="ob-orb-rays" />
              <div className="ob-orb-halo" />
              <div className="ob-orb-core" />
            </motion.div>
          </motion.div>
        </div>

        <motion.div
          className="ob-horizon"
          style={{ top: horizonY }}
          initial={{ scaleX: 0, opacity: 0 }}
          animate={{ scaleX: showHorizon ? 1 : 0.2, opacity: showHorizon ? 1 : 0 }}
          transition={{ duration: 1.4, delay: n === 0 ? 0.25 : 0, ease: EASE }}
        />

        <AnimatePresence mode="wait">
          {n === 0 && (
            <motion.div key="0" className="ob-step welcome" {...stepAnim}>
              <h1 className="ob-word" aria-label="Lumen">
                {"Lumen".split("").map((c, i) => (
                  <motion.span
                    key={i}
                    className="ch"
                    initial={{ opacity: 0, y: 26, filter: "blur(14px)" }}
                    animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                    transition={{ duration: 1.1, delay: 1.5 + i * 0.09, ease: EASE }}
                  >
                    {c}
                  </motion.span>
                ))}
                <span className="sheen" aria-hidden="true">
                  Lumen
                </span>
              </h1>
              <motion.p className="ob-lead" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 2.3, duration: 1 }}>
                Apprenez une langue comme vous avez appris la vôtre : en lisant et en écoutant ce qui vous passionne. Un mot à la fois, jusqu'à ce que tout s'éclaire.
              </motion.p>
              <motion.div className="ob-welcome-actions" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 2.8, duration: 0.8, ease: EASE }}>
                <button className="ob-cta" onClick={() => setN(1)}>
                  Commencer <Icon name="forward" size={16} stroke={2} />
                </button>
                {!replay && (
                  <button className="ob-restore-link" onClick={openRestore}>
                    <Icon name="cloud" size={15} /> J'ai déjà utilisé Lumen : retrouver ma progression
                  </button>
                )}
              </motion.div>
            </motion.div>
          )}

          {n === 1 && (
            <motion.div key="1" className="ob-step form" {...stepAnim}>
              <h2>Quelles langues voulez-vous apprendre ?</h2>
              <p className="ob-sub">Chacune a sa bibliothèque et son vocabulaire. Vous pourrez en ajouter ou en retirer plus tard.</p>
              <div className="ob-langs-scroll">
                <div className="ob-group">Avec dictionnaire hors ligne</div>
                <div className="ob-grid langs">{LANGS.filter((l) => CORE_LANGS.includes(l.code)).map((l, i) => langCard(l, 0.15 + i * 0.06))}</div>
                <div className="ob-group">Avec la traduction par l'IA et la voix naturelle</div>
                <div className="ob-grid langs compact">{LANGS.filter((l) => !CORE_LANGS.includes(l.code)).map((l, i) => langCard(l, 0.5 + i * 0.025))}</div>
              </div>
              <div className="ob-actions">
                <button className="ob-link" onClick={() => setN(0)}>
                  Retour
                </button>
                <button className="ob-cta" disabled={!langs.length} onClick={() => setN(2)}>
                  Continuer <Icon name="forward" size={16} stroke={2} />
                </button>
              </div>
            </motion.div>
          )}

          {n === 2 && (
            <motion.div key="2" className="ob-step form" {...stepAnim}>
              <h2>Une IA qui vit sur votre Mac</h2>
              <p className="ob-sub">Elle traduit chaque mot dans son contexte, sans jamais rien envoyer en ligne. Le modèle se télécharge pendant que vous commencez à lire.</p>
              <div className="ob-grid profiles">
                {PROFILES.map((p, i) => (
                  <motion.button
                    key={p.id}
                    className={`ob-card ${profile === p.id ? "on" : ""}`}
                    onClick={() => setProfile(p.id)}
                    onMouseMove={glow}
                    initial={{ opacity: 0, y: 18 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ delay: 0.15 + i * 0.08, duration: 0.6, ease: EASE }}
                  >
                    <span className="ob-rays">
                      {[1, 2, 3].map((r) => (
                        <i key={r} className={r <= p.rays ? "on" : ""} />
                      ))}
                    </span>
                    <strong className="pname">{p.name}</strong>
                    <span className="pdesc">{p.desc}</span>
                    <span className="psize">{formatBytes(p.size)}</span>
                  </motion.button>
                ))}
              </div>
              <div className="ob-actions">
                <button className="ob-link" onClick={() => setN(3)}>
                  Plus tard
                </button>
                <button className="ob-cta" onClick={startDownload}>
                  <Icon name="download" size={16} stroke={2} /> Télécharger et continuer
                </button>
              </div>
            </motion.div>
          )}

          {n === 3 && restored && (
            <motion.div key="3r" className="ob-step ready" {...stepAnim}>
              <h2>Bon retour</h2>
              <p className="ob-sub">
                {formatNumber(restored.counts.known)} mots connus et {formatNumber(restored.counts.lessons)} leçon{restored.counts.lessons > 1 ? "s" : ""} vous attendent, exactement là où vous les aviez laissés.
              </p>
              <button className="ob-cta" onClick={finishRestored} disabled={!!bloom}>
                Retrouver ma bibliothèque <Icon name="library" size={16} />
              </button>
            </motion.div>
          )}

          {n === 3 && !restored && (
            <motion.div key="3" className="ob-step ready" {...stepAnim}>
              <h2>Tout est prêt</h2>
              <p className="ob-sub">
                Une courte histoire vous attend dans chaque langue choisie. Touchez les mots inconnus, écoutez la page, puis terminez-la : les mots compris rejoignent votre vocabulaire.
              </p>
              {offerBackup && (
                <motion.label className="ob-backup" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.35, duration: 0.6, ease: EASE }}>
                  <Icon name="cloud" size={20} />
                  <span>
                    <strong>Sauvegarder ma progression dans {backup.icloud ? "iCloud Drive" : "le dossier choisi"}</strong>
                    <small>Une copie à l'abri, retrouvée en un clic si ce Mac s'efface ou sur un nouveau Mac.</small>
                  </span>
                  <Switch on={saveCloud} onChange={setSaveCloud} label="Sauvegarder ma progression" />
                </motion.label>
              )}
              <button className="ob-cta" onClick={finish} disabled={!!bloom || !langs.length}>
                Ouvrir ma première lecture <Icon name="book" size={16} />
              </button>
              {!langs.length && (
                <button className="ob-link" onClick={() => setN(1)}>
                  Choisir une langue d'abord
                </button>
              )}
            </motion.div>
          )}

          {n === 4 && (
            <motion.div key="4" className="ob-step form" {...stepAnim}>
              <h2>Retrouver ma progression</h2>
              <p className="ob-sub">Vos mots, vos expressions, vos leçons et vos réglages reviennent tels que vous les aviez laissés.</p>
              {restoring ? (
                <div className="ob-restoring">
                  <Orb size={30} />
                  <strong>{RESTORE_STAGE[restoring.stage] ?? "Restauration…"}</strong>
                  <div className="ob-progress">
                    <i style={{ width: `${restoring.stage === "media" ? 8 + restoring.value * 92 : restoring.stage === "apply" ? 100 : 8}%` }} />
                  </div>
                </div>
              ) : found === null ? (
                <div className="ob-search">
                  <Orb size={20} /> Recherche dans {backup?.icloud === false ? "le dossier choisi" : "votre iCloud Drive"}…
                </div>
              ) : found.length ? (
                <div className="ob-grid backups">
                  {found.map((b, i) => (
                    <motion.button
                      key={b.key}
                      className={`ob-card backup ${picked === b.key ? "on" : ""}`}
                      onClick={() => setPicked(b.key)}
                      onMouseMove={glow}
                      aria-pressed={picked === b.key}
                      initial={{ opacity: 0, y: 18 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.1 + i * 0.07, duration: 0.6, ease: EASE }}
                    >
                      <span className="ob-device">
                        <Icon name="laptop" size={15} /> {b.device_name}
                      </span>
                      <strong className="pname">{formatNumber(b.counts.known)} mots connus</strong>
                      <span className="pdesc">
                        {formatNumber(b.counts.lessons)} leçon{b.counts.lessons > 1 ? "s" : ""}
                        {b.counts.langs.length ? ` en ${b.counts.langs.map((l) => langInfo(l).name.toLowerCase()).join(", ")}` : ""}
                        <br />
                        Sauvegardée {formatWhen(b.saved_at)}
                      </span>
                      <AnimatePresence>
                        {picked === b.key && (
                          <motion.span className="tick" initial={{ scale: 0 }} animate={{ scale: 1 }} exit={{ scale: 0 }} transition={{ type: "spring", stiffness: 500, damping: 22 }}>
                            <Icon name="check" size={14} stroke={2.6} />
                          </motion.span>
                        )}
                      </AnimatePresence>
                    </motion.button>
                  ))}
                </div>
              ) : (
                <p className="ob-empty">{searchError || `Aucune sauvegarde de Lumen dans ${backup?.icloud === false ? "ce dossier" : "votre iCloud Drive"}.`}</p>
              )}
              {restoreError && !restoring && <p className="ob-error">{restoreError}</p>}
              <div className="ob-actions">
                <button className="ob-link" onClick={() => setN(0)} disabled={!!restoring}>
                  Retour
                </button>
                {found !== null && !found.length ? (
                  <>
                    <button className="ob-link" onClick={chooseFolder}>
                      Choisir un dossier…
                    </button>
                    <button className="ob-cta" onClick={() => setN(1)}>
                      Commencer sans sauvegarde <Icon name="forward" size={16} stroke={2} />
                    </button>
                  </>
                ) : (
                  <button className="ob-cta" onClick={doRestore} disabled={!picked || !!restoring || found === null}>
                    Restaurer <Icon name="forward" size={16} stroke={2} />
                  </button>
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <div className="ob-dots" aria-hidden="true">
        {/* la restauration (étape 4) n'a pas de point : elle remplace les étapes 1 à 3 */}
        {[0, 1, 2, 3].map((i) => (
          <i key={i} className={i === n ? "on" : ""} />
        ))}
      </div>

      {bloom && (
        <motion.div
          className="ob-bloom"
          style={{ left: bloom.x, top: bloom.y }}
          initial={{ scale: 0.4, opacity: 0.6 }}
          animate={{ scale: Math.max(window.innerWidth, window.innerHeight) / 40, opacity: 1 }}
          transition={{ duration: 1.1, ease: [0.7, 0, 0.3, 1] }}
        />
      )}
    </div>
  );
}
