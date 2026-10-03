# CLAUDE.md : Lumen

Lumen est une application Mac pour apprendre les langues en lisant et en écoutant, dans l'esprit de LingQ. Toute l'intelligence tourne **en local** : traduction en contexte et chat par Qwen3.5 (llama.cpp, Metal), transcription par Qwen3-ASR (llama.cpp) et Whisper (whisper.cpp, Metal), dictionnaires hors ligne. Mac d'abord (Apple Silicon), Windows et web ensuite.

- **Interface** : en français et en anglais, au choix (accueil et Réglages, réglage `ui_lang`). La langue de l'interface est aussi celle des traductions de l'IA, du chat et des dictionnaires.
- **Langues étudiées** : les 31 langues de la voix naturelle Supertonic 3 (`text::LANGS` et `LANGS` dans `langs.ts`). Six d'entre elles ont en plus un dictionnaire hors ligne dans la langue de l'interface (`coreLangs()`) : `en it de pt ru es` en français, `it de pt ru es fr` en anglais ; les autres ont la traduction par l'IA et la voix naturelle. L'arabe s'affiche de droite à gauche (`rtl` dans `LangInfo`). Le japonais est découpé caractère par caractère (UAX 29, sans dictionnaire de segmentation).
- **Priorité absolue** : la beauté du design et des animations. Le nom « Lumen » guide l'identité visuelle : lumière, aube, halo, lanterne.
- **Propriétaire** : Ulysse (compte GitHub `LivingTwice`). Il n'est pas développeur de métier : expliquer simplement, en français, sans jargon inutile.
- **Copie de référence du code** : `~/Documents/Lumen` sur le Mac d'Ulysse. C'est là qu'on modifie, compile et publie.

## Stack

- **Coquille** : Tauri 2.12 (Rust 2021). Fenêtre `titleBarStyle: Overlay`, transparente, vibrance macOS « sidebar », `macOSPrivateApi`, `acceptFirstMouse`, ouverte en grand au lancement (`maximized` : toute la place de l'écran sans plein écran ; 1380 × 880 au centre si on la réduit). Pas de `center` dans `tauri.conf.json` : Tauri replacerait la fenêtre *après* l'agrandissement, décalée vers le bas à droite ; sans position imposée, macOS la centre lui-même.
- **Interface** : React 19, TypeScript 5.9, Vite 8, `motion` 14 (animations), `zustand` 5 (état global).
- **Polices** : Geist (interface), Newsreader (titres), Literata (texte de lecture, gère le cyrillique). Via `@fontsource-variable`.
- **IA locale** : crate `llama-cpp-2` 0.1.158, feature `metal`, modèles GGUF Qwen3.5 (0.8B, 2B, 4B) téléchargés depuis Hugging Face (unsloth).
- **Transcription** : `whisper-rs` 0.16 dans un exécutable séparé `lumen-whisper` (sidecar) pour le minutage des mots ; Qwen3-ASR 1.7B (GGUF Q8_0 de ggml-org, modèle + partie audio `mmproj`, 2,5 Go, facultatif) pour le texte, par l'entrée audio de llama.cpp (feature `mtmd` de `llama-cpp-2`, dans l'application principale).
- **Voix naturelle** (prononciation des mots, audio des leçons de texte) : modèle Supertonic 3 int8 (31 langues, 10 voix : 0 à 4 féminines F1-F5, 5 à 9 masculines M1-M5, ordre de `voice.bin`) exécuté par l'outil officiel `sherpa-onnx-offline-tts` 1.13.8. Moteur et modèle téléchargés à la demande, rien n'est ajouté au paquet de l'app.
- **Base de données** : SQLite via `rusqlite` (bundled), mode WAL.
- **Imports côté interface** : `@mozilla/readability` (pages web), `jszip` (EPUB), `pdfjs-dist` (PDF).
- **YouTube** : `yt-dlp` (système ou géré par Lumen) avec un moteur JavaScript (deno, node 22+ ou QuickJS géré).
- **Mises à jour** : `tauri-plugin-updater` + `tauri-plugin-process`, paquets signés minisign, publiés sur GitHub.

## Fonctionnalités en place

### Lecture

- Texte découpé en **pages** (~230 mots), mots colorés selon leur **statut** : nouveau (absent de la table `terms`), 1, 2, 3 (en apprentissage), 4 (connu), 5 (ignoré).
- Sélection d'un mot au clic ; de **plusieurs mots** en glissant, par Maj + clic ou Maj + ← → (bande de surlignage continue, espaces compris). Jusqu'à 5 mots : sens en contexte (`aiWord`) ; au-delà : traduction globale du passage (`aiSentence`). Une sélection n'est jamais enregistrée d'office.
- **Expressions** (comme les LingQ de phrase) : bouton « Créer l'expression » (8 mots au plus, `EXPR_MAX_WORDS` dans `WordPanel.tsx`). L'expression devient un terme (clé avec espaces), repérée dans toutes les leçons par un trait continu à la couleur de son niveau ; toucher un de ses mots affiche un rappel qui l'ouvre.
- **Panneau du mot** (`WordPanel`) : dictionnaire hors ligne (forme de base, sens), puis sens précis en contexte par l'IA, en streaming. Traduction de la phrase entière.
- **« Terminer la page »** : les mots nouveaux non consultés passent en connus.
- **Reprise exacte**, même après fermeture de l'app : page (`page`), mot atteint (`anchor`, mot lu à voix haute ou mot sur la ligne de lecture à 30 % de la hauteur) et seconde atteinte dans l'audio ou la vidéo (`position`). Écriture au plus une fois par seconde, puis à chaque pause, saut ou sortie. À l'ouverture : retour au mot avec un bref halo, ou lanterne sur le mot et média calé à la seconde. Leçon terminée ou média écouté jusqu'au bout : on repart du début. La dernière leçon ouverte (réglage `last_lesson`) reste « en cours » d'un lancement à l'autre.
- **Barre latérale repliable** dans une leçon (bouton en haut à gauche) : elle glisse et s'efface, la colonne de lecture s'élargit, la lanterne se recale. Le choix est mémorisé (réglage `reader_sidebar`) ; hors leçon, la barre est toujours là.
- **Simplifier** : l'IA réécrit la leçon au niveau A1, A2, B1 ou B2 et crée une nouvelle leçon.
- **Raccourcis** : flèches (mot suivant ou précédent), Maj + flèches (étendre la sélection), `1` `2` `3` (statut), `K` ou `4` (connu), `X` (ignorer), `0` (remettre à nouveau), Espace (lecture audio), Entrée (terminer la page), `C` (chat sur la leçon), Échap (fermer, quitter le plein écran), PageUp et PageDown.

