#!/usr/bin/env bash
# Installs Comic Reader for the current user: binary in ~/.local/bin, a
# .desktop launcher entry, an icon in the hicolor theme so it shows up with
# an icon in your application menu/taskbar/dock, and registers it as the
# default handler for .cbz/.cb7/.cbr (not the generic .zip/.7z/.rar
# extensions, which stay pointed at whatever you already use for those).
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
APPS_DIR="$HOME/.local/share/applications"
MIME_DIR="$HOME/.local/share/mime/packages"

mkdir -p "$BIN_DIR" "$ICON_DIR" "$APPS_DIR" "$MIME_DIR"

cp "$DIR/rusty_comic_reader" "$BIN_DIR/rusty_comic_reader"
chmod +x "$BIN_DIR/rusty_comic_reader"

cp "$DIR/comic-reader.png" "$ICON_DIR/comic-reader.png"
cp "$DIR/comic-reader-mime.xml" "$MIME_DIR/comic-reader.xml"

sed "s#__EXEC_PATH__#$BIN_DIR/rusty_comic_reader#" "$DIR/comic-reader.desktop" > "$APPS_DIR/comic-reader.desktop"

update-desktop-database "$APPS_DIR" 2>/dev/null || true
update-mime-database "$HOME/.local/share/mime" 2>/dev/null || true
gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

if command -v xdg-mime >/dev/null 2>&1; then
    xdg-mime default comic-reader.desktop application/x-cbz application/x-cb7 application/x-cbr
fi

echo "Installed. Launch \"Comic Reader\" from your application menu, or run: $BIN_DIR/rusty_comic_reader"
