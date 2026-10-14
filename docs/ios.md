# Lumen sur iPhone et iPad

Le plan du chantier, ses choix et ce qui reste à faire. Tenu à jour à chaque étape.

## Où on en est (13 octobre 2026)

- **Lumen démarre sur iPhone** (simulateur iPhone 17 Pro, iOS 26.5) avec le **même code** que sur Mac et Windows : l'interface React et tout le cœur Rust, llama.cpp compris (sur la puce graphique, Metal). Projet Xcode dans `src-tauri/gen/apple`.
- **Interface iPhone, première version** : accueil, barre d'onglets en verre (Bibliothèque, Découvrir, Importer au centre, Progrès, Plus), capsule « Lecture en cours » au-dessus, feuille « Plus » (playlists, chat, vocabulaire, réglages, langue étudiée), bibliothèque à deux colonnes, lecteur en pages composées pour l'écran, carte du mot touché. Zones réservées de l'écran (encoche, barre du bas) respectées, thèmes clair et sombre.
- **Interface iPhone, seconde passe (13 octobre 2026)**, vérifiée à 402 et 375 px de large, clair et sombre : accueil aux boutons toujours en bas de l'écran (on ne pouvait plus quitter l'étape des langues), langues en drapeaux avec leur nom (`flagOf` de `langs.ts`), horizon qui remonte sur un petit écran ; Réglages en liste puis page (« ‹ Réglages », `settingsPane` du store) ; Chat en liste puis conversation ; feuilles (Importer, playlists) qui montent du bas ; menu « Aa » sur toute la largeur ; carte du mot sur toute la largeur ; série des Progrès, vocabulaire en fiches, bibliothèque vide ; aucune vue ne glisse plus de côté.
- **Sign in with ChatGPT** : fait pour toutes les versions (voir plus bas). Sur iPhone, il manque l'écran de connexion intégré (étape 6).
- Tout le reste fonctionne sur iPhone tant que cela ne lance pas un autre programme (voir le tableau suivant) : ces fonctions répondent pour l'instant par une erreur.

## Le principe

- **Un seul code.** iOS est une troisième plateforme, comme Windows : ce qui diffère se décide par `cfg(target_os = "ios")` (ou `cfg(mobile)`) côté Rust, `isIOS` / `isMobile` / `useCompact()` côté interface, `styles/mobile.css` pour l'allure. Le Mac et Windows ne changent pas.
- **Chaque changement est porté.** Toute nouveauté se vérifie aussi avec `node scripts/build-ios.mjs --check` (règle dans CLAUDE.md).
- **La qualité d'abord, la chaleur ensuite.** Les mêmes modèles que sur Mac quand l'appareil les porte ; quand il chauffe, Lumen ralentit (moins de fils, pauses entre les morceaux), il ne dégrade pas la qualité en silence.
- **Android plus tard**, avec Tauri aussi : d'où le choix de briques qui existent sur les deux (llama.cpp, sherpa-onnx) plutôt que de tout réécrire en Swift.

## Ce qui ne peut pas marcher tel quel sur iPhone

iOS interdit à une app de lancer un autre programme. Tout ce qui passe aujourd'hui par un programme à part doit tourner **dans** l'app.

| Sur Mac et Windows | Sur iPhone et iPad | Étape |
| --- | --- | --- |
| `lumen-whisper`, programme à part (Whisper) | Whisper dans l'app. Le conflit avec llama.cpp (deux copies de ggml) se règle en cachant celle de whisper.cpp : édition de liens préalable (`ld -r -exported_symbols_list`, seules les fonctions `whisper_*` restent visibles). Repli possible : WhisperKit (Core ML) | 4 |
| `sherpa-onnx-offline-tts`, programme téléchargé (voix Supertonic 3) | La bibliothèque sherpa-onnx (xcframework officiel), appelée en direct ; le même modèle Supertonic 3, les 31 langues | 3 |
| `sherpa-onnx-offline`, programme téléchargé (Parakeet, `parakeet.rs`) | La même bibliothèque sherpa-onnx, le même modèle (`parakeet-tdt-0.6b-v3-q8`, morceaux de 15 s) ; d'ici là, le téléchargement répond « pas encore disponible sur cet appareil » | 4 |
| yt-dlp, Deno ou QuickJS (YouTube) | Python embarqué (BeeWare, Python 3.13+ officiellement sur iOS) avec yt-dlp, et JavaScriptCore pour ses défis JavaScript. Refusé par l'App Store (règle 5.2.3) : seulement hors App Store | 7 |
| `afconvert`, `sips`, `tar` (outils du Mac) | AVFoundation (déjà utilisé par `compress.rs`, présent sur iOS), et les bibliothèques de la version Windows (`image`, `tar`, `bzip2`, déjà déclarées pour iOS) | 3 et 5 |
| Sauvegarde dans `iCloud Drive/Lumen` | Le même dossier, choisi une fois dans l'app Fichiers (signet de sécurité) : la progression du Mac se retrouve sur l'iPhone, sans compte développeur payant | 5 |
| Mises à jour automatiques | App Store ou TestFlight (greffons réservés au Mac et à Windows) | — |
| Barre des menus, raccourcis | Barre d'onglets, gestes ; sur iPad, les raccourcis du clavier | 1 |
| Connexion à ChatGPT : navigateur + adresse locale | Safari mettrait Lumen en veille pendant la connexion : écran de connexion intégré (ASWebAuthenticationSession, petit greffon Swift), même adresse locale | 6 |

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

