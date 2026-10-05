import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { Orb, Segmented } from "../components/ui";
import { api, errorText, isTauri } from "../lib/api";
import { exportVocabulary } from "../lib/menu";
import { count, t } from "../lib/i18n";
import { inLang } from "../lib/langs";
import { useApp } from "../lib/store";
import { pronounce } from "../lib/pronounce";
import type { Term, TermQuery } from "../lib/types";

const PAGE = 60;

export function Vocabulary() {
  const lang = useApp((s) => s.lang)();
  const settings = useApp((s) => s.settings);
  const toast = useApp((s) => s.toast);
  const refreshKnown = useApp((s) => s.refreshKnown);
  const [filter, setFilter] = useState<TermQuery["filter"]>("learning");
  const [search, setSearch] = useState("");
  const [items, setItems] = useState<Term[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);

  const load = async (offset = 0) => {
    setLoading(true);
    try {
      const res = await api().termsList({ lang, filter, search, limit: PAGE, offset });
      setItems((prev) => (offset ? [...prev, ...res.items] : res.items));
      setTotal(res.total);
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    const t = window.setTimeout(() => void load(0), search ? 180 : 0);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lang, filter, search]);

  const setStatus = async (tm: Term, status: number) => {
    setItems((prev) => prev.map((x) => (x.term === tm.term ? { ...x, status: status as Term["status"] } : x)));
    await api().termSet({ lang, term: tm.term, status });
    if (status === 4 || tm.status === 4) void refreshKnown();
  };

  const exportCsv = () => exportVocabulary(lang);

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>{t("Vocabulaire", "Vocabulary")}</h1>
              <p>{t(`Les mots et expressions que vous avez rencontrés ${inLang(lang)}.`, `The words and phrases you have met ${inLang(lang)}.`)}</p>
            </div>
            {isTauri && (
              <button className="btn soft" onClick={exportCsv}>
                <Icon name="export" size={16} /> {t("Exporter en CSV", "Export as CSV")}
              </button>
            )}
          </header>

          <div className="vocab-toolbar">
            <Segmented
              id="vocab-filter"
              label={t("Filtrer", "Filter")}
              value={filter}
              onChange={(v) => setFilter(v as TermQuery["filter"])}
              options={[
                { value: "learning", label: t("En apprentissage", "Learning") },
                { value: "known", label: t("Connus", "Known") },
                { value: "phrases", label: t("Expressions", "Phrases") },
                { value: "ignored", label: t("Ignorés", "Ignored") },
                { value: "all", label: t("Tous", "All") },
              ]}
            />
            <label className="search" style={{ marginLeft: "auto" }}>
              <Icon name="search" size={16} />
              <input data-find placeholder={t("Chercher un mot ou un sens", "Search a word or a meaning")} value={search} onChange={(e) => setSearch(e.target.value)} aria-label={t("Chercher", "Search")} />
            </label>
            <span className="muted num">{count(total, "terme", "termes", "term", "terms")}</span>
          </div>

          {!loading && items.length === 0 ? (
            <div className="empty">
              <Orb size={40} />
              <h3>{t("Rien ici pour l'instant", "Nothing here yet")}</h3>
              <p>
                {t(
                  "Touchez les mots pendant la lecture : ils apparaîtront ici, avec leur traduction et la phrase où vous les avez croisés.",
                  "Tap words while you read: they will appear here, with their translation and the sentence where you met them.",
                )}
              </p>
            </div>
          ) : (
            <div className="vocab-list">
              <div className="vocab-row head">
                <span>{t("Terme", "Term")}</span>
                <span>{t("Sens", "Meaning")}</span>
                <span>{t("Rencontré dans", "Met in")}</span>
                <span>{t("Statut", "Status")}</span>
              </div>
              {items.map((tm, i) => (
                <motion.div
                  key={tm.term}
                  className="vocab-row"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  transition={{ delay: Math.min(i, 20) * 0.012 }}
                >
                  <span className="vocab-term" style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    {tm.term}
                    <button
                      className="icon-btn"
                      style={{ width: 26, height: 26 }}
                      onClick={() => void pronounce(tm.term, lang, settings[`voice_${lang}`])}
                      aria-label={t(`Prononcer ${tm.term}`, `Pronounce ${tm.term}`)}
                    >
                      <Icon name="speaker" size={14} />
                    </button>
                  </span>
                  <span className="vocab-tr">{tm.translation || <span className="muted">·</span>}</span>
                  <span className="vocab-ctx" title={tm.context}>
                    {tm.context || "·"}
                  </span>
                  <span className="status-dots" role="radiogroup" aria-label={t(`Statut de ${tm.term}`, `Status of ${tm.term}`)}>
                    {[1, 2, 3, 4].map((s) => (
                      <button key={s} role="radio" aria-checked={tm.status === s} className={`s${s} ${tm.status === s ? "on" : ""}`} onClick={() => setStatus(tm, s)}>
                        {s === 4 ? <Icon name="check" size={13} stroke={2.4} /> : s}
                      </button>
                    ))}
                  </span>
                </motion.div>
              ))}
            </div>
          )}
          {items.length < total && (
            <div style={{ display: "flex", justifyContent: "center", padding: 20 }}>
              <button className="btn soft" onClick={() => load(items.length)} disabled={loading}>
                {t("Afficher plus", "Show more")}
              </button>
            </div>
          )}
        </div>
      </div>
    </>
  );
}
