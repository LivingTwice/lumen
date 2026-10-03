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

Version 2 : les 24 autres langues de Lumen, définitions en français et en anglais
(version `dictionaries-2` de lumen-releases, fichiers fr-<code>.db.gz et en-<code>.db.gz).
Sources, dans un dossier hors iCloud (≈ 4 Go, puis ≈ 13 Go une fois découpées) :
  curl -LO https://kaikki.org/dictionary/raw-wiktextract-data.jsonl.gz      -> en-raw.jsonl.gz
  curl -LO https://kaikki.org/frwiktionary/raw-wiktextract-data.jsonl.gz    -> fr-raw.jsonl.gz
  curl -LO http://ftp.edrdg.org/pub/Nihongo/JMdict.gz
  curl -LO http://ftp.edrdg.org/pub/Nihongo/kanjidic2.xml.gz
  freq/<code>_full.txt (fi hu tr) : github.com/hermitdave/FrequencyWords, content/2018
  python3 tools/build_dicts.py --split <sources>         (une fois : en/, fr/, en-tr, fr-tr)
  python3 tools/build_dicts.py --ud <sources>            (corpus Universal Dependencies : ud/)
  python3 tools/build_dicts.py --v2 <sources> <sortie> [fr-hu en-ja …]
orjson (pip install orjson) accélère beaucoup la lecture, sans être indispensable.
"""
import gzip, json, os, re, shutil, sqlite3, sys, unicodedata
from collections import defaultdict
try:
    import orjson
    loads, dumps = orjson.loads, orjson.dumps
except ImportError:
    loads, dumps = json.loads, lambda d: json.dumps(d, ensure_ascii=False).encode()

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

def finish(db, tmp, dst, source, n_e, n_f, forms_index=True, meta_table=True):
    db.executescript("""
      CREATE INDEX ie ON entries(k);
      CREATE INDEX iw ON entries(word);
    """)
    if meta_table:
        db.execute("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)")
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


# ====================== Version 2 : les 24 autres langues ======================

V2_LANGS = "nl sv da fi et lv lt pl cs sk sl hr hu ro bg uk el tr ar hi id vi ko ja".split()
# Wiktionnaire anglais : le croate est rangé sous le serbo-croate (alphabet latin seulement)
EN_CODES = {"hr": ["sh"]}
FR_CODES = {"hr": ["hr", "sh"]}
# tableaux de flexion immenses (finnois : 37 millions de formes, suffixes possessifs
# compris) : seules les formes rencontrées dans un grand corpus de sous-titres restent
CORPUS_FORMS = {"fi", "hu", "tr"}
NON_LATIN = {"uk", "bg", "el", "ar", "hi", "ko", "ja"}
STRIPS_ACCENTS = {"", "en", "it", "de", "pt", "ru", "es", "uk", "bg"}  # text::strips_accents
AR_MARKS = re.compile("[\u064b-\u065f\u0670\u0640]")
AR_ALEF = str.maketrans("أإآٱ", "اااا")
CYRILLIC = re.compile("[Ѐ-ӿ]")
SKIP_POS_V2 = SKIP_POS_EN | {"soft-redirect", "hanja", "kanji", "han_tu", "chu_nom", "root", "unknown", "other"}
POS_EN.update({"classifier": "Classifier", "counter": "Counter", "num": "Numeral", "adj_noun": "Adjectival noun",
               "onomatopoeia": "Onomatopoeia", "verb_phrase": "Verb phrase", "noun_phrase": "Noun phrase"})
POS_FR = {"noun": "Nom", "verb": "Verbe", "adj": "Adjectif", "adv": "Adverbe", "pron": "Pronom",
          "intj": "Interjection", "num": "Numéral", "det": "Déterminant", "conj": "Conjonction",
          "prep": "Préposition", "postp": "Postposition", "article": "Article", "particle": "Particule",
          "phrase": "Locution", "prep_phrase": "Locution prépositive", "proverb": "Proverbe",
          "contraction": "Contraction", "abbrev": "Abréviation", "classifier": "Classificateur",
          "counter": "Classificateur", "onomatopoeia": "Onomatopée", "adj_noun": "Nom adjectival",
          "verb_phrase": "Locution verbale", "noun_phrase": "Locution nominale"}
SKIP_TITLES_FR = SKIP_POS + ("Suffixe", "Préfixe", "Infixe", "Interfixe", "Circonfixe", "Affixe",
                             "Lettre", "Ponctuation", "Onomatopée", "Particule numérale")

# accents des manuels, absents des textes : accent tonique lituanien (« niẽko »), accents
# de ton croates (« kȕća », mais « ć » reste) et slovènes
TONES = {"lt": "\u0300\u0301\u0303", "hr": "\u0300\u0302\u0304\u030f\u0311",
         "sl": "\u0300\u0301\u0302\u0304\u030f\u0311\u0323\u0327\u0328"}

def strip_tones(s: str, lang: str) -> str:
    out, base = [], ""
    for c in s:
        if unicodedata.combining(c):
            if c in TONES[lang] or (c == "\u0301" and lang == "hr" and base != "c") or (c == "\u0307" and lang == "lt" and base == "i"):
                continue
        else:
            base = c
        out.append(c)
    return "".join(out)

def plain_word(w: str, lang: str) -> str:
    """Mot à afficher sans les accents des manuels (« kàr » -> « kar »), casse gardée."""
    s = unicodedata.normalize("NFD", w)
    if lang in STRIPS_ACCENTS:
        s = s.replace("\u0301", "").replace("\u0300", "")
    if lang in TONES:
        s = strip_tones(s, lang)
    return unicodedata.normalize("NFC", s)

def key2(s: str, lang: str) -> str:
    """Clé de recherche d'un mot (miroir de dict::key dans dict.rs)."""
    s = unicodedata.normalize("NFD", s.strip().lower().replace("’", "'"))
    if lang in STRIPS_ACCENTS:
        s = s.replace("\u0301", "").replace("\u0300", "")
    if lang == "tr":
        s = s.replace("i\u0307", "i")  # « İ » mis en minuscule
    if lang in TONES:
        s = strip_tones(s, lang)
    s = unicodedata.normalize("NFC", s)
    if lang == "ar":
        s = AR_MARKS.sub("", s).translate(AR_ALEF)  # voyelles brèves, alifs hamzés
    return " ".join(s.split())

def loose(s: str) -> str:
    """Sans aucun signe diacritique : rapproche « kȕća » (accent tonal) de « kuća »."""
    return "".join(c for c in unicodedata.normalize("NFD", s.lower()) if not unicodedata.combining(c))

def unstress(s: str) -> str:
    """Sans accent tonique (« чита́ть »), comme les titres des pages du Wiktionnaire."""
    return unicodedata.normalize("NFC", unicodedata.normalize("NFD", s).replace("\u0301", "").replace("\u0300", ""))

def ipa_of(d) -> str:
    for snd in d.get("sounds") or []:
        if snd.get("ipa"):
            return snd["ipa"].strip("\\/[] ")
    return ""

# ---------- étiquettes grammaticales : (place, anglais, français) ----------

