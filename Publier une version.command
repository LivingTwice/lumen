#!/bin/bash
# Publie une nouvelle version de Lumen : compilation (Mac, puis Windows sur ce
# même Mac), signature, DMG et installateur Windows, envoi du code et de la version
# sur GitHub (dépôt LivingTwice/lumen). Les Lumen installés, sur Mac comme sur PC,
# la proposeront d'eux-mêmes. Il faut être connecté au compte GitHub du dépôt et
# avoir la clé ~/.tauri/lumen-updater.key : personne d'autre ne peut publier.
cd "$(dirname "$0")"
LOG="$(pwd)/publication.log"
exec > >(tee "$LOG") 2>&1
echo ""
echo "  ☀  Lumen : publier une nouvelle version"
echo "  ───────────────────────────────────────"
OWNER="LivingTwice"
SOURCE="$OWNER/lumen"
# ancien dépôt des versions : les Lumen jusqu'à 0.7.0 y lisent leurs mises à jour et
# leurs dictionnaires. Plus aucune version n'y est publiée, mais il doit rester public
# (ni privé, ni archivé) : son latest.json renvoie ces Lumen vers chaque nouvelle version.
RELAY="$OWNER/lumen-releases"
# seule la clé d'origine signe des mises à jour que les Lumen installés acceptent : sans
# elle, s'arrêter avant mac-env.sh (qui en créerait une nouvelle, refusée par tous)
KEY="$HOME/.tauri/lumen-updater.key"
if [ ! -f "$KEY" ]; then
  echo ""
  echo "✗ Clé de signature introuvable ($KEY) : seul l'ordinateur qui la garde peut publier une version."
  read -n 1 -s -r -p "Appuyez sur une touche pour fermer…"
  exit 1
fi
source scripts/mac-env.sh

step "GitHub"
command -v gh >/dev/null 2>&1 || "$BREW" install gh || fail "GitHub CLI n'a pas pu être installé."
if ! gh auth status >/dev/null 2>&1; then
  echo "  Connexion à GitHub : suivez les instructions (un code s'affiche, puis le navigateur s'ouvre)."
  gh auth login --hostname github.com --git-protocol https --web || fail "Connexion à GitHub impossible."
fi
echo "  ok : $(gh api user --jq .login)"

CURRENT="$(node -p 'require("./package.json").version')"
SUGGEST="$(node -p '(v=>{const p=v.split(".").map(Number);p[2]++;return p.join(".")})(require("./package.json").version)')"
echo ""
read -r -p "  Version actuelle $CURRENT. Nouvelle version [$SUGGEST] : " VERSION
VERSION="${VERSION:-$SUGGEST}"
# ce qui change : les nouveautés du journal (src/changelog.json), sinon une phrase
NOTES="$(node scripts/release.mjs preview "$VERSION" 2>/dev/null)"
if [ -n "$NOTES" ]; then
  echo ""
  echo "  Nouveautés de cette version (src/changelog.json) :"
  echo "$NOTES" | sed -n '1,/^---$/p' | grep -v '^---$' | sed 's/^/    /'
else
  read -r -p "  Ce qui change (une phrase) : " NOTES
  NOTES="${NOTES:-Améliorations et corrections.}"
fi

step "Numéro de version"
node scripts/release.mjs bump "$VERSION" || fail "Mise à jour du numéro de version impossible."
node scripts/release.mjs stamp "$VERSION" || fail "Les nouveautés n'ont pas pu être datées."

step "Compilation de la version $VERSION"
npm run app:build -- --bundles app || fail "La compilation a échoué."

step "Version Windows (compilée sur ce Mac, une vingtaine de minutes la première fois)"
WINDOWS=0
if node scripts/build-windows.mjs; then
  WINDOWS=1
else
  echo "  (version Windows non compilée : la version Mac est publiée seule)"
fi

step "Manifeste de mise à jour"
node scripts/release.mjs manifest "$VERSION" "$NOTES" "$ARCH" "$SOURCE" || fail "Manifeste impossible."

step "Image disque d'installation (DMG)"
npx tauri bundle --bundles dmg >/dev/null 2>&1 || echo "  (DMG non créé, la mise à jour automatique reste possible)"

step "Code source (dépôt public $SOURCE)"
[ -d .git ] || git init -q
git add -A && git commit -q -m "Lumen $VERSION" || true
git branch -M main 2>/dev/null
if ! git remote get-url origin >/dev/null 2>&1; then
  gh repo view "$SOURCE" >/dev/null 2>&1 || gh repo create "$SOURCE" --public --description "Lumen : code source" >/dev/null
  git remote add origin "https://github.com/$SOURCE.git"
fi
gh auth setup-git >/dev/null 2>&1
# la version est rattachée à ce commit : sans le code envoyé, on ne publie pas
git push -q -u origin main || fail "Envoi du code impossible : la version n'est pas publiée."

step "Publication sur GitHub"
ASSETS=("$BUNDLE/release/Lumen_${VERSION}_${ARCH}.app.tar.gz" "$BUNDLE/release/latest.json")
for d in "$BUNDLE"/dmg/Lumen_${VERSION}_*.dmg; do [ -f "$d" ] && ASSETS+=("$d"); done
WINSETUP="$BUNDLE/release/Lumen_${VERSION}_x64-setup.exe"
[ "$WINDOWS" = 1 ] && [ -f "$WINSETUP" ] && ASSETS+=("$WINSETUP")
gh release create "v$VERSION" "${ASSETS[@]}" --repo "$SOURCE" --target "$(git rev-parse HEAD)" --title "Lumen $VERSION" --notes "$NOTES" --latest || fail "La publication a échoué."

step "Relais pour les Lumen jusqu'à 0.7.0"
# ils lisent l'ancien dépôt : le latest.json de sa dernière version les envoie vers celle-ci
RELAY_TAG="$(gh release view --repo "$RELAY" --json tagName --jq .tagName 2>/dev/null)"
if [ -n "$RELAY_TAG" ] && gh release upload "$RELAY_TAG" "$BUNDLE/release/latest.json" --repo "$RELAY" --clobber >/dev/null 2>&1; then
  echo "  ok"
else
  echo "  (relais non mis à jour : les Lumen jusqu'à 0.7.0 ne verront pas cette version)"
fi

mkdir -p Distribution
cp "$BUNDLE"/dmg/Lumen_${VERSION}_*.dmg Distribution/ 2>/dev/null
[ -f "$WINSETUP" ] && cp "$WINSETUP" Distribution/
echo ""
echo "✓ Lumen $VERSION est publié : https://github.com/$SOURCE/releases/tag/v$VERSION"
echo "  Les Lumen installés proposeront la mise à jour dans les heures qui viennent (ou via Réglages › À propos)."
echo ""
read -n 1 -s -r -p "Appuyez sur une touche pour fermer…"
