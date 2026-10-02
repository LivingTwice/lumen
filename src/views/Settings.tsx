import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { Segmented, Switch } from "../components/ui";
import { api, isTauri } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { LANGS, STARTERS, langInfo } from "../lib/langs";
import { PROFILES } from "../lib/profiles";
import { formatBytes, useApp } from "../lib/store";
import { loadVoices, sayWord, voicesFor } from "../lib/tts";
import { useUpdate } from "../lib/updater";
import type { LangCode, ModelRow } from "../lib/types";
import { LingqSection } from "./LingqSection";

function ModelLine({ m }: { m: ModelRow }) {
  const dl = useApp((s) => s.downloads[m.id]);
  const download = useApp((s) => s.download);
  const cancel = useApp((s) => s.cancelDownload);
  const refresh = useApp((s) => s.refreshModels);
  const setSetting = useApp((s) => s.setSetting);
  const settings = useApp((s) => s.settings);
  const activeKey = m.kind === "llm" ? "llm_model" : "asr_model";
  const isActive = settings[activeKey] === m.id;

  const remove = async () => {
    if (!(await confirmAsk(`Supprimer ${m.name} (${formatBytes(m.size)}) de ce Mac ?`, "Supprimer le modèle", "Supprimer"))) return;
    await api().modelDelete(m.id);
    await refresh();
  };

  return (
    <div className="set-row model-row">
      <div className="grow">
        <strong>
          {m.name} {isActive && m.installed && <span className="chip light" style={{ marginLeft: 6, height: 22 }}>Actif</span>}
        </strong>
        <span>
          {m.detail} · {formatBytes(m.size)}
        </span>
        {dl && !dl.error && (
          <>
            <div className="bar live">
              <i style={{ width: `${(dl.received / Math.max(1, dl.total)) * 100}%` }} />
            </div>
            <span className="num">
              {formatBytes(dl.received)} sur {formatBytes(dl.total)}
              {dl.speed > 0 ? ` · ${formatBytes(dl.speed)}/s` : ""}
            </span>
          </>
        )}
        {dl?.error && <span style={{ color: "var(--danger)" }}>{dl.error}</span>}
      </div>
      {m.installed ? (
        <>
          {!isActive && (
            <button className="btn sm soft" onClick={() => setSetting(activeKey, m.id)}>
              Utiliser
            </button>
          )}
          <button className="icon-btn" onClick={remove} aria-label={`Supprimer ${m.name}`}>
            <Icon name="trash" size={16} />
          </button>
        </>
      ) : dl && !dl.error ? (
        <button className="btn sm ghost" onClick={() => cancel(m.id)}>
          Annuler
        </button>
      ) : (
        <button className="btn sm outline" onClick={() => (setSetting(activeKey, m.id), download(m.id))}>
          <Icon name="download" size={14} /> {m.partial > 0 || dl?.error ? "Reprendre" : "Télécharger"}
        </button>
      )}
    </div>
  );
}

