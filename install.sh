#!/bin/bash
# Install roo to ~/bin
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DEST="$HOME/bin/roo"

mkdir -p "$HOME/bin"
cp "$SCRIPT_DIR/roo" "$DEST"
chmod +x "$DEST"

echo "installed roo → $DEST"
