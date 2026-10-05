import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { Avatar, AvatarArt } from "../components/Avatar";
import { Icon } from "../components/Icon";
import { Segmented } from "../components/ui";
import { api, errorText } from "../lib/api";
import { useBackup } from "../lib/backup";
import { confirmAsk } from "../lib/dialogs";
import { count, locale, t } from "../lib/i18n";
import { langInfo } from "../lib/langs";
import { useApp } from "../lib/store";
import type { LangCode } from "../lib/types";
import {
  HUES,
  INTEREST_MAX,
  INTERESTS_MAX,
  NAME_MAX,
  WHY_MAX,
  avatarString,
  avatarStyles,
  choosePhoto,
  interestLabel,
  interestSuggestions,
  interestsString,
  newSeed,
  oneLine,
  useUser,
  type AvatarSpec,
  type AvatarStyle,
} from "../lib/user";

/* Réglages › Profil : nom, avatar, ce qui motive, centres d'intérêt, et où
   vit ce profil. Rien de communautaire : il reste sur ce Mac et dans la
   sauvegarde iCloud de l'apprenant. */

const EASE = [0.2, 0.8, 0.2, 1] as const;

/** Champ texte enregistré en douceur : après une courte pause, et en quittant le champ. */
export function useDraft(key: string, max: number): [string, (v: string) => void, () => void] {
  const saved = useApp((s) => s.settings[key] ?? "");
  const setSetting = useApp((s) => s.setSetting);
  const [draft, setDraft] = useState(saved);
  const timer = useRef<number | undefined>(undefined);
  const editing = useRef(false);
  // une restauration ou un autre écran peut changer la valeur : le champ suit, sauf pendant la frappe
  useEffect(() => {
    if (!editing.current) setDraft(saved);
  }, [saved]);
  const commit = (v = draft) => {
    window.clearTimeout(timer.current);
    editing.current = false;
    const clean = v.trim();
    if (clean !== saved) void setSetting(key, clean);
    if (clean !== v) setDraft(clean);
  };
  const change = (v: string) => {
    const next = oneLine(v, max);
    editing.current = true;
    setDraft(next);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => commit(next), 500);
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return [draft, change, () => commit()];
}

/** « Sur Lumen depuis mars 2026 · 4 210 mots connus en italien et en anglais » */
function useJourney(): string {
  const langs = useApp((s) => s.langs)();
  const version = useApp((s) => s.libraryVersion);
  const [line, setLine] = useState("");
  const key = langs.join(",");
  useEffect(() => {
    let live = true;
    void Promise.all(key.split(",").filter(Boolean).map((l) => api().stats(l as LangCode).catch(() => null))).then((all) => {
      if (!live) return;
      const firsts = all.map((s) => s?.first_day).filter((d): d is string => !!d).sort();
      const known = all.reduce((n, s) => n + (s?.known ?? 0), 0);
      const parts: string[] = [];
      if (firsts[0]) {
        const since = new Date(firsts[0] + "T12:00:00").toLocaleDateString(locale(), { month: "long", year: "numeric" });
        parts.push(t(`Sur Lumen depuis ${since}`, `On Lumen since ${since}`));
      }
      if (known > 0) parts.push(count(known, "mot connu", "mots connus", "known word", "known words"));
      setLine(parts.join(" · "));
    });
    return () => {
      live = false;
    };
  }, [key, version]);
  return line;
}

