# caspar-AV's hosted demo

caspar-AV drives a CasparCG server over AMCP and listens to its OSC telemetry,
so it can't be hosted in any useful sense — there is no server for a page on the
public internet to drive. What is hosted is a **click-through demo**: the real,
unmodified console, replaying responses recorded from `caspar-avd` itself
running against `scripts/fake-caspar.py`.

That fake speaks real AMCP framing, real `REQ`/`RES` correlation and pushes real
OSC, and stands in for media-scanner. So the demo isn't a mockup of caspar-AV —
it's caspar-AV's own output, captured.

**Firing a cue works.** `record-demo.sh` fires all five cues in `demo-show.json`
against the running bridge and records what it did, so clicking *Fire* in the
demo puts the bridge's actual AMCP into the command log — `PLAY 1-10
WALK_IN/LOGO_LOOP LOOP MIX 25` and the rest, exactly as it compiled them.

**Media thumbnails are real too**, recorded from the scanner and inlined by the
shim (they're `<img src>`, which never goes through `fetch`).

Nothing is live, and nothing is saved. Editing a cue, dragging a screen or
sending a raw AMCP command reports that the click went nowhere rather than
faking success.

## What's here

| File | What it is |
|---|---|
| `record-demo.sh` | Rebuilds everything: fake server, bridge, recording, assembly |
| `record-fixtures.mjs` | Records a running backend's reads, telemetry and writes (vendored) |
| `demo-shim.js` | Replays the recording over `fetch` / `WebSocket` / `<img>` (vendored) |
| `build-demo.sh` | Assembles the built console + shim + fixtures into a site (vendored) |
| `serve-demo.py` | Serves it with a static host's headers, for local checking (vendored) |
| `demo-fixtures.json` | The recording. Regenerate it; don't hand-edit it |
| `dist/` | **Committed build output** — what Cloudflare Pages serves |
| `dist/` | **Committed build output** — what Cloudflare Pages serves |

The vendored files come from `stoatworks-backend/pages-demo`. Fix them there and
copy out, or the copies drift.

## Rebuilding and publishing

```bash
demo/record-demo.sh            # fake server + bridge + record + assemble
demo/serve-demo.py --dir demo/dist  # check it locally first
git add demo/dist && git commit && git push
```

Cloudflare Pages publishes `demo/dist` from the repo with **no build command**.
It has to be committed: assembling the demo means running the bridge against a
fake CasparCG and capturing what it says, which a build container can't do.

## Rules the demo has to keep

- **It always says it's a demo.** The banner isn't optional.
- **Fixtures are recorded, never authored.** A hand-written fixture is a guess
  about what the software does, and guesses drift away from the code.
- **An unrecorded control says so.** Don't turn one into a fake success message —
  that's a demo showing behaviour the software doesn't have.