GRAM = {}
for place, pairs in enumerate([
    [("infinitive", "infinitif"), ("participle", "participe"), ("gerund", "gérondif"), ("supine", "supin"),
     ("converb", "converbe"), ("verbal-noun", "nom verbal"), ("noun-from-verb", "nom verbal"),
     ("adverbial", "adverbial"), ("sequential", "séquentiel"), ("connective", "connectif")],
    [("first-person", "1re personne"), ("second-person", "2e personne"), ("third-person", "3e personne"),
     ("impersonal", "impersonnel")],
    [("singular", "singulier"), ("plural", "pluriel"), ("dual", "duel")],
    [("nominative", "nominatif"), ("genitive", "génitif"), ("dative", "datif"), ("accusative", "accusatif"),
     ("instrumental", "instrumental"), ("locative", "locatif"), ("vocative", "vocatif"),
     ("prepositional", "prépositionnel"), ("ablative", "ablatif"), ("partitive", "partitif"),
     ("essive", "essif"), ("translative", "translatif"), ("inessive", "inessif"), ("elative", "élatif"),
     ("illative", "illatif"), ("adessive", "adessif"), ("allative", "allatif"), ("comitative", "comitatif"),
     ("abessive", "abessif"), ("instructive", "instructif"), ("terminative", "terminatif"),
     ("sublative", "sublatif"), ("delative", "délatif"), ("superessive", "superessif"),
     ("causal-final", "causal-final"), ("temporal", "temporel"), ("distributive", "distributif"),
     ("sociative", "sociatif"), ("oblique", "oblique"), ("construct", "état construit"),
     ("equative", "équatif"), ("direct", "direct")],
    [("masculine", "masculin"), ("feminine", "féminin"), ("neuter", "neutre"), ("common", "commun"),
     ("animate", "animé"), ("inanimate", "inanimé"), ("personal", "personnel"), ("virile", "viril"),
     ("nonvirile", "non viril")],
    [("present", "présent"), ("past", "passé"), ("future", "futur"), ("imperfect", "imparfait"),
     ("perfect", "parfait"), ("pluperfect", "plus-que-parfait"), ("aorist", "aoriste"),
     ("preterite", "prétérit"), ("non-past", "non-passé"), ("future-perfect", "futur antérieur")],
    [("perfective", "perfectif"), ("imperfective", "imperfectif"), ("progressive", "progressif")],
    [("indicative", "indicatif"), ("subjunctive", "subjonctif"), ("conditional", "conditionnel"),
     ("imperative", "impératif"), ("optative", "optatif"), ("potential", "potentiel"), ("jussive", "jussif"),
     ("hortative", "exhortatif"), ("interrogative", "interrogatif"), ("renarrative", "médiatif")],
    [("active", "actif"), ("passive", "passif"), ("reflexive", "réfléchi"), ("causative", "causatif"),
     ("mediopassive", "médiopassif")],
    [("definite", "défini"), ("indefinite", "indéfini")],
    [("comparative", "comparatif"), ("superlative", "superlatif"), ("diminutive", "diminutif"),
     ("augmentative", "augmentatif")],
    [("negative", "négatif"), ("formal", "formel"), ("informal", "familier"), ("polite", "poli"),
     ("short-form", "forme courte"), ("long-form", "forme longue"), ("possessive", "possessif"),
     ("contraction", "contraction")],
]):
    for en, fr in pairs:
        GRAM[en] = (place, en, fr)
FORM_TAGS_V2 = INFLECTION | set(GRAM) | {"clitic", "stem", "attributive", "singular-possessive", "plural-possessive", "possessed-single",
                                          "possessed-many", "rare", "archaic", "dated", "colloquial"}

def describe(tags, native: str) -> str:
    """« genitive plural », « 1re personne du singulier, présent de l'indicatif »."""
    by = defaultdict(list)
    for t in sorted(set(tags), key=lambda t: (GRAM[t][0], t) if t in GRAM else (99, t)):
        if t in GRAM:
            by[GRAM[t][0]].append(GRAM[t][1 if native == "en" else 2])
    j = {p: "/".join(v) for p, v in by.items()}
    if not j:
        return ""
    if native == "en":
        return " ".join(j[p] for p in (1, 2, 3, 4, 5, 6, 7, 8, 0, 9, 10, 11) if p in j)
    parts = []
    if 0 in j:
        parts.append(j[0])
    if 1 in j:
        parts.append(f"{j[1]} du {j[2]}" if 2 in j else j[1])
    nominal = " ".join(j[p] for p in (3, 4) if p in j)
    if 2 in j and 1 not in j:
        nominal = f"{nominal} {j[2]}".strip()
    if nominal:
        parts.append(nominal)
    tense = " ".join(j[p] for p in (5, 6) if p in j)
    mood = j.get(7, "")
    if tense and mood:
        parts.append(f"{tense} {'de l’' if mood[0] in 'aeiouéè' else 'du '}{mood}")
    elif tense or mood:
        parts.append(tense or mood)
    parts += [j[p] for p in (8, 9, 10, 11) if p in j]
    s = ", ".join(parts)
    return s[:1].upper() + s[1:]

OF_TAIL = re.compile(r"\s+(of|de|d’|d'|du verbe|de l’adjectif|du nom)\s*\S.*$", re.S)

def gloss_desc(gl, tags, native: str) -> str:
    """Description d'une forme à partir de sa définition (« genitive plural of ház »,
    « Vocatif singulier de abaka. »), sans le mot de base, que l'app affiche déjà."""
    note = gl[0] if gl else ""
    if note.rstrip().endswith(":"):  # « inflection of зелёный: » puis le détail
        note = gl[-1] if len(gl) > 1 else ""
    note = OF_TAIL.sub("", note.strip()).rstrip(":.; ")
    note = re.sub(r"\s*\([^()]*\)", "", note)  # translittérations, gloses entre parenthèses
    if not note or len(note) > 70 or note.lower() in ("form", "forme", "flexion") or note.lower().startswith("inflection"):
        note = describe(tags, native) or note
    return clean_gloss(note)

# ---------- sources ----------

def jsonl(path):
    with open(path, "rb") as f:
        for line in f:
            yield loads(line)

class Sources:
    """Dossier des sources découpées par --split (en/, fr/, en-tr, fr-tr, JMdict, freq/)."""
    def __init__(self, root):
        self.root = root

    def path(self, *p):
        return os.path.join(self.root, *p)

    def entries(self, edition, lang):
        codes = (EN_CODES if edition == "en" else FR_CODES).get(lang, [lang])
        for code in codes:
            p = self.path(edition, code + ".jsonl")
            if not os.path.exists(p):
                continue
            for d in jsonl(p):
                if lang == "hr" and CYRILLIC.search(d.get("word") or ""):
                    continue
                yield d

    def attested(self, lang):
        """Mots rencontrés dans le corpus de sous-titres (formes fléchies à garder), ou None."""
        p = self.path("freq", f"{lang}_full.txt")
        if lang not in CORPUS_FORMS or not os.path.exists(p):
            return None
        out = set()
        with open(p, encoding="utf-8") as f:
            for line in f:
                out.add(key2(line.rsplit(" ", 1)[0], lang))
        return out

