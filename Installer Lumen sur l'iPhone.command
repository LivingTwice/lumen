#!/bin/bash
# Installe Lumen sur l'iPhone (ou l'iPad) branché à ce Mac, sans passer par l'App Store.
# Il faut : Xcode, votre identifiant Apple ajouté dans Xcode › Réglages › Comptes (gratuit),
# l'iPhone branché par câble, déverrouillé, « Se fier à cet ordinateur », mode développeur activé.
# Avec un identifiant gratuit, l'app s'ouvre 7 jours : relancez simplement ce fichier ensuite.
cd "$(dirname "$0")"
LOG="$(pwd)/installation-iphone.log"
exec > >(tee "$LOG") 2>&1
echo ""
echo "  ☀  Lumen : installation sur l'iPhone"
echo "  ─────────────────────────────────────"
[ -d node_modules ] || npm install
node scripts/build-ios.mjs --install
echo ""
read -n 1 -s -r -p "Vous pouvez fermer cette fenêtre."
