#!/usr/bin/env bash
# Installs Comic Reader for the current user: binary in ~/.local/bin, a
# .desktop launcher entry, and an icon in the hicolor theme so it shows up
# with an icon in your application menu/taskbar/dock.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
APPS_DIR="$HOME/.local/share/applications"

mkdir -p "$BIN_DIR" "$ICON_DIR" "$APPS_DIR"

cp "$DIR/rusty_comic_reader" "$BIN_DIR/rusty_comic_reader"
chmod +x "$BIN_DIR/rusty_comic_reader"

cp "$DIR/comic-reader.png" "$ICON_DIR/comic-reader.png"

sed "s#__EXEC_PATH__#$BIN_DIR/rusty_comic_reader#" "$DIR/comic-reader.desktop" > "$APPS_DIR/comic-reader.desktop"

update-desktop-database "$APPS_DIR" 2>/dev/null || true
gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Installed. Launch \"Comic Reader\" from your application menu, or run: $BIN_DIR/rusty_comic_reader"
