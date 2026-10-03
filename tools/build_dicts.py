#!/usr/bin/env python3
"""Construit les dictionnaires compacts de Lumen à partir des extractions
du Wiktionnaire publiées par kaikki.org (wiktextract).

Définitions en français (interface en français), Wiktionnaire français :
  python3 tools/build_dicts.py <dossier_jsonl> src-tauri/resources/dicts
  (fichiers <Langue>.jsonl : Anglais, Espagnol, Italien…)
Définitions en anglais (interface en anglais), Wiktionnaire anglais :
  python3 tools/build_dicts.py --en <dossier_jsonl> src-tauri/resources/dicts-en
  (fichiers kaikki.org/dictionary/<Language>/kaikki.org-dictionary-<Language>.jsonl
  renommés <Language>.jsonl : Italian, Spanish, German, Portuguese, Russian, French)
Chaque fichier produit <code>.db.gz (SQLite compressé), même schéma dans les deux cas.
Licence des données : CC BY-SA (Wiktionnaire), attribution affichée dans l'app.
"""
import gzip, json, os, re, shutil, sqlite3, sys, unicodedata

LANGS = {"Anglais": "en", "Espagnol": "es", "Italien": "it", "Allemand": "de",
         "Portugais": "pt", "Russe": "ru"}
# Wiktionnaire anglais : pas d'anglais (un anglophone ne l'étudie pas), le français en plus
LANGS_EN = {"Italian": "it", "Spanish": "es", "German": "de", "Portuguese": "pt",
            "Russian": "ru", "French": "fr"}
SKIP_POS = ("Nom propre", "Prénom", "Nom de famille", "Symbole", "Lettre",
            "Nom scientifique", "Sinogramme", "Erreur", "Variante typographique")
MAX_GLOSS = 3

def norm(s: str) -> str:
    s = unicodedata.normalize("NFD", s.lower())
    s = s.replace("́", "").replace("̀", "")  # accents toniques (russe)
    s = unicodedata.normalize("NFC", s)
    return s.replace("’", "'").strip()

def clean_pos(p: str) -> str:
    return re.sub(r"\s+\d+$", "", p or "").strip()

def clean_gloss(g: str) -> str:
    g = re.sub(r"\s+", " ", g).strip()
    if len(g) > 140:
        g = g[:137].rsplit(" ", 1)[0] + "…"
    return g

def build(src: str, dst: str):
    tmp = dst + ".tmp"
    if os.path.exists(tmp):
        os.remove(tmp)
    db = sqlite3.connect(tmp)
    db.executescript("""
      PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
      CREATE TABLE entries(k TEXT, word TEXT, pos TEXT, ipa TEXT, gloss TEXT, rank INTEGER);
      CREATE TABLE forms(k TEXT, lemma TEXT, note TEXT);
    """)
    seen_forms = set()
    n_e = n_f = 0
    with open(src, encoding="utf-8") as f:
        for line in f:
            d = json.loads(line)
            word = d.get("word") or ""
            pos = clean_pos(d.get("pos_title", ""))
            if not word or any(pos.startswith(s) for s in SKIP_POS):
                continue
            if len(word) > 60 or word.count(" ") > 4:
                continue
            k = norm(word)
            senses = d.get("senses") or []
            if pos.startswith("Forme"):
                for s in senses:
                    fo = s.get("form_of") or []
                    if fo and fo[0].get("word"):
                        lemma = fo[0]["word"]
                        key = (k, lemma)
                        if key in seen_forms:
                            continue
                        seen_forms.add(key)
                        note = clean_gloss((s.get("glosses") or [""])[0])
                        db.execute("INSERT INTO forms VALUES(?,?,?)", (k, lemma, note))
                        n_f += 1
                continue
            glosses = []
            for s in senses:
                for g in s.get("glosses") or []:
                    g = clean_gloss(g)
                    if g and g not in glosses and not g.startswith("Définition manquante"):
                        glosses.append(g)
                    break
                if len(glosses) >= MAX_GLOSS:
                    break
            if not glosses:
                continue
            ipa = ""
            for snd in d.get("sounds") or []:
                if snd.get("ipa"):
                    ipa = snd["ipa"].strip("\\/[] ")
                    break
            db.execute("INSERT INTO entries VALUES(?,?,?,?,?,?)",
                       (k, word, pos, ipa, "␞".join(glosses), n_e))
            n_e += 1
            for fm in d.get("forms") or []:
                form = fm.get("form")
                if not form or form == word or " " in form or len(form) > 40:
                    continue
                fk = norm(form)
                key = (fk, word)
                if key in seen_forms:
                    continue
                seen_forms.add(key)
                db.execute("INSERT INTO forms VALUES(?,?,?)", (fk, word, ""))
                n_f += 1
    finish(db, tmp, dst, "Wiktionnaire via kaikki.org (CC BY-SA 4.0)", n_e, n_f)

