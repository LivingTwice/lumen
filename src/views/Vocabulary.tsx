import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { Orb, Segmented } from "../components/ui";
import { api, errorText, isTauri } from "../lib/api";
import { pickSavePath } from "../lib/dialogs";
import { langInfo } from "../lib/langs";
import { formatNumber, useApp } from "../lib/store";
import { sayWord } from "../lib/tts";
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

  const setStatus = async (t: Term, status: number) => {
    setItems((prev) => prev.map((x) => (x.term === t.term ? { ...x, status: status as Term["status"] } : x)));
    await api().termSet({ lang, term: t.term, status });
    if (status === 4 || t.status === 4) void refreshKnown();
  };

  const exportCsv = async () => {
    const path = await pickSavePath(`lumen-${lang}-vocabulaire.csv`);
    if (!path) return;
    try {
      await api().exportVocab(lang, path);
      toast("Vocabulaire exporté (compatible Anki)", "light");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>Vocabulaire</h1>
              <p>Les mots et expressions que vous avez rencontrés en {langInfo(lang).name.toLowerCase()}.</p>
            </div>
            {isTauri && (
              <button className="btn soft" onClick={exportCsv}>
                <Icon name="export" size={16} /> Exporter en CSV
              </button>
            )}
          </header>

          <div className="vocab-toolbar">
            <Segmented
              id="vocab-filter"
              label="Filtrer"
              value={filter}
              onChange={(v) => setFilter(v as TermQuery["filter"])}
              options={[
                { value: "learning", label: "En apprentissage" },
                { value: "known", label: "Connus" },
                { value: "phrases", label: "Expressions" },
                { value: "ignored", label: "Ignorés" },
                { value: "all", label: "Tous" },
              ]}
            />
            <label className="search" style={{ marginLeft: "auto" }}>
              <Icon name="search" size={16} />
              <input placeholder="Chercher un mot ou un sens" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Chercher" />
            </label>
            <span className="muted num">{formatNumber(total)} terme{total > 1 ? "s" : ""}</span>
          </div>

          {!loading && items.length === 0 ? (
            <div className="empty">
              <Orb size={40} />
              <h3>Rien ici pour l'instant</h3>
              <p>Touchez les mots pendant la lecture : ils apparaîtront ici, avec leur traduction et la phrase où vous les avez croisés.</p>
            </div>
          ) : (
            <div className="vocab-list">
              <div className="vocab-row head">
                <span>Terme</span>
                <span>Sens</span>
                <span>Rencontré dans</span>
                <span>Statut</span>
              </div>
              {items.map((t, i) => (
                <motion.div
                  key={t.term}
                  className="vocab-row"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  transition={{ delay: Math.min(i, 20) * 0.012 }}
                >
                  <span className="vocab-term" style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    {t.term}
                    <button className="icon-btn" style={{ width: 26, height: 26 }} onClick={() => sayWord(t.term, lang, settings[`voice_${lang}`])} aria-label={`Prononcer ${t.term}`}>
                      <Icon name="speaker" size={14} />
                    </button>
                  </span>
                  <span className="vocab-tr">{t.translation || <span className="muted">—</span>}</span>
                  <span className="vocab-ctx" title={t.context}>
                    {t.context || "—"}
                  </span>
                  <span className="status-dots" role="radiogroup" aria-label={`Statut de ${t.term}`}>
                    {[1, 2, 3, 4].map((s) => (
                      <button key={s} role="radio" aria-checked={t.status === s} className={`s${s} ${t.status === s ? "on" : ""}`} onClick={() => setStatus(t, s)}>
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
                Afficher plus
              </button>
            </div>
          )}
        </div>
      </div>
    </>
  );
}
