#!/bin/sh
# Opt-in, disposable VM user session; not an installed desktop or permission mock.
# Run inside dbus-run-session as an unprivileged test user with XDG_RUNTIME_DIR.
set -eu
: "${XDG_RUNTIME_DIR:?private runtime required}"
export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
export XDG_CURRENT_DESKTOP=sway XDG_SESSION_TYPE=wayland
cleanup() {
    for pid in ${portal_pid:-} ${backend_pid:-} ${wireplumber_pid:-} ${pipewire_pid:-} ${animation_pid:-} ${sway_pid:-}; do kill "$pid" 2>/dev/null || true; done
    wait || true
}
trap cleanup EXIT INT TERM
mkdir -p "$XDG_RUNTIME_DIR/config/xdg-desktop-portal-wlr"
printf '%s\n' '[screencast]' 'chooser_type=none' 'output_name=HEADLESS-1' 'max_fps=10' > "$XDG_RUNTIME_DIR/config/xdg-desktop-portal-wlr/config"
export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/config"
printf '%s\n' 'output HEADLESS-1 mode 1280x720' 'output HEADLESS-1 bg #345678 solid_color' > "$XDG_RUNTIME_DIR/sway.conf"
sway -c "$XDG_RUNTIME_DIR/sway.conf" > "$XDG_RUNTIME_DIR/sway.log" 2>&1 & sway_pid=$!
for attempt in $(seq 1 100); do
    socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'wayland-*' -type s | head -1)
    if [ -n "$socket" ]; then break; fi
    sleep .05
done
export WAYLAND_DISPLAY="$(basename "$socket")"
export SWAYSOCK="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'sway-ipc.*.sock' -type s | head -1)"
dbus-update-activation-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_RUNTIME_DIR XDG_CONFIG_HOME
pipewire > "$XDG_RUNTIME_DIR/pipewire.log" 2>&1 & pipewire_pid=$!
wireplumber > "$XDG_RUNTIME_DIR/wireplumber.log" 2>&1 & wireplumber_pid=$!
"${NMT_PORTAL_WLR:-/usr/libexec/xdg-desktop-portal-wlr}" -l "${NMT_PORTAL_LOG_LEVEL:-ERROR}" > "$XDG_RUNTIME_DIR/backend.log" 2>&1 & backend_pid=$!
/usr/libexec/xdg-desktop-portal > "$XDG_RUNTIME_DIR/portal.log" 2>&1 & portal_pid=$!
sleep 2
# Move the real compositor output so cadence can be observed as well as a first
# static frame. Only a synthetic solid background is ever placed on this output.
(while true; do swaymsg output HEADLESS-1 bg '#345678' solid_color >/dev/null; sleep .2; swaymsg output HEADLESS-1 bg '#456789' solid_color >/dev/null; sleep .2; done) & animation_pid=$!
"$@"
