#!/bin/bash
set -e

echo "Uninstalling Auralis..."

rm -rf "$HOME/.local/share/auralis"
rm -f "$HOME/.local/bin/auralis"
rm -f "$HOME/.local/share/applications/auralis.desktop"
rm -f "$HOME/.local/share/icons/auralis.png"

update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

echo "Auralis uninstalled."