export function Settings() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const models = useApp((s) => s.models);
  const download = useApp((s) => s.download);
  const info = useApp((s) => s.info);
  const lang = useApp((s) => s.lang)();
  const langs = useApp((s) => s.langs)();
  const refreshKnown = useApp((s) => s.refreshKnown);
  const bump = useApp((s) => s.bumpLibrary);
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>([]);
  const upd = useUpdate();

  useEffect(() => {
    loadVoices().then(() => setVoices(voicesFor(lang)));
  }, [lang]);

  const pickProfile = (id: string) => {
    const p = PROFILES.find((x) => x.id === id)!;
    void setSetting("llm_model", p.llm);
    void setSetting("asr_model", p.asr);
    const m = models.find((x) => x.id === p.llm);
    if (m && !m.installed) void download(p.llm);
  };
  const activeProfile = PROFILES.find((p) => p.llm === settings.llm_model)?.id;

  const toggleLang = async (code: LangCode) => {
    const has = langs.includes(code);
    if (has && langs.length === 1) return;
    const next = has ? langs.filter((l) => l !== code) : [...langs, code];
    await setSetting("langs", next.join(","));
    if (!has) {
      await api().lessonCreate({ lang: code, title: STARTERS[code].title, text: STARTERS[code].text, collection: "Pour commencer" });
      bump();
    }
    if (has && code === lang) {
      await setSetting("lang", next[0]);
      await refreshKnown();
    }
  };

  const voiceKey = `voice_${lang}`;

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view">
        <div className="settings">
          <header className="page-head">
            <div>
              <h1>Réglages</h1>
              <p>Tout fonctionne sur votre Mac, sans compte ni connexion.</p>
            </div>
          </header>

          <section className="set-section">
            <h2>IA locale</h2>
            <p>Choisissez la puissance des modèles. Ils sont téléchargés une seule fois puis fonctionnent hors ligne.</p>
            <div className="profile-grid" style={{ marginBottom: 14 }}>
              {PROFILES.map((p) => (
                <button key={p.id} className={`profile ${activeProfile === p.id ? "on" : ""}`} onClick={() => pickProfile(p.id)}>
                  <span className="rays">
                    {[1, 2, 3].map((r) => (
                      <i key={r} className={r <= p.rays ? "on" : ""} />
                    ))}
                  </span>
                  <strong>{p.name}</strong>
                  <span>{p.desc}</span>
                  <span className="size">{formatBytes(p.size)}</span>
                </button>
              ))}
            </div>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <span className="eyebrow">Traduction et réécriture</span>
                </div>
              </div>
              {models
                .filter((m) => m.kind === "llm")
                .map((m) => (
                  <ModelLine key={m.id} m={m} />
                ))}
              <div className="set-row">
                <div className="grow">
                  <span className="eyebrow">Transcription audio et vidéo</span>
                </div>
              </div>
              {models
                .filter((m) => m.kind === "asr")
                .map((m) => (
                  <ModelLine key={m.id} m={m} />
                ))}
            </div>
          </section>

          <section className="set-section">
            <h2>Langues étudiées</h2>
            <p>Chaque langue a sa bibliothèque, son vocabulaire et ses progrès.</p>
            <div className="set-card">
              {LANGS.map((l) => (
                <div key={l.code} className="set-row">
                  <span className="lang-badge" style={{ background: l.color }}>
                    {l.badge}
                  </span>
                  <div className="grow">
                    <strong>{l.name}</strong>
                    <span>
                      {l.native}
                      {info?.dict_langs.includes(l.code) ? " · dictionnaire hors ligne inclus" : ""}
                    </span>
                  </div>
                  <Switch on={langs.includes(l.code)} onChange={() => toggleLang(l.code)} label={`Étudier ${l.name}`} />
                </div>
              ))}
            </div>
          </section>

          <section className="set-section">
            <h2>Lecture</h2>
            <p>Réglez la page à votre œil.</p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>Taille du texte</strong>
                  <span className="num">{settings.font_size} px</span>
                </div>
                <input className="range" style={{ width: 220 }} type="range" min={17} max={32} value={settings.font_size} onChange={(e) => setSetting("font_size", e.target.value)} aria-label="Taille du texte" />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Interligne</strong>
                </div>
                <Segmented
                  id="lh"
                  value={settings.line_height}
                  onChange={(v) => setSetting("line_height", v)}
                  options={[
                    { value: "1.55", label: "Serré" },
                    { value: "1.75", label: "Normal" },
                    { value: "1.95", label: "Aéré" },
                  ]}
                />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Marquage des mots</strong>
                  <span>Teinte douce ou simple soulignement</span>
                </div>
                <Segmented
                  id="ws"
                  value={settings.word_style}
                  onChange={(v) => setSetting("word_style", v)}
                  options={[
                    { value: "tint", label: "Teinte" },
                    { value: "line", label: "Soulignement" },
                  ]}
                />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Terminer la page marque les mots bleus comme connus</strong>
                  <span>Le principe de LingQ : un mot lu sans être consulté est compris.</span>
                </div>
                <Switch on={settings.finish_marks_known !== "0"} onChange={(v) => setSetting("finish_marks_known", v ? "1" : "0")} label="Marquer comme connus" />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Traduire automatiquement la phrase</strong>
                  <span>Après chaque mot touché</span>
                </div>
                <Switch on={settings.auto_sentence !== "0"} onChange={(v) => setSetting("auto_sentence", v ? "1" : "0")} label="Traduire la phrase" />
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>Voix</h2>
            <p>Les voix du système, hors ligne. Pour de meilleures voix : Réglages Système › Accessibilité › Contenu énoncé › Voix du système › Gérer les voix.</p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>Voix pour l'{langInfo(lang).name.toLowerCase()}</strong>
                  <span>{voices.length ? `${voices.length} voix disponibles` : "Aucune voix installée pour cette langue"}</span>
                </div>
                <select className="select" value={settings[voiceKey] ?? ""} onChange={(e) => setSetting(voiceKey, e.target.value)} aria-label="Voix">
                  <option value="">Automatique (la plus naturelle)</option>
                  {voices.map((v) => (
                    <option key={v.voiceURI} value={v.voiceURI}>
                      {v.name} · {v.lang}
                    </option>
                  ))}
                </select>
                <button className="icon-btn" onClick={() => sayWord(STARTERS[lang].text.split(".")[0], lang, settings[voiceKey], Number(settings.tts_rate) || 0.95)} aria-label="Écouter la voix">
                  <Icon name="speaker" />
                </button>
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>Vidéos en ligne</h2>
            <p>YouTube et la plupart des sites vidéo. Le son est transcrit sur votre Mac et l'image téléchargée en haute définition pour regarder avec la transcription synchronisée.</p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>Navigateur connecté à YouTube</strong>
                  <span>Utilisé seulement si YouTube demande de se connecter pour une vidéo.</span>
                </div>
                <select className="select" value={settings.youtube_browser ?? ""} onChange={(e) => setSetting("youtube_browser", e.target.value)} aria-label="Navigateur">
                  <option value="">Aucun</option>
                  <option value="safari">Safari</option>
                  <option value="chrome">Chrome</option>
                  <option value="firefox">Firefox</option>
                  <option value="brave">Brave</option>
                  <option value="edge">Edge</option>
                  <option value="vivaldi">Vivaldi</option>
                  <option value="opera">Opera</option>
                </select>
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Composants vidéo</strong>
                  <span>{info?.ytdlp ? "Installés et tenus à jour automatiquement." : "Installés automatiquement au premier import (environ 40 Mo)."}</span>
                </div>
                <span className={`dot ${info?.ytdlp ? "ok" : ""}`} />
              </div>
            </div>
          </section>

          <LingqSection />

          <section className="set-section">
            <h2>Apparence</h2>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>Thème</strong>
                </div>
                <Segmented
                  id="theme"
                  value={settings.theme}
                  onChange={(v) => setSetting("theme", v)}
                  options={[
                    { value: "system", label: "Système" },
                    { value: "light", label: "Clair" },
                    { value: "dark", label: "Sombre" },
                  ]}
                />
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>À propos</h2>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>Lumen {info?.version}</strong>
                  <span>Données : {info?.data_dir}</span>
                </div>
                {isTauri && info && (
                  <button
                    className="btn sm soft"
                    onClick={() => import("@tauri-apps/plugin-opener").then((o) => o.revealItemInDir(info.data_dir + "/lumen.db"))}
                  >
                    Ouvrir le dossier
                  </button>
                )}
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Mises à jour</strong>
                  <span>
                    {upd.phase === "checking"
                      ? "Recherche en cours…"
                      : upd.phase === "uptodate"
                        ? "Lumen est à jour."
                        : upd.phase === "available"
                          ? `La version ${upd.version} est disponible.`
                          : upd.phase === "downloading"
                            ? `Téléchargement… ${Math.round(upd.progress * 100)} %`
                            : upd.phase === "ready"
                              ? "Installée : redémarrez Lumen pour l'utiliser."
                              : upd.phase === "error"
                                ? `Impossible de vérifier : ${upd.error}`
                                : "Lumen vérifie automatiquement au démarrage et toutes les six heures."}
                  </span>
                </div>
                {upd.phase === "available" ? (
                  <button className="btn sm primary" onClick={upd.install}>
                    Mettre à jour
                  </button>
                ) : upd.phase === "ready" ? (
                  <button className="btn sm primary" onClick={upd.restart}>
                    Redémarrer
                  </button>
                ) : (
                  <button className="btn sm soft" onClick={() => upd.check(true)} disabled={upd.phase === "checking" || upd.phase === "downloading"}>
                    Rechercher
                  </button>
                )}
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Écran d'accueil</strong>
                  <span>Revoir l'aube de Lumen. Vos leçons et votre vocabulaire restent intacts.</span>
                </div>
                <button className="btn sm soft" onClick={() => useApp.getState().setReplay(true)}>
                  Revoir l'accueil
                </button>
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>Crédits</strong>
                  <span>
                    Dictionnaires : Wiktionnaire via kaikki.org (CC BY-SA 4.0). Traduction : Qwen3.5 (Apache 2.0) par llama.cpp (MIT). Transcription : Whisper (MIT) par whisper.cpp. Polices : Literata, Newsreader, Geist (OFL).
                  </span>
                </div>
              </div>
            </div>
          </section>
        </div>
      </div>
    </>
  );
}
