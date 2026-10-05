# Lumen

**Apprendre les langues en lisant et en écoutant, avec une IA qui tourne sur votre ordinateur.**
Application Mac (puce Apple) et Windows (10 et 11), dans l'esprit de LingQ.

*Learn languages by reading and listening, with an AI that runs on your own computer. [English below](#english).*

**[Télécharger Lumen](https://github.com/LivingTwice/lumen-releases/releases/latest)** : DMG pour Mac, installateur pour PC. Les mises à jour s'installent ensuite d'elles-mêmes.

## Ce que fait Lumen

- **Lire** : les mots se colorent selon ce que vous savez déjà (nouveau, en apprentissage, connu). Touchez un mot : le dictionnaire hors ligne donne sa forme de base et ses sens, l'IA donne le sens précis dans la phrase. Expressions, « Terminer la page », pages qui tiennent dans l'écran, polices et couleurs de page au choix.
- **Écouter** : la « lanterne » suit chaque mot de l'audio ou de la vidéo, au mot près (minutage de Whisper). Voix naturelle pour prononcer un mot ou créer l'audio d'une leçon de texte. Sous-titres interactifs en plein écran.
- **Importer tout ce qu'on aime** : texte, page web, EPUB, PDF, sous-titres, audio et vidéo de l'ordinateur, liens YouTube, podcasts, Spotify, TikTok… transcrits sur l'ordinateur (Whisper et Qwen3-ASR). Import depuis LingQ.
- **Découvrir** : vidéos, podcasts, actualités et chansons récentes dans 31 langues, rangés par niveau (A1 à C1) selon les mots que vous connaissez. Recherche sur YouTube, Dailymotion, podcasts, chansons (paroles minutées) et Wikipédia, avec aperçu avant d'en faire une leçon.
- **Chat** : un professeur de langues qui connaît votre leçon, sur l'ordinateur (Qwen3.5), avec réflexion si on la demande.
- **Progrès** : temps d'apprentissage, mots lus, série de jours, objectif quotidien, calendrier.
- **Sauvegarde** dans iCloud Drive, Dropbox, Google Drive, OneDrive ou un disque, sans serveur ni compte.
- **31 langues étudiées**, chacune avec un dictionnaire hors ligne ; interface en français et en anglais.

## Vos données restent chez vous

Traduction, chat, transcription, voix et dictionnaires tournent **sur l'ordinateur**. Lumen n'a ni compte, ni serveur, ni statistiques d'usage. Ne partent en ligne que ce que vous demandez :

- les contenus que vous cherchez ou importez (YouTube, podcasts, paroles sur LRCLIB, Wikipédia…) ;
- si vous les activez, avec votre propre clé : l'**IA en ligne** (DeepSeek, Gemini, Mistral, OpenAI, Claude, OpenRouter ou votre serveur), qui reçoit le mot touché et sa phrase, ou vos questions au chat ; les **podcasts sur mesure** (Gemini), qui reçoivent le sujet et les mots à faire revenir. Votre profil n'est jamais envoyé ;
- la sauvegarde, dans le nuage que vous choisissez.

Le code est public justement pour que chacun puisse le vérifier.

## Installer

- **Mac** (M1 et suivants, macOS 13 ou plus récent) : ouvrir le DMG et glisser Lumen dans Applications. L'application n'est pas encore signée par un certificat Apple « Developer ID » : au tout premier lancement, aller dans **Réglages Système › Confidentialité et sécurité** et cliquer sur **Ouvrir quand même**.
- **PC** (Windows 10 ou 11, 64 bits) : lancer `Lumen_<version>_x64-setup.exe`. Pas de droits d'administrateur nécessaires. L'installateur n'est pas encore signé : si Windows affiche « Windows a protégé votre ordinateur », cliquer sur **Informations complémentaires**, puis **Exécuter quand même**. L'IA locale calcule sur le processeur (AVX2 requis : Intel depuis 2013, AMD depuis 2015).

Les modèles d'IA (de 0,5 à 3 Go selon le profil choisi) se téléchargent au premier lancement, puis tout marche hors ligne.

## Compiler Lumen

Prérequis : macOS sur puce Apple, outils en ligne de commande d'Apple, Homebrew, Node.js, CMake, Rust 1.85 ou plus récent. Le plus simple : double-cliquer sur **`Compiler Lumen.command`**, qui installe ce qui manque, compile, installe Lumen dans `/Applications` et crée le DMG dans `Distribution/`.

```bash
npm install
npm run dev                              # interface seule dans le navigateur, backend simulé (http://localhost:1420)
npm run app:dev                          # vraie application, rechargement à chaud de l'interface
source scripts/mac-env.sh && npm run app:build -- --bundles app,dmg   # version installable
node scripts/build-windows.mjs           # version Windows, compilée sur le Mac (cargo-xwin, NSIS)
```

La première compilation prend une dizaine de minutes (llama.cpp et whisper.cpp).

Vérifications :

```bash
npx tsc -b
cd src-tauri && cargo check && cargo test --lib
node scripts/build-windows.mjs --check
```

**Mises à jour signées** : les paquets de mise à jour sont signés par une clé privée qui ne quitte jamais l'ordinateur de publication (`~/.tauri/lumen-updater.key`, absente de ce dépôt). Une copie compilée par quelqu'un d'autre crée sa propre clé : elle ne peut pas publier de mise à jour pour les Lumen installés.

## Architecture

```
src/                     Interface : React 19, TypeScript, motion, zustand
  lib/api.ts             Seul point d'entrée vers le natif (Tauri, ou backend simulé lib/mock.ts)
  views/, components/    Vues (bibliothèque, lecteur, Découvrir, chat, progrès, réglages…)
src-tauri/               Application native : Rust, Tauri 2
  src/ai.rs              Traduction et chat : Qwen3.5 par llama.cpp (Metal sur Mac)
  src/asr.rs             Texte des transcriptions : Qwen3-ASR par llama.cpp
  lumen-whisper/         Minutage mot à mot : whisper.cpp, dans un processus séparé
  src/dict.rs            Dictionnaires hors ligne (SQLite)
  src/voice.rs           Voix naturelle : Supertonic 3 par sherpa-onnx
  src/discover.rs        Découvrir : sources, flux, classements, niveaux
  src/backup.rs          Sauvegarde dans un dossier de nuage
tools/build_dicts.py     Construction des dictionnaires
```

whisper.cpp et llama.cpp embarquent chacun leur copie de ggml : Whisper tourne donc dans un petit exécutable à part (`lumen-whisper`), ce qui évite les conflits. Le détail de chaque fonction, les règles de code et les tests réels sont dans [`CLAUDE.md`](CLAUDE.md).

## Licence

Copyright © 2026 Ulysse Rives.

Lumen est un logiciel libre : vous pouvez le redistribuer et le modifier selon les termes de la **GNU General Public License**, version 3 ou (à votre choix) toute version ultérieure. Voir [`LICENSE`](LICENSE).

Composants et données d'autres auteurs, sous leur propre licence :

- Dictionnaires : Wiktionnaire et Wiktionary via kaikki.org, corpus Universal Dependencies, JMdict et KANJIDIC2 (Electronic Dictionary Research and Development Group) : CC BY-SA 4.0. Les fichiers de `src-tauri/resources/dicts/` restent sous cette licence.
- Modèles téléchargés à la demande : Qwen3.5 et Qwen3-ASR (Apache 2.0), Whisper (MIT), Supertonic 3 (Supertone).
- Moteurs : llama.cpp et whisper.cpp (MIT), sherpa-onnx (Apache 2.0), yt-dlp (Unlicense), Tauri (MIT ou Apache 2.0), React (MIT), pdf.js et Readability (Apache 2.0).
- Polices : Literata, Newsreader, Geist (SIL Open Font License).

---

## English

**Lumen** is a language-learning app for Mac (Apple silicon) and Windows 10/11, in the spirit of LingQ: you read and listen to content you enjoy, words are colored by what you already know, and tapping a word gives its dictionary entry and its meaning in context. Translation, chat (Qwen3.5 via llama.cpp), transcription (Whisper and Qwen3-ASR), natural voice (Supertonic 3) and dictionaries all run **on your computer**. No account, no server, no analytics. An online AI (with your own API key) and custom podcasts (Gemini) are optional and off by default.

- **Download**: [latest release](https://github.com/LivingTwice/lumen-releases/releases/latest).
- **Features**: 31 study languages with offline dictionaries; word-by-word synced audio and video (the "lantern"); import from text, web pages, EPUB, PDF, subtitles, local audio/video, YouTube, podcasts, Spotify and more; Discover (videos, podcasts, news and songs by CEFR level); a local language-tutor chat; progress and streaks; backup to iCloud Drive, Dropbox, Google Drive or OneDrive. The interface is in French and English.
- **Build**: `npm install`, then `npm run app:dev` (dev) or `source scripts/mac-env.sh && npm run app:build -- --bundles app,dmg`. Windows builds are cross-compiled from the Mac with `node scripts/build-windows.mjs`. The browser preview (`npm run dev`) uses a mock backend. Code comments and developer notes ([`CLAUDE.md`](CLAUDE.md)) are in French.
- **Licence**: GNU GPL v3 or later ([`LICENSE`](LICENSE)). Third-party data and components keep their own licences (see above); the bundled dictionaries are CC BY-SA 4.0.
