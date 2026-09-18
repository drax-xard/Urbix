#!/bin/sh
# server/test.sh
#
# Headless smoke test: boots the server on a throwaway port and curls the
# endpoints, asserting the JSON invariants a viewer depends on. No jq needed
# (python3 is used only to pick a built cell for the interior probes).
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
check "/api/config sdk"       "$cfg" '"sdk":"0.16.0"'
check "/api/config paving"    "$cfg" '"arterial_every"'

chunk=$(curl -fsS "http://localhost:$PORT/api/chunk?cx=0&cy=0" 2>/dev/null)
check "/api/chunk cells"      "$chunk" '"cell_count":1024'
check "/api/chunk cell0"      "$chunk" '"x":0,"z":0'
check "/api/chunk flags"      "$chunk" '"flags":'
check "/api/chunk negative"   "$(curl -fsS "http://localhost:$PORT/api/chunk?cx=-1&cy=-1" 2>/dev/null)" '"cx":-1'

batch=$(curl -fsS "http://localhost:$PORT/api/chunks?cx=0&cy=0&r=1" 2>/dev/null)
check "/api/chunks batch"     "$batch" '"chunks":['
check "/api/chunks count"     "$batch" '"cx":1'

# Find a built cell (h > 0) from chunk (0,0) for the interior probes.
wx=$(printf '%s' "$chunk" | python3 -c "
import json,sys
for c in json.load(sys.stdin)['cells']:
    if c['h'] > 0:
        print(c['x']); break
" 2>/dev/null)
wz=$(printf '%s' "$chunk" | python3 -c "
import json,sys
for c in json.load(sys.stdin)['cells']:
    if c['h'] > 0:
        print(c['z']); break
" 2>/dev/null)
if [ -z "${wx:-}" ]; then
    echo "FAIL interior probe (no built cell in chunk 0,0)"
    fail=1
else
    interior=$(curl -fsS "http://localhost:$PORT/api/interior?wx=$wx&wz=$wz" 2>/dev/null)
    check "/api/interior floors"  "$interior" '"floor_count":'
    check "/api/interior tiles"   "$interior" '"tiles":['
    check "/api/interior kinds"   "$interior" '"kinds":['
    check "/api/interior furn"    "$interior" '"furn":['

    rooms=$(curl -fsS "http://localhost:$PORT/api/rooms?wx=$wx&wz=$wz" 2>/dev/null)
    check "/api/rooms records"    "$rooms" '"rooms":['
    check "/api/rooms unit"       "$rooms" '"unit":'
fi

zone=$(curl -fsS "http://localhost:$PORT/api/zone?wx=100&wz=200" 2>/dev/null)
check "/api/zone weights"     "$zone" '"weights":['

code=$(curl -s -o /dev/null -w '%{http_code}' "http://localhost:$PORT/api/nope" 2>/dev/null)
[ "$code" = "404" ] && echo "PASS /api 404" || { echo "FAIL /api 404 (got $code)"; fail=1; }

trap - EXIT
kill "$SRV" 2>/dev/null
wait "$SRV" 2>/dev/null

# --config override path: chunky bands apply, CLI --seed wins over the file.
PORT2=$((PORT + 1))
./server/serve --port "$PORT2" --seed 7 --web server/www --config server/chunky.overrides >/dev/null 2>&1 &
SRV2=$!
trap 'kill "$SRV2" 2>/dev/null; wait "$SRV2" 2>/dev/null' EXIT
sleep 0.5
cfg2=$(curl -fsS "http://localhost:$PORT2/api/config" 2>/dev/null)
check "/api/config chunky"   "$cfg2" '"height_max":[110.0,14.0,45.0,20.0,2.0]'
check "/api/config cliseed"  "$cfg2" '"seed":7'
trap - EXIT
kill "$SRV2" 2>/dev/null
wait "$SRV2" 2>/dev/null

[ "$fail" -eq 0 ] || { echo "test.sh: failures"; exit 1; }
echo "test.sh: all passed"
