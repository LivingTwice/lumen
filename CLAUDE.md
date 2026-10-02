# CLAUDE.md : Lumen

Lumen est une application Mac pour apprendre les langues en lisant et en écoutant, dans l'esprit de LingQ. Toute l'intelligence tourne **en local** : traduction en contexte par Qwen3.5 (llama.cpp, Metal), transcription par Whisper (whisper.cpp, Metal), dictionnaires hors ligne. Mac d'abord (Apple Silicon), Windows et web ensuite.

- **Interface** : entièrement en français.
- **Langues étudiées** : anglais, italien, allemand, portugais, russe, espagnol (codes `en it de pt ru es`).
- **Priorité absolue** : la beauté du design et des animations. Le nom « Lumen » guide l'identité visuelle : lumière, aube, halo, lanterne.
- **Propriétaire** : Ulysse (compte GitHub `LivingTwice`). Il n'est pas développeur de métier : expliquer simplement, en français, sans jargon inutile.
- **Copie de référence du code** : `~/Documents/Lumen` sur le Mac d'Ulysse. C'est là qu'on modifie, compile et publie.

## Stack

- **Coquille** : Tauri 2.12 (Rust 2021). Fenêtre `titleBarStyle: Overlay`, transparente, vibrance macOS « sidebar », `macOSPrivateApi`, `acceptFirstMouse`.
- **Interface** : React 19, TypeScript 5.9, Vite 8, `motion` 14 (animations), `zustand` 5 (état global).
- **Polices** : Geist (interface), Newsreader (titres), Literata (texte de lecture, gère le cyrillique). Via `@fontsource-variable`.
- **IA locale** : crate `llama-cpp-2` 0.1.158, feature `metal`, modèles GGUF Qwen3.5 (0.8B, 2B, 4B) téléchargés depuis Hugging Face (unsloth).
- **Transcription** : `whisper-rs` 0.16 dans un exécutable séparé `lumen-whisper` (sidecar).
- **Base de données** : SQLite via `rusqlite` (bundled), mode WAL.
- **Imports côté interface** : `@mozilla/readability` (pages web), `jszip` (EPUB), `pdfjs-dist` (PDF).
- **YouTube** : `yt-dlp` (système ou géré par Lumen) avec un moteur JavaScript (deno, node 22+ ou QuickJS géré).
- **Mises à jour** : `tauri-plugin-updater` + `tauri-plugin-process`, paquets signés minisign, publiés sur GitHub.

## Fonctionnalités en place

### Lecture

- Texte découpé en **pages** (~230 mots), mots colorés selon leur **statut** : nouveau (absent de la table `terms`), 1, 2, 3 (en apprentissage), 4 (connu), 5 (ignoré).
- Sélection d'un mot au clic, d'une **expression** en glissant sur plusieurs mots.
- **Panneau du mot** (`WordPanel`) : dictionnaire hors ligne (forme de base, sens), puis sens précis en contexte par l'IA, en streaming. Traduction de la phrase entière.
- **« Terminer la page »** : les mots nouveaux non consultés passent en connus.
- **Simplifier** : l'IA réécrit la leçon au niveau A1, A2, B1 ou B2 et crée une nouvelle leçon.
- **Raccourcis** : flèches (mot suivant ou précédent), `1` `2` `3` (statut), `K` ou `4` (connu), `X` (ignorer), `0` (remettre à nouveau), Espace (lecture audio), Entrée (terminer la page), Échap (fermer, quitter le mode cinéma), PageUp et PageDown.

### Écoute

- **Voix du système** (Web Speech API dans WKWebView) avec surlignage mot à mot : la **lanterne**.
- **Audio importé** synchronisé grâce aux horodatages mot à mot de Whisper.
- **Vidéo** : l'audio est le maître, la vidéo muette le suit (correction de dérive par ajustement de vitesse, ou saut au-delà de 0,35 s). Sous-titres interactifs (toucher un mot le traduit), sous-titres traduits en option, taille ajustable, **mode cinéma** plein écran.

### Import

