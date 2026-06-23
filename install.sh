#!/bin/bash
# Install whiskd to ~/bin
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DEST="$HOME/bin/whiskd"

mkdir -p "$HOME/bin"
cp "$SCRIPT_DIR/whiskd" "$DEST"
chmod +x "$DEST"

echo "installed whiskd → $DEST"
