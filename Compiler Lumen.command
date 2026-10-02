#!/bin/bash
# Compile Lumen, l'installe dans Applications et l'ouvre.
cd "$(dirname "$0")"
LOG="$(pwd)/compilation.log"
exec > >(tee "$LOG") 2>&1
echo ""
echo "  ☀  Lumen : compilation et installation"
echo "  ──────────────────────────────────────"
source scripts/mac-env.sh

step "Compilation (une dizaine de minutes la première fois)"
npm run app:build -- --bundles app || fail "La compilation a échoué."
APP="$BUNDLE/macos/Lumen.app"
[ -d "$APP" ] || fail "Lumen.app est introuvable après la compilation."

step "Installation dans Applications"
osascript -e 'tell application "Lumen" to quit' >/dev/null 2>&1
sleep 1
rm -rf "/Applications/Lumen.app"
ditto "$APP" "/Applications/Lumen.app" || fail "Impossible de copier Lumen dans Applications."
echo "  ok : /Applications/Lumen.app"

# (le DMG se fabrique après l'installation : cette étape efface Lumen.app du dossier de compilation)
step "Image disque d'installation (DMG)"
mkdir -p Distribution
if npx tauri bundle --bundles dmg >/dev/null 2>&1; then
  cp "$BUNDLE"/dmg/*.dmg Distribution/ 2>/dev/null && echo "  ok : dossier Distribution"
else
  echo "  (DMG non créé : autorisez le Terminal à contrôler le Finder si macOS le demande, puis relancez)"
fi

echo ""
echo "✓ Lumen est installé. Ouverture…"
open "/Applications/Lumen.app"
echo ""
read -n 1 -s -r -t 20 -p "Vous pouvez fermer cette fenêtre."
