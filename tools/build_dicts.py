#!/usr/bin/env python3
"""Construit les dictionnaires compacts de Lumen à partir des extractions
Wiktionnaire (français) publiées par kaikki.org (wiktextract).

Usage : python3 tools/build_dicts.py <dossier_jsonl> <dossier_sortie>
Chaque fichier <Langue>.jsonl produit <code>.db.gz (SQLite compressé).
Licence des données : CC BY-SA (Wiktionnaire), attribution affichée dans l'app.
"""
import gzip, json, os, re, shutil, sqlite3, sys, unicodedata

LANGS = {"Anglais": "en", "Espagnol": "es", "Italien": "it", "Allemand": "de",
         "Portugais": "pt", "Russe": "ru"}
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
    db.executescript("""
      CREATE INDEX ie ON entries(k);
      CREATE INDEX iw ON entries(word);
      CREATE INDEX iff ON forms(k);
      CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
    """)
    db.execute("INSERT INTO meta VALUES('source','Wiktionnaire via kaikki.org (CC BY-SA 4.0)')")
    db.commit()
    db.execute("VACUUM")
    db.close()
    with open(tmp, "rb") as fi, gzip.open(dst, "wb", compresslevel=9) as fo:
        shutil.copyfileobj(fi, fo)
    raw = os.path.getsize(tmp)
    os.remove(tmp)
    print(f"{os.path.basename(dst)}: {n_e} entrées, {n_f} formes, "
          f"{raw/1e6:.1f} Mo -> {os.path.getsize(dst)/1e6:.1f} Mo gz")

if __name__ == "__main__":
    src_dir, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    only = sys.argv[3:] or list(LANGS)
    for name in only:
        p = os.path.join(src_dir, name + ".jsonl")
        if os.path.exists(p):
            build(p, os.path.join(out_dir, LANGS[name] + ".db.gz"))
