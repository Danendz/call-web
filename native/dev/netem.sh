#!/bin/sh
# Makes this Mac's UDP (so a call's media, not signaling) slow and lossy with dummynet, to see how calls hold up on
# a long, lossy link: two `join`s on this Mac, or the app against another device. Needs sudo.
#
#   sudo dev/netem.sh mild           200 ms round trip, 3% loss each way
#   sudo dev/netem.sh bad            300 ms round trip, 8% loss each way, 2 Mbit/s each way
#   sudo dev/netem.sh <rtt ms> <loss %> [kbit/s]
#   sudo dev/netem.sh stop           back to normal (also reloads /etc/pf.conf)
#   sudo dev/netem.sh status
#
# While it's on, pf runs only these rules: macOS's own pf anchors come back with `stop`.
set -eu

TOKEN=/tmp/call-danya-netem.token

stop() {
    dnctl -q flush 2>/dev/null || true
    pfctl -q -f /etc/pf.conf 2>/dev/null || true
    if [ -f "$TOKEN" ]; then
        pfctl -q -X "$(cat "$TOKEN")" 2>/dev/null || true
        rm -f "$TOKEN"
    fi
}

start() {
    rtt=$1 loss=$2 kbit=${3:-}
    # Every packet passes one pipe on its way: each holds half the round trip and drops its share
    delay=$((rtt / 2))
    plr=$(echo "$loss" | awk '{ printf "%.4f", $1 / 100 }')
    bw=""
    if [ -n "$kbit" ]; then bw="bw ${kbit}Kbit/s"; fi

    stop
    for pipe in 1 2 3; do
        # shellcheck disable=SC2086
        dnctl pipe $pipe config delay "$delay" plr "$plr" $bw
    done
    # On loopback (two joins on this Mac) a packet goes out and comes back in lo0: shaped once, going out. Elsewhere
    # only this side is shaped, so what comes in is shaped too.
    pfctl -q -f - <<EOF
dummynet-anchor "call-danya"
anchor "call-danya"
EOF
    pfctl -q -a call-danya -f - <<EOF
dummynet out quick on lo0 proto udp all pipe 1
dummynet out quick on ! lo0 proto udp all pipe 2
dummynet in quick on ! lo0 proto udp all pipe 3
EOF
    pfctl -E 2>&1 | sed -n 's/^Token : //p' > "$TOKEN"
    echo "UDP now: ${rtt} ms round trip, ${loss}% loss each way${kbit:+, ${kbit} kbit/s each way}. Undo with: sudo $0 stop"
}

if [ "$(id -u)" -ne 0 ]; then
    echo "Run it with sudo: sudo $0 ${*:-mild}" >&2
    exit 1
fi

case "${1:-}" in
    mild) start 200 3 ;;
    bad) start 300 8 2000 ;;
    stop) stop && echo "Back to normal." ;;
    status) dnctl list; pfctl -a call-danya -s dummynet 2>/dev/null ;;
    [0-9]*) start "$1" "${2:?loss %}" "${3:-}" ;;
    *) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 1 ;;
esac