### Écoute

- **Voix du système** (Web Speech API dans WKWebView) avec surlignage mot à mot : la **lanterne**.
- **Prononciation d'un mot ou d'une expression** (haut-parleur du panneau du mot, vocabulaire) : voix naturelle Supertonic si elle est installée (Réglages › Voix, 149 Mo, une voix au choix par langue, réglage `tts_voice_<lang>`, sinon `tts_voice`), sinon voix du système. La prononciation se prépare dès qu'un mot est touché (12 mots au plus), une seule à la fois, les préparations dépassées sont abandonnées. L'outil est lancé à la demande puis s'arrête (≈ 0,8 s par nouveau texte) ; le son est débarrassé de ses silences, égalisé, et gardé en cache (`media/voice/`, réponse immédiate ensuite).
- **Audio importé** synchronisé grâce aux horodatages mot à mot de Whisper. Minutage précis dans `lumen-whisper` : alignement DTW de whisper.cpp (l'instant DTW d'un fragment marque sa *fin* : un mot commence à la fin du fragment précédent), correction de l'avance moyenne de 90 ms dans le flot, puis calage sur l'attaque réelle de la voix (seuil relatif à la voix, vraie pause ≥ 50 ms, vraie attaque ≥ 30 ms). Mesuré sur des références : écart moyen 30 à 45 ms, contre 0,4 à 0,55 s avec les horodatages classiques.
- **Lanterne** : part 60 ms avant le mot (`LANTERN_LEAD`, durée de son déplacement), saute directement d'une ligne à l'autre, boîte centrée sur la hauteur des capitales (même marge au-dessus et sous la ligne de base, mesurée sur la police).
- **Recaler la lanterne** (bouton du lecteur, leçons audio et vidéo dont `timing_v` < 2 : ancien Whisper, LingQ) : l'audio est réécouté et les mots entendus sont alignés sur le texte existant (`media::align_timings` : ancres de trois mots uniques, puis plus longue sous-suite commune, mots manquants interpolés). Le texte ne change pas ; refus si moins de 30 % des mots sont retrouvés.
- **Créer l'audio** (bouton du lecteur, leçons de texte) : `voice::lesson_audio` découpe la leçon en phrases (`text::sentences`, au plus 40 mots par morceau), les fait prononcer trois à la fois, les assemble avec des pauses (0,32 s entre phrases, 0,75 s entre paragraphes), égalise le volume et compresse en AAC avec `afconvert` (`media/<horodatage>.voice.m4a`). Les mots sont répartis dans chaque phrase (`lingq::spread_words`, écart moyen de 0,12 s), puis, si Whisper est installé, recalés au mot près (`timing_v` = 2). La leçon devient une leçon audio ; un bouton permet de recréer l'audio avec une autre voix (l'ancien fichier est supprimé). Annulation : `model_cancel("voice:<id>")`.
- **Vidéo** : l'audio est le maître, la vidéo muette le suit (correction de dérive par ajustement de vitesse, ou saut au-delà de 0,35 s). Au-dessus du texte, l'image seule (rien n'est écrit dessus), taille ajustable. **Plein écran** (vrai plein écran macOS, permission `core:window:allow-set-fullscreen`) : l'image aux proportions réelles de la vidéo, et dessous, dans le noir, les sous-titres interactifs : quelques mots à la fois (5 au plus, coupés de préférence à une virgule), la lanterne qui glisse de mot en mot, toucher un mot le traduit, traduction de la phrase en option. Quitter par Échap, le bouton, ou le bouton vert du Mac.

### Playlists

- **Playlists** (vue `playlists`, entrée de la barre latérale) : des leçons d'une langue dans l'ordre choisi, comme sur LingQ. Tables `playlists` (`current_id` : leçon où l'écoute en est) et `playlist_items` (position ; une leçon supprimée en sort d'elle-même, `ON DELETE CASCADE`). Commandes `playlists_list`, `playlist_create`, `playlist_update` (nom, leçons dans l'ordre, `current`, 0 = depuis le début), `playlist_delete`.
- **Organiser** : créer (puis la feuille « Ajouter des leçons » s'ouvre), renommer d'un double-clic sur le titre, glisser la poignée d'une ligne pour changer l'ordre (`Reorder` de motion, enregistré au lâcher), retirer une leçon, supprimer la playlist (les leçons restent). Depuis la bibliothèque : menu d'une leçon › « Ajouter à une playlist… » (`AddToPlaylist`).
- **Écouter** : « Écouter » ou « Reprendre » ouvre la leçon en cours de la playlist et lance la lecture (`openLesson(id, { playlist, autoplay })`). Dans le lecteur, un bandeau au centre de la barre du haut montre la playlist, la place de la leçon, précédente et suivante ; le retour mène à la playlist. À la fin de l'audio (ou de la voix du système sur la dernière page), la carte « Ensuite » s'affiche 8 s avec un anneau de lumière, puis la leçon suivante s'ouvre et joue. La playlist suivie est retenue d'un lancement à l'autre (réglage `last_playlist`). La fenêtre de Lumen autorise la lecture sans clic (`autoplay` de wry) ; un navigateur de test peut la refuser, le lecteur reste alors en pause.

### Chat

- **Chat avec l'IA locale** (vue `chat` de la barre latérale ; onglet « Chat » du panneau de droite du lecteur, à côté de « Mot », touche `C`), dans l'esprit de Lynx sur LingQ. Même modèle que la traduction des mots (`llm_model`). Consignes de professeur de langues : `CHAT_SYSTEM` dans `ai.rs`, avec le nombre de mots connus. Réponse en français ; si l'apprenant écrit dans une autre langue (`looks_french`), une consigne explicite lui fait répondre dans la langue étudiée puis corriger les fautes en français.
- **Leçon jointe** : d'un clic (« Joindre une leçon » ; l'onglet du lecteur joint la leçon ouverte et reprend sa dernière conversation) ou par commande dans le champ (`/leçon`, aussi `/sans-leçon`, `/nouveau`, `/réflexion`). Leçon entière si elle tient dans 24 Ko, sinon un extrait autour de la page lue (`lesson_excerpt`, position `reading` envoyée par le lecteur). Depuis le panneau du mot : « Demander au chat » (la question reprend la traduction en contexte déjà trouvée). Mots cités entre guillemets : indices du dictionnaire (`dict_hint`, jamais un renvoi douteux vers un homonyme).
- **Réflexion** (bouton « Réflexion », réglage `chat_think`) : le mode « thinking » de Qwen3.5, affiché en direct puis replié (« A réfléchi pendant 12 s »). **Effort** (`chat_effort` : Rapide, Équilibré, Approfondi) : Qwen n'a pas de réglage d'effort, Lumen borne la réflexion à 512, 1 536 ou 4 096 jetons (`think_budget`) puis la clôt comme le préconise Qwen (`THINK_STOP`).
- Conversations enregistrées (tables `chats` et `chat_messages`, réflexion comprise), par langue, renommables (double-clic sur le titre) et supprimables ; une leçon supprimée laisse ses conversations, sans leçon. Réponse au fil de l'eau ; bouton « Arrêter » (`model_cancel("chat:<id>")`) : ce qui est écrit est gardé. Rendu : `components/Markdown.tsx` (listes, tableaux de conjugaison, exemples en italique dans la police de lecture), espaces insécables françaises (`frenchSpaces`).

### Import

- Texte collé, EPUB (un chapitre par leçon), PDF, TXT, Markdown, sous-titres SRT et VTT.
- **Lien universel** (onglet « Lien », `link.rs`, commandes `link_probe` puis `import_link`) : Lumen regarde ce qu'il y a derrière n'importe quelle adresse. Sites vidéo et audio connus (`VIDEO_SITES`, `AUDIO_SITES`) : yt-dlp d'abord (`media::yt_info`, `-J --flat-playlist` : vidéo, playlist, chaîne dont on garde l'onglet des vidéos). Autres pages : les médias qu'elles déclarent (Open Graph, balises audio et vidéo, JSON-LD, lecteurs YouTube, Vimeo, Dailymotion, SoundCloud intégrés), à défaut les adresses de fichiers écrites dans leur code (jamais une radio en direct : `live_host`, en-têtes `icy-`) ; page d'émission (`og:type` autre qu'`article`) : son flux RSS, mieux renseigné ; en dernier recours yt-dlp. Flux RSS et Atom (`feed_info`), Apple Podcasts (annuaire `itunes.apple.com/lookup`), fichier direct (téléchargé tel quel, `link::download`, sans yt-dlp). **Spotify** (fichiers protégés contre la copie) : données publiques de son lecteur intégré (`open.spotify.com/embed/…`, `__NEXT_DATA__`) ; épisode et podcast retrouvés dans le flux public du podcast par l'annuaire d'Apple (`find_episode` : titre, durée, date) ; morceau, album, playlist : le même morceau sur YouTube (`ytsearch1:`). Exclusivité Spotify : refus expliqué.
- **Choix de ce qu'on importe** (`LinkFound` dans `ImportSheet.tsx`) : le texte de l'article (Readability), le son ou la vidéo transcrits, ou **le son avec le texte de la page** (`reference` de `media::lesson_transcript` : le texte est gardé tel quel et la lanterne calée dessus si la moitié de ses mots s'entendent, sinon transcription). Podcast, playlist, album : liste à cocher, importée élément par élément (un échec n'arrête pas les suivants). Couverture téléchargée et réduite par `sips` (1 280 px, JPEG), sauf YouTube qui a ses miniatures. Un lien collé dans l'onglet « Texte » propose de l'ouvrir comme lien.
- Audio et vidéo locaux (MP3, M4A, WAV, FLAC, OGG, MP4, MOV, MKV…) et en ligne (lien universel), transcrits par `media::lesson_transcript` : Whisper repère chaque mot dans le temps (et enregistre le son décodé, `lumen-whisper --pcm`) ; si **Qwen3-ASR** est installé (catégorie `asrtext`) et connaît la langue (23 langues de Lumen, `asr::language_name` ; sauf et lv lt sk sl hr bg uk), il écrit le texte et le minutage de Whisper y est recalé (`align_timings`, texte écarté sous 50 % de mots retrouvés ; en cas d'échec, Whisper seul). Mesuré sur des leçons LingQ italiennes : récits lus, 0,3 à 2,6 % de mots faux et aucune phrase sautée, contre 4,5 à 9,7 % et 47 mots sautés pour Whisper ; conversation libre de 22 min, erreurs comparables, sans les boucles de Whisper. Durée : environ le double de Whisper seul (37 s pour 5 min sur M2 Pro). Qwen3-ASR découpe le son à ±30 s dans les silences (`asr::split_on_silence`), impose la langue (`language Italian<asr_text>`), coupe les boucles (`repeated_tail`), et chaque morceau passe sur le GPU à son tour (`Engine::exclusive`) : le chat garde la main entre deux.
- **Texte aéré comme sur LingQ** (`media::airy`, à chaque transcription) : une phrase par paragraphe ; une phrase d'au plus deux mots rejoint la suivante ; une phrase de plus de 55 mots (parole spontanée peu ponctuée) est coupée à sa plus longue pause, à défaut après une virgule. Les horodatages suivent (positions UTF-16).
- **YouTube** et autres sites : audio `ba[ext=m4a]` pour la transcription, vidéo `bv*[vcodec^=avc1][height<=1080]` téléchargée en parallèle. Si YouTube exige une vérification, nouvel essai avec `--cookies-from-browser` (réglage `youtube_browser`). Bouton « Télécharger la vidéo » sur les anciennes leçons sans image.
- Glisser-déposer n'importe où dans la fenêtre.
- **Langues étudiées** (réglage `langs`) : choisies à l'accueil et dans Réglages (liste de vos langues, puis « Ajouter une langue » en pastilles). Le menu de la barre latérale ne montre que vos langues. Les écritures de réglages passent par une file (`setSetting`) pour garder l'ordre des clics. L'import LingQ n'ajoute que des langues nouvelles au moment où il démarre (il ne remet jamais une langue retirée pendant qu'il tournait) et ne coche par défaut que vos langues déjà étudiées.
- **LingQ** (Réglages › LingQ, clé API personnelle) : mots connus et ignorés, LingQ (traductions, notes, contexte), leçons de tous les cours (créés, importés et suivis) avec audio. Les horodatages de LingQ sont par phrase : `lingq.rs` les répartit sur les mots. Fusion sans recul de statut (niveaux LingQ 1-3 → 1-3, niveau 4 et ✓ → connu), activité du jour non touchée, leçons dédupliquées par la colonne `ext_id` (`lingq:<id>`). L'import continue en arrière-plan (carte dans la barre latérale).

### Sauvegarde

- **Dans iCloud Drive, sans serveur** (`backup.rs`) : la progression est copiée dans `iCloud Drive/Lumen/` (ou un dossier choisi : Dropbox, Google Drive, clé USB ; réglage `backup_dir`), macOS l'envoie dans le compte iCloud de l'utilisateur. Gratuit dans la limite de son forfait, rien ne passe par un serveur de Lumen.
- **Contenu** : la base entière (copie cohérente par `VACUUM INTO` depuis une seconde connexion, sans la table `tcache`, compressée en gzip : `Progression.lumen`) et un manifeste `Infos.json` (appareil, date, nombres de mots et de leçons, médias cités). Médias communs à tous les Macs dans `Lumen/Médias/` : couvertures toujours, audio (`backup_audio`, oui par défaut), vidéos (`backup_video`, non par défaut : les vidéos en ligne se retéléchargent). Une vidéo qui porte aussi le son compte comme audio. La clé LingQ et `backup_dir` ne quittent jamais le Mac (`LOCAL_ONLY`).
- **Un dossier par Mac et par profil** (« MacBook Pro (clé) ») : clé = `profile_id` (réglage créé à la première sauvegarde, il voyage avec la base) + identifiant du Mac (`device-id`, tiré du numéro du matériel). Une installation neuve n'écrase jamais une sauvegarde : elle a un autre profil, et un profil sans progrès (leçons d'accueil seules, aucun mot) n'est pas sauvegardé.
- **Quand** : toutes les 10 minutes si la base a changé (`total_changes`), au lancement si l'app a été quittée sans sauvegarder, à la fermeture (`RunEvent::Exit`), et sur demande. Historique : une version par jour, 14 jours (`Historique/AAAA-MM-JJ.lumen` + `.json`). Ménage des médias : seuls les fichiers que ce Mac a cessé de citer, et qu'aucun manifeste (de tous les Macs, historique compris) ne cite plus ; un manifeste illisible suspend le ménage. Fichiers posés d'un seul geste (préparés dans le dossier de données puis renommés).
- **État d'envoi** lu auprès de macOS (`NSURLUbiquitousItemIsUploadedKey`, objc2) : « dans iCloud », « envoi en cours », erreur (iCloud plein…). Le dossier iCloud n'est lu qu'une fois la sauvegarde acceptée (macOS demande alors l'accès à iCloud Drive ; texte de la demande dans `src-tauri/Info.plist`).
- **Restaurer** (Réglages › Sauvegarde, ou « J'ai déjà utilisé Lumen : retrouver ma progression » à l'accueil) : la plus récente ou la version d'un jour précédent. Base décompressée et remise à niveau (`db::open`), chemins des médias ramenés vers ce Mac (fichiers rapatriés de `Médias/`, absents mis à `NULL`), réglages propres au Mac conservés (`KEEP_ON_RESTORE`), copie de sécurité `lumen.avant-restauration.db`, puis remplacement de la base ouverte par l'API de sauvegarde de SQLite (`Connection::restore`). L'interface relit tout (`reloadProgress`).
- **Interface** : section Réglages › Sauvegarde (balise lumineuse, audio, vidéos, emplacement, « Sauvegarder maintenant », sauvegardes trouvées) ; carte de la barre latérale qui propose la sauvegarde tant qu'elle n'est pas choisie (`backup_on` vide ; « Plus tard » : une semaine, `backup_snooze`) et signale un échec ; interrupteur à la dernière étape de l'accueil. Événement `backup` émis après chaque sauvegarde.

### Langue de l'interface

- **Français ou anglais** (réglage `ui_lang`) : choisi par un sélecteur en haut à droite de l'accueil, et dans Réglages › Langue de l'interface. Au premier lancement, celle du Mac (français si le Mac est en français, anglais sinon) ; une installation déjà accueillie sans ce réglage reste en français. Le choix est gardé à la restauration d'une sauvegarde (`KEEP_ON_RESTORE`).
- **Elle décide aussi** de la langue des traductions (sens en contexte, phrases, sous-titres traduits), du chat (consignes `CHAT_SYSTEM` ou `CHAT_SYSTEM_EN`) et des dictionnaires. Les traductions anglaises ont leurs propres clés de cache (`w3en`, `s1en`) : celles en français restent valables.
- **Dictionnaires à définitions anglaises** (Wiktionnaire anglais via kaikki.org, `tools/build_dicts.py --en`) pour `it es de pt ru fr` : 9 à 16 Mo chacun, 74 Mo en tout, **téléchargés à la demande** (ouverture d'une leçon, recherche d'un mot, passage à l'anglais) depuis la version `dictionaries-en-1` de `lumen-releases`, puis décompressés dans `dicts/<lang>-en-v1.db`. Ils ne sont pas dans le paquet (l'app ne grossit pas, ni ses mises à jour) ni dans git (`src-tauri/resources/dicts-en/` est ignoré). Pendant le téléchargement, le panneau du mot l'indique (`pending`).
- **Textes que macOS affiche lui-même** (demande d'accès à iCloud Drive) : `src-tauri/locales/{en,fr}.lproj/InfoPlist.strings`, choisis selon la langue du Mac.

### Le reste

- **Bibliothèque** : leçons par langue et collection, pourcentage de mots connus, mots nouveaux, avancement (bande de lumière au bas de la couverture, à la seconde près pour l'audio et la vidéo).
- **Couvertures** (`components/Cover.tsx`) : image choisie par l'utilisateur (petit bouton au survol, réduite à 1 280 px en JPEG puis copiée dans `media/`, colonne `cover_path`), sinon miniature YouTube (`maxresdefault` puis `hqdefault`), sinon œuvre SVG générée et reproductible (aube, halo, aurore ou prisme, graine = identifiant, couleurs = teinte) avec grain photographique.
- **Vocabulaire** : recherche, filtres (tous, en apprentissage, connus, ignorés, expressions), changement de statut, export CSV compatible Anki.
- **Progrès** : mots connus, paliers, mots lus par jour, temps d'écoute (30 jours).
- **Réglages** : langue de l'interface, thème (suit le Mac par défaut), typographie de lecture, voix, modèles IA (téléchargement avec reprise, suppression), vidéos en ligne (navigateur pour les cookies, état des composants), sauvegarde, import LingQ, mises à jour, « Revoir l'accueil ».
- **Petit guide** (`components/Guide.tsx`, `styles/guide.css`) : quatre cartes illustrées et animées pour les néophytes : les couleurs des mots, toucher puis terminer la page, la lanterne, l'IA locale et ses quatre rôles (le traducteur Qwen3.5, l'oreille Whisper, la plume Qwen3-ASR, la voix Supertonic, avec leur état sur ce Mac). Phrase d'exemple « la lumière du matin » dans la langue étudiée (`SAMPLES` : `it es de pt ru fr en`, sinon l'italien). Proposé une seule fois par une carte du panneau du mot (réglage `guide_seen`), puis par le lien « Comment marche Lumen ? » ; aussi dans Réglages › À propos et par « Qu'est-ce qu'un modèle ? » (Réglages › IA locale, ouvre la carte des modèles). État `guide` du store (`openGuide(carte)`, `closeGuide`). Le guide capte le clavier (phase de capture) : Entrée ou les flèches ne touchent jamais la page derrière. Scènes figées si macOS réduit les animations.
- **Accueil** (`Onboarding`) : aube animée (ciel en parallaxe, astre qui se lève à l'horizon, poussières de lumière, révélation lettre par lettre). Clair = aube, sombre = nuit chaude. **Jamais de fond bleu.** Étapes : bienvenue, langues, profil IA (avec « Qu'est-ce qu'un modèle ? » qui se déplie), prêt (avec l'interrupteur de sauvegarde), puis éclosion lumineuse vers la première leçon. Depuis la bienvenue, « retrouver ma progression » : sauvegardes trouvées, restauration, profil IA (les modèles ne voyagent pas), « Bon retour ».
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
                                                            ──► link.rs    (lien universel : pages, podcasts, Spotify)
                                                            ──► lingq.rs   (import LingQ)
                                                            ──► voice.rs   ──► sherpa-onnx-offline-tts (voix)
                                                            ──► backup.rs  ──► iCloud Drive/Lumen (sauvegarde)
```

- **Un seul point d'entrée vers le natif** : `src/lib/api.ts`. L'interface `Api` a deux implémentations : Tauri (`invoke`) et un **backend simulé** (`src/lib/mock.ts`) utilisé dans le navigateur.
- **Streaming** (traduction IA, progression des téléchargements et des imports) : `tauri::ipc::Channel` côté Rust, `new Channel<T>()` côté TypeScript.
- **Pourquoi un sidecar Whisper** : llama.cpp et whisper.cpp embarquent chacun leur propre copie de ggml ; les lier dans le même binaire provoque des conflits de symboles. `lumen-whisper` est lancé par `media.rs` et répond en lignes JSON. Qwen3-ASR, lui, passe par llama.cpp : il tourne dans l'application (`asr.rs`), chargé le temps d'une transcription.
- **Pourquoi garder Whisper avec Qwen3-ASR** : Qwen3-ASR ne donne pas d'horodatages (son aligneur ne couvre que 11 langues et 5 minutes) ; le minutage DTW de `lumen-whisper` (30 à 45 ms d'écart) sert la lanterne, recalé sur le texte de Qwen3-ASR (91 à 94 % des mots retrouvés).
- **IA** : prompts ChatML avec un bloc `<think></think>` vide (désactive le raisonnement de Qwen3.5), exemples few-shot, indices du dictionnaire. Les requêtes sont **interruptibles** par un compteur d'époque (une nouvelle sélection annule la précédente). Les réponses mot sont mises en cache (table `tcache`, clé versionnée `w3`). `Engine::run` (paramètres `Gen`) sert aussi le chat : réflexion permise avec son budget, réflexion et réponse transmises à part (`Piece`), `Priority::Stoppable` (arrêtée par son drapeau, jamais par un clic sur un mot). Échantillonnage `Exact` (glouton) pour les traductions, `Natural` pour le chat (température, top-p, pénalité « DRY » contre les boucles des petits modèles). Une génération à la fois (`run_lock`) : pendant une réponse du chat, la traduction d'un mot attend son tour.
- **Médias** : copiés dans `$APPDATA/media/`, servis à la WebView par le protocole `asset` (portée limitée à `$APPDATA/media/**`).
- **LingQ** : API non documentée, lue avec prudence (champs optionnels, reprises sur 429 et 5xx). v2 pour `known-words` et `ignored-words`, v3 pour `cards`, `collections/my`, `search?shelf=my_lessons`, `collections/{id}/lessons` et `lessons/{id}` (`tokenizedText`, `audioUrl`). La clé n'est envoyée qu'à `www.lingq.com`. Lire une leçon par l'API la fait remonter dans l'étagère « Continuer » de LingQ.
- **Dictionnaires** : Wiktionnaire français via kaikki.org, compilés en SQLite, livrés compressés dans `src-tauri/resources/dicts/*.db.gz`, décompressés au premier usage. En anglais : Wiktionnaire anglais, même schéma (table des formes `WITHOUT ROWID`), téléchargé à la demande (voir « Langue de l'interface »).
- **Traduction de l'interface** : chaque texte visible est écrit dans les deux langues, côte à côte, `t("Terminer la page", "Finish page")` (`src/lib/i18n.ts`), avec `count(n, "leçon", "leçons", "lesson", "lessons")` pour les nombres. Changer de langue remonte toute l'interface (clé `ui_lang` sur la racine de `App`). Côté Rust, `i18n::t(fr, en)` et `tr!(fr, en, args)` (`src-tauri/src/i18n.rs`), langue mise à jour par `settings_set("ui_lang")` et au démarrage ; les consignes de l'IA reçoivent la langue explicitement (`native`), pour rester testables.
- **Données utilisateur** : `~/Library/Application Support/app.lumen.reader/` (`lumen.db`, `models/`, `media/` dont `media/voice/`, `tools/` dont le moteur de voix, dictionnaires décompressés, `device-id`, `lumen.avant-restauration.db` après une restauration).
- **Pourquoi un dossier iCloud Drive pour la sauvegarde** : CloudKit exige un compte développeur Apple payant et une signature Developer ID (Lumen est signé ad hoc) ; un serveur (Supabase, Firebase) coûterait, ferait de Lumen le gardien des données de ses utilisateurs et casserait le « tout en local ». Un simple dossier marche aussi avec Dropbox, Google Drive ou OneDrive, et sur Windows plus tard. Les chemins des médias sont absolus dans la base : la restauration les réécrit.
- **Pourquoi un outil téléchargé pour la voix** : l'app est signée avec le « hardened runtime », qui refuse de charger une bibliothèque téléchargée ; un exécutable séparé (comme yt-dlp) n'a pas ce problème, et l'app ne grossit pas.

## Structure des dossiers clés

```
src/
  App.tsx                 Démarrage, thème, glisser-déposer, routage des vues, accueil
  main.tsx                Montage React, polices, styles
  components/
    Sidebar.tsx           Navigation, langue active, compteur de mots connus, carte de mise à jour
    AddToPlaylist.tsx     Feuille « Ajouter à une playlist » (depuis la bibliothèque)
    UpdateCard.tsx        Carte « Lumen X est disponible »
    Guide.tsx             Petit guide : quatre cartes animées (principe, page, lanterne, modèles d'IA)
    BackupCard.tsx        Proposition de sauvegarde, alerte de sauvegarde interrompue
    LingqCard.tsx         Avancement de l'import LingQ dans la barre latérale
    Cover.tsx             Couvertures (image choisie, miniature YouTube, œuvre générée, mosaïque des playlists)
    Markdown.tsx          Mise en forme des réponses du chat (sans HTML), espaces insécables françaises
    ui.tsx                Composants partagés (Orb, Segmented, Switch, Sheet, Menu, Toasts, CountUp, useGlow)
    Icon.tsx              Icônes SVG maison
  lib/
    i18n.ts               Langue de l'interface : t(fr, en), count, dates et nombres (locale)
    api.ts                Interface Api + implémentation Tauri (invoke, Channel, convertFileSrc)
    mock.ts               Backend simulé pour le navigateur (doit rester aligné sur api.ts)
    store.ts              État global zustand : réglages (DEFAULTS), vue, leçon, imports, toasts, modèles
    types.ts              Types partagés (miroir des structures Rust sérialisées)
    tokenize.ts           Découpage en mots, pages, phrases (miroir de text.rs)
    importers.ts          Extraction : web, EPUB, PDF, sous-titres
    tts.ts                Voix du système, événements de frontière de mot
    updater.ts            Store des mises à jour (check, install, restart)
    lingq.ts              Store de l'import LingQ (analyse, import, progression)
    pronounce.ts          Prononciation des mots : voix naturelle (préparée, en cache) ou voix du système
    playlists.ts          Playlists : leçons dans l'ordre, durée, reprise, lancement de l'écoute
    chat.ts               Store du chat : conversations, réponse en cours, leçon jointe, questions proposées
    backup.ts             Store de la sauvegarde : état, sauvegarde, liste, restauration, relecture de toute l'app
    covers.ts             Liens YouTube, choix et réduction des images de couverture
    langs.ts              Langues, salutations, textes de départ (STARTERS)
    profiles.ts           Profils IA (Léger, Équilibré, Maximum)
    dialogs.ts            Confirmations natives
  views/
    Onboarding.tsx        Accueil animé
    Library.tsx           Bibliothèque
    Playlists.tsx         Playlists : toutes, une playlist (ordre, ajout, retrait), choix des leçons
    Chat.tsx              Vue Chat : liste des conversations, conversation ouverte
    chat/
      ChatThread.tsx      Conversation (messages, réflexion, champ, commandes /, effort), aussi dans le lecteur
    ImportSheet.tsx       Feuille d'import (texte, lien universel et son choix, fichiers, audio et vidéo ; étapes de progression)
    Vocabulary.tsx        Vocabulaire
    Progress.tsx          Progrès
    Settings.tsx          Réglages
    LingqSection.tsx      Réglages › LingQ (clé API, analyse du compte, import)
    BackupSection.tsx     Réglages › Sauvegarde (activation, médias, emplacement, restauration)
    reader/
      Reader.tsx          Lecteur : pages, sélection, raccourcis, lanterne, Simplifier
      WordPanel.tsx       Panneau du mot (dictionnaire + IA)
      Player.tsx          Lecture audio/vidéo/voix, synchronisation, poignée impérative
      VideoStage.tsx      Cadre vidéo, sous-titres interactifs, mode cinéma
      PlaylistBar.tsx     Playlist suivie : bandeau du haut, carte « Ensuite »
      ReaderChat.tsx      Onglets « Mot » et « Chat » du panneau de droite, chat de la leçon
  styles/
    app.css               Système visuel : variables clair/sombre, boutons, barre latérale, toasts
    views.css             Bibliothèque, vocabulaire, progrès, réglages
    reader.css            Lecteur, lanterne, vidéo, cinéma
    onboarding.css        Accueil (variables .ob pour clair et sombre)
    chat.css              Chat (vue, panneau du lecteur, champ, réflexion)
    guide.css             Petit guide et son invitation dans le panneau du mot

src-tauri/
  tauri.conf.json         Fenêtre, bundle (DMG, ressources, sidecar), updater (clé publique, URL)
  Info.plist              Ajouts fusionnés à l'Info.plist de l'app (langues, texte de la demande d'accès à iCloud Drive)
  locales/                Textes de l'Info.plist traduits (en.lproj, fr.lproj), copiés dans le paquet
  Cargo.toml              Workspace (app + lumen-whisper), dépendances, profil release (LTO thin)
  capabilities/default.json  Permissions accordées à la WebView
  src/
    lib.rs                Plugins, setup (dossier de données, DB, dictionnaires), liste des commandes
    i18n.rs               Langue de l'interface côté natif : t(fr, en), tr!(fr, en, …)
    commands.rs           Toutes les commandes #[tauri::command]
    db.rs                 Schéma, migrations, requêtes (leçons, mots, activité, cache)
    ai.rs                 Moteur llama.cpp (réflexion bornée, arrêt), prompts (mots, phrases, chat), analyse des réponses
    asr.rs                Qwen3-ASR : texte des transcriptions (entrée audio de llama.cpp, découpe dans les silences)
    dict.rs               Dictionnaires dans la langue de l'interface (français livrés, anglais téléchargés), recherche
    text.rs               Découpage UAX 29, élisions, normalisation, offsets UTF-16
    media.rs              Transcription (sidecar), horodatages, yt-dlp
    tools.rs              yt-dlp et QuickJS gérés (installation, mise à jour hebdomadaire)
    models.rs             Catalogue et téléchargement des modèles avec reprise
    link.rs               Lien universel : analyse (pages, yt-dlp, flux RSS, Apple Podcasts, Spotify), téléchargement, couverture
    lingq.rs              Import LingQ (vocabulaire, cours, leçons, audio)
    voice.rs              Voix naturelle : moteur sherpa-onnx, Supertonic 3, nettoyage du son, cache
    backup.rs             Sauvegarde (iCloud Drive ou dossier choisi), historique, ménage, restauration, état d'envoi iCloud
    state.rs              AppState partagé
  lumen-whisper/          Sidecar de transcription
  resources/dicts/        Dictionnaires compressés, définitions en français (≈ 66 Mo)
  resources/dicts-en/     Dictionnaires à définitions anglaises, construits sur place (ignorés par git, publiés à part)
  dmg/background.png      Fond du DMG (source : design/dmg-background.html)

scripts/
  mac-env.sh              Vérifie et installe les outils, prépare la clé de signature
  release.mjs             bump (numéro de version partout) et manifest (latest.json)
  build-sidecar.mjs       Compile lumen-whisper sous le nom attendu par Tauri
tools/build_dicts.py      Reconstruction des dictionnaires depuis kaikki.org (--en : Wiktionnaire anglais)
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

- Test réel de l'IA : `LUMEN_TEST_MODEL=/chemin/Qwen3.5-2B-Q4_K_M.gguf cargo test --release --lib live -- --ignored --nocapture` (dont `chat_live` : chat sans puis avec réflexion bornée, entraînement, arrêt en cours de route ; `english_live` : mots, phrase et chat pour un apprenant anglophone, avec les repères du dictionnaire anglais s'il est construit).
- Dictionnaires anglais : `python3 tools/build_dicts.py --en <dossier des .jsonl de kaikki.org> src-tauri/resources/dicts-en` ; téléchargement réel comme dans l'app (version `dictionaries-en-1`, ou un serveur local avec `LUMEN_DICT_BASE=http://127.0.0.1:8765`) : `cargo test --lib dict_download_live -- --ignored --nocapture`.
- Test réel de LingQ (base jetable, rien n'est écrit dans Lumen) : `LUMEN_LINGQ_KEY=… cargo test --lib lingq_live -- --ignored --nocapture`.
- Test réel de la voix (télécharge le moteur dans un dossier jetable) : `LUMEN_VOICE_MODEL=/chemin/sherpa-onnx-supertonic-3-tts-int8-2026-05-11 cargo test --lib voice_live -- --ignored --nocapture` ; téléchargement complet comme dans l'app : `cargo test --lib voice_download_live -- --ignored --nocapture` ; audio d'une leçon (+ recalage si `LUMEN_ASR_MODEL` est donné ; copier d'abord `binaries/lumen-whisper-aarch64-apple-darwin` en `target/debug/deps/lumen-whisper`) : `cargo test --lib lesson_audio_live -- --ignored --nocapture`.
- Test réel du recalage : `LUMEN_TEST_TEXT=texte.txt LUMEN_TEST_WORDS=mots.json LUMEN_TEST_LANG=it cargo test --lib realign_live -- --ignored --nocapture` (mots : sortie `done` de `lumen-whisper`) ; même chose pour la mise en page aérée : `airy_live` (`LUMEN_AIRY_OUT` pour enregistrer le texte).
- Test réel de Qwen3-ASR : `LUMEN_ASR_DIR=dossier (modèle + mmproj) LUMEN_TEST_AUDIO=a.wav,b.wav LUMEN_TEST_LANG=it cargo test --release --lib asr_live -- --ignored --nocapture` (WAV f32 16 kHz mono : `afconvert -f WAVE -d LEF32@16000 -c 1`). Import complet comme dans l'app (Whisper, Qwen3-ASR, recalage, mise en page ; copier d'abord `binaries/lumen-whisper-aarch64-apple-darwin` en `target/release/deps/lumen-whisper`) : `LUMEN_ASR_MODEL=ggml-….bin LUMEN_ASR_DIR=… LUMEN_TEST_AUDIO=son.mp3 cargo test --release --lib transcript_live -- --ignored --nocapture`.
- Test réel du lien universel : `LUMEN_TEST_URL="lien1 lien2" cargo test --lib link_live -- --ignored --nocapture` (analyse seule, affiche ce qui est trouvé) ; téléchargements (épisode RFI, couverture réduite) : `cargo test --lib link_download_live -- --ignored --nocapture` ; texte de la page gardé ou écarté selon ce qu'on entend (Whisper réel ; copier d'abord le sidecar en `target/debug/deps/lumen-whisper`) : `LUMEN_ASR_MODEL=ggml-….bin LUMEN_TEST_AUDIO=son.mp3 LUMEN_TEST_LANG=fr cargo test --lib reference_live -- --ignored --nocapture`.
- Test réel de la sauvegarde sur une copie des données (dossier jetable, rien n'est écrit dans iCloud ni dans les données d'origine) : `LUMEN_BACKUP_DATA="$HOME/Library/Application Support/app.lumen.reader" cargo test --lib backup_live -- --ignored --nocapture` ; état d'envoi iCloud (lecture seule) : `cargo test --lib icloud_state_live -- --ignored --nocapture`.
- En mode `npm run dev`, `window.__lumen = { api, useApp }` est exposé pour piloter l'état depuis la console. Le backend simulé propose une sauvegarde fictive « iMac du salon » pour essayer la restauration.

## Règles de code

### Langue et ton

- **Tout texte visible existe en français et en anglais** : `t("…", "…")` dans l'interface, `t`/`tr!` dans le natif. Jamais de texte visible écrit dans une seule langue ; dans `mock.ts` aussi. Le français suit la typographie française (espace avant `:` `?` `!`, guillemets « », `…`), l'anglais la typographie anglaise (pas d'espace avant `:`, guillemets “ ”). Un pourcentage : `t(`${n} %`, `${n}%`)`.
- Attention à l'ombre : une variable locale nommée `t` (minuterie, jeton) cache la fonction `t()` ; la nommer autrement.
- Un texte calculé une fois au chargement d'un module resterait dans l'ancienne langue : passer par une fonction ou un accesseur (`get label() { return t(…) }`).
- Les marqueurs internes échangés avec le natif (`annulé`, `interrompu`, `NO_MODEL:`, `NO_VOICE:`) ne se traduisent pas.
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
- **Migrations** : uniquement additives, dans `db.rs`, avec `add_column(conn, table, colonne, déclaration)` (teste l'existence via `pragma_table_info`). Ne jamais casser une base existante : les utilisateurs mettent à jour sans réinstaller (le test `resume_position_and_cover` ouvre une base d'ancienne version).
- **Offsets de texte en UTF-16** pour correspondre aux index des chaînes JavaScript.
- **Normalisation identique** entre `text.rs` et `tokenize.ts` (minuscules, apostrophes unifiées, élisions it/fr/pt/ca). Les accents U+0301 et U+0300 ne sont retirés que pour `en it de pt ru es uk bg` (`strips_accents`, `normalize_for`) : en vietnamien, grec, tchèque, français…, ils distinguent des mots. Ne jamais changer cette liste pour une langue déjà proposée (les clés du vocabulaire existant changeraient). Toute modification d'un côté se reporte de l'autre.
- Ne jamais lier whisper.cpp dans l'application principale : Whisper reste dans `lumen-whisper` (Qwen3-ASR, par llama.cpp, peut y vivre).
- Travail long (IA, transcription, téléchargement) : asynchrone, interruptible, avec progression envoyée par `Channel`.
- Changer le format d'une réponse IA mise en cache impose de changer la clé de cache (`w3` → `w4`).
- **Sauvegarde** : un nouveau réglage secret ou propre à ce Mac va dans `LOCAL_ONLY` (et `KEEP_ON_RESTORE`) de `backup.rs` ; une nouvelle colonne qui contient un chemin de fichier doit être reprise par `media_refs` et par la restauration (chemins absolus, réécrits d'un Mac à l'autre). Changer la forme des fichiers de sauvegarde impose d'augmenter `FORMAT`.

### Interface et design

- **Animations avec `motion`** (ressorts doux, entrées décalées). Respecter `prefers-reduced-motion` (règle déjà présente dans `app.css`).
- **Couleurs uniquement via les variables CSS** du thème (clair et sombre). Tester les deux thèmes.
- Palette chaude et lumineuse : or, ambre, crème le jour ; nuit chaude et brune le soir. Pas de bleu dominant sur l'accueil.
- Effets de lumière subtils et signifiants (halo de sélection, lanterne, reflets) plutôt que décoratifs ou bruyants.
- Le lecteur doit rester confortable : pas d'animation qui distrait pendant la lecture.
- **motion et CSS ne se partagent jamais `transform`** : un élément animé par motion (`layout`, `initial`/`animate`) ne porte ni `transition` CSS sur `transform` ni survol en `transform` ; on l'enveloppe (`.lesson-cell` animée autour de `.lesson-card` et de son survol). Liste filtrée : `AnimatePresence mode="popLayout"` (enfants qui transmettent leur `ref`), `layout="position"`, transition `layout` sans délai ; le décalage d'une carte à l'autre ne vaut que pour la première apparition. Les polices sont chargées avant le premier affichage (`fontsReady` dans `main.tsx`) pour que rien ne change de hauteur à leur arrivée.
- État global dans `store.ts` ; état local dans le composant. Réglages persistés par `setSetting(clé, valeur)` (chaînes), valeurs par défaut dans `DEFAULTS`.
- Les composants qui dépendent d'une leçon (`Player`, `VideoStage`) sont rendus avec `key={lesson.id}` pour repartir à zéro d'une leçon à l'autre.
- **WebKit** (moteur de la fenêtre Lumen) : ne pas étirer (`align-items: stretch`) un élément qui tient sa hauteur d'`aspect-ratio` dans un conteneur flex dont la hauteur change ; au retour, WebKit garde l'ancienne hauteur étirée (c'est ce qui faisait disparaître la vidéo après le plein écran). Vérifier une mise en page délicate dans WebKit, pas seulement dans Chrome.

### Versions, signature, publication

- **Ne jamais modifier le numéro de version à la main** : `node scripts/release.mjs bump X.Y.Z` met à jour `package.json`, `tauri.conf.json` et `Cargo.toml` ensemble (le script de publication le fait).
- La **clé privée** des mises à jour est `~/.tauri/lumen-updater.key` : ne jamais la copier dans le projet, ne jamais la committer, ne jamais la régénérer (les Lumen installés refuseraient toutes les mises à jour suivantes).
- `plugins.updater.pubkey` dans `tauri.conf.json` est inscrite par `mac-env.sh` : ne pas la remplacer.
- Mises à jour publiées sur `LivingTwice/lumen-releases` (public), code source sur `LivingTwice/lumen` (privé). URL lue par l'app : `https://github.com/LivingTwice/lumen-releases/releases/latest/download/latest.json`, plateforme `darwin-aarch64`.
- **Dictionnaires anglais** : version à part `dictionaries-en-1` de `lumen-releases` (les six `<langue>.db.gz`), créée avec `--latest=false` ; elle ne doit **jamais** devenir « latest », sinon les Lumen installés ne trouvent plus `latest.json`. Le script de publication passe `--latest` à chaque version de Lumen. Pour les refaire : nouvelle version `dictionaries-en-2` et `EN_URL`, `EN_VERSION` dans `dict.rs`.
- Signature ad hoc (`signingIdentity: "-"`), pas encore de Developer ID : sur un autre Mac, premier lancement via Réglages Système › Confidentialité et sécurité › Ouvrir quand même.

### Prudence

- Ne pas supprimer de données utilisateur (leçons, mots, médias) sans confirmation explicite via `confirmAsk` (la restauration d'une sauvegarde aussi).
- Ne pas ajouter de dépendance lourde sans raison : l'application doit rester légère (les modèles IA se téléchargent à part).
- `public/` contient des fichiers de test (ignorés par git) : ne pas y mettre de ressources nécessaires à l'app.

## Pistes suivantes

- Transcription par Apple SpeechAnalyzer (macOS 26) en complément de Whisper.
- Lecture de la leçon entière avec la voix naturelle (il faudra des horodatages mot à mot pour la lanterne).
- Japonais (segmentation en mots par dictionnaire, côté Rust et interface) et chinois.
- Version Windows (même code, WebView2), puis version web.
- Révision douce en contexte des mots en apprentissage.
- Synchronisation entre plusieurs Macs (aujourd'hui : sauvegarde et restauration ; il faudra fusionner mots, leçons et suppressions, et des identifiants de leçons communs aux appareils).
- Signature Developer ID et notarisation Apple.
