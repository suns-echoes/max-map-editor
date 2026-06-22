#!/bin/sh
# M.A.X. Map Editor - optional Linux installer.
#
# Installs the portable directory into a fixed location and adds a desktop
# entry + icons, so the editor shows up in your application menu. The editor
# itself does NOT need installing - the unzipped folder runs as-is; this
# script is purely for desktop integration.
#
# Usage:  ./install.sh [DEST]
#         DEST defaults to ~/.local/share/max-map-editor
#
# What it does:
#   1. copies the app (binary, resources/, MANUAL.md, LICENSE) to DEST
#   2. asks for your M.A.X. game directory and writes it into
#      DEST/resources/config/mme.ini ([Paths] MaxPath)
#   3. installs the .desktop file and icons into ~/.local/share

set -eu

here="$(cd "$(dirname "$0")" && pwd)"
dest="${1:-$HOME/.local/share/max-map-editor}"

if [ ! -x "$here/max-map-editor" ]; then
	echo "error: max-map-editor binary not found next to install.sh" >&2
	echo "       run this script from inside the unzipped release folder" >&2
	exit 1
fi

# 1 - copy the app ----------------------------------------------------------
echo "Installing to: $dest"
mkdir -p "$dest"
cp -p "$here/max-map-editor" "$dest/"
cp -pR "$here/resources" "$dest/"
[ -f "$here/MANUAL.md" ] && cp -p "$here/MANUAL.md" "$dest/"
[ -f "$here/LICENSE" ] && cp -p "$here/LICENSE" "$dest/"
[ -f "$here/THIRD-PARTY-LICENSES.md" ] && cp -p "$here/THIRD-PARTY-LICENSES.md" "$dest/"

# 2 - MaxPath ----------------------------------------------------------------
printf "Path to your M.A.X. game directory (Enter to skip): "
read -r max_path
if [ -n "$max_path" ]; then
	if [ -d "$max_path" ]; then
		# Replace the MaxPath= line in place; the key always exists in the
		# shipped mme.ini. (This is the freshly-copied install, not your
		# accumulated overrides under resources/user/config.)
		sed -i "s|^MaxPath=.*|MaxPath=$max_path|" "$dest/resources/config/mme.ini"
		echo "MaxPath set to: $max_path"
	else
		echo "warning: '$max_path' is not a directory - skipped." >&2
		echo "         Set it later in $dest/resources/config/mme.ini" >&2
	fi
else
	echo "Skipped. Set it later in $dest/resources/config/mme.ini ([Paths] MaxPath)."
fi

# 3 - desktop entry + icons ---------------------------------------------------
apps="$HOME/.local/share/applications"
mkdir -p "$apps"
sed "s|^Exec=.*|Exec=$dest/max-map-editor|; s|^Path=.*|Path=$dest|" \
	"$here/resources/icons/max-map-editor.desktop" > "$apps/max-map-editor.desktop"

for size in 32 64 128 256 512; do
	icon_dir="$HOME/.local/share/icons/hicolor/${size}x${size}/apps"
	mkdir -p "$icon_dir"
	cp -p "$here/resources/icons/icon-$size.png" "$icon_dir/max-map-editor.png"
done

# Refresh caches when the tools exist; harmless to skip.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" || true

echo
echo "Done. 'M.A.X. Map Editor' is now in your application menu."
echo
echo "To uninstall, delete:"
echo "  $dest"
echo "  $apps/max-map-editor.desktop"
echo "  ~/.local/share/icons/hicolor/*/apps/max-map-editor.png"
