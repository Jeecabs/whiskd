#!/bin/bash
# Build whiskd and install it to ~/bin
set -e

cd "$(dirname "$0")"
DEST="$HOME/bin/whiskd"

cargo build --release
mkdir -p "$HOME/bin"
# install(1) writes a new file instead of overwriting in place, which would
# invalidate the code signature of a running binary on macOS.
install -m 755 target/release/whiskd "$DEST"

echo "installed whiskd → $DEST"