def split(root):
    """Découpe les extractions complètes de kaikki.org : une ligne allégée par entrée et par
    langue (en/<code>.jsonl, fr/<code>.jsonl), et les tables de traduction des mots anglais
    (en-tr.jsonl) et français (fr-tr.jsonl) vers ces langues."""
    import subprocess
    targets = set(V2_LANGS) | {"sh"}
    keep_tr = targets | {"fr", "en"}
    for ed in ("en", "fr"):
        os.makedirs(os.path.join(root, ed), exist_ok=True)
        files = {}
        def out(path, d):
            f = files.get(path)
            if f is None:
                f = files[path] = open(os.path.join(root, path + ".jsonl"), "wb")
            f.write(dumps(d) + b"\n")
        proc = subprocess.Popen(["gzip", "-dc", os.path.join(root, f"{ed}-raw.jsonl.gz")], stdout=subprocess.PIPE)
        for n, line in enumerate(proc.stdout):
            try:
                d = loads(line)
            except ValueError:
                continue
            lc = d.get("lang_code")
            if lc in targets:
                r = {k: d[k] for k in ("word", "lang_code", "pos", "pos_title") if k in d}
                r["senses"] = [{k: s[k] for k in ("glosses", "tags", "form_of", "alt_of") if s.get(k)}
                               for s in d.get("senses") or []]
                if d.get("forms"):
                    r["forms"] = [{k: f[k] for k in ("form", "tags", "source") if k in f} for f in d["forms"]]
                ipas = [{"ipa": s["ipa"]} for s in d.get("sounds") or [] if s.get("ipa")][:2]
                if ipas:
                    r["sounds"] = ipas
                out(f"{ed}/{lc}", r)
            elif lc == ed and d.get("translations"):
                tr = [{k: t[k] for k in ("lang_code", "word", "sense", "sense_index") if k in t}
                      for t in d["translations"] if t.get("lang_code") in keep_tr and t.get("word")]
                if tr:
                    out(f"{ed}-tr", {"word": d.get("word"), "pos": d.get("pos"), "translations": tr})
            if n % 2_000_000 == 0:
                print(ed, n, flush=True)
        for f in files.values():
            f.close()

# ---------- traductions : équivalents français de chaque mot ----------

def clean_tr(w: str) -> str:
    w = (w or "").strip()
    if not w or len(w) > 40 or any(c in w for c in "()[]{}0123456789=/") or w.count(" ") > 3:
        return ""
    if w.startswith("-") or w.endswith("-"):  # affixes (« -iste »)
        return ""
    return w

def fr_def(g: str) -> str:
    """Définition du Wiktionnaire français, sans ses notes ; vide pour un simple renvoi."""
    g = re.split(r"\s+Note d[’']usage", clean_gloss(g))[0].rstrip(". ")
    if g.startswith(("Variante", "Autre orthographe", "Ancienne orthographe", "Orthographe alternative")):
        return ""
    return g

def load_pairs(src, langs):
    """{langue: {mot: [(équivalent français, nature, poids, groupe)]}} d'après les tables de
    traduction : « maison » → hu « ház » (Wiktionnaire français, inversé), et « house »
    (sens « abode ») → hu « ház » + fr « maison » (Wiktionnaire anglais, même sens)."""
    codes = {c: L for L in langs for c in FR_CODES.get(L, [L])}
    out = {L: defaultdict(list) for L in langs}
    for d in jsonl(src.path("fr-tr.jsonl")):
        F = clean_tr(d.get("word"))
        if not F:
            continue
        for t in d["translations"]:
            L = codes.get(t.get("lang_code"))
            w = clean_tr(t.get("word"))
            if L and w and not (L == "hr" and CYRILLIC.search(w)):
                # le premier sens du mot français compte plus que ses sens secondaires
                out[L][w].append((F, d.get("pos"), 2 if t.get("sense_index") in (None, 1) else 1.5, None, None))
    gid = 0
    for d in jsonl(src.path("en-tr.jsonl")):
        groups = defaultdict(lambda: defaultdict(list))
        for t in d["translations"]:
            w = clean_tr(t.get("word"))
            if w:
                groups[t.get("sense") or ""][t.get("lang_code")].append(w)
        for by in groups.values():
            frs = list(dict.fromkeys(by.get("fr", [])))[:4]
            if not frs:
                continue
            gid += 1
            for code, ws in by.items():
                L = codes.get(code)
                if not L:
                    continue
                for w in dict.fromkeys(ws):
                    if L == "hr" and CYRILLIC.search(w):
                        continue
                    for i, f in enumerate(frs):
                        out[L][w].append((f, d.get("pos"), 1 if i == 0 else 0.6, gid, d.get("word")))
    return out

# natures grossières, pour rattacher une traduction quand les deux langues ne classent pas
# le mot de la même façon (adverbe ici, particule là)
COARSE = {"noun": "n", "name": "n", "verb": "v", "adj": "a", "num": "a", "det": "f"}

RANK = {"fr": {}, "en": {}}  # rang de fréquence des mots français et anglais (sous-titres)

def fr_bonus(f: str, native="fr") -> float:
    """Les mots courants d'abord : « donner » plutôt que « bailler »."""
    r = min((RANK[native].get(w, 10**6) for w in re.findall(r"\w+", f.lower()) if w not in ("to", "se", "s")), default=10**6)
    return 2 if r < 5000 else 1 if r < 20000 else 0.5 if r < 50000 else 0

def load_en_pairs(src, langs):
    """{langue: {mot: [(mot anglais, nature, poids, groupe, None)]}} : tables de traduction
    du Wiktionnaire anglais lues à l'envers (« therefore » → sl « zato »)."""
    codes = {c: L for L in langs for c in EN_CODES.get(L, [L])}
    out = {L: defaultdict(list) for L in langs}
    for d in jsonl(src.path("en-tr.jsonl")):
        E = clean_tr(d.get("word"))
        if not E:
            continue
        shown = f"to {E}" if d.get("pos") == "verb" else E
        for t in d["translations"]:
            L = codes.get(t.get("lang_code"))
            w = clean_tr(t.get("word"))
            if L and w and not (L == "hr" and CYRILLIC.search(w)):
                out[L][w].append((shown, d.get("pos"), 1.5, None, None))
    return out

def trans_lines(items, limit, native="fr"):
    """Équivalents rangés : tables qui les donnent, rang dans le sens anglais qui les relie au
    mot, fréquence en français. Ceux d'un même sens anglais vont sur la même ligne ; ceux qui
    pèsent trop peu face au premier sont écartés (« carter » pour « ház », maison)."""
    score, bonus, shown, groups, order = defaultdict(float), defaultdict(float), {}, defaultdict(set), {}
    for f, w, g, b in items:
        f = f.replace("’", "'")
        lf = loose(f)  # « connaître » et « connaitre » ne font qu'un
        score[lf] += w
        bonus[lf] = max(bonus[lf], b)
        shown.setdefault(lf, f)
        order.setdefault(lf, len(order))
        if g is not None:
            groups[lf].add(g)
    for lf in score:
        score[lf] += bonus[lf] + fr_bonus(lf, native)
    ranked = sorted(score, key=lambda x: (-score[x], order[x]))
    lines, used = [], set()
    for f in ranked:
        if f in used or score[f] < 0.3 * score[ranked[0]]:
            continue
        line = [f]
        used.add(f)
        for g in ranked:
            if len(line) < 3 and g not in used and groups[f] & groups[g]:
                line.append(g)
                used.add(g)
        lines.append(", ".join(shown[x] for x in line))
        if len(lines) >= limit:
            break
    return lines