Fait dans le cœur commun (`chatgpt.rs`, Réglages › IA › ChatGPT), donc pour Mac, Windows et iPhone. Vérifié le 2026-10-13 pour Mac et Windows : connexion entière contre un faux OpenAI (`sign_in_goes_all_the_way`), compilation et tests compilés pour Windows, écran de connexion dans WebKit à l'allure du Mac et de Windows ; Lumen repasse devant le navigateur à son retour (sûr sous Windows, au mieux sur macOS).

- **Qui peut s'en servir** : les abonnés **Plus et Pro**. Un compte gratuit peut se connecter, mais OpenAI ne laisse pas son forfait servir aux autres apps (erreur `subscription_sharing_user_not_eligible`, dite clairement).
- **Ce que cela consomme** : la part « ChatGPT Work et Codex » du forfait, pas les messages du chat de ChatGPT. Sur Plus, une fenêtre de 5 heures partagée par toutes les apps connectées, et une limite par semaine que l'apprenant fixe pour chaque app (ChatGPT, Réglages › Utilisation). L'unité exacte (jetons ou demandes) n'est pas publiée.
- **Un mot, une demande, toujours** : l'API n'a pas de conversation gardée (`store: false` obligatoire). Faire « un grand chat » pour plusieurs mots obligerait à renvoyer tout l'historique à chaque mot : chaque traduction coûterait de plus en plus cher au forfait et serait plus lente. Une demande courte et indépendante par mot (consignes et exemples identiques, puis le mot et sa phrase) reste la bonne forme, comme pour les clés.
- **Beaucoup de mots touchés en quelques secondes** : une traduction attend 0,35 s avant de partir ; un autre mot touché entre-temps la remplace, rien n'est envoyé ni compté. Au plus 30 traductions par minute. Chaque traduction est gardée (un mot déjà traduit ne repart jamais). Limite du forfait atteinte : 15 minutes de pause, la traduction se fait avec l'IA de l'appareil, un message explique et mène à « Gérer l'utilisation ». Vérifié sans abonnement, avec un faux serveur d'OpenAI (`chatgpt::tests`) : 20 mots touchés à 50 ms d'intervalle → **une seule demande** part, celle du dernier mot, aucune erreur.
- **Le chat** de Lumen garde sa forme (la conversation, bornée, est renvoyée à chaque question) : c'est là que le forfait sert le plus.
- **À vérifier au premier essai réel** (un abonné Plus, en y allant doucement, compteur des demandes du jour sous les yeux dans Réglages) : le renouvellement des jetons (paramètre `resource`), le nom des modèles proposés (« mini » conseillé), l'effort de réflexion « none » accepté, l'adresse de « Gérer l'utilisation ».

## Distribution : une décision à prendre

| Voie | Coût | Ce que cela permet |
| --- | --- | --- |
| Xcode et un identifiant Apple gratuit | 0 | Lumen sur votre iPhone, à réinstaller depuis le Mac tous les 7 jours ; 3 apps au plus |
| Apple Developer Program | 99 € par an | TestFlight (versions de 90 jours, jusqu'à 10 000 testeurs), App Store, tous les droits (mémoire augmentée, iCloud), plus de réinstallation |
| AltStore PAL (Union européenne) | 99 € par an aussi (même compte), plus l'abonnement d'AltStore | Hors App Store : les fonctions YouTube y sont possibles |

L'App Store refuse les apps qui téléchargent des vidéos de YouTube : si Lumen y va, la version App Store n'aura pas l'import YouTube (les leçons YouTube viendront du Mac par la sauvegarde) ; une version hors App Store pourra l'avoir.

## Les étapes

0. **Fondations** — fait : projet Xcode, compilation, navigation iPhone, accueil, bibliothèque, lecteur, Sign in with ChatGPT.
1. **Toute l'interface à la main** : l'aperçu de Découvrir en feuille qui monte du bas (Chat, Réglages, Progrès, Vocabulaire et Importer sont faits) ; tourner les pages d'un balayage ; sélection de plusieurs mots au doigt ; carte du mot en feuille sous le pouce ; iPad avec clavier ; la visite guidée adaptée.
2. **IA locale sur iPhone** : Qwen3.5 sur un vrai appareil (vitesse, mémoire, chaleur), modes par appareil, régulateur thermique.
3. **La voix** : sherpa-onnx en bibliothèque, Supertonic 3 ; son des leçons en AAC par AVFoundation.
4. **La transcription** : Whisper dans l'app (ggml caché), Qwen3-ASR 0.6B et 1.7B, Parakeet v3 (aussi sur Mac et Windows), SpeechAnalyzer en option ; travail en arrière-plan.
5. **Fichiers et sauvegarde** : importer depuis Fichiers et depuis « Partager » (Safari, Mail), dossier de sauvegarde choisi dans iCloud Drive, retrouver la progression du Mac.
6. **ChatGPT sur iPhone** : écran de connexion intégré.
7. **YouTube**, si Lumen est distribué hors App Store.
8. **Android**, avec les mêmes briques.

## Pour travailler

- Simulateur : `node scripts/build-ios.mjs --sim` (« iPhone 17 Pro » par défaut, interface rechargée à chaud).
- Vérifier que tout compile pour iOS : `node scripts/build-ios.mjs --check`.
- Votre iPhone : brancher l'iPhone, ajouter votre identifiant Apple dans Xcode (Réglages › Comptes), puis `node scripts/build-ios.mjs --device`.
- Dans le navigateur, l'allure de l'iPhone avec le backend simulé : `http://localhost:1420/?platform=ios` (ou `ipad`), fenêtre étroite.
- Ne pas relancer `tauri ios init` : il réécrirait le projet Xcode (Accelerate ajouté à la main dans `project.yml`, icônes tirées de `design/icon-source.png`). Après une modification de `project.yml` : `xcodegen generate` dans `src-tauri/gen/apple`.