def finish(db, tmp, dst, source, n_e, n_f, forms_index=True):
    db.executescript("""
      CREATE INDEX ie ON entries(k);
      CREATE INDEX iw ON entries(word);
      CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
    """)
    if forms_index:
        db.execute("CREATE INDEX iff ON forms(k)")
    db.execute("INSERT INTO meta VALUES('source',?)", (source,))
    db.commit()
    db.execute("VACUUM")
    db.close()
    with open(tmp, "rb") as fi, gzip.open(dst, "wb", compresslevel=9) as fo:
        shutil.copyfileobj(fi, fo)
    raw = os.path.getsize(tmp)
    os.remove(tmp)
    print(f"{os.path.basename(dst)}: {n_e} entrées, {n_f} formes, "
          f"{raw/1e6:.1f} Mo -> {os.path.getsize(dst)/1e6:.1f} Mo gz")

# ---------- Wiktionnaire anglais ----------

POS_EN = {"noun": "Noun", "verb": "Verb", "adj": "Adjective", "adv": "Adverb",
          "pron": "Pronoun", "intj": "Interjection", "num": "Numeral",
          "det": "Determiner", "conj": "Conjunction", "prep": "Preposition",
          "postp": "Postposition", "article": "Article", "particle": "Particle",
          "phrase": "Phrase", "prep_phrase": "Prepositional phrase",
          "proverb": "Proverb", "contraction": "Contraction", "abbrev": "Abbreviation"}
SKIP_POS_EN = {"name", "character", "symbol", "suffix", "prefix", "infix", "interfix",
               "circumfix", "affix", "punct", "romanization", "letter", "syllable"}
# formes des tableaux de conjugaison et de déclinaison : seules les étiquettes
# flexionnelles comptent (« прочитать », perfectif de « читать », est un autre verbe)
FORM_SOURCES = {"conjugation", "declension", "inflection"}
NOT_FORMS = {"table-tags", "inflection-template", "class", "romanization", "canonical"}
INFLECTION = {"plural", "singular", "feminine", "masculine", "neuter", "first-person",
              "second-person", "third-person", "present", "past", "future", "participle",
              "gerund", "imperfect", "perfect", "preterite", "indicative", "subjunctive",
              "imperative", "conditional", "infinitive", "comparative", "superlative",
              "nominative", "genitive", "dative", "accusative", "instrumental",
              "prepositional", "locative", "vocative", "historic", "pluperfect",
              "definite", "indefinite", "strong", "weak", "mixed", "positive"}
RARE = {"obsolete", "archaic", "rare", "dated"}

def plain(word: str) -> str:
    """Mot sans accent tonique (russe : « чита́ть » -> « читать »)."""
    s = unicodedata.normalize("NFD", word).replace("\u0301", "").replace("\u0300", "")
    return unicodedata.normalize("NFC", s)