def mentions(line: str, glosses) -> bool:
    """La traduction figure déjà dans une définition (« maison » dans « Maison. »)."""
    same = lambda x: loose(x).replace("’", "'").replace("…", "...")
    first = same(line.split(",")[0].strip())
    return any(re.search(r"(^|\W)" + re.escape(first) + r"($|\W)", same(g)) for g in glosses)

# ---------- écriture ----------

def write_v2(dst, lang, native, entries, forms, source, extra_meta=None):
    """entries : [(clé, mot, nature, API, [sens])] ; forms : {(clé, forme de base): description}.
    Les descriptions sont rangées une fois dans `notes` (des millions de formes, quelques
    centaines de descriptions)."""
    tmp = dst + ".tmp"
    if os.path.exists(tmp):
        os.remove(tmp)
    db = sqlite3.connect(tmp)
    db.executescript("""
      PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA page_size=4096;
      CREATE TABLE entries(k TEXT, word TEXT, pos TEXT, ipa TEXT, gloss TEXT, rank INTEGER);
      CREATE TABLE forms(k TEXT, lemma TEXT, note INTEGER, PRIMARY KEY(k, lemma)) WITHOUT ROWID;
      CREATE TABLE notes(id INTEGER PRIMARY KEY, text TEXT);
    """)
    db.executemany("INSERT INTO entries VALUES(?,?,?,?,?,?)",
                   ((k, w, p, i, "␞".join(g), n) for n, (k, w, p, i, g) in enumerate(entries)))
    notes, rows = {}, []
    for (k, lemma), desc in forms.items():
        rows.append((k, lemma, notes.setdefault(desc, len(notes) + 1) if desc else 0))
    rows.sort()
    db.executemany("INSERT INTO forms VALUES(?,?,?)", rows)
    db.executemany("INSERT INTO notes VALUES(?,?)", ((i, d) for d, i in notes.items()))
    db.execute("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)")
    meta = {"format": "2", "lang": lang, "native": native, **(extra_meta or {})}
    db.executemany("INSERT INTO meta VALUES(?,?)", meta.items())
    finish(db, tmp, dst, source, len(entries), len(rows), forms_index=False, meta_table=False)

def finish_forms(forms, entries, lang):
    """Grec : l'accent manque dans les textes en capitales ; la clé sans accent mène au mot."""
    if lang != "el":
        return
    keys = {e[0] for e in entries}
    for k, w in [(e[0], e[1]) for e in entries] + [(k, l) for (k, l) in list(forms)]:
        lk = loose(k)
        if lk != k and lk not in keys and (lk, w) not in forms:
            forms[(lk, w)] = forms.get((k, w), "")

# auxiliaires et pronoms réfléchis des formes composées des tableaux (« mȉslio sam »,
# « θα δω », « heb gezien ») : le mot qui reste est une forme du verbe
CLITICS = set("""
sam si je smo ste su bih bi bismo biste ću ćeš će ćemo ćete se sem sva sta so bom boš bo bova bosta
bomo boste bodo jsem jsi jsme jste jsou bych bys by bychom byste sa som sme sú budem budeš bude
budeme budete budú budou się będę będziesz będzie będziemy będziecie będą съм си е сме сте са ще
бих би бихме бихте биха буду будеш буде будемо будете будуть ся ben bent is zijn was waren heb
hebt heeft hebben had hadden zal zult zullen zou zouden word wordt worden werd werden har hade ska
skall skulle vil ville att er var blir blev havde bliver skal am ai a ați au aș ar voi vei va vom
veți vor să fi θα να έχω έχεις έχει έχουμε έχετε έχουν είχα είχες είχε είχαμε είχατε είχαν en et ei
emme ette eivät älä älkää है हैं हूँ हो था थी थे थीं
""".split())

class Forms(dict):
    """Formes fléchies : (clé, forme de base) -> (priorité, étiquettes, description). Une forme
    décrite par une définition l'emporte sur les tableaux ; entre deux lignes de tableau, la plus
    simple (« volt » : 3e personne du passé, plutôt que 2e personne de politesse)."""
    def __init__(self, lang, attested):
        super().__init__()
        self.lang, self.attested = lang, attested

    def add(self, form, lemma, desc="", prio=1, ntags=0):
        if not form or not lemma or len(form) > 40:
            return
        if " " in form.strip():
            content = [w for w in form.split() if w.lower() not in CLITICS]
            if len(content) != 1:
                return
            form = content[0]
        k = key2(form, self.lang)
        if not k or k == key2(lemma, self.lang) or (self.attested is not None and k not in self.attested):
            return
        cur = self.get((k, lemma))
        cand = (prio if desc else 9, ntags, desc)
        if cur is None or cand[:2] < cur[:2]:
            self[(k, lemma)] = cand

# ---------- formes rencontrées dans des textes annotés (Universal Dependencies) ----------

# corpus sous licence CC BY-SA 4.0 seulement, pour les langues où le Wiktionnaire liste
# mal les formes les plus courantes (participes slovènes « rekel », impératifs lettons…)
UD_TREEBANKS = {"sl": ["Slovenian-SSJ", "Slovenian-SST"], "lt": ["Lithuanian-ALKSNIS", "Lithuanian-HSE"],
                "lv": ["Latvian-LVTB"], "et": ["Estonian-EWT"], "hr": ["Croatian-SET"],
                "ko": ["Korean-Kaist", "Korean-GSD"], "id": ["Indonesian-GSD"], "tr": ["Turkish-BOUN"]}
UD_TAGS = {
    "Case=Nom": "nominative", "Case=Gen": "genitive", "Case=Dat": "dative", "Case=Acc": "accusative",
    "Case=Ins": "instrumental", "Case=Loc": "locative", "Case=Voc": "vocative", "Case=Abl": "ablative",
    "Case=Par": "partitive", "Case=Ess": "essive", "Case=Tra": "translative", "Case=Ine": "inessive",
    "Case=Ela": "elative", "Case=Ill": "illative", "Case=Ade": "adessive", "Case=All": "allative",
    "Case=Com": "comitative", "Case=Abe": "abessive", "Case=Ter": "terminative",
    "Number=Sing": "singular", "Number=Plur": "plural", "Number=Dual": "dual",
    "Person=1": "first-person", "Person=2": "second-person", "Person=3": "third-person",
    "Gender=Masc": "masculine", "Gender=Fem": "feminine", "Gender=Neut": "neuter",
    "Tense=Pres": "present", "Tense=Past": "past", "Tense=Fut": "future", "Tense=Imp": "imperfect",
    "Tense=Pqp": "pluperfect", "Mood=Ind": "indicative", "Mood=Sub": "subjunctive",
    "Mood=Cnd": "conditional", "Mood=Imp": "imperative", "Mood=Opt": "optative", "Mood=Pot": "potential",
    "VerbForm=Inf": "infinitive", "VerbForm=Part": "participle", "VerbForm=Ger": "gerund",
    "VerbForm=Sup": "supine", "VerbForm=Conv": "converb", "Voice=Act": "active", "Voice=Pass": "passive",
    "Aspect=Perf": "perfective", "Aspect=Imp": "imperfective", "Degree=Cmp": "comparative",
    "Degree=Sup": "superlative", "Definite=Def": "definite", "Definite=Ind": "indefinite",
    "Polarity=Neg": "negative",
}