- Texte collé, page web (extraction de l'article), EPUB (un chapitre par leçon), PDF, TXT, Markdown, sous-titres SRT et VTT.
- Audio et vidéo locaux (MP3, M4A, WAV, FLAC, OGG, MP4, MOV, MKV…), transcrits par Whisper.
- **YouTube** et autres sites : audio `ba[ext=m4a]` pour la transcription, vidéo `bv*[vcodec^=avc1][height<=1080]` téléchargée en parallèle. Si YouTube exige une vérification, nouvel essai avec `--cookies-from-browser` (réglage `youtube_browser`). Bouton « Télécharger la vidéo » sur les anciennes leçons sans image.
- Glisser-déposer n'importe où dans la fenêtre.
- **LingQ** (Réglages › LingQ, clé API personnelle) : mots connus et ignorés, LingQ (traductions, notes, contexte), leçons de tous les cours (créés, importés et suivis) avec audio. Les horodatages de LingQ sont par phrase : `lingq.rs` les répartit sur les mots. Fusion sans recul de statut (niveaux LingQ 1-3 → 1-3, niveau 4 et ✓ → connu), activité du jour non touchée, leçons dédupliquées par la colonne `ext_id` (`lingq:<id>`). L'import continue en arrière-plan (carte dans la barre latérale).

### Le reste

- **Bibliothèque** : leçons par langue et collection, pourcentage de mots connus, mots nouveaux.
- **Vocabulaire** : recherche, filtres (tous, en apprentissage, connus, ignorés, expressions), changement de statut, export CSV compatible Anki.
- **Progrès** : mots connus, paliers, mots lus par jour, temps d'écoute (30 jours).
- **Réglages** : thème (suit le Mac par défaut), typographie de lecture, voix, modèles IA (téléchargement avec reprise, suppression), vidéos en ligne (navigateur pour les cookies, état des composants), import LingQ, mises à jour, « Revoir l'accueil ».
- **Accueil** (`Onboarding`) : aube animée (ciel en parallaxe, astre qui se lève à l'horizon, poussières de lumière, révélation lettre par lettre). Clair = aube, sombre = nuit chaude. **Jamais de fond bleu.** Étapes : bienvenue, langues, profil IA, prêt, puis éclosion lumineuse vers la première leçon.
- **Mises à jour automatiques** : vérification 8 s après le démarrage puis toutes les 6 h, carte discrète dans la barre latérale, installation en un clic puis redémarrage.

## Architecture

```
Interface React (src/)                     Rust (src-tauri/src/)
  views/*  ──► lib/store.ts (zustand)
              lib/api.ts ── invoke() ──────►  commands.rs ──► db.rs      (SQLite)
                         ◄── Channel ───────                ──► dict.rs    (dictionnaires)
                                                            ──► ai.rs      (llama.cpp, Metal)
                                                            ──► media.rs   ──► lumen-whisper (sidecar)
                                                            ──► tools.rs   (yt-dlp, QuickJS)
                                                            ──► models.rs  (téléchargements GGUF)
                                                            ──► lingq.rs   (import LingQ)
```

- **Un seul point d'entrée vers le natif** : `src/lib/api.ts`. L'interface `Api` a deux implémentations : Tauri (`invoke`) et un **backend simulé** (`src/lib/mock.ts`) utilisé dans le navigateur.
- **Streaming** (traduction IA, progression des téléchargements et des imports) : `tauri::ipc::Channel` côté Rust, `new Channel<T>()` côté TypeScript.
- **Pourquoi un sidecar Whisper** : llama.cpp et whisper.cpp embarquent chacun leur propre copie de ggml ; les lier dans le même binaire provoque des conflits de symboles. `lumen-whisper` est lancé par `media.rs` et répond en lignes JSON.
- **IA** : prompts ChatML avec un bloc `<think></think>` vide (désactive le raisonnement de Qwen3.5), exemples few-shot, indices du dictionnaire. Les requêtes sont **interruptibles** par un compteur d'époque (une nouvelle sélection annule la précédente). Les réponses mot sont mises en cache (table `tcache`, clé versionnée `w3`).
- **Médias** : copiés dans `$APPDATA/media/`, servis à la WebView par le protocole `asset` (portée limitée à `$APPDATA/media/**`).
- **LingQ** : API non documentée, lue avec prudence (champs optionnels, reprises sur 429 et 5xx). v2 pour `known-words` et `ignored-words`, v3 pour `cards`, `collections/my`, `search?shelf=my_lessons`, `collections/{id}/lessons` et `lessons/{id}` (`tokenizedText`, `audioUrl`). La clé n'est envoyée qu'à `www.lingq.com`. Lire une leçon par l'API la fait remonter dans l'étagère « Continuer » de LingQ.
- **Dictionnaires** : Wiktionnaire français via kaikki.org, compilés en SQLite, livrés compressés dans `src-tauri/resources/dicts/*.db.gz`, décompressés au premier usage.
- **Données utilisateur** : `~/Library/Application Support/app.lumen.reader/` (`lumen.db`, `models/`, `media/`, dictionnaires décompressés).

## Structure des dossiers clés

```
src/
  App.tsx                 Démarrage, thème, glisser-déposer, routage des vues, accueil
  main.tsx                Montage React, polices, styles
  components/
    Sidebar.tsx           Navigation, langue active, compteur de mots connus, carte de mise à jour
    UpdateCard.tsx        Carte « Lumen X est disponible »
    LingqCard.tsx         Avancement de l'import LingQ dans la barre latérale
    ui.tsx                Composants partagés (Orb, Segmented, Switch, Sheet, Menu, Toasts, CountUp, useGlow)
    Icon.tsx              Icônes SVG maison
  lib/
    api.ts                Interface Api + implémentation Tauri (invoke, Channel, convertFileSrc)
    mock.ts               Backend simulé pour le navigateur (doit rester aligné sur api.ts)
    store.ts              État global zustand : réglages (DEFAULTS), vue, leçon, imports, toasts, modèles
    types.ts              Types partagés (miroir des structures Rust sérialisées)
    tokenize.ts           Découpage en mots, pages, phrases (miroir de text.rs)
    importers.ts          Extraction : web, EPUB, PDF, sous-titres
    tts.ts                Voix du système, événements de frontière de mot
    updater.ts            Store des mises à jour (check, install, restart)
    lingq.ts              Store de l'import LingQ (analyse, import, progression)
    langs.ts              Langues, salutations, textes de départ (STARTERS)
    profiles.ts           Profils IA (Léger, Équilibré, Maximum)
    dialogs.ts            Confirmations natives
  views/
    Onboarding.tsx        Accueil animé
    Library.tsx           Bibliothèque
    ImportSheet.tsx       Feuille d'import (toutes sources, étapes de progression)
    Vocabulary.tsx        Vocabulaire
    Progress.tsx          Progrès
    Settings.tsx          Réglages
    LingqSection.tsx      Réglages › LingQ (clé API, analyse du compte, import)
    reader/
      Reader.tsx          Lecteur : pages, sélection, raccourcis, lanterne, Simplifier
      WordPanel.tsx       Panneau du mot (dictionnaire + IA)
      Player.tsx          Lecture audio/vidéo/voix, synchronisation, poignée impérative
      VideoStage.tsx      Cadre vidéo, sous-titres interactifs, mode cinéma
  styles/
    app.css               Système visuel : variables clair/sombre, boutons, barre latérale, toasts
    views.css             Bibliothèque, vocabulaire, progrès, réglages
    reader.css            Lecteur, lanterne, vidéo, cinéma
    onboarding.css        Accueil (variables .ob pour clair et sombre)

src-tauri/
  tauri.conf.json         Fenêtre, bundle (DMG, ressources, sidecar), updater (clé publique, URL)
  Cargo.toml              Workspace (app + lumen-whisper), dépendances, profil release (LTO thin)
  capabilities/default.json  Permissions accordées à la WebView
  src/
    lib.rs                Plugins, setup (dossier de données, DB, dictionnaires), liste des commandes
    commands.rs           Toutes les commandes #[tauri::command]
    db.rs                 Schéma, migrations, requêtes (leçons, mots, activité, cache)
    ai.rs                 Moteur llama.cpp, prompts, analyse des réponses
    dict.rs               Recherche dans les dictionnaires
    text.rs               Découpage UAX 29, élisions, normalisation, offsets UTF-16
    media.rs              Transcription (sidecar), horodatages, yt-dlp
    tools.rs              yt-dlp et QuickJS gérés (installation, mise à jour hebdomadaire)
    models.rs             Catalogue et téléchargement des modèles avec reprise
    lingq.rs              Import LingQ (vocabulaire, cours, leçons, audio)
    state.rs              AppState partagé
  lumen-whisper/          Sidecar de transcription
  resources/dicts/        Dictionnaires compressés (≈ 66 Mo)
  dmg/background.png      Fond du DMG (source : design/dmg-background.html)

scripts/
  mac-env.sh              Vérifie et installe les outils, prépare la clé de signature
  release.mjs             bump (numéro de version partout) et manifest (latest.json)
  build-sidecar.mjs       Compile lumen-whisper sous le nom attendu par Tauri
tools/build_dicts.py      Reconstruction des dictionnaires depuis kaikki.org
design/                   Icône (SVG, PNG 1024), fond du DMG
Compiler Lumen.command    Double-clic : compile, installe dans /Applications, crée le DMG
Publier une version.command  Double-clic : version, compilation signée, publication GitHub
```

## Lancer l'app en local

Prérequis (installés automatiquement par `Compiler Lumen.command` la première fois) : Command Line Tools d'Apple, Homebrew, Node.js, CMake, Rust ≥ 1.85.

```bash
npm install

# Interface seule dans le navigateur, backend simulé, rechargement instantané
npm run dev                  # http://localhost:1420

# Vraie application, rechargement à chaud de l'interface (Rust recompilé à chaque changement)
npm run app:dev

# Version finale installable (la clé de signature est exportée par mac-env.sh)
source scripts/mac-env.sh && npm run app:build -- --bundles app
```

- Le plus simple pour Ulysse : double-cliquer **`Compiler Lumen.command`** (compile, installe dans `/Applications`, crée le DMG dans `Distribution/`, ouvre Lumen).
- Publier une mise à jour : double-cliquer **`Publier une version.command`**.
- La première compilation Rust prend une dizaine de minutes (llama.cpp et whisper.cpp).

### Vérifications avant de terminer une tâche

```bash
npx tsc -b                         # types TypeScript (doit être silencieux)
cd src-tauri && cargo check        # compilation Rust
cd src-tauri && cargo test --lib   # tests unitaires
```

- Test réel de l'IA : `LUMEN_TEST_MODEL=/chemin/Qwen3.5-2B-Q4_K_M.gguf cargo test --release --lib live -- --ignored --nocapture`.
- Test réel de LingQ (base jetable, rien n'est écrit dans Lumen) : `LUMEN_LINGQ_KEY=… cargo test --lib lingq_live -- --ignored --nocapture`.
- En mode `npm run dev`, `window.__lumen = { api, useApp }` est exposé pour piloter l'état depuis la console.

## Règles de code

### Langue et ton

- **Tout texte visible est en français**, avec la typographie française : espace avant `:` `?` `!`, guillemets « », points de suspension `…`.
- **Pas de tiret cadratin** (U+2014) dans les textes de l'interface ni dans les messages.
- Commentaires de code en français, courts, qui expliquent le pourquoi.
- Messages d'erreur compréhensibles par un non-développeur.

### Ajouter une commande native (5 endroits, toujours tous)

1. `src-tauri/src/commands.rs` : la fonction `#[tauri::command]`, retour `R<T>` (= `Result<T, String>`), erreurs via `.map_err(err)`.
2. `src-tauri/src/lib.rs` : l'ajouter à `generate_handler![…]`.
3. `src/lib/types.ts` : les types échangés (champs en `snake_case`, comme Serde les sérialise).
4. `src/lib/api.ts` : la méthode dans l'interface `Api` et son `invoke("nom_snake", { argsEnCamelCase })` (Tauri convertit les paramètres Rust `snake_case` en `camelCase` côté JS).
5. `src/lib/mock.ts` : une implémentation simulée crédible, pour que `npm run dev` continue de fonctionner.

Si la commande demande une nouvelle permission (plugin, fenêtre), l'ajouter à `capabilities/default.json`.

### Rust

- Base de données : `state.db.lock()` (parking_lot), jamais de verrou tenu pendant un appel IA ou réseau.
- **Migrations** : uniquement additives, dans `db.rs`, en testant l'existence de la colonne via `pragma_table_info` (exemple : `video_path`). Ne jamais casser une base existante : les utilisateurs mettent à jour sans réinstaller.
- **Offsets de texte en UTF-16** pour correspondre aux index des chaînes JavaScript.
- **Normalisation identique** entre `text.rs` et `tokenize.ts` (minuscules, suppression de U+0301 et U+0300, apostrophes unifiées, élisions it/fr/pt/ca). Toute modification d'un côté se reporte de l'autre.
- Ne jamais lier whisper.cpp dans l'application principale : la transcription reste dans `lumen-whisper`.
- Travail long (IA, transcription, téléchargement) : asynchrone, interruptible, avec progression envoyée par `Channel`.
- Changer le format d'une réponse IA mise en cache impose de changer la clé de cache (`w3` → `w4`).

### Interface et design

- **Animations avec `motion`** (ressorts doux, entrées décalées). Respecter `prefers-reduced-motion` (règle déjà présente dans `app.css`).
- **Couleurs uniquement via les variables CSS** du thème (clair et sombre). Tester les deux thèmes.
- Palette chaude et lumineuse : or, ambre, crème le jour ; nuit chaude et brune le soir. Pas de bleu dominant sur l'accueil.
- Effets de lumière subtils et signifiants (halo de sélection, lanterne, reflets) plutôt que décoratifs ou bruyants.
- Le lecteur doit rester confortable : pas d'animation qui distrait pendant la lecture.
- État global dans `store.ts` ; état local dans le composant. Réglages persistés par `setSetting(clé, valeur)` (chaînes), valeurs par défaut dans `DEFAULTS`.
- Les composants qui dépendent d'une leçon (`Player`, `VideoStage`) sont rendus avec `key={lesson.id}` pour repartir à zéro d'une leçon à l'autre.

### Versions, signature, publication

- **Ne jamais modifier le numéro de version à la main** : `node scripts/release.mjs bump X.Y.Z` met à jour `package.json`, `tauri.conf.json` et `Cargo.toml` ensemble (le script de publication le fait).
- La **clé privée** des mises à jour est `~/.tauri/lumen-updater.key` : ne jamais la copier dans le projet, ne jamais la committer, ne jamais la régénérer (les Lumen installés refuseraient toutes les mises à jour suivantes).
- `plugins.updater.pubkey` dans `tauri.conf.json` est inscrite par `mac-env.sh` : ne pas la remplacer.
- Mises à jour publiées sur `LivingTwice/lumen-releases` (public), code source sur `LivingTwice/lumen` (privé). URL lue par l'app : `https://github.com/LivingTwice/lumen-releases/releases/latest/download/latest.json`, plateforme `darwin-aarch64`.
- Signature ad hoc (`signingIdentity: "-"`), pas encore de Developer ID : sur un autre Mac, premier lancement via Réglages Système › Confidentialité et sécurité › Ouvrir quand même.

### Prudence

- Ne pas supprimer de données utilisateur (leçons, mots, médias) sans confirmation explicite via `confirmAsk`.
- Ne pas ajouter de dépendance lourde sans raison : l'application doit rester légère (les modèles IA se téléchargent à part).
- `public/` contient des fichiers de test (ignorés par git) : ne pas y mettre de ressources nécessaires à l'app.

## Pistes suivantes

- Transcription par Apple SpeechAnalyzer (macOS 26) en complément de Whisper.
- Voix neuronales locales (Kokoro).
- Japonais et chinois (segmentation spécifique).
- Version Windows (même code, WebView2), puis version web.
- Révision douce en contexte des mots en apprentissage.
- Signature Developer ID et notarisation Apple.
