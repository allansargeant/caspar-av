#!/usr/bin/env bash
#
# Rebuild caspar-AV's hosted demo end to end.
#
# Runs the bridge against scripts/fake-caspar.py — which speaks real AMCP
# framing, real REQ/RES correlation and pushes real OSC telemetry, and stands in
# for media-scanner — then records what caspar-avd actually serves, fires every
# cue in the demo show and records the bridge's real reaction to each.
#
# The result is committed to demo/dist and served by Cloudflare Pages straight
# from the repo: assembling it means running the bridge against a fake server,
# which a build container can't do, so the built output lives in git.
set -euo pipefail

cd "$(dirname "$0")/.."

# Ports deliberately off the defaults so recording can't attach to a real
# CasparCG, or to a fake one someone already has running.
AMCP_PORT=5251
SCANNER_PORT=8001
BIND=127.0.0.1:8097
BASE="http://$BIND"
CUES=(walk-in opener keynote break blackout)

echo "==> Building caspar-avd and the console"
cargo build -q --release --bin caspar-avd
(cd console && npm ci --silent && npm run build)

echo "==> Starting the fake CasparCG + media-scanner"
python3 scripts/fake-caspar.py --port "$AMCP_PORT" --scanner-port "$SCANNER_PORT" \
  >/tmp/caspar-av-demo-fake.log 2>&1 &
FAKE_PID=$!
sleep 2

echo "==> Starting caspar-avd"
./target/release/caspar-avd \
  --show demo-show.json --bind "$BIND" \
  --port "$AMCP_PORT" --scanner-port "$SCANNER_PORT" --web web \
  >/tmp/caspar-av-demo-avd.log 2>&1 &
AVD_PID=$!
cleanup() { kill "$AVD_PID" "$FAKE_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 60); do
  curl -sf "$BASE/api/state" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "$BASE/api/state" >/dev/null || {
  echo "error: caspar-avd did not start; see /tmp/caspar-av-demo-avd.log" >&2; exit 1; }

POST_ARGS=()
for cue in "${CUES[@]}"; do POST_ARGS+=(--post "/api/cues/$cue/fire"); done

echo "==> Recording"
node demo/record-fixtures.mjs \
  --base "$BASE" \
  --app "caspar-AV" --repo "https://github.com/stoatworks-labs/caspar-av" \
  --get /api/state --get /api/telemetry --get /api/show \
  --expand '/api/state:media[].name:/api/media/{}/thumbnail' \
  --ws /ws/ui --ws-seconds 12 \
  "${POST_ARGS[@]}" \
  --out demo/demo-fixtures.json

echo "==> Assembling the site"
demo/build-demo.sh --src web --fixtures demo/demo-fixtures.json --out demo/dist

echo
echo "Check it as a static host will serve it:"
echo "  demo/serve-demo.py --dir demo/dist"
echo "Then commit demo/dist — Cloudflare Pages publishes it straight from the repo."
