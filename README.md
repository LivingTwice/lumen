# Lumen

Application Mac pour apprendre les langues en lisant et en écoutant, dans l'esprit de LingQ, avec une IA qui tourne entièrement sur l'ordinateur.

## Ce que fait la version 0.1

- **Lecteur** : texte découpé en pages, mots colorés selon leur statut (nouveau, 1, 2, 3, connu, ignoré), sélection d'un mot ou d'une expression (glisser sur plusieurs mots), « Terminer la page » qui fait passer les mots non consultés en connus.
- **Traduction en contexte** : Qwen3.5 (0.8B, 2B ou 4B) exécuté par llama.cpp avec Metal. Le dictionnaire hors ligne (Wiktionnaire en français) donne la forme de base et les sens, l'IA donne le sens précis dans la phrase et traduit la phrase entière.
- **Lecture et écoute synchronisées** : voix du système avec surlignage mot à mot (la « lanterne »), ou audio importé synchronisé grâce aux horodatages de Whisper.
- **Import** : texte collé, page web (extraction de l'article), EPUB (un chapitre par leçon), PDF, TXT, Markdown, sous-titres SRT et VTT, audio et vidéo (MP3, M4A, WAV, FLAC, OGG, MP4, MOV…), vidéos YouTube et autres sites (yt-dlp, installé et mis à jour automatiquement). Glisser-déposer n'importe où dans la fenêtre.
- **Vidéo** : image en haute définition synchronisée avec la transcription, sous-titres interactifs (touchez un mot pour le traduire), sous-titres traduits en option, mode cinéma plein écran.
- **Mises à jour automatiques** : vérification au démarrage et toutes les six heures, installation en un clic.
- **Simplifier** : l'IA réécrit une leçon au niveau A1, A2, B1 ou B2 et en fait une nouvelle leçon.
- **Vocabulaire** : recherche, filtres, changement de statut, export CSV (compatible Anki).
- **Progrès** : mots connus, paliers, mots lus par jour, temps d'écoute.
- **Langues** : anglais, italien, allemand, portugais, russe, espagnol. Interface en français.

## Compiler, installer, publier

Tout passe par deux fichiers à double-cliquer dans ce dossier :

- **Compiler Lumen.command** : vérifie et installe les outils (outils d'Apple, Homebrew, Node, CMake, Rust), compile Lumen, crée le DMG dans `Distribution/` et installe l'application dans `/Applications`.
- **Publier une version.command** : demande le nouveau numéro de version et une phrase de notes, compile, signe le paquet de mise à jour, sauvegarde le code dans le dépôt privé `LivingTwice/lumen` et publie la version sur `LivingTwice/lumen-releases`. Les Lumen installés la détectent et proposent de l'installer.

La première publication demande de se connecter une fois à GitHub (un code s'affiche, le navigateur s'ouvre).

### Clé de signature des mises à jour

Créée au premier lancement des scripts dans `~/.tauri/lumen-updater.key`. **Sauvegardez-la** : sans elle, impossible de publier une mise à jour que les Lumen déjà installés accepteront.

### Installer sur un autre Mac

Ouvrir le DMG de `Distribution/` (ou de la page des versions GitHub) et glisser Lumen dans Applications. L'application est signée localement, pas encore par un certificat Apple « Developer ID » : au tout premier lancement sur un autre Mac, macOS peut refuser de l'ouvrir. Il suffit alors d'aller dans **Réglages Système › Confidentialité et sécurité** et de cliquer sur **Ouvrir quand même**. Les mises à jour suivantes s'installent sans avertissement.

Pour supprimer cet avertissement : adhérer à l'Apple Developer Program, puis renseigner `signingIdentity` et les variables de notarisation (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`) : Tauri signe et notarise alors automatiquement.

Version actuelle : Mac à puce Apple (M1 et suivants).

### En ligne de commande

```bash
npm install
npm run app:dev                          # développement avec rechargement à chaud
npm run app:build -- --bundles app,dmg   # compilation (clé de mise à jour requise)
```

## Architecture

```
src/                     Interface (React, TypeScript, Motion)
  lib/                   API, état global, découpage du texte, voix, imports
  views/                 Bibliothèque, Lecteur, Vocabulaire, Progrès, Réglages, Accueil
  styles/                Système visuel (clair et sombre)
src-tauri/               Application native (Rust, Tauri 2)
  src/ai.rs              Moteur Qwen3.5 (llama.cpp, Metal), requêtes interrompables
  src/dict.rs            Dictionnaires hors ligne (SQLite compressé, décompressé au premier usage)
  src/db.rs              Leçons, mots, activité, réglages, cache des traductions
  src/text.rs            Découpage en mots (UAX 29), élisions, clés normalisées
  src/media.rs           Import audio et vidéo, YouTube, construction des horodatages
  src/tools.rs           yt-dlp et moteur JavaScript gérés automatiquement
  src/models.rs          Catalogue et téléchargement des modèles (avec reprise)
  lumen-whisper/         Transcription (whisper.cpp, Metal) dans un processus séparé
  resources/dicts/       Dictionnaires (Wiktionnaire via kaikki.org, CC BY-SA 4.0)
tools/build_dicts.py     Reconstruction des dictionnaires depuis kaikki.org
```

whisper.cpp et llama.cpp embarquent chacun leur propre copie de ggml : la transcription tourne donc dans un petit exécutable séparé (`lumen-whisper`), lancé par l'application, ce qui évite tout conflit et isole les plantages éventuels.

Les données de l'utilisateur sont dans `~/Library/Application Support/app.lumen.reader/` (base `lumen.db`, modèles, médias, dictionnaires décompressés).

## Aperçu dans un navigateur

`npm run dev` puis http://localhost:1420 : l'interface tourne avec un backend simulé (`src/lib/mock.ts`), utile pour travailler le design sans compiler l'application.

## Tests

```bash
cd src-tauri
cargo test --lib                                   # tests unitaires
LUMEN_TEST_MODEL=/chemin/Qwen3.5-2B-Q4_K_M.gguf \
  cargo test --release --lib live -- --ignored --nocapture   # test réel de traduction
```

## Pistes suivantes

- Transcription par Apple SpeechAnalyzer (macOS 26) en complément de Whisper.
- Voix neuronales locales (Kokoro) en plus des voix du système.
- Japonais et chinois (segmentation spécifique).
- Version Windows (même code, WebView2), puis version web.
- Révision douce en contexte des mots en apprentissage.

## Licences des composants

Qwen3.5 : Apache 2.0. llama.cpp, whisper.cpp, Whisper : MIT. Wiktionnaire : CC BY-SA 4.0. Polices Literata, Newsreader, Geist : SIL OFL.
