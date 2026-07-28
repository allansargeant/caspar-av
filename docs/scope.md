# Scope — what is actually built

The honest breakdown. The headline caveat governs everything below:

> **caspar-AV has never been run against a real CasparCG server.** It was
> developed and verified against `scripts/fake-caspar.py`, which implements the
> protocol behaviours taken from the 2.5.0 source — real framing, real
> `REQ`/`RES` correlation, real `BEGIN`/`COMMIT` semantics, real OSC bundles.
> That validates the bridge's *logic*. It does not validate assumptions about
> how a real server behaves under load, with real media, or with real hardware
> outputs.

## Built and verified

**Protocol (`amcp`)** — 38 tests

- Command building with the server's real escaping rules, verified against
  `tokenize.cpp`: `\\`, `\"`, `\n`, quoting for spaces and parentheses.
- Response decoding framed by status code, including split reads and bare LF.
- `REQ`/`RES` correlation, with a test that forces out-of-order replies.
- `BEGIN`/`COMMIT` batching: `BEGIN` unanswered, one reply per inner command,
  one `COMMIT` reply.
- Connection loss wakes waiters instead of hanging to the timeout.
- Typed builders for the full 2.5.0 command surface.

**Telemetry (`casparosc`)** — 13 tests

- OSC 1.0 decoding: all types, nested bundles, blob padding, malformed packets
  rejected rather than panicking.
- State tree with a typed digest; a leaf can become a branch as the server
  changes what it reports.
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

Verified in a browser against the fake server: connection and reconnect, live
telemetry rendering with position and frame time, screen creation, drag-to-map
writing `MIXER FILL` through the API, transport commands, and the raw command
line round-tripping a 200 multi-line response.

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
