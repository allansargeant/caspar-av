# Architecture

## Shape

```
┌ Browser ─────────────────────────────────────────────────┐
│ console/ — React + Vite, six pages on one shared frame   │
└───────────────┬──────────────────────────────────────────┘
                │ GET /api/state · WS /ws/ui  (whole snapshot)
                │ POST/PATCH/DELETE /api/…    (commands)
┌ caspar-avd ───┴──────────────────────────────────────────┐
│ api.rs      HTTP + WebSocket, serves the built console   │
│ bridge.rs   supervised connection, snapshot, command log │
│ show.rs     canvas · screens · cues · pads → AMCP        │
└──┬──────────────────┬───────────────────┬────────────────┘
   │ AMCP/TCP 5250    │ OSC/UDP           │ HTTP 8000
   │                  │ (ephemeral port)  │
┌──┴──────────────────┴───┐          ┌────┴──────────┐
│ CasparCG Server 2.5     │          │ media-scanner │
└─────────────────────────┘          └───────────────┘
```

Four crates:

| Crate | Responsibility |
|---|---|
| `amcp` | Protocol codec and async client. Knows nothing about shows. |
| `casparosc` | OSC decoding and the telemetry state tree. |
| `scanner` | media-scanner HTTP client. |
| `showd` | The daemon: bridge, show model, API. Binary `caspar-avd`. |

The split is deliberate: `amcp` and `casparosc` are reusable by anything that
needs to talk to CasparCG, and neither depends on the show model. If the show
layer turns out to be wrong, the protocol work survives it.

## The snapshot contract

One type, `Snapshot`, is the entire read interface. The console renders it and
holds no authoritative state of its own.

```rust
struct Snapshot {
    health: Health,            // connecting | connected | down — of CasparCG
    server: ServerInfo,        // host, port, version, paths, OSC port in use
    channels: Vec<ChannelState>, // live, from OSC
    media: Vec<MediaItem>,     // from media-scanner
    templates: Vec<Template>,  // with GDD schemas where published
    fonts: Vec<String>,
    scanner_up: bool,
    show: Show,                // the intent
    warnings: Vec<String>,     // configuration problems worth pre-empting
    log: Vec<LogEntry>,        // recent commands and their replies
}
```

Two health signals are kept separate on purpose, because conflating them sends
people to the wrong fault:

- the **browser's** connection to `caspar-avd` (`ConnStatus` in the console);
- **CasparCG's** connection to the daemon (`health` in the snapshot).

A third is surfaced too: telemetry can be absent while commands work perfectly,
so the console says "no telemetry" rather than showing an empty channel list.

The WebSocket pushes the snapshot on a 200 ms tick, but only when the serialised
form changed. Crude, and exactly right at this size: an idle rig costs one
comparison per tick instead of a frame's worth of traffic. The console falls
back to polling `/api/state` when the socket cannot be kept up — a proxy that
will not forward upgrades is a real thing on a show network.

## The show model

Caspar knows channels and layers. A show needs screens on a canvas, and cues.

```
Show
├── canvas   width × height (pixels, for display; maths is normalised 0..1)
├── screens  [ Screen { id, name, channel, layer, rect, corners, enabled, opacity } ]
├── cues     [ Cue { id, name, actions[], follow, colour } ]
├── pads     [ Pad { index, cue } ]
└── grid     (cols, rows)
```

A **screen** is the unit of output. It compiles to:

| Field | Command |
|---|---|
| `rect` | `MIXER FILL c-l x y w h` |
| `corners` (when not identity) | `MIXER PERSPECTIVE c-l x1 y1 … x4 y4` |
| `enabled`, `opacity` | `MIXER OPACITY c-l v` |

Identity corners are deliberately *not* sent: an identity corner-pin still costs
a transform on the layer and makes the server's state harder to read when
debugging a rig.

A **cue** is a list of actions compiled to commands and sent as one
`BEGIN`/`COMMIT` batch. That is the whole reason cues exist here rather than
being a list of buttons — the server locks every touched channel and releases
them on the same frame.

An action naming an unknown screen fails the **entire cue** rather than firing
the actions that happen to resolve. Half a cue on stage is worse than none, and
the operator is told which screen is missing.

`Action::Raw` carries any AMCP command verbatim. Every professional media server
needs an escape hatch; without one, the model's gaps become dead ends.

## Show intent vs. live state

The show is **intent**; live state comes back over OSC. They are never merged.

This is what allows two consoles to agree, and it is why dragging a screen shows
the *requested* position immediately while the Channels page shows what the
server is actually doing. If they disagree, that is information, not a bug to
paper over.

## Connection supervision

`Bridge::supervise` reconnects with exponential backoff to 5 s — CasparCG gets
restarted mid-rig routinely, and the console should recover without a reload. On
connect it identifies the server (`VERSION`, `INFO PATHS`), then negotiates
telemetry:

1. bind an ephemeral UDP port;
2. `OSC SUBSCRIBE <that port>`;
3. if refused (a pre-2.5 server), fall back to the shared port 6250;
4. if that is taken too, carry on without telemetry and say so.

Step 3 matters because on a machine also running the Caspar client, port 6250 is
already spoken for.

## Persistence

The show is a JSON file, written by `--show <path>`. Saving is polled every two
seconds and only on change, written to a temporary file and renamed, so an
interrupted save cannot leave a half-written show. Edits arrive in bursts while
someone drags a screen; a show file is not worth an fsync per frame.

## The console

Ported from [OpenStage](https://github.com/allansargeant/openstage)'s console —
the shell, the connection layer with its polling fallback, the inspector idiom,
the `?window=<page>` pop-out for a second display, and the palette.

The page model is Resolve's: a bottom page-tab bar, and every page built from one
five-region `Frame` (toolbar / left / centre / right / bottom). The bottom dock
is the command log on every page — what was sent, what came back — because on a
show, "did that command actually land?" is the question that gets asked.

What did **not** port is page content: OpenStage's domain is render nodes, sync
groups and a show canvas; this one's is channels, layers, media and cues.
