#!/bin/sh
# Build and install AULA F87 Control (Rust) for the current user:
#   - the aula-f87-control command in ~/.local/bin
#   - the "AULA F87 Control" app-menu entry and icons
#   - start at login in the background, with the top-bar icon
#     (turn it off from the icon's menu: "Start at login")
#
# Usage: ./install.sh                 build, install, and start the app in the background
#        ./install.sh --no-autostart  the same, without starting at login
#        ./install.sh --remove        uninstall
#
# Building needs Rust (https://rustup.rs) and the GTK 4 / libadwaita development files:
#   Ubuntu/Debian: sudo apt install libgtk-4-dev libadwaita-1-dev pipewire-bin
#   Fedora:        sudo dnf install gtk4-devel libadwaita-devel pipewire-utils
#   Arch:          sudo pacman -S gtk4 libadwaita pipewire
set -e

here=$(cd "$(dirname "$0")" && pwd)
app_id="io.github.aula_f87.Control"
bin="$HOME/.local/bin/aula-f87-control"
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$app_id.desktop"

# Ask a running copy (Rust or Python) to quit; it answers on D-Bus.
quit_running() {
    gdbus call --session --dest "$app_id" --object-path /io/github/aula_f87/Control \
        --method org.gtk.Actions.Activate quit '[]' '{}' >/dev/null 2>&1 && sleep 1
}

if [ "$1" = "--remove" ]; then
    quit_running || true
    [ -x "$bin" ] && "$bin" setup --remove
    rm -f "$bin"
    if [ -e "$bin-previous" ]; then
        mv "$bin-previous" "$bin"
        echo "Put back the previous aula-f87-control command."
    fi
    exit 0
fi

if [ -z "${AULA_F87_BINARY:-}" ]; then
    command -v cargo >/dev/null 2>&1 || { echo "Rust is needed to build: https://rustup.rs"; exit 1; }
    (cd "$here" && cargo build --release --locked)
    AULA_F87_BINARY="$here/target/release/aula-f87-control"
fi
command -v pw-record >/dev/null 2>&1 || echo "Music sync needs pw-record: sudo apt install pipewire-bin"

mkdir -p "$(dirname "$bin")"
# An earlier install (e.g. the Python app's launcher) is kept as aula-f87-control-previous.
if [ -e "$bin" ] || [ -L "$bin" ]; then
    if ! cmp -s "$bin" "$AULA_F87_BINARY"; then
        [ -e "$bin-previous" ] || [ -L "$bin-previous" ] || { mv "$bin" "$bin-previous"; echo "Kept the previous command as $bin-previous"; }
    fi
fi
rm -f "$bin"
cp "$AULA_F87_BINARY" "$bin"
chmod +x "$bin"

if [ "$1" = "--no-autostart" ]; then
    mkdir -p "$(dirname "$autostart")"
    printf '[Desktop Entry]\nType=Application\nName=AULA F87 Control\nExec=%s --background\nHidden=true\n' "$bin" > "$autostart"
fi
"$bin" setup

# Restart a running copy so the update takes effect, and start it now (top-bar
# icon only) when start at login is on.
if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    was_running=0
    quit_running && was_running=1
    if ! grep -q '^Hidden=true' "$autostart" 2>/dev/null || [ "$was_running" = 1 ]; then
        (setsid "$bin" --background >/dev/null 2>&1 &) || true
    fi
fi
echo "Installed. Open \"AULA F87 Control\" from the app menu, or run: aula-f87-control"
