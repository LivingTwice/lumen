#!/bin/bash
# Compile la version Windows de Lumen sur ce Mac : l'installateur à copier sur
# un PC (Windows 10 ou 11) pour l'essayer. Rien n'est publié.
cd "$(dirname "$0")"
LOG="$(pwd)/compilation-windows.log"
exec > >(tee "$LOG") 2>&1
echo ""
echo "  ☀  Lumen : compilation pour Windows"
echo "  ───────────────────────────────────"
source scripts/mac-env.sh

step "Version Windows (une vingtaine de minutes la première fois)"
node scripts/build-windows.mjs || fail "La compilation pour Windows a échoué."
VERSION="$(node -p 'require("./package.json").version')"
SETUP="src-tauri/target/windows/x86_64-pc-windows-msvc/release/bundle/nsis/Lumen_${VERSION}_x64-setup.exe"
mkdir -p Distribution
cp "$SETUP" Distribution/ || fail "L'installateur est introuvable."

echo ""
echo "✓ Installateur Windows : Distribution/Lumen_${VERSION}_x64-setup.exe"
echo "  Copiez-le sur un PC et double-cliquez dessus. Au premier lancement, Windows peut"
echo "  avertir (« Windows a protégé votre ordinateur ») : Informations complémentaires › Exécuter quand même."
open Distribution
echo ""
read -n 1 -s -r -t 30 -p "Vous pouvez fermer cette fenêtre."
