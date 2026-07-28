# AMCP, as CasparCG 2.5.0 actually implements it

Notes taken from the server source, not the community wiki. Where the two
disagree, the source wins and the difference is called out — several of these
would silently corrupt a control connection if taken from the wiki.

Source references are to [CasparCG/server](https://github.com/CasparCG/server)
at 2.5.0.

## Transport

- **TCP, default port 5250**, configured under `<controllers><tcp>` in
  `casparcg.config`.
- UTF-8, commands terminated `\r\n`, command names case-insensitive.

## Response framing is decided by the status code

This is the single most important thing to get right, and the wiki does not
state it plainly. Framing is **not** one line per response:

| Code | Meaning | Data |
|---|---|---|
| `100` | async notification | none |
| `101` | async notification | **exactly one** following line |
| `200` | success | **many** lines, terminated by an empty line |
| `201` | success | **exactly one** following line |
| `202` | success | none |
| `4xx` | client error | none |
| `5xx` | server error | none |

Get this wrong and the stream desynchronises for every later command — the
failure shows up as replies attributed to the wrong command, long after the
mistake. Implemented in [`response.rs`](../crates/amcp/src/response.rs).

Note the sharp edge in the `200` case: an empty *data* line is indistinguishable
from the terminator. That is the protocol's design, not a parser limitation.

## `REQ <id>` → `RES <id>`, and why it is mandatory here

Prefixing a command with `REQ <id>` makes the server prefix its reply
`RES <id> ` (`AMCPCommand.cpp:34`).

This is not optional politeness for a bridge. The server dispatches each command
to a queue chosen by its channel:

```cpp
commandQueues_.at(channel_index + 1)->AddCommand(std::move(wrapped));
```
— `AMCPProtocolStrategy.cpp:216`

So a command on channel 2 can be answered *before* an earlier command on
channel 1. Matching replies by arrival order — the obvious implementation, and a
common one — mis-attributes them. Every command caspar-AV sends carries a `REQ`
id, and `crates/amcp/tests/client.rs` forces the out-of-order case deterministically.

## Batching: `BEGIN` … `COMMIT`

Handled at the protocol layer, not as registered commands
(`AMCPProtocolStrategy.cpp:248`). The behaviour, from `AMCPCommandQueue::Execute`:

1. `BEGIN` opens a batch and **is never answered**. Waiting for a reply to it
   deadlocks. It carries the request id the final `COMMIT` reply will use.
2. Commands between `BEGIN` and `COMMIT` are queued, not executed.
3. `COMMIT` creates a delayed stage per channel, queues every command, takes a
   lock on each touched channel, then releases them **together** and waits.
4. Replies: **one per inner command**, *plus* one for the batch —
   `202 COMMIT OK`, or `202 COMMIT PARTIAL` if any command failed.

Point 3 is what makes a multi-screen cue land on a single frame, and is the
reason caspar-AV models cues as batches rather than loops.

A batch of one is shortcut by the server anyway, so the client sends a single
command unwrapped.

## Parameter escaping

From the tokenizer (`tokenize.cpp`), which is stricter than the wiki implies:

- Tokens split on spaces; `"` toggles a quoted run.
- `\` starts an escape, and **only `\\`, `\"` and `\n` mean anything** —
  any other escaped character is *silently dropped*. So a Windows path written
  raw (`C:\media\clip`) loses characters. Always escape backslashes.
- Unquoted `(` opens a parameter-list token that runs to the matching `)`, which
  is why any value containing parentheses must be quoted.
- `""` produces a deliberate empty token.
- There is no escape for a bare CR; it would terminate the line early.

Implemented in [`command.rs`](../crates/amcp/src/command.rs).

## Telemetry is pushed, not polled

CasparCG answers no "what is playing" question. It **pushes** its whole monitor
state as an OSC bundle over UDP, once per frame.

Two ways to receive it:

1. **Implicitly.** Connecting an AMCP client subscribes that client's *IP
   address* on `<osc><default-port>` (6250), unless
   `<disable-send-to-amcp-clients>` is set (`server.cpp:334`).
2. **Explicitly** — `OSC SUBSCRIBE <port>`, new in 2.5
   (`AMCPCommandsImpl.cpp:1817`). The server sends to that port on the
   connecting address.

caspar-AV uses (2): it binds an ephemeral port and claims its own feed, so it
does not contend for 6250 with the Caspar client or a second daemon on the same
machine. It falls back to the shared port when `OSC SUBSCRIBE` is refused.

Address shape, from `layer.cpp:132` and `stage.cpp:221`:

```
/channel/<n>/format
/channel/<n>/framerate
/channel/<n>/profiler/time
/channel/<n>/stage/layer/<i>/foreground/producer
/channel/<n>/stage/layer/<i>/foreground/paused
/channel/<n>/stage/layer/<i>/foreground/file/{path,name,time,frame,fps,video/width,…}
/channel/<n>/stage/layer/<i>/background/…
```

`file/time` and `file/frame` carry **two** values: current and total. An empty
slot reports the producer `empty` rather than being omitted.

Because key sets vary by version, producer and consumer, the bridge keeps the
**whole raw tree** as well as a typed digest — see
[`state.rs`](../crates/osc/src/state.rs).

## media-scanner is not optional

In 2.5, `CLS`, `TLS`, `FLS`, `CINF` and every `THUMBNAIL` command are HTTP-proxied
straight to the media-scanner service (`AMCPCommandsImpl.cpp:1451`). Without it
they return `501 … FAILED`, and the server has no media list of its own. This is
the most common "my media is missing" cause, so the console says so explicitly
rather than showing an empty grid.

Talking to the scanner directly over HTTP (default port 8000) is strictly better
than through the proxy:

| Route | Returns |
|---|---|
| `GET /media` | full ffprobe metadata as JSON |
| `GET /media/info/<ID>` | metadata for one item (id upper-cased) |
| `GET /media/thumbnail/<ID>` | a real PNG |
| `GET /templates` | templates **with GDD schemas** |
| `GET /cls`, `/tls`, `/fls` | the AMCP-shaped text listings |
| `GET /thumbnail/<ID>` | base64 PNG wrapped in an AMCP status line |

**GDD** (Graphics Data Definition) is the interesting one: HTML templates can
embed a JSON Schema of their own fields, which the console turns into a real
form instead of a free-text JSON box.

## Command surface

The authority is `register_commands()` at `AMCPCommandsImpl.cpp:1739`, not the
wiki. Commands present in 2.5.0 that the wiki misses or misstates include
`CALLBG`, `APPLY`, `MIXER INVERT`, `OSC SUBSCRIBE` and `OSC UNSUBSCRIBE`.

Typed builders for the full set live in
[`commands.rs`](../crates/amcp/src/commands.rs).

### The commands that matter for a media server

| Command | Why |
|---|---|
| `MIXER FILL c-l x y w h` | Position and scale in normalised units — screen placement on a canvas |
| `MIXER PERSPECTIVE c-l x1 y1 … x4 y4` | Four-corner warp — genuine projector keystone |
| `MIXER CLIP` / `MIXER CROP` | Masking and source trim — the basis of blending and edge shaping |
| `MIXER OPACITY … <frames> <tween>` | Every mixer command takes a duration and easing curve, so fades are server-side and frame-accurate |
| `BEGIN` / `COMMIT` | Simultaneity across screens |
| `LOADBG` + `PLAY` | Pre-roll then take — a clean change rather than a cold start |
