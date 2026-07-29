# AMCP, as CasparCG 2.5.0 actually implements it

Notes taken from the server source **and verified against a running 2.5.0
server**, not from the community wiki. Where they disagree, the observed
behaviour wins and the difference is called out — several of these would
silently corrupt a control connection if taken from the wiki.

Source references are to [CasparCG/server](https://github.com/CasparCG/server)
at 2.5.0. `scripts/protocol-probe.py` re-checks every claim on this page against
a live server; it uses raw sockets and none of this project's code, so it is
able to disprove them.

## Read this first: the two-word command trap

A command whose name is two words puts its **target between the words**:

```text
MIXER 1-10 FILL 0 0 0.5 0.5      ✓ 202 MIXER OK
MIXER FILL 1-10 0 0 0.5 0.5      ✗ 400 ERROR
CG 1-20 ADD 1 lower-third 1      ✓
CG ADD 1-20 1 lower-third 1      ✗ 400 ERROR
```

The parser (`amcp_command_repository.cpp:165`) pops **one** token as the command
name, *then* parses the channel spec, and only then joins the following token to
form `MIXER FILL`. Registration under the name `"MIXER FILL"`
(`AMCPCommandsImpl.cpp:1774`) makes the wrong order look right.

Global two-word commands have no target and are unaffected: `DATA STORE x y`,
`INFO CONFIG`, `CLEAR ALL`, `OSC SUBSCRIBE 6250`, `THUMBNAIL LIST`.

This one cost caspar-AV a rewrite of every `MIXER` and `CG` builder — the entire
output-mapping model emitted `400 ERROR` until a real server was put in front of
it.

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
| `400` | malformed command | **one line — the offending command, echoed** |
| `401` `402` `500` `503` | other failures | none |
| `404` etc. from commands | command-level failure | none |

`400` is the exception that catches people out, and the wiki does not mention it:

```text
→ NOSUCHCOMMAND
← 400 ERROR\r\nNOSUCHCOMMAND\r\n     ← two lines
→ PLAY 1-10 MISSING
← 404 PLAY FAILED\r\n                ← one line
```

It is built at `AMCPProtocolStrategy.cpp:151`. Treating `400` as dataless leaves
the echoed command in the buffer, where it is either dropped or — if it happens
to begin with three digits — misread as the next response.

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

### What is actually on the wire

Captured from a live 2.5.0 server playing an 8-second clip. Type tags included,
because several of them are not what the source suggests:

```text
/channel/1/format                                    ,s   "720p2500"
/channel/1/framerate                                 ,ii  [25, 1]        ← rational!
/channel/1/mixer/audio/volume                        ,i×16               (with a consumer)
/channel/1/output/port/<id>/consumer                 ,s   "ffmpeg"       (with a consumer)
/channel/1/output/port/<id>/file/fps                 ,f   25.0
/channel/1/output/port/<id>/file/frame               ,i   77
/channel/1/stage/layer/10/foreground/producer        ,s   "ffmpeg"
/channel/1/stage/layer/10/foreground/paused          ,F
/channel/1/stage/layer/10/foreground/loop            ,T
/channel/1/stage/layer/10/foreground/file/name       ,s   "TESTCLIP"
/channel/1/stage/layer/10/foreground/file/path       ,s   "/opt/caspar/media/TESTCLIP.mp4"
/channel/1/stage/layer/10/foreground/file/time       ,ff  [1.28, 8.0]    ← position, duration
/channel/1/stage/layer/10/foreground/file/clip       ,ff  [0.0, 8.0]     ← in-point, trimmed length
/channel/1/stage/layer/10/foreground/file/streams/0/fps ,ii [25, 1]      ← rational!
/channel/1/stage/layer/10/background/producer        ,s   "empty"
```

Four traps in that list:

1. **`framerate` is a rational**, `,ii` = numerator and denominator, not a
   float. Read as a scalar it yields nothing at all.
2. **There is no `file/frame` and no `file/video/width`** on the producer side,
   however plausible they look in the headers. `file/fps` and `file/frame` *do*
   exist — but under `output/port/<id>/`, describing the **consumer**. Frame
   numbers for a playing clip have to be derived as `time × fps`.
3. **Per-frame updates are only `file/time`, `file/clip` and `loop`**
   (`av_producer.cpp:985`). Everything else is published once when the producer
   opens.
4. **Keys are never retracted.** When a colour producer replaces a clip, the
   server simply stops sending `file/*` — it does not send a tombstone. A mirror
   that only inserts will happily report a colour producer playing a file that
   finished minutes ago. caspar-AV expires anything that stops arriving.

An empty slot reports the producer `empty` rather than being omitted.

**`profiler/time` is never published**, with or without a consumer, despite
appearing in the source. There is no per-channel frame-time metric to display.

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

### Verified on real output

`scripts/verify-mapping.py` drives a real server, has it `PRINT` real frames and
checks where the pixels landed — because a `202` proves the command parsed, not
that anything moved. All five `MIXER FILL` placements land exactly on the
requested cells, and `MIXER PERSPECTIVE` produces a genuine wedge:

```text
MIXER 1-10 FILL 0.25 0.25 0.5 0.5        MIXER 1-10 PERSPECTIVE 0 0 1 0.4 1 0.6 0 1

                                         %=
                                         @@@*:
    %@@%                                 @@@@@#=
    @@@@                                 @@@@@@@@
    @@@@                                 @@@@@@@@
    %@@%                                 @@@@@#=
                                         @@@*:
                                         #=
```

Three things that make this test lie if you let them:

- **`CLEAR` does not reset mixer transforms.** They are separate state and
  survive it, so each case inherits the previous one's warp. `MIXER <ch> CLEAR`
  is the reset.
- **`PRINT` is asynchronous.** It returns `202` immediately and writes the PNG
  afterwards, so reading the newest file straight away gives the previous frame.
- **`PRINT` captures a stale frame when the channel has no consumer.** 2.5.0
  only mixes channels that have one, so the transient consumer `PRINT` installs
  grabs what was already buffered. Printing twice and discarding the first is
  the workaround.

### Other small traps

- **Colours are `#AARRGGBB`** — alpha first. `#FF0000FF` is opaque *blue*.
- **`PING` is intercepted before `REQ` is parsed** (`AMCPProtocolStrategy.cpp:126`),
  so it is the one command that must be sent **without** a request id. It replies
  `PONG <args>` with no status code at all, so a client that only recognises
  status lines will wait for a reply that never comes in a form it understands.