def fetch_ud(root):
    """Télécharge les corpus annotés (fichiers .conllu) dans <sources>/ud/<langue>/."""
    import subprocess
    for lang, banks in UD_TREEBANKS.items():
        os.makedirs(os.path.join(root, "ud", lang), exist_ok=True)
        for bank in banks:
            listing = json.loads(subprocess.check_output(
                ["curl", "-sfL", f"https://api.github.com/repos/UniversalDependencies/UD_{bank}/contents/"]))
            for f in listing:
                if f["name"].endswith(".conllu"):
                    subprocess.check_call(["curl", "-sfL", "-o", os.path.join(root, "ud", lang, f["name"]), f["download_url"]])
                    print(lang, f["name"], flush=True)

def ud_forms(src, lang, forms, native):
    """(forme, forme de base) de chaque mot des corpus annotés, décrite par ses traits."""
    folder = src.path("ud", lang)
    if not os.path.isdir(folder):
        return
    for name in sorted(os.listdir(folder)):
        if not name.endswith(".conllu"):
            continue
        with open(os.path.join(folder, name), encoding="utf-8") as f:
            for line in f:
                cols = line.rstrip("\n").split("\t")
                if len(cols) != 10 or not cols[0].isdigit():
                    continue
                form, lemma, upos, feats = cols[1], cols[2], cols[3], cols[5]
                if upos in ("PROPN", "PUNCT", "NUM", "SYM", "X") or lemma in ("_", ""):
                    continue
                if lang == "ko":  # « 학교+에서 », « 가+았+다 » : le premier morphème
                    first = lemma.split("+")[0]
                    lemma = first + "다" if upos in ("VERB", "ADJ", "AUX") else first
                tags = [UD_TAGS[x] for x in feats.split("|") if x in UD_TAGS]
                forms.add(form, lemma, describe(tags, native), 3, len(tags))

def resolve_lemmas(forms, heads, lang):
    """Formes de base écrites avec un accent tonique ou tonal (« kȕća ») : ramenées au mot du
    dictionnaire (« kuća ») ; les formes qui mènent à un mot absent sont retirées."""
    by_key, by_loose = {}, defaultdict(set)
    for k, w in heads:
        by_key.setdefault(k, w)
        by_loose[loose(w)].add(w)
    via = {}  # forme -> sa forme de base, pour les formes de formes
    for (k, lemma) in forms:
        via.setdefault(k, lemma)
    def find(lemma):
        target = by_key.get(key2(lemma, lang)) or by_key.get(key2(unstress(lemma), lang))
        if not target:
            cands = by_loose.get(loose(lemma), set())
            target = next(iter(cands)) if len(cands) == 1 else None
        return target
    out = {}
    for (k, lemma), val in forms.items():
        desc = val[2] if isinstance(val, tuple) else val
        target = find(lemma)
        if not target and key2(lemma, lang) in via:  # « nėra » : négation de « yrà », forme de « būti »
            target = find(via[key2(lemma, lang)])
        if target and key2(target, lang) != k and not out.get((k, target)):
            out[(k, target)] = desc
    return out

# ---------- définitions en anglais ----------

def build_en2(src, lang, dst, pairs=None):
    forms = Forms(lang, src.attested(lang))
    entries = []
    for d in src.entries("en", lang):
        word, pos = d.get("word") or "", d.get("pos") or ""
        if not word or pos in SKIP_POS_V2 or len(word) > 60 or word.count(" ") > 4:
            continue
        common, rare = [], []
        for s in d.get("senses") or []:
            gl = s.get("glosses") or []
            target = (s.get("form_of") or s.get("alt_of") or [{}])[0].get("word")
            if target:
                forms.add(word, target, gloss_desc(gl, s.get("tags") or [], "en"), 0)
                continue
            if not gl:
                continue
            g = clean_gloss(gl[0])
            if g and g not in common and g not in rare:
                (rare if RARE & set(s.get("tags") or []) else common).append(g)
        for fm in d.get("forms") or []:
            tags = set(fm.get("tags") or [])
            if fm.get("form") != word and not tags & NOT_FORMS and (fm.get("source") in FORM_SOURCES or (tags and tags <= FORM_TAGS_V2)):
                forms.add(fm.get("form"), word, describe(tags, "en"), 1, len(tags))
        glosses = (common + rare)[:MAX_GLOSS]
        if glosses:
            entries.append((key2(word, lang), word, POS_EN.get(pos, pos.capitalize()), ipa_of(d), glosses))
    # mots sans entrée, connus par les tables de traduction (« zato » : therefore)
    have = {e[0] for e in entries}
    by_pos = defaultdict(lambda: defaultdict(list))
    shown = {}
    for w, items in (pairs or {}).items():
        k = key2(w, lang)
        if k in have:
            continue
        shown.setdefault(k, plain_word(w, lang))
        for e, pos, weight, g, _ in items:
            by_pos[k][pos].append((e, weight, g, 0))
    for k, poss in by_pos.items():
        for pos, items in poss.items():
            lines = trans_lines(items, MAX_GLOSS, "en")
            if lines:
                entries.append((k, shown[k], POS_EN.get(pos, (pos or "").capitalize()), "", lines))
    # formes connues du seul Wiktionnaire français (tableaux slovènes, formes lettones…)
    for d in src.entries("fr", lang):
        word = d.get("word") or ""
        for s in d.get("senses") or []:
            fo = (s.get("form_of") or [{}])[0].get("word")
            if fo:
                tags = s.get("tags") or []
                forms.add(word, fo, describe(tags, "en"), 2, len(tags))
        for fm in d.get("forms") or []:
            tags = set(fm.get("tags") or [])
            if tags and tags <= FORM_TAGS_V2:
                forms.add(fm.get("form"), word, describe(tags, "en"), 2, len(tags))
    ud_forms(src, lang, forms, "en")
    forms = resolve_lemmas(forms, [(e[0], e[1]) for e in entries], lang)
    finish_forms(forms, entries, lang)
    write_v2(dst, lang, "en", entries, forms, "English Wiktionary via kaikki.org (CC BY-SA 4.0)"
             + (", Universal Dependencies (CC BY-SA 4.0)" if lang in UD_TREEBANKS else ""))

# ---------- définitions en français ----------

