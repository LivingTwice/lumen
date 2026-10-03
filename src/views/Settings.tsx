import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { Segmented, Switch } from "../components/ui";
import { api, isTauri } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { t, type UiLang } from "../lib/i18n";
import { LANGS, STARTERS, coreLangs, langInfo, langLower, starterCollection, theLang } from "../lib/langs";
import { PROFILES } from "../lib/profiles";
import { formatBytes, useApp } from "../lib/store";
import { naturalVoiceFor, naturalVoices, pronounce } from "../lib/pronounce";
import { loadVoices, sayWord, voicesFor } from "../lib/tts";
import { useUpdate } from "../lib/updater";
import type { LangCode, ModelRow } from "../lib/types";
import { BackupSection } from "./BackupSection";
import { LingqSection } from "./LingqSection";

function ModelLine({ m }: { m: ModelRow }) {
  const dl = useApp((s) => s.downloads[m.id]);
  const download = useApp((s) => s.download);
  const cancel = useApp((s) => s.cancelDownload);
  const refresh = useApp((s) => s.refreshModels);
  const setSetting = useApp((s) => s.setSetting);
  const settings = useApp((s) => s.settings);
  // la voix n'a qu'un modèle : pas de choix « Utiliser »
  const activeKey = m.kind === "llm" ? "llm_model" : m.kind === "asr" ? "asr_model" : null;
  const isActive = !!activeKey && settings[activeKey] === m.id;

  const remove = async () => {
    const ok = await confirmAsk(
      t(`Supprimer ${m.name} (${formatBytes(m.size)}) de ce Mac ?`, `Delete ${m.name} (${formatBytes(m.size)}) from this Mac?`),
      t("Supprimer le modèle", "Delete the model"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return;
    await api().modelDelete(m.id);
    await refresh();
  };

  return (
    <div className="set-row model-row">
      <div className="grow">
        <strong>
          {m.name} {isActive && m.installed && <span className="chip light" style={{ marginLeft: 6, height: 22 }}>{t("Actif", "Active")}</span>}
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
              {t(`${formatBytes(dl.received)} sur ${formatBytes(dl.total)}`, `${formatBytes(dl.received)} of ${formatBytes(dl.total)}`)}
              {dl.speed > 0 ? ` · ${formatBytes(dl.speed)}/s` : ""}
            </span>
          </>
        )}
        {dl?.error && <span style={{ color: "var(--danger)" }}>{dl.error}</span>}
      </div>
      {m.installed ? (
        <>
          {activeKey && !isActive && (
            <button className="btn sm soft" onClick={() => setSetting(activeKey, m.id)}>
              {t("Utiliser", "Use")}
            </button>
          )}
          <button className="icon-btn" onClick={remove} aria-label={t(`Supprimer ${m.name}`, `Delete ${m.name}`)}>
            <Icon name="trash" size={16} />
          </button>
        </>
      ) : dl && !dl.error ? (
        <button className="btn sm ghost" onClick={() => cancel(m.id)}>
          {t("Annuler", "Cancel")}
        </button>
      ) : (
        <button className="btn sm outline" onClick={() => (activeKey && setSetting(activeKey, m.id), download(m.id))}>
          <Icon name="download" size={14} /> {m.partial > 0 || dl?.error ? t("Reprendre", "Resume") : t("Télécharger", "Download")}
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
      // la leçon d'accueil, sans doublon si la langue avait déjà été étudiée
      const s = STARTERS[code];
      const exists = (await api().lessonsList(code)).some((x) => x.title === s.title);
      if (!exists) await api().lessonCreate({ lang: code, title: s.title, text: s.text, collection: starterCollection() });
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
              <h1>{t("Réglages", "Settings")}</h1>
              <p>{t("Tout fonctionne sur votre Mac, sans compte ni connexion.", "Everything works on your Mac, with no account and no connection.")}</p>
            </div>
          </header>

          <section className="set-section" id="set-ui-lang">
            <h2>{t("Langue de l'interface", "Interface language")}</h2>
            <p>
              {t(
                "C'est aussi la langue des traductions, des explications du chat et des dictionnaires.",
                "It is also the language of translations, chat explanations and dictionaries.",
              )}
            </p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Lumen en", "Lumen in")}</strong>
                </div>
                <Segmented
                  id="ui-lang"
                  label={t("Langue de l'interface", "Interface language")}
                  value={settings.ui_lang || "fr"}
                  onChange={(v) => void setSetting("ui_lang", v as UiLang)}
                  options={[
                    { value: "fr", label: "Français" },
                    { value: "en", label: "English" },
                  ]}
                />
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>{t("IA locale", "Local AI")}</h2>
            <p>{t("Choisissez la puissance des modèles. Ils sont téléchargés une seule fois puis fonctionnent hors ligne.", "Choose how powerful the models are. They are downloaded once, then work offline.")}</p>
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
                  <span className="eyebrow">{t("Traduction et réécriture", "Translation and rewriting")}</span>
                </div>
              </div>
              {models
                .filter((m) => m.kind === "llm")
                .map((m) => (
                  <ModelLine key={m.id} m={m} />
                ))}
              <div className="set-row">
                <div className="grow">
                  <span className="eyebrow">{t("Transcription audio et vidéo", "Audio and video transcription")}</span>
                </div>
              </div>
              {models
                .filter((m) => m.kind === "asr")
                .map((m) => (
                  <ModelLine key={m.id} m={m} />
                ))}
              <div className="set-row">
                <div className="grow">
                  <span className="eyebrow">{t("Texte des transcriptions", "Transcript text")}</span>
                  <span>
                    {t(
                      "Facultatif, conseillé : Qwen3-ASR écrit le texte, plus juste et sans phrase sautée, et Whisper repère chaque mot dans le temps pour la lanterne. Il couvre 23 langues ; l'estonien, le letton, le lituanien, le slovaque, le slovène, le croate, le bulgare et l'ukrainien restent transcrits par Whisper seul.",
                      "Optional, recommended: Qwen3-ASR writes the text, more accurately and without skipped sentences, and Whisper times each word for the lantern. It covers 23 languages; Estonian, Latvian, Lithuanian, Slovak, Slovenian, Croatian, Bulgarian and Ukrainian are still transcribed by Whisper alone.",
                    )}
                  </span>
                </div>
              </div>
              {models
                .filter((m) => m.kind === "asrtext")
                .map((m) => (
                  <ModelLine key={m.id} m={m} />
                ))}
            </div>
          </section>

          <section className="set-section" id="set-langs">
            <h2>{t("Langues étudiées", "Languages you study")}</h2>
            <p>{t("Chaque langue a sa bibliothèque, son vocabulaire et ses progrès. Retirer une langue garde ses leçons et ses mots.", "Each language has its own library, vocabulary and progress. Removing a language keeps its lessons and words.")}</p>
            <div className="set-card">
              {langs.map((code) => {
                const l = langInfo(code);
                return (
                  <div key={l.code} className="set-row">
                    <span className="lang-badge" style={{ background: l.color }}>
                      {l.badge}
                    </span>
                    <div className="grow">
                      <strong>{l.name}</strong>
                      <span>
                        {l.native}
                        {coreLangs().includes(l.code)
                          ? t(" · dictionnaire hors ligne inclus", " · offline dictionary (downloaded on first use)")
                          : t(" · traduction par l'IA et voix naturelle", " · AI translation and natural voice")}
                      </span>
                    </div>
                    <Switch on onChange={() => toggleLang(l.code)} label={t(`Ne plus étudier ${langLower(l.code)}`, `Stop studying ${l.name}`)} />
                  </div>
                );
              })}
            </div>
            <div className="lang-add">
              <span className="eyebrow">{t("Ajouter une langue", "Add a language")}</span>
              <div className="lang-chips">
                {LANGS.filter((l) => !langs.includes(l.code)).map((l) => (
                  <button key={l.code} className="lang-chip" onClick={() => toggleLang(l.code)} title={`${l.name} · ${l.native}`}>
                    <span className="lang-badge" style={{ background: l.color }}>
                      {l.badge}
                    </span>
                    {l.name}
                    <Icon name="plus" size={13} stroke={2.2} />
                  </button>
                ))}
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>{t("Lecture", "Reading")}</h2>
            <p>{t("Réglez la page à votre œil.", "Adjust the page to your eye.")}</p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Taille du texte", "Text size")}</strong>
                  <span className="num">{settings.font_size} px</span>
                </div>
                <input className="range" style={{ width: 220 }} type="range" min={17} max={32} value={settings.font_size} onChange={(e) => setSetting("font_size", e.target.value)} aria-label={t("Taille du texte", "Text size")} />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Interligne", "Line spacing")}</strong>
                </div>
                <Segmented
                  id="lh"
                  value={settings.line_height}
                  onChange={(v) => setSetting("line_height", v)}
                  options={[
                    { value: "1.55", label: t("Serré", "Tight") },
                    { value: "1.75", label: "Normal" },
                    { value: "1.95", label: t("Aéré", "Airy") },
                  ]}
                />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Marquage des mots", "Word highlighting")}</strong>
                  <span>{t("Teinte douce ou simple soulignement", "Soft tint or simple underline")}</span>
                </div>
                <Segmented
                  id="ws"
                  value={settings.word_style}
                  onChange={(v) => setSetting("word_style", v)}
                  options={[
                    { value: "tint", label: t("Teinte", "Tint") },
                    { value: "line", label: t("Soulignement", "Underline") },
                  ]}
                />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Terminer la page marque les mots bleus comme connus", "Finishing the page marks blue words as known")}</strong>
                  <span>{t("Le principe de LingQ : un mot lu sans être consulté est compris.", "The LingQ principle: a word you read without looking it up is understood.")}</span>
                </div>
                <Switch on={settings.finish_marks_known !== "0"} onChange={(v) => setSetting("finish_marks_known", v ? "1" : "0")} label={t("Marquer comme connus", "Mark as known")} />
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Traduire automatiquement la phrase", "Translate the sentence automatically")}</strong>
                  <span>{t("Après chaque mot touché", "After each word you tap")}</span>
                </div>
                <Switch on={settings.auto_sentence !== "0"} onChange={(v) => setSetting("auto_sentence", v ? "1" : "0")} label={t("Traduire la phrase", "Translate the sentence")} />
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>{t("Voix", "Voice")}</h2>
            <p>
              {t(
                "La voix naturelle prononce les mots et les expressions que vous touchez, et lit toute une leçon de texte avec le bouton « Créer l'audio ». Elle est calculée sur votre Mac, seulement quand vous en avez besoin. Les voix du système lisent la leçon à voix haute sans préparation.",
                "The natural voice pronounces the words and phrases you tap, and reads a whole text lesson with the “Create audio” button. It is computed on your Mac, only when you need it. The system voices read the lesson aloud with no preparation.",
              )}
            </p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <span className="eyebrow">{t("Voix naturelle", "Natural voice")}</span>
                </div>
              </div>
              {models
                .filter((m) => m.kind === "tts")
                .map((m) => (
                  <ModelLine key={m.id} m={m} />
                ))}
              {models.some((m) => m.kind === "tts" && m.installed) && (
                <div className="set-row">
                  <div className="grow">
                    <strong>{t(`Voix pour ${theLang(lang)}`, `Voice for ${theLang(lang)}`)}</strong>
                    <span>{t("10 voix, comprises dans le téléchargement ; elle lit aussi l'audio créé pour vos leçons", "10 voices, included in the download; it also reads the audio created for your lessons")}</span>
                  </div>
                  <select className="select" value={naturalVoiceFor(lang)} onChange={(e) => setSetting(`tts_voice_${lang}`, e.target.value)} aria-label={t("Voix naturelle", "Natural voice")}>
                    {[...new Set(naturalVoices().map((v) => v.group))].map((g) => (
                      <optgroup key={g} label={g}>
                        {naturalVoices().filter((v) => v.group === g).map((v) => (
                          <option key={v.id} value={v.id}>
                            {v.name}
                          </option>
                        ))}
                      </optgroup>
                    ))}
                  </select>
                  <button className="icon-btn" onClick={() => void pronounce(STARTERS[lang].text.split(/[.!?。]/)[0].split(/\s+/).slice(0, 8).join(" "), lang, settings[voiceKey])} aria-label={t("Écouter la voix naturelle", "Listen to the natural voice")}>
                    <Icon name="speaker" />
                  </button>
                </div>
              )}
              <div className="set-row">
                <div className="grow">
                  <span className="eyebrow">{t("Lecture à voix haute", "Reading aloud")}</span>
                </div>
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t(`Voix du système pour ${theLang(lang)}`, `System voice for ${theLang(lang)}`)}</strong>
                  <span>
                    {voices.length
                      ? t(`${voices.length} voix disponible${voices.length > 1 ? "s" : ""}`, `${voices.length} voice${voices.length > 1 ? "s" : ""} available`)
                      : t("Aucune voix installée pour cette langue", "No voice installed for this language")}
                    {t(" · d'autres voix dans Réglages Système › Accessibilité › Contenu énoncé", " · more voices in System Settings › Accessibility › Spoken Content")}
                  </span>
                </div>
                <select className="select" value={settings[voiceKey] ?? ""} onChange={(e) => setSetting(voiceKey, e.target.value)} aria-label={t("Voix", "Voice")}>
                  <option value="">{t("Automatique (la plus naturelle)", "Automatic (the most natural)")}</option>
                  {voices.map((v) => (
                    <option key={v.voiceURI} value={v.voiceURI}>
                      {v.name} · {v.lang}
                    </option>
                  ))}
                </select>
                <button className="icon-btn" onClick={() => sayWord(STARTERS[lang].text.split(".")[0], lang, settings[voiceKey], Number(settings.tts_rate) || 0.95)} aria-label={t("Écouter la voix", "Listen to the voice")}>
                  <Icon name="speaker" />
                </button>
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>{t("Vidéos en ligne", "Online videos")}</h2>
            <p>
              {t(
                "YouTube et la plupart des sites vidéo. Le son est transcrit sur votre Mac et l'image téléchargée en haute définition pour regarder avec la transcription synchronisée.",
                "YouTube and most video sites. The sound is transcribed on your Mac and the picture downloaded in high definition, so you can watch with the transcript in sync.",
              )}
            </p>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Navigateur connecté à YouTube", "Browser signed in to YouTube")}</strong>
                  <span>{t("Utilisé seulement si YouTube demande de se connecter pour une vidéo.", "Used only if YouTube asks you to sign in for a video.")}</span>
                </div>
                <select className="select" value={settings.youtube_browser ?? ""} onChange={(e) => setSetting("youtube_browser", e.target.value)} aria-label={t("Navigateur", "Browser")}>
                  <option value="">{t("Aucun", "None")}</option>
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
                  <strong>{t("Composants vidéo", "Video components")}</strong>
                  <span>
                    {info?.ytdlp
                      ? t("Installés et tenus à jour automatiquement.", "Installed and kept up to date automatically.")
                      : t("Installés automatiquement au premier import (environ 40 Mo).", "Installed automatically on the first import (about 40 MB).")}
                  </span>
                </div>
                <span className={`dot ${info?.ytdlp ? "ok" : ""}`} />
              </div>
            </div>
          </section>

          <BackupSection />

          <LingqSection />

          <section className="set-section">
            <h2>{t("Apparence", "Appearance")}</h2>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Thème", "Theme")}</strong>
                </div>
                <Segmented
                  id="theme"
                  value={settings.theme}
                  onChange={(v) => setSetting("theme", v)}
                  options={[
                    { value: "system", label: t("Système", "System") },
                    { value: "light", label: t("Clair", "Light") },
                    { value: "dark", label: t("Sombre", "Dark") },
                  ]}
                />
              </div>
            </div>
          </section>

          <section className="set-section">
            <h2>{t("À propos", "About")}</h2>
            <div className="set-card">
              <div className="set-row">
                <div className="grow">
                  <strong>Lumen {info?.version}</strong>
                  <span>{t(`Données : ${info?.data_dir ?? ""}`, `Data: ${info?.data_dir ?? ""}`)}</span>
                </div>
                {isTauri && info && (
                  <button
                    className="btn sm soft"
                    onClick={() => import("@tauri-apps/plugin-opener").then((o) => o.revealItemInDir(info.data_dir + "/lumen.db"))}
                  >
                    {t("Ouvrir le dossier", "Open the folder")}
                  </button>
                )}
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Mises à jour", "Updates")}</strong>
                  <span>
                    {upd.phase === "checking"
                      ? t("Recherche en cours…", "Checking…")
                      : upd.phase === "uptodate"
                        ? t("Lumen est à jour.", "Lumen is up to date.")
                        : upd.phase === "available"
                          ? t(`La version ${upd.version} est disponible.`, `Version ${upd.version} is available.`)
                          : upd.phase === "downloading"
                            ? t(`Téléchargement… ${Math.round(upd.progress * 100)} %`, `Downloading… ${Math.round(upd.progress * 100)}%`)
                            : upd.phase === "ready"
                              ? t("Installée : redémarrez Lumen pour l'utiliser.", "Installed: restart Lumen to use it.")
                              : upd.phase === "error"
                                ? t(`Impossible de vérifier : ${upd.error}`, `Couldn't check: ${upd.error}`)
                                : t("Lumen vérifie automatiquement au démarrage et toutes les six heures.", "Lumen checks automatically at launch and every six hours.")}
                  </span>
                </div>
                {upd.phase === "available" ? (
                  <button className="btn sm primary" onClick={upd.install}>
                    {t("Mettre à jour", "Update")}
                  </button>
                ) : upd.phase === "ready" ? (
                  <button className="btn sm primary" onClick={upd.restart}>
                    {t("Redémarrer", "Restart")}
                  </button>
                ) : (
                  <button className="btn sm soft" onClick={() => upd.check(true)} disabled={upd.phase === "checking" || upd.phase === "downloading"}>
                    {t("Rechercher", "Check")}
                  </button>
                )}
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Écran d'accueil", "Welcome screen")}</strong>
                  <span>{t("Revoir l'aube de Lumen. Vos leçons et votre vocabulaire restent intacts.", "See Lumen's dawn again. Your lessons and vocabulary stay intact.")}</span>
                </div>
                <button className="btn sm soft" onClick={() => useApp.getState().setReplay(true)}>
                  {t("Revoir l'accueil", "Replay the welcome")}
                </button>
              </div>
              <div className="set-row">
                <div className="grow">
                  <strong>{t("Crédits", "Credits")}</strong>
                  <span>
                    {t(
                      "Dictionnaires : Wiktionnaire via kaikki.org (CC BY-SA 4.0). Traduction : Qwen3.5 (Apache 2.0) par llama.cpp (MIT). Transcription : Qwen3-ASR (Apache 2.0) par llama.cpp et Whisper (MIT) par whisper.cpp. Polices : Literata, Newsreader, Geist (OFL).",
                      "Dictionaries: Wiktionary via kaikki.org (CC BY-SA 4.0). Translation: Qwen3.5 (Apache 2.0) with llama.cpp (MIT). Transcription: Qwen3-ASR (Apache 2.0) with llama.cpp, and Whisper (MIT) with whisper.cpp. Fonts: Literata, Newsreader, Geist (OFL).",
                    )}
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
