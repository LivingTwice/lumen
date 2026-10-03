#!/bin/bash
# Publie une nouvelle version de Lumen : compilation, signature, DMG,
# envoi sur GitHub. Les Lumen installés la proposeront d'eux-mêmes.
cd "$(dirname "$0")"
LOG="$(pwd)/publication.log"
exec > >(tee "$LOG") 2>&1
echo ""
echo "  ☀  Lumen : publier une nouvelle version"
echo "  ───────────────────────────────────────"
OWNER="LivingTwice"
RELEASES="$OWNER/lumen-releases"
SOURCE="$OWNER/lumen"
source scripts/mac-env.sh

step "GitHub"
command -v gh >/dev/null 2>&1 || "$BREW" install gh || fail "GitHub CLI n'a pas pu être installé."
if ! gh auth status >/dev/null 2>&1; then
  echo "  Connexion à GitHub : suivez les instructions (un code s'affiche, puis le navigateur s'ouvre)."
  gh auth login --hostname github.com --git-protocol https --web || fail "Connexion à GitHub impossible."
fi
gh repo view "$RELEASES" >/dev/null 2>&1 || gh repo create "$RELEASES" --public --description "Versions de Lumen (installation et mises à jour)" >/dev/null || fail "Création du dépôt $RELEASES impossible."
echo "  ok : $(gh api user --jq .login)"

CURRENT="$(node -p 'require("./package.json").version')"
SUGGEST="$(node -p '(v=>{const p=v.split(".").map(Number);p[2]++;return p.join(".")})(require("./package.json").version)')"
echo ""
read -r -p "  Version actuelle $CURRENT. Nouvelle version [$SUGGEST] : " VERSION
VERSION="${VERSION:-$SUGGEST}"
read -r -p "  Ce qui change (une phrase) : " NOTES
NOTES="${NOTES:-Améliorations et corrections.}"

step "Numéro de version"
node scripts/release.mjs bump "$VERSION" || fail "Mise à jour du numéro de version impossible."

step "Compilation de la version $VERSION"
npm run app:build -- --bundles app || fail "La compilation a échoué."

step "Manifeste de mise à jour"
node scripts/release.mjs manifest "$VERSION" "$NOTES" "$ARCH" "$RELEASES" || fail "Manifeste impossible."

step "Image disque d'installation (DMG)"
npx tauri bundle --bundles dmg >/dev/null 2>&1 || echo "  (DMG non créé, la mise à jour automatique reste possible)"

step "Sauvegarde du code (dépôt privé)"
[ -d .git ] || git init -q
git add -A && git commit -q -m "Lumen $VERSION" || true
git branch -M main 2>/dev/null
if ! git remote get-url origin >/dev/null 2>&1; then
  gh repo view "$SOURCE" >/dev/null 2>&1 || gh repo create "$SOURCE" --private --description "Lumen : code source" >/dev/null
  git remote add origin "https://github.com/$SOURCE.git"
fi
gh auth setup-git >/dev/null 2>&1
git push -q -u origin main || echo "  (envoi du code impossible, la publication continue)"

step "Publication sur GitHub"
ASSETS=("$BUNDLE/release/Lumen_${VERSION}_${ARCH}.app.tar.gz" "$BUNDLE/release/latest.json")
for d in "$BUNDLE"/dmg/Lumen_${VERSION}_*.dmg; do [ -f "$d" ] && ASSETS+=("$d"); done
gh release create "v$VERSION" "${ASSETS[@]}" --repo "$RELEASES" --title "Lumen $VERSION" --notes "$NOTES" --latest || fail "La publication a échoué."

mkdir -p Distribution
cp "$BUNDLE"/dmg/Lumen_${VERSION}_*.dmg Distribution/ 2>/dev/null
echo ""
echo "✓ Lumen $VERSION est publié : https://github.com/$RELEASES/releases/tag/v$VERSION"
echo "  Les Lumen installés proposeront la mise à jour dans les heures qui viennent (ou via Réglages › À propos)."
echo ""
read -n 1 -s -r -p "Appuyez sur une touche pour fermer…"
