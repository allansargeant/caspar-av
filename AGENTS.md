# AGENTS.md — bringing an LLM up to speed on caspar-AV

Orientation for an AI assistant (or a new human) picking this project up cold. There is no
`CLAUDE.md` here; this is the entry point.

---

## 1. What this is

A **live-events media server built on CasparCG** — a show canvas with screens on it, cues
that change several outputs on one frame, a media library, GDD template forms and a trigger
grid, all in a browser.

CasparCG is presented as a *broadcast playout* engine, but the pieces needed to drive it as
a *live-events media server* are already in it and simply unexposed. caspar-AV adds the
layer above: a **show** — canvas, screens, cues, pads — that compiles down to AMCP.

Public repo, MIT, **v0.1.1**. Not affiliated with or endorsed by the CasparCG project.

## 2. Why a daemon and not just a web page

CasparCG speaks **AMCP over raw TCP** (5250) and pushes state as **OSC over UDP**. A browser
can do neither. So `caspar-avd` is a small Rust daemon holding those connections and serving
a React console over ordinary HTTP.

```
Browser console (React/Vite)
        │  REST + WebSocket (snapshot mirror)
caspar-avd (Rust)                     ← show model, cue compiler, command log
        │  AMCP/TCP 5250      │  OSC/UDP        │  HTTP 8000
CasparCG Server 2.5           telemetry          media-scanner
```

**The console is a passive mirror.** It holds no authoritative state — it renders the
daemon's snapshot and sends commands. That is what lets two operators on two laptops see the
same thing and a reconnecting browser be immediately correct. Do not move authority into the
console.

## 3. Layout

```
crates/
  amcp      Protocol codec + async client. Command building, response framing,
            REQ/RES correlation, BEGIN/COMMIT batching. 38 tests.
  osc       (casparosc) OSC decoder + telemetry state tree, raw plus typed digest. 13 tests.
  scanner   media-scanner HTTP client — media, ffprobe metadata, thumbnails,
            templates with GDD schemas, fonts. 5 tests.
  showd     (caspar-avd) The bridge: supervised connection, show model, cue
            compiler, REST + WebSocket API, serves the console. 12 tests.
  diag      Vendored diagnostics module — self-contained, copied across the fleet unchanged.
console/    React/Vite console, six pages on one shared frame. Ported from OpenStage's.
demo/       Recorded replay for the public demo — see §7.
docs/       architecture.md, decisions.md, amcp.md, scope.md, test-server.md, releasing.md
```

`docs/decisions.md` is a genuine decision log — read it before arguing with a design choice;
most of the surprising ones are already answered there.

## 4. AMCP traps that have already cost time

These are protocol facts, derived from the CasparCG 2.5.0 **source** rather than the wiki,
and then confirmed against a real server. Getting any of them wrong fails quietly.

- **Two-word commands put the target BETWEEN the words.** It is `MIXER 1-10 FILL`, not
  `MIXER FILL 1-10`. This one broke every `MIXER` and `CG` command until the real server
  caught it.
- **`REQ`/`RES` correlation is mandatory.** Replies are not ordered with respect to
  requests. Never match a response to a command by position.
- **`BEGIN` is never answered.** Waiting for a reply to it deadlocks. Only the `COMMIT`
  comes back.
- **A cue fails whole, never partially** — that is deliberate, see `decisions.md`.

## 5. Commands

```bash
cargo build --release
cd console && npm ci && npm run build && cd ..
./target/release/caspar-avd --host <caspar-host> --show show.json   # then localhost:8080
```

`.claude/launch.json` defines a `caspar-avd` preview config on port 8080.

There are two verification scripts, and they deliberately **share no code with the crates**
so they can actually disprove a claim:

- `scripts/protocol-probe.py` — checks 22 protocol claims over raw sockets.
- `scripts/verify-mapping.py` — has the server `PRINT` real frames to confirm `MIXER FILL`
  and `MIXER PERSPECTIVE` move actual pixels.
- `scripts/fake-caspar.py` — a protocol fixture that renders nothing. Good enough to develop
  against; not evidence of anything visual.

## 6. Status — state it precisely

Built, tested, and **verified against a real CasparCG 2.5.0 server** (Ubuntu 24.04, headless,
Mesa llvmpipe), which caught six genuine bugs. But it has **never run on real output hardware
or in a live show**, and no picture has ever come out of anything driven by it. Keep that
distinction intact in the README and in `docs/scope.md` — a new build does not make an
untested path tested.

**Not built:** timeline/timecode playback, auto-follow execution (cues carry a follow time;
nothing fires it), soft-edge blending UI, MIDI/OSC control in, multi-server rigs, audio
metering.

## 7. The demo is a recording

<https://caspar-av-demo.stoatworks-labs.com> replays a recording of `caspar-avd` running
against `scripts/fake-caspar.py`. Firing a cue really does compile AMCP and fill the command
log, but nothing is driving a server and nothing is saved. If you change the console's shape,
the recording goes stale — see `demo/README.md`.

## 8. Conventions

- Public repo. "Commit" means commit **and** push.
- Ships the user-facing AI disclaimer in the README.
- Releases are cut with the local release harness, not CI. The README's `## Download` block
  is generated between the `<!-- downloads:start -->` markers — don't hand-edit it.

## 9. Related

- **`caspar-test-vm`** — the disposable CasparCG 2.5 test server this is verified against.
  There is no macOS/arm64 CasparCG build, so it runs as Ubuntu **amd64** under QEMU TCG, and
  headless needs **Xvfb + llvmpipe**, not EGL.
- **`openstage`** — where the console's six-page frame came from.

## Diagnostics

Log through `crates/diag`, never `println!`. `diag` is deliberately self-contained and
dependency-light so it can be copied into the other repos unchanged — fix a bug here and
check whether the other copies need it too.
See [docs/diagnostics.md](docs/diagnostics.md).
