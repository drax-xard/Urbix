#!/bin/sh
# server/test.sh
#
# Headless smoke test: boots the server on a throwaway port and curls the
# endpoints, asserting the JSON invariants a viewer depends on. No jq needed.
# Run from 3d-explorer-sdk/:
#   ./server/test.sh
set -u

PORT="${PORT:-18331}"
SEED=445566
cd "$(dirname "$0")/.." || exit 1

[ -x server/serve ] || { echo "server not built; run ./server/build.sh first"; exit 1; }

./server/serve --port "$PORT" --seed "$SEED" --web server/www >/dev/null 2>&1 &
SRV=$!
trap 'kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null' EXIT
sleep 0.5

fail=0
check() { # check <label> <got> <needle>
    if printf '%s' "$2" | grep -qF -- "$3"; then
        echo "PASS $1"
    else
        echo "FAIL $1 (missing: $3)"
        fail=1
    fi
}

cfg=$(curl -fsS "http://localhost:$PORT/api/config" 2>/dev/null)
check "/api/config zones"     "$cfg" '"zone_hues"'
check "/api/config seed"      "$cfg" "\"seed\":$SEED"

chunk=$(curl -fsS "http://localhost:$PORT/api/chunk?cx=0&cy=0" 2>/dev/null)
check "/api/chunk cells"      "$chunk" '"cell_count":1024'
check "/api/chunk cell0"      "$chunk" '"x":0,"z":0'
check "/api/chunk negative"   "$(curl -fsS "http://localhost:$PORT/api/chunk?cx=-1&cy=-1" 2>/dev/null)" '"cx":-1'

interior=$(curl -fsS "http://localhost:$PORT/api/interior?wx=1&wz=1" 2>/dev/null)
check "/api/interior floors"  "$interior" '"floor_count":'
check "/api/interior room"    "$interior" '"tiles":['

zone=$(curl -fsS "http://localhost:$PORT/api/zone?wx=100&wz=200" 2>/dev/null)
check "/api/zone weights"     "$zone" '"weights":['

code=$(curl -s -o /dev/null -w '%{http_code}' "http://localhost:$PORT/api/nope" 2>/dev/null)
[ "$code" = "404" ] && echo "PASS /api 404" || { echo "FAIL /api 404 (got $code)"; fail=1; }

trap - EXIT
kill "$SRV" 2>/dev/null
wait "$SRV" 2>/dev/null
[ "$fail" -eq 0 ] || { echo "test.sh: failures"; exit 1; }
echo "test.sh: all passed"