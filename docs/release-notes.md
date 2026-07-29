The first release: a live-events media server built on **CasparCG** as the render
engine — screens on a canvas, frame-accurate cues, a media library, GDD-driven
template forms and a trigger grid, in a browser.

CasparCG speaks AMCP over raw TCP and pushes state as OSC over UDP, neither of
which a browser can do, so this is a small Rust bridge daemon (`caspar-avd`)
that holds the connection and serves a React console over ordinary HTTP.

## What it does

| Page | |
|---|---|
| **Media** | Library from media-scanner with thumbnails; double-click to play |
| **Screens** | Output mapping — drag to write `MIXER FILL`, corner-pin writes `MIXER PERSPECTIVE` |
| **Channels** | Live state from OSC, plus a raw AMCP command line |
| **Cues** | Fired as one `BEGIN`/`COMMIT` batch, so every screen changes on the same frame |
| **Templates** | Data forms generated from a template's own GDD schema |
| **Grid** | Cues as trigger pads |

## Verified against a real server

Developed against the CasparCG 2.5.0 source, then checked against a **running
2.5.0 server** with two tools that share no code with the crates — a raw-socket
protocol probe (22/22 checks) and a geometry verifier that has the server `PRINT`
real frames. That found six genuine bugs, including one that made **every**
`MIXER` and `CG` command fail: a two-word command puts its target *between* its
words (`MIXER 1-10 FILL …`, not `MIXER FILL 1-10 …`).

`docs/amcp.md` records the protocol as it actually behaves, including several
points where the community wiki is wrong.

**Not yet verified:** real output hardware (DeckLink, NDI, projectors),
broadcast resolutions, sustained load, or a live show. The HTML/CEF producer was
never exercised — the test rig had no GPU. See `docs/scope.md`.

## Running it

```bash
tar xzf caspar-av-<platform>.tar.gz
./caspar-avd --host <your-caspar-host> --show myshow.json
```

Then open <http://localhost:8080>. No CasparCG to hand? `python3
scripts/fake-caspar.py` speaks real AMCP framing and pushes real OSC telemetry.

Against your own server, `python3 scripts/protocol-probe.py --host <host>`
re-checks every protocol assumption this build relies on.

## Notes

- Needs **media-scanner** for the media library, templates and thumbnails —
  CasparCG 2.5 proxies `CLS`/`TLS`/`CINF`/`THUMBNAIL` to it and has no listing of
  its own. The console says so plainly when it is not running.
- Binaries are unsigned. On macOS, clear the quarantine flag before running:
  `xattr -dr com.apple.quarantine caspar-avd`.
- Linux and Windows binaries are cross-compiled from macOS and compiled-only —
  they have not been executed on those platforms.
