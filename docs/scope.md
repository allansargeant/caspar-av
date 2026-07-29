# Scope — what is actually built

The honest breakdown. The headline caveat governs everything below:

> **caspar-AV has been verified against a real CasparCG 2.5.0 server** — Ubuntu
> 24.04 amd64, headless, OpenGL 4.5 via Mesa llvmpipe, under emulation. That
> covers the protocol, the telemetry and the output geometry, and it found six
> real bugs that testing against a self-written fake never would have.
>
> It has **not** been run on real output hardware (DeckLink, NDI, a projector),
> at broadcast resolutions, under sustained load, or in a live show. Emulated
> software rendering says nothing about frame timing on a real rig.

## What running it for real changed

Verification against a live server found six bugs. Every one was in code that
passed its own tests against a fake server written from the same reading of the
source — which is exactly why the fake could not catch them:

| Bug | Effect |
|---|---|
| `MIXER FILL 1-10 …` instead of `MIXER 1-10 FILL …` | **Every** `MIXER` and `CG` command returned `400`. The entire output-mapping and template model was inert. |
| `400 ERROR` treated as carrying no data | The echoed command line was left in the buffer, risking mis-framing of the next response. |
| `framerate` read as a float | It is a `,ii` rational; frame rate was always empty. |
| fps read from `file/fps` | It lives at `file/streams/0/fps`, as a rational. `file/fps` is consumer-side. |
| `file/frame`, `file/video/width` read | Never published for a producer; permanently empty. Frame numbers are now derived from `time × fps`. |
| Telemetry keys never expired | A colour producer loaded over a clip kept reporting the finished clip's name and position. |

Plus one dead field removed: `profiler/time` is never published, with or without
a consumer.

## Built and verified

**Protocol (`amcp`)** — 44 tests

- Command building with the server's real escaping rules, verified against
  `tokenize.cpp`: `\\`, `\"`, `\n`, quoting for spaces and parentheses.
- Response decoding framed by status code, including the `400`-echoes-its-command
  case, split reads and bare LF.
- Two-word commands placing the target between their words, and `PING` — which
  must be sent without a request id and answers with no status code.
- `REQ`/`RES` correlation, with a test that forces out-of-order replies.
- `BEGIN`/`COMMIT` batching: `BEGIN` unanswered, one reply per inner command,
  one `COMMIT` reply.
- Connection loss wakes waiters instead of hanging to the timeout.
- Typed builders for the full 2.5.0 command surface.

**Telemetry (`casparosc`)** — 20 tests

- OSC 1.0 decoding: all types, nested bundles, blob padding, malformed packets
  rejected rather than panicking.
- State tree with a typed digest; a leaf can become a branch as the server
  changes what it reports, and keys that stop arriving expire.
- Rational values (`framerate`, stream fps) decoded as num/den pairs.
- A transcript test built from real captured telemetry, not invented data.
- UDP listener that survives a bad datagram.

**media-scanner (`scanner`)** — 5 tests

- Media with ffprobe metadata, duration and resolution derivation, media kind.
- Unknown metadata fields preserved rather than dropped.
- Templates with GDD schemas; fonts; thumbnails; path encoding.

**Bridge (`showd`)** — 12 tests

- Show model: screens → `MIXER FILL`/`PERSPECTIVE`/`OPACITY`, cues → batches.
- Cue compilation, including whole-cue failure on an unknown screen.
- Show round-trips through JSON; a minimal file fills in defaults.
- Configuration warnings: two screens on one layer, a pad firing a missing cue.
- Supervised reconnect, telemetry subscription with fallback, atomic show saves.

**Console** — six pages, type-checked and built

Verified in a browser **against the real server**: connection and reconnect,
live telemetry with position/duration/fps, screen creation, drag-to-map writing
`MIXER FILL` through the API, transport commands, and the raw command line
round-tripping a 200 multi-line response.

**Protocol conformance** — `scripts/protocol-probe.py`, 22 checks, all passing
against CasparCG 2.5.0. Raw sockets, no shared code with the crates.

**Output geometry** — `scripts/verify-mapping.py`, driving a real server to
`PRINT` real frames: five `MIXER FILL` placements land exactly on the requested
cells, and `MIXER PERSPECTIVE` produces a genuine corner-pin wedge (left edge
8/8 rows lit, right edge 2/8).

## Partial

- **Cue auto-follow** — cues carry a `follow` time; nothing fires it. The field
  is stored and editable, and does nothing.
- **Mixer coverage** — the API accepts every mixer property; the console's
  inspector exposes fill, opacity and corner-pin. The rest are reachable only
  through the raw command line.
- **Template control** — add / update / next / stop work. `CG INVOKE` is in the
  API but has no UI. GDD forms handle strings and enums; nested objects and
  arrays fall back to the JSON editor.
- **Grid** — pads fire cues, keys 1–9 and 0 are bound. No MIDI, no page banks.
- **Canvas editor** — drag and resize write `MIXER FILL`. Corner-pin is numeric
  only; there is no drag-the-corners handle, which is the natural way to align a
  projector and is the most obvious missing gesture.

## Not started

- **Timeline / timecode.** No transport, no timecode lock, no LTC.
- **Soft-edge blending.** The commands exist; no mask generation, no UI.
- **Multi-server rigs.** One CasparCG server per daemon.
- **Control input.** No OSC-in, MIDI, Art-Net or sACN.
- **Audio.** Volume actions compile; no metering, no routing UI.
- **Recording / streaming consumers.** `ADD`/`REMOVE` exist in the command
  builders; no UI.
- **Release engineering.** No CI, no release workflow, no packaged binaries.
- **Show file versioning.** No schema version field yet.

## Known sharp edges

- **Show edits are last-write-wins.** Two consoles editing the same screen will
  clobber each other. Fine for one operator; not a merge model.
- **The media library refreshes on a 15-second timer.** A file added mid-show
  will not appear until it does, or until Rescan is pressed.
- **`Health::Down` appears only after a connection attempt completes**, so there
  is a brief honest "connecting" window at startup.
- **The command log holds 300 entries** in memory and is not persisted.
- **Telemetry freezes rather than clears when the server goes away.** Keys expire
  by packet count, and no packets arrive, so the Channels page keeps showing the
  last known positions. The header turns red and reads "CasparCG offline", but
  the frozen figures could still be misread at a glance.
- **The HTML/CEF producer was never exercised.** The test rig has no GPU, so
  CEF's GPU process dies on start. Templates were verified as AMCP command
  round-trips, not as rendered graphics.
- **`scripts/verify-mapping.py` needs the test VM**, since it reads back PNGs
  over `./ssh.sh`. It is kept as the record of how the geometry claim was
  checked, not as a portable test.
