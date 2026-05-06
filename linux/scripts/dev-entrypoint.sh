#!/usr/bin/env bash
set -euo pipefail

if [ -z "${DISPLAY:-}" ]; then
    export DISPLAY=:99
fi

if ! pgrep -x Xvfb >/dev/null 2>&1; then
    rm -f /tmp/.X*-lock /tmp/.X11-unix/X* 2>/dev/null || true
    Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp +extension RANDR &
    for _ in $(seq 1 50); do
        if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
            break
        fi
        sleep 0.05
    done
fi

if ! pgrep -x fluxbox >/dev/null 2>&1; then
    mkdir -p /root/.fluxbox
    if [ ! -f /root/.fluxbox/init ]; then
        printf '%s\n' 'session.screen0.rootCommand: true' > /root/.fluxbox/init
    fi
    fluxbox >/dev/null 2>&1 &
fi

if ! pgrep -x x11vnc >/dev/null 2>&1; then
    VNC_PASS="${VNC_PASSWORD:-nostrvpn}"
    mkdir -p /root/.vnc
    x11vnc -storepasswd "$VNC_PASS" /root/.vnc/passwd >/dev/null 2>&1
    x11vnc -display "$DISPLAY" -forever -shared -rfbport 5900 \
        -rfbauth /root/.vnc/passwd -bg -quiet \
        -noxdamage -noxrecord -noxfixes >/dev/null 2>&1
fi

DBUS_SOCK=/tmp/nostr-vpn-dbus.sock
DBUS_ADDR="unix:path=$DBUS_SOCK"
if ! pgrep -f "dbus-daemon.*$DBUS_SOCK" >/dev/null 2>&1; then
    rm -f "$DBUS_SOCK"
    dbus-daemon --session --fork --address="$DBUS_ADDR" --nopidfile --nosyslog
fi
export DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDR"

cat >/etc/profile.d/nostr-vpn-dbus.sh <<EOF
export DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDR"
EOF
chmod 0644 /etc/profile.d/nostr-vpn-dbus.sh

if [ "$#" -eq 0 ]; then
    exec bash
fi

exec "$@"