def build_fr2(src, lang, dst, pairs):
    """Wiktionnaire français (définitions) + tables de traduction des deux Wiktionnaires
    (équivalents), sur le squelette du Wiktionnaire anglais (mots, natures, prononciation,
    formes fléchies, décrites en français)."""
    forms = Forms(lang, src.attested(lang))
    heads = {}  # clé -> {"word", "pos": [natures dans l'ordre], "ipa", "en": {nature: [sens anglais]}}
    def head(k, word, pos, ipa, en_gl=None):
        h = heads.setdefault(k, {"word": word, "pos": [], "ipa": "", "en": {}})
        if pos and pos not in h["pos"]:
            h["pos"].append(pos)
        h["ipa"] = h["ipa"] or ipa
        for p, gl in (en_gl or {}).items():
            h["en"].setdefault(p, []).extend(gl)
    for d in src.entries("en", lang):
        word, pos = d.get("word") or "", d.get("pos") or ""
        if not word or pos in SKIP_POS_V2 or len(word) > 60 or word.count(" ") > 4:
            continue
        lemma_sense, en_gl = False, {}
        for s in d.get("senses") or []:
            target = (s.get("form_of") or s.get("alt_of") or [{}])[0].get("word")
            if target:
                forms.add(word, target, describe(s.get("tags") or [], "fr"), 0, len(s.get("tags") or []))
            elif s.get("glosses"):
                lemma_sense = True
                en_gl.setdefault(pos, []).append(s["glosses"][0].lower())
        for fm in d.get("forms") or []:
            tags = set(fm.get("tags") or [])
            if fm.get("form") != word and not tags & NOT_FORMS and (fm.get("source") in FORM_SOURCES or (tags and tags <= FORM_TAGS_V2)):
                forms.add(fm.get("form"), word, describe(tags, "fr"), 1, len(tags))
        if lemma_sense:
            head(key2(word, lang), word, pos, ipa_of(d), en_gl)
    # Wiktionnaire français : définitions (prioritaires) et formes décrites
    defs, titles = defaultdict(list), {}
    for d in src.entries("fr", lang):
        word, pos = d.get("word") or "", d.get("pos") or ""
        title = clean_pos(d.get("pos_title", ""))
        if not word or pos == "name" or any(title.startswith(s) for s in SKIP_TITLES_FR) or len(word) > 60:
            continue
        k = key2(word, lang)
        for s in d.get("senses") or []:
            fo = (s.get("form_of") or s.get("alt_of") or [{}])[0].get("word")
            gl = s.get("glosses") or []
            if fo:  # la description du Wiktionnaire français d'abord
                forms.add(word, fo, gloss_desc(gl, s.get("tags") or [], "fr"), -1)
            elif gl and "no-gloss" not in (s.get("tags") or []):
                g = fr_def(gl[0])
                if g and not g.startswith("Définition manquante") and g not in defs[(k, pos)]:
                    defs[(k, pos)].append(g)
        if defs.get((k, pos)):
            titles.setdefault((k, pos), title)
            head(k, word, pos, ipa_of(d))
        for fm in d.get("forms") or []:
            tags = set(fm.get("tags") or [])
            if tags and tags <= FORM_TAGS_V2:
                forms.add(fm.get("form"), word, describe(tags, "fr"), 1, len(tags))
    # traductions, rattachées aux mots du dictionnaire (au besoin sans accent tonal)
    by_loose = defaultdict(set)
    for k in heads:
        by_loose[loose(k)].add(k)
    trans = defaultdict(list)  # (clé, nature) -> [(équivalent, poids, groupe)]
    for w, items in pairs.items():
        k = key2(w, lang)
        if k not in heads:
            near = by_loose.get(loose(w), set())
            if len(near) == 1:
                k = next(iter(near))
            else:
                head(k, plain_word(w, lang), None, "")
        h = heads[k]
        for f, pos, weight, g, en_word in items:
            if pos not in h["pos"]:
                same = [p for p in h["pos"] if COARSE.get(p, "f") == COARSE.get(pos, "f")]
                if same or len(h["pos"]) == 1:
                    pos = (same or h["pos"])[0]  # natures différentes d'une langue à l'autre
                elif not h["pos"]:
                    h["pos"].append(pos)
                else:
                    continue
            b = 0
            if en_word:
                # « house » est le premier sens anglais de « ház » : ses traductions passent devant
                ens = h["en"].get(pos) or [x for v in h["en"].values() for x in v]
                pat = re.compile(r"(^|\W)" + re.escape(en_word.lower()) + r"($|\W)")
                i = next((i for i, x in enumerate(ens[:6]) if pat.search(x)), None)
                if i is not None:
                    b = (4 if weight >= 1 else 2) / (1 + i)
            trans[(k, pos)].append((f, weight, g, b))
    entries = []
    for k, h in heads.items():
        for pos in h["pos"]:
            glosses = defs.get((k, pos), [])[:MAX_GLOSS]
            for line in trans_lines(trans.get((k, pos), []), MAX_GLOSS):
                if len(glosses) >= MAX_GLOSS:
                    break
                if not mentions(line, glosses):
                    glosses.append(line)
            if glosses:
                entries.append((k, h["word"], titles.get((k, pos)) or POS_FR.get(pos, (pos or "").capitalize()), h["ipa"], glosses))
    ud_forms(src, lang, forms, "fr")
    forms = resolve_lemmas(forms, [(e[0], e[1]) for e in entries], lang)
    finish_forms(forms, entries, lang)
    write_v2(dst, lang, "fr", entries, forms, "Wiktionnaire et English Wiktionary via kaikki.org (CC BY-SA 4.0)"
             + (", Universal Dependencies (CC BY-SA 4.0)" if lang in UD_TREEBANKS else ""))

# ---------- japonais : JMdict et KANJIDIC2 (EDRDG, CC BY-SA 4.0) ----------

# (début de la nature dans JMdict, anglais, français) ; la classe entre parenthèses sert
# à retrouver la forme du dictionnaire d'un verbe conjugué (dict.rs, deinflect_ja)
JM_POS = [
    ("Ichidan verb", "Verb (ichidan)", "Verbe (ichidan)"),
    ("Godan verb", "Verb (godan)", "Verbe (godan)"),
    ("Kuru verb", "Verb (kuru)", "Verbe (kuru)"),
    ("suru verb", "Verb (suru)", "Verbe (suru)"),
    ("noun or participle which takes the aux. verb suru", "Noun (suru)", "Nom (suru)"),
    ("adjective (keiyoushi)", "Adjective (-i)", "Adjectif (-i)"),
    ("auxiliary adjective", "Auxiliary (-i)", "Auxiliaire (-i)"),
    ("adjectival nouns or quasi-adjectives", "Adjective (-na)", "Adjectif (-na)"),
    ("'taru' adjective", "Adjective (-taru)", "Adjectif (-taru)"),
    ("pre-noun adjectival", "Pre-noun adjectival", "Adjectif prénominal"),
    ("noun or verb acting prenominally", "Prenominal", "Prénominal"),
    ("auxiliary verb", "Auxiliary verb", "Verbe auxiliaire"),
    ("auxiliary", "Auxiliary", "Auxiliaire"),
    ("adverb", "Adverb", "Adverbe"),
    ("pronoun", "Pronoun", "Pronom"),
    ("particle", "Particle", "Particule"),
    ("conjunction", "Conjunction", "Conjonction"),
    ("interjection", "Interjection", "Interjection"),
    ("counter", "Counter", "Classificateur"),
    ("numeric", "Numeral", "Numéral"),
    ("copula", "Copula", "Copule"),
    ("expressions", "Expression", "Expression"),
    ("noun, used as a suffix", "Suffix", "Suffixe"),
    ("noun, used as a prefix", "Prefix", "Préfixe"),
    ("suffix", "Suffix", "Suffixe"),
    ("prefix", "Prefix", "Préfixe"),
    ("noun", "Noun", "Nom"),
]
# nature française de JMdict -> nature du Wiktionnaire
JM_CODE = {"Nom": "noun", "Verbe": "verb", "Adjectif": "adj", "Adverbe": "adv", "Pronom": "pron",
           "Particule": "particle", "Conjonction": "conj", "Interjection": "intj", "Expression": "phrase"}