export function ProfileSection() {
  const me = useUser();
  const setSetting = useApp((s) => s.setSetting);
  const toast = useApp((s) => s.toast);
  const langs = useApp((s) => s.langs)();
  const still = !!useReducedMotion();
  const [name, setName, saveName] = useDraft("user_name", NAME_MAX);
  const [why, setWhy, saveWhy] = useDraft("user_why", WHY_MAX);
  const journey = useJourney();
  const agree = useApp((s) => s.settings.user_agree ?? "");

  const setAvatar = (a: AvatarSpec) => void setSetting("user_avatar", avatarString(a));

  const pickPhoto = async () => {
    try {
      const data = await choosePhoto();
      if (!data) return;
      await setSetting("user_photo", data);
      setAvatar({ ...me.avatar, style: "photo" });
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const removePhoto = async () => {
    await setSetting("user_photo", "");
    if (me.avatar.style === "photo") setAvatar({ ...me.avatar, style: "initial" });
  };

  const clearAll = async () => {
    const ok = await confirmAsk(
      t(
        "Effacer votre nom, votre avatar, votre photo, ce qui vous motive et vos centres d'intérêt ? Vos mots, vos leçons et vos progrès ne changent pas.",
        "Erase your name, avatar, photo, what motivates you and your interests? Your words, lessons and progress stay as they are.",
      ),
      t("Effacer le profil", "Erase the profile"),
      t("Effacer", "Erase"),
    );
    if (!ok) return;
    for (const k of ["user_name", "user_avatar", "user_photo", "user_why", "user_interests", "user_agree"]) await setSetting(k, "");
  };

  return (
    <>
      <motion.div className="me-hero" initial={still ? false : { opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.4, ease: EASE }}>
        <span className="me-hero-glow" aria-hidden="true" />
        <span className="me-hero-avatar">
          <Avatar size={92} />
        </span>
        <div className="me-hero-text">
          <label className="me-name-label" htmlFor="me-name">
            {t("Comment Lumen vous appelle", "What Lumen calls you")}
          </label>
          <input
            id="me-name"
            className="me-name"
            value={name}
            placeholder={t("Votre prénom ou un pseudo", "Your first name or a nickname")}
            onChange={(e) => setName(e.target.value)}
            onBlur={saveName}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            spellCheck={false}
            autoComplete="off"
          />
          <span className="me-journey">
            {langs.map((l) => (
              <span key={l} className="lang-badge" style={{ background: langInfo(l).color }} title={langInfo(l).name}>
                {langInfo(l).badge}
              </span>
            ))}
            <span>{journey}</span>
          </span>
        </div>
      </motion.div>

      <section className="set-section">
        <h3>{t("Avatar", "Avatar")}</h3>
        <p>{t("Une lumière à vous, votre initiale ou une photo. Il vous accompagne dans la barre latérale.", "A light of your own, your initial or a photo. It stays with you in the sidebar.")}</p>
        <AvatarPicker onPhoto={pickPhoto} onRemovePhoto={removePhoto} />
      </section>

      <section className="set-section">
        <h3>{t("Ce qui vous fait apprendre", "Why you're learning")}</h3>
        <p>
          {t(
            "Une phrase, pour vous. Lumen vous la rappelle dans Progrès, et le chat s'en souvient pour choisir ses exemples.",
            "One sentence, for you. Lumen reminds you of it in Progress, and the chat keeps it in mind when choosing examples.",
          )}
        </p>
        <div className="me-why-field">
          <input
            className="input me-why"
            value={why}
            placeholder={t("Par exemple : parler avec ma famille à Naples", "For example: talk with my family in Naples")}
            onChange={(e) => setWhy(e.target.value)}
            onBlur={saveWhy}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            aria-label={t("Ce qui vous fait apprendre", "Why you're learning")}
          />
          <span className="me-count num" aria-hidden="true">
            {why.length > WHY_MAX - 30 ? WHY_MAX - why.length : ""}
          </span>
        </div>
      </section>

      <section className="set-section">
        <h3>{t("Quand le chat vous écrit", "When the chat writes to you")}</h3>
        <p>
          {t(
            "Pour les accords, en français comme dans la langue étudiée : « prêt » ou « prête », « pronto » ou « pronta ».",
            "For agreement in languages with grammatical gender, such as French or Italian: “prêt” or “prête”, “pronto” or “pronta”.",
          )}
        </p>
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{t("Accorder", "Agree")}</strong>
            </div>
            <Segmented
              id="me-agree"
              label={t("Accords", "Agreement")}
              value={agree}
              onChange={(v) => void setSetting("user_agree", v)}
              options={[
                { value: "f", label: t("Au féminin", "Feminine") },
                { value: "m", label: t("Au masculin", "Masculine") },
                { value: "", label: t("Sans préférence", "No preference") },
              ]}
            />
          </div>
        </div>
      </section>

      <section className="set-section">
        <h3>{t("Vos centres d'intérêt", "Your interests")}</h3>
        <p>
          {t(
            `Le chat y puise ses exemples, et les podcasts sur mesure vous les proposent comme sujets. ${INTERESTS_MAX}\u00a0au plus.`,
            `The chat draws its examples from them, and custom podcasts suggest them as topics. Up to ${INTERESTS_MAX}.`,
          )}
        </p>
        <Interests />
      </section>

      <section className="set-section">
        <h3>{t("Où vit votre profil", "Where your profile lives")}</h3>
        <WhereItLives />
      </section>

      {!me.empty || me.why || me.interests.length || agree ? (
        <div className="me-erase">
          <button className="btn sm ghost" onClick={() => void clearAll()}>
            {t("Effacer mon profil", "Erase my profile")}
          </button>
        </div>
      ) : null}
    </>
  );
}

/** Choix de l'avatar : la photo, l'initiale ou une lumière ; sa teinte ; « Une autre ». */
function AvatarPicker({ onPhoto, onRemovePhoto }: { onPhoto(): void; onRemovePhoto(): void }) {
  const me = useUser();
  const photo = useApp((s) => s.settings.user_photo ?? "");
  const setSetting = useApp((s) => s.setSetting);
  // rien de choisi encore : aucune option n'est allumée
  const current: AvatarStyle | null = me.empty ? null : me.avatar.style;
  const set = (patch: Partial<AvatarSpec>) => void setSetting("user_avatar", avatarString({ ...me.avatar, ...patch }));
  const generated = current === "dawn" || current === "halo" || current === "aurora" || current === "night";
  // pas encore de graine : celle des aperçus, pour que la lumière choisie soit bien celle qu'on a vue
  const [fallback] = useState(newSeed);
  const seed = me.avatar.seed || fallback;

  const choose = (style: AvatarStyle) => set({ style, seed });

  return (
    <div className="set-card me-avatar-card">
      <div className="me-avatars" role="radiogroup" aria-label={t("Avatar", "Avatar")}>
        <button
          role="radio"
          aria-checked={current === "photo"}
          className={`me-avatar-opt ${current === "photo" ? "on" : ""}`}
          onClick={() => (photo ? choose("photo") : onPhoto())}
        >
          {current === "photo" && <motion.span layoutId="me-avatar-ring" className="me-avatar-ring" transition={{ type: "spring", stiffness: 520, damping: 38 }} />}
          <span className={`avatar ${photo ? "" : "me-photo-empty"}`} style={{ width: 56, height: 56 }}>
            {photo ? <img className="avatar-img" src={photo} alt="" draggable={false} /> : <Icon name="image" size={20} />}
          </span>
          <span className="me-avatar-label">{t("Photo", "Photo")}</span>
        </button>
        {avatarStyles().map((s) => (
          <button key={s.id} role="radio" aria-checked={current === s.id} className={`me-avatar-opt ${current === s.id ? "on" : ""}`} onClick={() => choose(s.id)}>
            {current === s.id && <motion.span layoutId="me-avatar-ring" className="me-avatar-ring" transition={{ type: "spring", stiffness: 520, damping: 38 }} />}
            <span className="avatar" style={{ width: 56, height: 56 }}>
              <AvatarArt spec={{ style: s.id, hue: me.avatar.hue, seed }} name={me.name} />
            </span>
            <span className="me-avatar-label">{s.label}</span>
          </button>
        ))}
      </div>
      <div className="me-avatar-tools">
        <AnimatePresence mode="popLayout" initial={false}>
          {current === "photo" ? (
            <motion.div key="photo" className="me-tools-row" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
              <button className="btn sm soft" onClick={onPhoto}>
                <Icon name="image" size={14} /> {t("Changer de photo", "Change photo")}
              </button>
              <button className="btn sm ghost" onClick={onRemovePhoto}>
                {t("Retirer la photo", "Remove photo")}
              </button>
              <span className="me-tools-note">{t("Recadrée au carré, et réduite : elle reste légère.", "Cropped to a square and shrunk: it stays light.")}</span>
            </motion.div>
          ) : (
            <motion.div key="hues" className="me-tools-row" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
              <div className="me-hues" role="radiogroup" aria-label={t("Teinte", "Hue")}>
                {HUES.map((h) => (
                  <button
                    key={h}
                    role="radio"
                    aria-checked={me.avatar.hue === h}
                    aria-label={t(`Teinte ${h}°`, `Hue ${h}°`)}
                    className={`me-hue ${me.avatar.hue === h ? "on" : ""}`}
                    style={{ ["--h" as string]: h }}
                    onClick={() => set({ hue: h, seed, ...(me.empty ? { style: "initial" as AvatarStyle } : {}) })}
                  />
                ))}
              </div>
              {generated && (
                <button className="btn sm soft" onClick={() => set({ seed: newSeed() })}>
                  <Icon name="dice" size={14} /> {t("Une autre lumière", "Another light")}
                </button>
              )}
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}

/** Centres d'intérêt : suggestions à allumer, et ceux qu'on ajoute soi-même. */
function Interests() {
  const me = useUser();
  const setSetting = useApp((s) => s.setSetting);
  const [custom, setCustom] = useState("");
  const list = me.interests;
  const full = list.length >= INTERESTS_MAX;
  const save = (next: string[]) => void setSetting("user_interests", interestsString(next));
  const toggle = (id: string) => save(list.includes(id) ? list.filter((x) => x !== id) : full ? list : [...list, id]);
  const add = () => {
    const v = custom.trim();
    if (!v || full) return;
    // une suggestion tapée à la main s'allume plutôt que de faire doublon
    const known = interestSuggestions().find((x) => x.label.toLowerCase() === v.toLowerCase());
    const id = known?.id ?? v;
    if (!list.some((x) => x.toLowerCase() === id.toLowerCase())) save([...list, id]);
    setCustom("");
  };
  const mine = list.filter((x) => !x.startsWith("#"));
  return (
    <div className="me-interests">
      {interestSuggestions().map((s) => {
        const on = list.includes(s.id);
        return (
          <button key={s.id} className={`idea ${on ? "on" : ""}`} aria-pressed={on} disabled={!on && full} onClick={() => toggle(s.id)}>
            {on && <Icon name="check" size={12} stroke={2.4} />}
            {s.label}
          </button>
        );
      })}
      {mine.map((x) => (
        <span key={x} className="idea on me-own">
          {interestLabel(x)}
          <button onClick={() => toggle(x)} aria-label={t(`Retirer ${x}`, `Remove ${x}`)}>
            <Icon name="close" size={10} stroke={2.4} />
          </button>
        </span>
      ))}
      {!full && (
        <span className="idea me-add">
          <Icon name="plus" size={12} stroke={2.2} />
          <input
            value={custom}
            placeholder={t("Autre chose…", "Something else…")}
            onChange={(e) => setCustom(oneLine(e.target.value, INTEREST_MAX))}
            onKeyDown={(e) => e.key === "Enter" && add()}
            onBlur={add}
            aria-label={t("Ajouter un centre d'intérêt", "Add an interest")}
          />
        </span>
      )}
    </div>
  );
}

/** Où vit le profil : ce Mac, la sauvegarde iCloud (si elle est activée), personne d'autre. */
function WhereItLives() {
  const status = useBackup((s) => s.status);
  const openSettings = useApp((s) => s.openSettings);
  useEffect(() => {
    void useBackup.getState().refresh();
  }, []);
  const saved = !!status?.enabled;
  const place = status?.icloud === false ? t("dans le dossier de sauvegarde choisi", "in the chosen backup folder") : t("dans votre iCloud Drive", "in your iCloud Drive");
  return (
    <div className="set-card">
      <div className="set-row">
        <span className="set-row-icon">
          <Icon name="laptop" size={16} />
        </span>
        <div className="grow">
          <strong>{t("Sur ce Mac", "On this Mac")}</strong>
          <span>{t("Dans les données de Lumen, avec vos mots et vos leçons.", "In Lumen's data, with your words and lessons.")}</span>
        </div>
        <span className="dot ok" />
      </div>
      <div className="set-row">
        <span className="set-row-icon">
          <Icon name="cloud" size={16} />
        </span>
        <div className="grow">
          <strong>{saved ? t(`Et ${place}`, `And ${place}`) : t("Pas encore dans iCloud", "Not in iCloud yet")}</strong>
          <span>
            {saved
              ? t(
                  "Il part avec chaque sauvegarde de votre progression : un nouveau Mac le retrouve en même temps que vos mots.",
                  "It goes with every backup of your progress: a new Mac gets it back along with your words.",
                )
              : t(
                  "Activez la sauvegarde pour le retrouver, avec toute votre progression, si ce Mac s'efface ou sur un nouveau Mac.",
                  "Turn on the backup to get it back, with all your progress, if this Mac is wiped or on a new Mac.",
                )}
          </span>
        </div>
        {saved ? (
          <span className="dot ok" />
        ) : (
          <button className="btn sm soft" onClick={() => openSettings("backup")}>
            {t("Activer la sauvegarde", "Turn on the backup")}
          </button>
        )}
      </div>
      <div className="set-row">
        <span className="set-row-icon">
          <Icon name="eye" size={16} />
        </span>
        <div className="grow">
          <strong>{t("Visible de vous seul", "Seen by you alone")}</strong>
          <span>
            {t(
              "Lumen n'a ni compte ni communauté : votre profil n'est envoyé à personne. Seul le chat de ce Mac le lit, pour vous répondre plus personnellement.",
              "Lumen has no account and no community: your profile is sent to no one. Only the chat on this Mac reads it, to answer you more personally.",
            )}
          </span>
        </div>
      </div>
    </div>
  );
}
