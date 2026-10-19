# Lumen sur iPhone et iPad

Le plan du chantier, ses choix et ce qui reste à faire. Tenu à jour à chaque étape.

## Où on en est (18 octobre 2026)

- **Toutes les fonctions de Lumen tournent sur iPhone**, avec le **même code** que sur Mac et Windows (projet Xcode dans `src-tauri/gen/apple`). Vérifié dans le simulateur (iPhone 17 Pro, iOS 26.5) avec une copie des vraies données d'Ulysse (250 leçons italiennes) : lecture, mots, voix naturelle, IA locale, transcription (Whisper et Parakeet), YouTube et Dailymotion, Découvrir et la recherche, sauvegarde et progression du Mac. **Pas encore essayé sur un vrai iPhone** : vitesse de l'IA sur la puce graphique, mémoire, chaleur.
- **Interface** : barre d'onglets en verre, capsule « Lecture en cours », feuille « Plus », menus en feuilles qui montent du bas (un menu « … » ne passe plus jamais derrière une carte), pages tournées d'un balayage, retour haptique, bibliothèque et vocabulaire à la largeur du téléphone, Réglages en liste puis page.
- **La progression du Mac sur l'iPhone** : à l'accueil ou dans Réglages › Sauvegarde, on choisit dans l'app Fichiers le dossier « Lumen » où le Mac sauvegarde (iCloud Drive, ou Google Drive, Dropbox, OneDrive par leur app). L'iPhone y reprend tout (restauration), puis **fusionne** à chaque lancement et toutes les 10 minutes ce que le Mac a changé, et le Mac ce qu'a changé l'iPhone (voir « Synchronisation »).
- **Installer sans l'App Store** : double-clic sur `Installer Lumen sur l'iPhone.command` (voir « Distribution »).

## Synchronisation (sync.rs)

Chaque appareil sauvegarde dans son propre dossier (« iPhone de Léa (clé) »), et ajoute à sa base ce qu'a changé l'autre, sans rien écraser (`backup::merge_folder`, réglage `backup_sync`, oui par défaut ; « Fusionner » ou « Remplacer » dans Réglages › Sauvegarde). Une fusion ne relit un dossier que s'il a changé depuis la dernière (`sync_<clé>` : date de la sauvegarde lue, propre à l'appareil, jamais sauvegardée).

- **Leçons** : reconnues par leur `uid` (le même partout : empreinte de la langue, de la date de création, du titre et de l'identifiant LingQ ; donné aux anciennes leçons par `db::ensure_uids`). Une leçon nouvelle arrive avec son son et sa couverture s'ils sont dans `Médias/` ; l'avancement le plus récent l'emporte (page, mot, seconde, « Lu », d'après `opened_at`).
- **Mots** : le plus récemment modifié l'emporte (`updated_at`).
- **Suppressions** : table `tombstones` (leçon supprimée, mot remis à « nouveau »), écrite par `db::tombstone` ; une leçon supprimée d'un côté après sa dernière lecture disparaît de l'autre.
- **Activité** : pour chaque jour, le plus grand des deux compteurs (rien n'est compté deux fois). **Playlists** : celle du même nom reçoit les leçons qui lui manquent. Conversations du chat et réglages : propres à chaque appareil.
- **Qui se fusionne** : seulement les appareils d'une même personne (même `profile_id` : la restauration l'apporte avec la base, « Fusionner » l'adopte), jamais deux fois le même appareil.
- **Dossier de l'app** : iOS change le chemin du dossier de données à chaque mise à jour ; au lancement, `db::rebase_media` ramène les chemins des médias dans le dossier actuel.
- Test : `cargo test --lib two_devices_meet_halfway` (deux bases qui se fusionnent l'une l'autre arrivent au même état) ; réel, sur une copie des données : `sync_live`.

## Le principe

- **Un seul code.** iOS est une troisième plateforme, comme Windows : ce qui diffère se décide par `cfg(target_os = "ios")` (ou `cfg(mobile)`) côté Rust, `isIOS` / `isMobile` / `useCompact()` côté interface, `styles/mobile.css` pour l'allure. Le Mac et Windows ne changent pas.
- **Chaque changement est porté.** Toute nouveauté se vérifie aussi avec `node scripts/build-ios.mjs --check` (règle dans CLAUDE.md).
- **La qualité d'abord, la chaleur ensuite.** Les mêmes modèles que sur Mac quand l'appareil les porte ; quand il chauffe, Lumen ralentit (moins de fils, pauses entre les morceaux), il ne dégrade pas la qualité en silence.
- **Android plus tard**, avec Tauri aussi : d'où le choix de briques qui existent sur les deux (llama.cpp, sherpa-onnx) plutôt que de tout réécrire en Swift.

## Ce qui ne peut pas marcher tel quel sur iPhone

iOS interdit à une app de lancer un autre programme. Tout ce qui passe aujourd'hui par un programme à part doit tourner **dans** l'app.

| Sur Mac et Windows | Sur iPhone et iPad | Où |
| --- | --- | --- |
| `lumen-whisper`, programme à part (Whisper) | whisper.cpp 1.9.4 compilé **dans** l'app avec la copie de ggml de llama.cpp (une seule ggml, aucun conflit de symboles) ; même minutage (`lumen-whisper/src/timing.rs`, partagé) | `src-tauri/whisper/`, `build.rs` (`whisper_inapp`), `whisper.rs` |
| `sherpa-onnx-offline-tts` (voix Supertonic 3) | La bibliothèque sherpa-onnx (crate officiel, son framework intégré à l'app), la voix gardée chargée | `sherpa_ios.rs`, `voice.rs` |
| `sherpa-onnx-offline` (Parakeet) | La même bibliothèque, le même modèle ; deux morceaux de 15 s à la fois (quatre sur un iPad), pour la mémoire | `sherpa_ios.rs`, `parakeet.rs` |
| yt-dlp, Deno ou QuickJS (YouTube) | Lumen parle lui-même à YouTube (InnerTube : recherche, chaînes, playlists ; lecture par le client visionOS avec un `visitorData`, puis iOS, puis Android VR) et à Dailymotion ; répond aux mêmes lignes de commande que yt-dlp. Autres sites : message clair (importer depuis le Mac) | `ytnative.rs`, `media::run_ytdlp` |
| `afconvert`, `sips`, `tar` | AVFoundation et AudioToolbox (`compress::pcm_mono`, `compress::write_aac`), bibliothèques `image`, `tar`, `bzip2` | `compress.rs`, `voice.rs` |
| Sauvegarde dans `iCloud Drive/Lumen` | Le dossier choisi une fois dans l'app Fichiers (`UIDocumentPickerViewController`), accès gardé par un signet de sécurité ; un fichier resté dans le nuage descend par `NSFileCoordinator` | `ios.rs`, `LumenNative.swift`, `backup.rs` |
| Mises à jour automatiques | Réinstallation depuis le Mac (puis TestFlight ou App Store) | — |
| Barre des menus, raccourcis | Barre d'onglets, gestes, menus en feuilles | `MobileNav.tsx`, `ui.tsx` |
| Connexion à ChatGPT : navigateur + adresse locale | Page de connexion posée sur Lumen (`SFSafariViewController`) : Lumen reste au premier plan, son adresse locale reçoit la réponse ; la fermer annule | `ios::web_open`, `LumenNative.swift` |
| Mémoire et chaleur de l'appareil | `proc::device_info` (modèle, mémoire, mémoire que iOS laisse, état thermique) ; profil d'IA recommandé selon la mémoire ; moitié moins de cœurs quand l'iPhone chauffe (`proc::hot`) | `proc.rs`, `profiles.ts` |

Le pont Swift (`gen/apple/Sources/lumen/LumenNative.swift`) expose des fonctions C (`@_cdecl`) ; `src-tauri/ios/stubs.c` en donne des versions « faibles » pour que la bibliothèque dynamique que Cargo lie aussi trouve ces symboles (dans l'app, les vraies l'emportent).

**Simulateur** : son Metal calcule faux pour llama.cpp (des « !!!! » au lieu des mots) : l'IA y calcule sur le processeur (`ai::gpu_wanted`) ; un vrai iPhone calcule sur sa puce graphique. Whisper, Parakeet et la voix y marchent comme sur l'appareil.

## Modèles et langues

Les 31 langues de Lumen : en it de pt ru es fr nl sv da fi et lv lt pl cs sk sl hr hu ro bg uk el tr ar hi id vi ko ja.

| Modèle | Rôle | Langues de Lumen | Sur iPhone |
| --- | --- | --- | --- |
| Qwen3.5 0.8B, 2B, 4B (llama.cpp) | sens des mots, phrases, chat, Simplifier | toutes | compilé (Metal) ; à mesurer sur appareil |
| Whisper Large v3 Turbo (whisper.cpp) | minutage au mot près (la lanterne), texte de secours | les 31 | étape 4 |
| Qwen3-ASR 1.7B et **0.6B** (llama.cpp, ggml-org) | texte des transcriptions | en connaît 23 (toutes sauf et lv lt sk sl hr bg uk) ; Lumen ne l'emploie qu'en 15 : pas sv da fi pl cs hu ro el, où Whisper seul fait mieux | 0.6B : moitié moins lourd, pour les iPhone de 8 Go |
| **Parakeet TDT 0.6B v3** (NVIDIA, sherpa-onnx ou Core ML) | texte des transcriptions, rapide, sur le processeur | en place sur Mac et Windows dans 18 langues : en it de pt ru es fr nl pl cs hu et lt sk sl hr bg uk (pas sv da fi ro lv, où Whisper fait mieux, ni el : pas de « ς » dans son vocabulaire) | modèle compressé par Lumen (1 Go) ; 2,4 Go de mémoire en lots de deux minutes |
| Apple SpeechAnalyzer (iOS 26) | texte et minutage, sur le moteur neuronal, rien à télécharger | liste lue sur l'appareil (au moins en fr de it es pt ja ko et d'autres) | option économe |
| Supertonic 3 (sherpa-onnx) | voix naturelle | les 31 | étape 3 |

**Qwen3-ASR et Parakeet se complètent** : Lumen n'emploie chacun que dans les langues où il a été mesuré au moins aussi bon que Whisper seul (FLEURS, 15 min par langue, octobre 2026 ; `asr::writes`, `parakeet::knows`), et le meilleur des deux quand les deux conviennent. Whisper reste partout pour le minutage précis de la lanterne et comme filet de sécurité. Parakeet donne ses propres temps, mesurés : 70 ms d'écart médian avec ceux de Whisper recalés, plus de 200 ms pour un mot sur dix (pas de 80 ms) ; sur un petit appareil, ils pourraient suffire, avec une lanterne moins fine.

## Modes selon l'appareil (proposition, à mesurer sur de vrais iPhone)

Lumen lit le modèle de l'appareil et la mémoire que iOS lui laisse vraiment (`os_proc_available_memory`), et recommande un mode ; l'apprenant peut en choisir un autre. Même idée que les profils actuels (Léger, Équilibré, Maximum), avec l'appareil nommé : « Recommandé pour votre iPhone 17 Pro ».

| Appareils | Mémoire | Mode recommandé | Traduction et chat | Transcription |
| --- | --- | --- | --- | --- |
| iPhone 17 Pro, 17 Pro Max, Air (A19 Pro) ; iPad à puce M | 12 Go et plus | Qualité | Qwen3.5 4B | Qwen3-ASR 1.7B ou Parakeet, + Whisper Turbo |
| iPhone 15 Pro, 16, 16 Pro, 16e, 17, 17e | 8 Go | Équilibré | Qwen3.5 2B | Qwen3-ASR 0.6B ou Parakeet, + Whisper Turbo |
| iPhone 13 Pro, 14, 14 Pro, 15 | 6 Go | Léger | Qwen3.5 0.8B (IA en ligne conseillée) | Parakeet, + Whisper Small |
| iPhone XS à 13 | 4 Go | En ligne | IA en ligne (0.8B possible, lent) | Whisper Base, lente |

- **Chaleur** : `ProcessInfo.thermalState`. Normal : plein régime. Sérieux : moitié des fils de calcul, une pause entre deux morceaux de transcription. Critique : les travaux de fond s'arrêtent et reprennent seuls. Mode économie d'énergie : rien de lourd sans demande.
- **Mémoire** : le droit « increased memory limit » (Xcode, Signing & Capabilities) porte la limite à environ 6 Go sur un iPhone de 12 Go ; un modèle ne se charge que s'il tient dans ce qui reste.
- **Arrière-plan** : une transcription lancée par l'apprenant continue écran éteint grâce à `BGContinuedProcessingTask` (iOS 26), sur le processeur (la puce graphique en arrière-plan n'est ouverte qu'aux iPad M3 et plus récents).

## Sign in with ChatGPT

Fait dans le cœur commun (`chatgpt.rs`, Réglages › IA › ChatGPT), donc pour Mac, Windows et iPhone (sur iPhone, page de connexion posée sur Lumen, voir plus haut ; compilée, pas encore essayée). Vérifié le 2026-10-13 pour Mac et Windows : connexion entière contre un faux OpenAI (`sign_in_goes_all_the_way`), compilation et tests compilés pour Windows, écran de connexion dans WebKit à l'allure du Mac et de Windows ; Lumen repasse devant le navigateur à son retour (sûr sous Windows, au mieux sur macOS).

- **Qui peut s'en servir** : les abonnés **Plus et Pro**. Un compte gratuit peut se connecter, mais OpenAI ne laisse pas son forfait servir aux autres apps (erreur `subscription_sharing_user_not_eligible`, dite clairement).
- **Ce que cela consomme** : la part « ChatGPT Work et Codex » du forfait, pas les messages du chat de ChatGPT. Sur Plus, une fenêtre de 5 heures partagée par toutes les apps connectées, et une limite par semaine que l'apprenant fixe pour chaque app (ChatGPT, Réglages › Utilisation). L'unité exacte (jetons ou demandes) n'est pas publiée.
- **Un mot, une demande, toujours** : l'API n'a pas de conversation gardée (`store: false` obligatoire). Faire « un grand chat » pour plusieurs mots obligerait à renvoyer tout l'historique à chaque mot : chaque traduction coûterait de plus en plus cher au forfait et serait plus lente. Une demande courte et indépendante par mot (consignes et exemples identiques, puis le mot et sa phrase) reste la bonne forme, comme pour les clés.
- **Beaucoup de mots touchés en quelques secondes** : une traduction attend 0,35 s avant de partir ; un autre mot touché entre-temps la remplace, rien n'est envoyé ni compté. Au plus 30 traductions par minute. Chaque traduction est gardée (un mot déjà traduit ne repart jamais). Limite du forfait atteinte : 15 minutes de pause, la traduction se fait avec l'IA de l'appareil, un message explique et mène à « Gérer l'utilisation ». Vérifié sans abonnement, avec un faux serveur d'OpenAI (`chatgpt::tests`) : 20 mots touchés à 50 ms d'intervalle → **une seule demande** part, celle du dernier mot, aucune erreur.
- **Le chat** de Lumen garde sa forme (la conversation, bornée, est renvoyée à chaque question) : c'est là que le forfait sert le plus.
- **À vérifier au premier essai réel** (un abonné Plus, en y allant doucement, compteur des demandes du jour sous les yeux dans Réglages) : le renouvellement des jetons (paramètre `resource`), le nom des modèles proposés (« mini » conseillé), l'effort de réflexion « none » accepté, l'adresse de « Gérer l'utilisation ».

## Distribution

**Aujourd'hui, sans rien payer** : Xcode et un identifiant Apple gratuit.

1. Xcode › Réglages › Comptes : ajouter son identifiant Apple, puis « Gérer les certificats » › « + » › « Apple Development ».
2. Brancher l'iPhone, le déverrouiller, « Se fier à cet ordinateur » ; activer Réglages › Confidentialité et sécurité › Mode développeur (l'iPhone redémarre).
3. Double-clic sur **`Installer Lumen sur l'iPhone.command`** (`node scripts/build-ios.mjs --install` : vérifie l'iPhone et la signature, compile la version complète, l'installe par `devicectl`, l'ouvre ; journal dans `installation-iphone.log`).
4. Première fois : Réglages › Général › VPN et gestion de l'appareil › l'identifiant › « Faire confiance ».
5. L'app s'ouvre 7 jours ; ensuite, relancer l'installation : Lumen se met à jour et garde toutes ses données.