JM_RARE = {"archaic", "obsolete term", "rare term", "dated term", "historical term"}
XML_LANG = "{http://www.w3.org/XML/1998/namespace}lang"

def jm_pos(tags, native):
    for start, en, fr in JM_POS:
        if any(t.startswith(start) for t in tags):
            return en if native == "en" else fr
    return ""

def jm_rank(pri):
    """Mots courants d'abord : listes de JMdict (news1, ichi1, spec1, gai1…), puis rang de
    fréquence dans la presse (nf01 à nf48), par centaines."""
    tier = 0 if pri & {"news1", "ichi1", "spec1", "gai1"} else 1 if pri & {"news2", "ichi2", "spec2", "gai2"} else 2 if pri else 3
    nf = min((int(p[2:]) for p in pri if p.startswith("nf") and p[2:].isdigit()), default=99)
    return tier * 1000 + nf

def jmdict(path):
    import xml.etree.ElementTree as ET
    for _, el in ET.iterparse(gzip.open(path)):
        if el.tag != "entry":
            continue
        kebs = [(k.findtext("keb"), {p.text for p in k.iter("ke_pri")}, any("search-only" in (i.text or "") for i in k.iter("ke_inf")))
                for k in el.iter("k_ele")]
        rebs = [(r.findtext("reb"), {p.text for p in r.iter("re_pri")}, [x.text for x in r.iter("re_restr")],
                 r.find("re_nokanji") is not None, any("search-only" in (i.text or "") for i in r.iter("re_inf")))
                for r in el.iter("r_ele")]
        senses, pos = [], []
        for s in el.iter("sense"):
            pos = [p.text for p in s.iter("pos")] or pos  # la nature vaut jusqu'au prochain changement
            gl = defaultdict(list)
            for g in s.iter("gloss"):
                if g.text:
                    gl[g.get(XML_LANG, "eng")].append(g.text)
            senses.append((pos, gl, {m.text for m in s.iter("misc")}))
        el.clear()
        yield kebs, rebs, senses

def build_ja(src, native, dst):
    lang_tag = "eng" if native == "en" else "fre"
    pairs = load_pairs(src, ["ja"])["ja"] if native == "fr" else {}
    frdefs = defaultdict(list)  # mot -> [(nature, définition)]
    if native == "fr":
        for d in src.entries("fr", "ja"):
            if d.get("pos") in ("character", "name", "suffix", "prefix"):
                continue
            for s in d.get("senses") or []:
                gl = s.get("glosses") or []
                if gl and not s.get("form_of") and "no-gloss" not in (s.get("tags") or []):
                    g = fr_def(gl[0])
                    if g and (d.get("pos"), g) not in frdefs[d["word"]]:
                        frdefs[d["word"]].append((d.get("pos"), g))
    senses_rows, keys, seen_main = [], [], set()  # (id, mot, lecture, nature, sens) ; (rang, clé, id, graphie, lecture)
    for kebs, rebs, senses in jmdict(src.path("JMdict.gz")):
        label = jm_pos(senses[0][0] if senses else [], native)
        usually_kana = any("word usually written using kana alone" in m for _, _, m in senses[:1])
        common, rare = [], []
        for pos, gl, misc in senses:
            if gl.get(lang_tag):
                line = clean_gloss("; ".join(gl[lang_tag][:4]))
                if line not in common and line not in rare:
                    (rare if misc & JM_RARE else common).append(line)
        lines = []
        # mot écrit d'ordinaire : ses définitions du Wiktionnaire et ses traductions
        # graphie qui relie l'entrée au Wiktionnaire et aux tables de traduction ; une lecture
        # en kana seulement pour un mot sans kanji (« うち » désigne aussi 内, l'intérieur)
        main = [k for k, _, so in kebs if not so] if kebs and not usually_kana else [] if kebs else [r for r, *_ , so in rebs if not so]
        main_key = main[0] if main else None
        if native == "fr" and main_key and main_key not in seen_main:
            seen_main.add(main_key)
            # définitions de même nature (家 : « maison » pour le nom, pas le suffixe)
            want = JM_CODE.get(label.split(" (")[0])
            lines += [g for p, g in frdefs.get(main_key, []) if not want or p == want][:MAX_GLOSS]
        for line in common + rare:
            if len(lines) < MAX_GLOSS and not mentions(line.split(";")[0], lines):
                lines.append(line)
        if native == "fr" and main_key and len(lines) < MAX_GLOSS:
            for line in trans_lines([(f, w, g, 0) for f, _, w, g, _ in pairs.get(main_key, [])], MAX_GLOSS):
                if len(lines) < MAX_GLOSS and not mentions(line, lines):
                    lines.append(line)
        if not lines:
            continue
        pri = set().union(*(p for _, p, _ in kebs), *(r[1] for r in rebs)) if (kebs or rebs) else set()
        rank = jm_rank(pri)
        eid = len(senses_rows) + 1
        shown_k = next((k for k, _, so in kebs if not so), kebs[0][0] if kebs else None)
        main_word = shown_k if kebs and not usually_kana else rebs[0][0]
        main_reading = next((r for r, _, restr, nokanji, _ in rebs if not nokanji and (not restr or main_word in restr)), "") if main_word != rebs[0][0] else ""
        senses_rows.append((eid, main_word, main_reading, label, "␞".join(lines)))
        # écrit en kanji : l'entrée est rangée sous chaque graphie, avec sa première lecture ;
        # le rang de chaque graphie, de chaque lecture, compte (嘴 se lit d'abord くちばし)
        for k, kp, kso in kebs:
            reading = next((r for r, _, restr, nokanji, _ in rebs if not nokanji and (not restr or k in restr)), "")
            keys.append((jm_rank(kp) if kp else rank + 500, key2(k, "ja"), eid, k if not kso else shown_k, reading))
        # et sous chaque lecture en kana ; en kana, un mot qu'on écrit d'ordinaire en kanji passe après
        for r, rp, restr, nokanji, so in rebs:
            word = r if usually_kana or not kebs else shown_k
            plain = not kebs or (usually_kana and r == rebs[0][0])
            keys.append(((jm_rank(rp) if rp else rank + 500) + (0 if plain else 1000), key2(r, "ja"), eid, word, r if word != r else ""))
    # kanji : sens et lectures (on, kun), après les mots
    import xml.etree.ElementTree as ET
    m_lang = "en" if native == "en" else "fr"
    for _, el in ET.iterparse(gzip.open(src.path("kanjidic2.xml.gz"))):
        if el.tag != "character":
            continue
        lit = el.findtext("literal")
        means = [m.text for m in el.iter("meaning") if m.get("m_lang", "en") == m_lang and m.text]
        on = [r.text for r in el.iter("reading") if r.get("r_type") == "ja_on"]
        kun = [r.text for r in el.iter("reading") if r.get("r_type") == "ja_kun"]
        el.clear()
        if lit and means:
            eid = len(senses_rows) + 1
            reading = "、".join(on[:3] + [k.replace(".", "") for k in kun[:3]])
            senses_rows.append((eid, lit, reading, "Kanji", clean_gloss(", ".join(means[:5]))))
            keys.append((9000, lit, eid, lit, reading))
    write_ja(dst, native, senses_rows, keys,
             "JMdict, KANJIDIC2 (EDRDG, CC BY-SA 4.0)" + (", Wiktionnaire via kaikki.org (CC BY-SA 4.0)" if native == "fr" else ""))

