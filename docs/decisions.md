# Decision log

Every significant choice, why it won, and what would change it.

## A bridge daemon, not a pure front end

**Forced, not chosen.** CasparCG speaks AMCP over raw TCP and pushes OSC over
UDP. A browser can do neither, and no amount of front-end work changes that. The
only question was what the daemon should own — and once it exists, it is the
right place for the show model, since that makes the console a mirror and lets
two operators agree.

## Rust, axum, and the OpenStage shape

Matches the rest of this fleet, and the snapshot-mirror pattern was already
proven against a live orchestrator in OpenStage. Reusing a shape that survived
contact with a real rig beat designing a new one.

## Verify against a real server, with code that shares nothing

The crates were built from the 2.5.0 source, tested against a fake server
written from *the same reading* of that source, and were entirely
self-consistent — while every `MIXER` and `CG` command was silently malformed.
A fake built from your own understanding can only confirm your understanding.

So `scripts/protocol-probe.py` uses raw sockets and none of this project's code,
and `scripts/verify-mapping.py` checks rendered pixels rather than status codes.
Both are able to prove the crates wrong, which is the only property that
mattered. Between them they found six bugs; see
[scope.md](scope.md#what-running-it-for-real-changed).

The probe's own expectations were wrong in the same way at first — it was written
from the same mistaken reading. It still caught the bug, because it asserted on
*observed bytes* rather than on what the code produced.

## Protocol facts from the source, not the wiki

The community wiki lags 2.5.0 and is wrong or silent on things that break a
bridge: response framing by status code, `REQ`/`RES` correlation, `BEGIN` never
being answered, and the tokenizer dropping unknown escapes. Every protocol
behaviour in `docs/amcp.md` cites the file and line it came from.

**Consequence:** when 2.6 lands, re-read `AMCPCommandsImpl.cpp` and
`AMCPProtocolStrategy.cpp` rather than the wiki.

## Correlate replies by `REQ` id, always

The tempting implementation is a FIFO queue of waiting callers. It is wrong: the
server dispatches to one queue *per channel*, so a channel-2 reply can overtake a
channel-1 reply. FIFO matching looks perfect in a demo and mis-attributes replies
under load — the worst class of bug, because the symptom appears somewhere else.

Cost: nine extra bytes per command. Taken without hesitation.

## Cues are `BEGIN`/`COMMIT` batches

A cue that loops over commands smears a multi-screen change across several
frames. `COMMIT` locks every touched channel and releases them together. This is
the single feature that makes CasparCG viable as a show media server rather than
a playout engine, and it is why the cue model exists at all.

## Screens map through `MIXER FILL` + `MIXER PERSPECTIVE`

Rejected: driving output geometry through channel `SET MODE` and separate
consumers per output. Fill and perspective are per-*layer*, animatable with a
duration and easing, and already exist. Corner-pin is genuine four-corner warp —
projector keystone with no external hardware.

**Open:** soft-edge blending. `MIXER CLIP` and blend modes give the pieces, but a
real edge blend needs a gradient mask per edge, probably as a template or an
image layer. Not designed yet.

## Normalised canvas units, pixels carried alongside

Mapping maths is 0..1, matching `MIXER FILL`. The canvas also carries pixel
dimensions so the console can say "3840×1080" rather than showing unitless
numbers. This is the same call OpenStage made, and the same reasoning: nothing in
the protocol needs pixels, but operators think in them.

## Claim a private OSC port

`OSC SUBSCRIBE <port>` (2.5+) instead of listening on the shared 6250. On a
machine also running the Caspar client — which is most commissioning machines —
6250 is contended, and two listeners on one port is a confusing failure. Falls
back to 6250 when the server refuses, and runs without telemetry if that fails
too, saying so rather than showing an empty screen.

## Keep the raw OSC tree as well as a typed digest

Caspar's key set varies by version, producer and consumer. A bridge that only
understood a fixed list would silently drop anything it had not been taught. The
tree costs little and makes `/api/telemetry` a real diagnostic.

## Talk to media-scanner directly over HTTP

The AMCP route (`CLS`/`TLS`/`CINF`/`THUMBNAIL`) is proxied to the scanner anyway,
and loses information doing it: text instead of JSON, base64-in-a-status-line
instead of a PNG, and no GDD schemas at all. Going direct gets full ffprobe
metadata and template schemas.

**Consequence:** two dependencies to explain instead of one. Handled by saying so
plainly in the UI when the scanner is down — that specific confusion ("my media
is missing") is the most common CasparCG support question there is.

## Expire telemetry keys rather than accumulating them

The server never retracts a key: when a producer changes, the keys that no
longer apply simply stop being sent. A mirror that only inserts therefore
reports a colour producer playing a clip that finished minutes ago — observed
live, not hypothesised.

Leaves are held flat, keyed by full OSC address, with the packet number they
were last seen in; anything unseen for 50 packets is dropped. Flat because
pruning a nested tree in place is far more code for the same result; counted in
packets rather than seconds so it needs no clock and stays deterministic under
test. The window is wide enough that a state split across several datagrams is
never mistaken for a key disappearing.

## Derive frame numbers rather than dropping them

The server publishes position in *seconds*; `file/frame` does not exist on the
producer side. Operators think in frames, and `time × fps` is exact when both
figures come from the server, so the digest derives it — and says so, in the
field's own documentation, rather than implying the server reported it.

## A hand-written OSC decoder

`rosc` would have done, and OpenStage uses it. Written by hand here because the
decoder is ~150 lines, the state tree is the actual work, and it removes a
dependency from a crate meant to be reusable. Every OSC 1.0 type is covered and
tested, including malformed-packet handling.

**Revisit if:** anything needs OSC *output* or pattern dispatch, at which point
`rosc` earns its place.

## Unrestricted raw AMCP from the console

Deliberate. A media server you cannot rescue from a command line at thirty
seconds to doors is a media server that fails in front of an audience. The
command line is on the Channels page, which is where you end up when something is
wrong, and every command and reply goes to the log.

## Fail a cue whole, never partially

An action naming an unknown screen aborts the cue. Firing "the parts that
resolved" puts a half-state on stage and hides the fault.

## Detect a lost connection without writing to it

`is_closed()` originally just asked whether the command channel had closed —
which only happens after a *failed write*. A server that went away while nobody
was sending therefore looked alive indefinitely: the supervisor's reconnect loop
never fired, and the console reported itself connected while serving telemetry
frozen at the moment the server died. Found by restarting the fake server under
a running daemon, which is exactly what happens when someone restarts CasparCG
mid-rig. The reader task sees EOF immediately, so it now sets a shared flag.

## Still open

- **Auto-follow.** Cues carry a follow time; nothing executes it. Needs a cue
  stack with a running position — a real feature, not a loose end to tie off.
- **Timeline.** Cue-based only so far. A timecode-locked timeline is the obvious
  next axis, and is how a large show is usually driven.
- **Multi-server.** One server per daemon. A large rig is several Caspar boxes,
  which needs the screen model to carry a server reference and the cue compiler
  to fan out — and then the frame-accuracy claim needs re-examining, since
  `COMMIT` only synchronises within one server.
- **Show file versioning.** No schema version yet. Cheap now, expensive later.
- **Consumers are visible in telemetry** (`output/port/<id>/…`) and audio levels
  arrive as `mixer/audio/volume`. Neither is surfaced yet; both are free wins
  discovered while verifying, not designed for.
- **Control input.** No MIDI, OSC-in or Art-Net. The grid is keyboard and mouse.