| Voie | Coût | Ce que cela permet |
| --- | --- | --- |
| Xcode et un identifiant Apple gratuit | 0 | Lumen sur votre iPhone, à réinstaller depuis le Mac tous les 7 jours ; 3 apps au plus |
| Apple Developer Program | 99 € par an | TestFlight (versions de 90 jours, jusqu'à 10 000 testeurs), App Store, tous les droits (mémoire augmentée, iCloud), plus de réinstallation |
| AltStore PAL (Union européenne) | 99 € par an aussi (même compte), plus l'abonnement d'AltStore | Hors App Store : les fonctions YouTube y sont possibles |

L'App Store refuse les apps qui téléchargent des vidéos de YouTube : si Lumen y va, la version App Store n'aura pas l'import YouTube (les leçons YouTube viendront du Mac par la sauvegarde) ; une version hors App Store pourra l'avoir.

## Les étapes

0. **Fondations** — fait : projet Xcode, compilation, navigation iPhone, accueil, bibliothèque, lecteur.
1. **Toute l'interface à la main** — fait : menus en feuilles, pages tournées d'un balayage, retour haptique, vues à la largeur du téléphone. Reste : iPad avec clavier, la visite guidée revue au doigt.
2. **IA locale** — fait dans le simulateur (processeur). Reste : mesurer sur un vrai iPhone (vitesse sur la puce graphique, mémoire, chaleur) et ajuster les profils recommandés.
3. **La voix** — fait : sherpa-onnx dans l'app, Supertonic 3, son des leçons en AAC.
4. **La transcription** — fait : Whisper dans l'app, Parakeet. Reste : Qwen3-ASR 0.6B, SpeechAnalyzer en option, travail en arrière-plan (`BGContinuedProcessingTask`).
5. **Fichiers et sauvegarde** — fait : dossier choisi dans Fichiers, restauration et fusion avec le Mac. Reste : importer depuis « Partager » (Safari, Mail).
6. **ChatGPT** — fait (compilé) : page de connexion intégrée. Reste : un essai réel.
7. **YouTube** — fait : Lumen lit YouTube et Dailymotion sans yt-dlp.
8. **Android**, avec les mêmes briques.

## Pour travailler

- Simulateur : `node scripts/build-ios.mjs --sim` (« iPhone 17 Pro » par défaut, interface rechargée à chaud).
- Vérifier que tout compile pour iOS : `node scripts/build-ios.mjs --check`.
- Votre iPhone : brancher l'iPhone, ajouter votre identifiant Apple dans Xcode (Réglages › Comptes), puis `node scripts/build-ios.mjs --device` (développement) ou `--install` (version complète, voir « Distribution »). Version complète dans le simulateur : `--sim-release`.
- **Télécommande de développement** (seulement `npm run dev` et `--sim`) : un script envoyé au serveur de Vite s'exécute dans la fenêtre de Lumen, simulateur compris, et son résultat revient : `curl -s localhost:1420/__remote/run --data 'return await api.lessonsList("it")'` (`scripts/vite-remote.ts`, `src/lib/devremote.ts` ; reçoit `api` et `useApp`). Pour essayer les fonctions natives sans toucher l'écran. Une modification du Rust recompile et relance l'app : un travail en cours (téléchargement, transcription) s'arrête.
- Dans le navigateur, l'allure de l'iPhone avec le backend simulé : `http://localhost:1420/?platform=ios` (ou `ipad`), fenêtre étroite.
- Ne pas relancer `tauri ios init` : il réécrirait le projet Xcode (Accelerate ajouté à la main dans `project.yml`, icônes tirées de `design/icon-source.png`). Après une modification de `project.yml` : `xcodegen generate` dans `src-tauri/gen/apple`.