def write_ja(dst, native, senses_rows, keys, source):
    """Japonais : chaque entrée de JMdict une seule fois (`senses`), retrouvée par chacune de
    ses graphies et lectures (`keys`) ; la vue `entries` a la forme des autres dictionnaires."""
    tmp = dst + ".tmp"
    if os.path.exists(tmp):
        os.remove(tmp)
    db = sqlite3.connect(tmp)
    # clés rangées par (clé, rang) sans index à part ; graphie et lecture seulement quand
    # elles diffèrent de celles de l'entrée (variantes, lectures secondaires)
    db.executescript("""
      PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
      CREATE TABLE senses(id INTEGER PRIMARY KEY, word TEXT, ipa TEXT, pos TEXT, gloss TEXT);
      CREATE TABLE keys(k TEXT, rank INTEGER, id INTEGER, word TEXT, ipa TEXT, PRIMARY KEY(k, rank)) WITHOUT ROWID;
      CREATE VIEW entries AS SELECT keys.k AS k, IFNULL(keys.word, senses.word) AS word, senses.pos AS pos,
        IFNULL(keys.ipa, senses.ipa) AS ipa, senses.gloss AS gloss, keys.rank AS rank
        FROM keys JOIN senses ON senses.id = keys.id;
      CREATE TABLE forms(k TEXT, lemma TEXT, note INTEGER, PRIMARY KEY(k, lemma)) WITHOUT ROWID;
      CREATE TABLE notes(id INTEGER PRIMARY KEY, text TEXT);
      CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
    """)
    db.executemany("INSERT INTO senses VALUES(?,?,?,?,?)", senses_rows)
    main = {eid: (w, i) for eid, w, i, _, _ in senses_rows}
    keys.sort(key=lambda x: (x[0], x[2]))  # mots courants d'abord, puis l'ordre de JMdict
    seen, rows = set(), []
    for rank, k, eid, word, ipa in keys:
        if (k, eid) not in seen:
            seen.add((k, eid))
            mw, mi = main[eid]
            rows.append((k, len(rows), eid, None if word == mw else word, None if ipa == mi else ipa))
    db.executemany("INSERT INTO keys VALUES(?,?,?,?,?)", rows)
    db.executemany("INSERT INTO meta VALUES(?,?)", {"format": "2", "lang": "ja", "native": native, "source": source}.items())
    db.commit()
    db.execute("VACUUM")
    db.close()
    with open(tmp, "rb") as fi, gzip.open(dst, "wb", compresslevel=9) as fo:
        shutil.copyfileobj(fi, fo)
    raw = os.path.getsize(tmp)
    os.remove(tmp)
    print(f"{os.path.basename(dst)}: {len(senses_rows)} entrées, {len(rows)} clés, "
          f"{raw/1e6:.1f} Mo -> {os.path.getsize(dst)/1e6:.1f} Mo gz")

# ---------- vérification : couverture des mots les plus fréquents ----------

def check(path_gz, lang, freq_path, sizes=(1000, 5000, 20000)):
    """Part des mots les plus fréquents (sous-titres) qui trouvent une définition."""
    tmp = path_gz + ".check.db"
    with gzip.open(path_gz, "rb") as fi, open(tmp, "wb") as fo:
        shutil.copyfileobj(fi, fo)
    db = sqlite3.connect(tmp)
    words = []
    with open(freq_path, encoding="utf-8") as f:
        for line in f:
            w = line.rsplit(" ", 1)[0]
            if any(c.isalpha() for c in w):
                words.append(w)
    found = []
    for w in words[:max(sizes)]:
        k = key2(w, lang)
        hit = db.execute("SELECT 1 FROM entries WHERE k=? LIMIT 1", (k,)).fetchone()
        if not hit:
            hit = db.execute("SELECT 1 FROM forms f JOIN entries e ON e.word=f.lemma WHERE f.k=? LIMIT 1", (k,)).fetchone()
        found.append(bool(hit))
    db.close()
    os.remove(tmp)
    res = [f"{100 * sum(found[:n]) / max(1, len(found[:n])):.0f} %" for n in sizes]
    missing = [w for w, ok in zip(words[:300], found[:300]) if not ok][:20]
    return res, missing

if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["--split"]:
        split(args[1])
        sys.exit()
    if args[:1] == ["--ud"]:
        fetch_ud(args[1])
        sys.exit()
    if args[:1] == ["--keys"]:
        # clés calculées ici, pour vérifier que dict.rs calcule les mêmes :
        # LUMEN_KEYS=<fichier> cargo test --lib keys_match_builder -- --ignored
        src = Sources(args[1])
        with open(args[2], "w", encoding="utf-8") as out:
            for lang in V2_LANGS:
                words = []
                for d in src.entries("en", lang):
                    words.append(d.get("word") or "")
                    words += [f.get("form") or "" for f in (d.get("forms") or [])[:6]]
                    if len(words) > 4000:
                        break
                p = src.path("freq", f"{lang}.txt")
                if os.path.exists(p):
                    with open(p, encoding="utf-8") as f:
                        words += [line.rsplit(" ", 1)[0] for line in f][:3000]
                for w in dict.fromkeys(words):
                    if w and "\t" not in w and "\n" not in w:
                        out.write(f"{lang}\t{w}\t{key2(w, lang)}\n")
        sys.exit()
    if args[:1] in (["--v2"], ["--check"]):
        src, out_dir = Sources(args[1]), args[2]
        os.makedirs(out_dir, exist_ok=True)
        todo = args[3:] or [f"{n}-{l}" for n in ("en", "fr") for l in V2_LANGS]
        if args[0] == "--check":
            for name in todo:
                native, lang = name.split("-")
                p, fq = os.path.join(out_dir, name + ".db.gz"), src.path("freq", f"{lang}.txt")
                if os.path.exists(p) and os.path.exists(fq):
                    res, missing = check(p, lang, fq)
                    print(name, "  top 1k / 5k / 20k :", " / ".join(res), "  absents :", " ".join(missing))
            sys.exit()
        fr_langs = [n.split("-")[1] for n in todo if n.startswith("fr-") and n != "fr-ja"]
        pairs = load_pairs(src, fr_langs) if fr_langs else {}
        for native in RANK:
            if os.path.exists(src.path("freq", f"{native}.txt")):
                with open(src.path("freq", f"{native}.txt"), encoding="utf-8") as f:
                    for i, line in enumerate(f):
                        RANK[native].setdefault(line.rsplit(" ", 1)[0], i)
        en_langs = [n.split("-")[1] for n in todo if n.startswith("en-") and n != "en-ja"]
        en_pairs = load_en_pairs(src, en_langs) if en_langs else {}
        for name in todo:
            native, lang = name.split("-")
            dst = os.path.join(out_dir, name + ".db.gz")
            if lang == "ja":
                build_ja(src, native, dst)
            elif native == "en":
                build_en2(src, lang, dst, en_pairs.get(lang, {}))
            else:
                build_fr2(src, lang, dst, pairs.get(lang, {}))
        sys.exit()
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