def form_note(gl, target, lang):
    """Description d'une forme : « first-person singular imperfect indicative of andare »."""
    note = gl[0] if gl else ""
    # « inflection of зелёный (zeljónyj): » suivi du détail dans la sous-définition
    if note.rstrip().endswith(":") and len(gl) > 1:
        note = f"{gl[-1]} of {target}"
    if lang == "ru":
        note = re.sub(r"\s*\([^()]*\)", "", note)  # translittération latine
    return clean_gloss(note.rstrip(":. "))

def build_en(src: str, dst: str, lang: str):
    tmp = dst + ".tmp"
    if os.path.exists(tmp):
        os.remove(tmp)
    db = sqlite3.connect(tmp)
    # formes rangées par clé (WITHOUT ROWID) : pas d'index à part, moitié moins de place
    db.executescript("""
      PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
      CREATE TABLE entries(k TEXT, word TEXT, pos TEXT, ipa TEXT, gloss TEXT, rank INTEGER);
      CREATE TABLE forms(k TEXT, lemma TEXT, note TEXT, PRIMARY KEY(k, lemma)) WITHOUT ROWID;
    """)
    # (clé, forme de base) -> description ; une description l'emporte sur une forme sans détail
    forms = {}
    def add_form(k, lemma, note):
        lemma = plain(lemma) if lang == "ru" else lemma
        if not lemma or norm(lemma) == k:
            return
        if not forms.get((k, lemma)):
            forms[(k, lemma)] = note
    n_e = 0
    with open(src, encoding="utf-8") as f:
        for line in f:
            d = json.loads(line)
            word = d.get("word") or ""
            pos = d.get("pos") or ""
            if not word or pos in SKIP_POS_EN or len(word) > 60 or word.count(" ") > 4:
                continue
            k = norm(word)
            common, rare = [], []
            for s in d.get("senses") or []:
                gl = s.get("glosses") or []
                target = (s.get("form_of") or s.get("alt_of") or [{}])[0].get("word")
                if target:
                    add_form(k, target, form_note(gl, target, lang))
                    continue
                if not gl:
                    continue
                g = clean_gloss(gl[0])
                if g and g not in common and g not in rare:
                    (rare if RARE & set(s.get("tags") or []) else common).append(g)
            glosses = (common + rare)[:MAX_GLOSS]
            for fm in d.get("forms") or []:
                form = fm.get("form")
                tags = set(fm.get("tags") or [])
                if not form or form == word or " " in form or len(form) > 40 or tags & NOT_FORMS:
                    continue
                if fm.get("source") in FORM_SOURCES or (tags and tags <= INFLECTION):
                    add_form(norm(form), word, "")
            if not glosses:
                continue
            ipa = ""
            for snd in d.get("sounds") or []:
                if snd.get("ipa"):
                    ipa = snd["ipa"].strip("\\/[] ")
                    break
            db.execute("INSERT INTO entries VALUES(?,?,?,?,?,?)",
                       (k, word, POS_EN.get(pos, pos.capitalize()), ipa, "␞".join(glosses), n_e))
            n_e += 1
    db.executemany("INSERT INTO forms VALUES(?,?,?)", ((k, l, n) for (k, l), n in forms.items()))
    finish(db, tmp, dst, "English Wiktionary via kaikki.org (CC BY-SA 4.0)", n_e, len(forms), forms_index=False)

if __name__ == "__main__":
    args = sys.argv[1:]
    english = args[:1] == ["--en"]
    if english:
        args = args[1:]
    src_dir, out_dir = args[0], args[1]
    langs = LANGS_EN if english else LANGS
    os.makedirs(out_dir, exist_ok=True)
    for name in args[2:] or list(langs):
        p = os.path.join(src_dir, name + ".jsonl")
        if os.path.exists(p):
            code = langs[name]
            if english:
                build_en(p, os.path.join(out_dir, code + ".db.gz"), code)
            else:
                build(p, os.path.join(out_dir, code + ".db.gz"))
