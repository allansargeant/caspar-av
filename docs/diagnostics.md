# Diagnostics

Three artefacts, because a failure on site needs different things at different
moments: something an operator can read now, something that survives a crash
nobody was watching, and something that can be sent in one piece afterwards.

Everything here lives in `crates/diag`. It is deliberately self-contained and
dependency-light so it can be copied into the other repos unchanged.

## Where things are written

| Platform | Directory |
| --- | --- |
| macOS | `~/Library/Logs/caspar-avd/` |
| Linux | `$XDG_STATE_HOME/caspar-avd/logs/` (default `~/.local/state/caspar-avd/logs/`) |
| Windows | `%LOCALAPPDATA%\caspar-avd\logs\` |

`CASPAR_AV_LOG_DIR` overrides it, which is how you point a whole rack at one
collected location. The path is also printed on the first line of every run, so
nobody has to remember the table above.

Logs are under state, not cache: a cache directory may be cleared at any time,
and the whole point of a crash report is that it outlives the crash.

## 1. The human log

`caspar-avd.YYYY-MM-DD.log`, rotated daily, seven files kept, no colour escapes.
Written non-blocking so logging never stalls the show path.

Console output goes to **stderr**, the file gets a plain copy. Anything the
daemon prints on stdout is program output, not logging — `--collect-diagnostics`
prints a path there and nothing else, so it can be used in a script.

Verbosity comes from `CASPAR_AV_LOG`, falling back to `RUST_LOG`, falling back
to `info,tower_http=warn`. It takes the usual `tracing` filter syntax:

```bash
CASPAR_AV_LOG=debug,showd::bridge=trace caspar-avd
```

## 2. The crash report

Written by a panic hook, before unwinding finishes, to
`caspar-avd-crash-<timestamp>.json`. The daemon prints the path on the way down.

It carries what is needed to diagnose a fault without reproducing it:

| Field | Why it is there |
| --- | --- |
| `app.version`, `app.git_rev`, `app.built_at` | A semver does not identify a binary that is three commits past the tag. `git_rev` is suffixed `-dirty` if the tree had uncommitted changes at build time. |
| `platform` | OS, arch, hostname, core count. |
| `process` | PID, the full argv, and when the process started. |
| `config` | The effective configuration, with secret-looking keys replaced by `<redacted>`. |
| `panic` | Message, source location, thread name, and a backtrace. |
| `recent_log` | The last 500 log lines, oldest first. |

The backtrace is always present. It uses `force_capture`, so it does not depend
on the operator having set `RUST_BACKTRACE` before the thing that crashed
crashed — which they never have.

`recent_log` comes from an in-memory ring, not from re-reading the log file.
The file writer is non-blocking, so at the moment of a panic the lines that
matter most — the last ones — are still sitting in its queue.

## 3. The diagnostics bundle

```bash
caspar-avd --collect-diagnostics
```

Writes `caspar-avd-diagnostics-<timestamp>.json` and prints its path. One file,
so "send me your diagnostics" is one instruction rather than a conversation
about which of six files were wanted.

It contains the same identity and config blocks as a crash report, plus the
last three log files (tail-capped at 5000 lines each), any of the five most
recent crash reports embedded whole, and the in-memory ring if the process is
running. `collection_warnings` records anything that could not be read —
collection is best-effort, because a missing log file must not stop the rest
being sent.

## Redaction

Keys are matched case-insensitively with `-` and `_` removed, against
`password`, `passwd`, `passphrase`, `secret`, `token`, `apikey`, `credential`,
`auth`, and `private`, at any depth including inside arrays.

It is deliberately over-eager. A redacted port number costs nothing; a token
left in a file that gets forwarded to a mailing list costs a great deal. If you
add a config field holding a credential, check that its name trips one of those
words rather than assuming it will be caught.

## Schema

Both documents carry `"schema": "stoatworks.diagnostics/1"` and a `kind` of
either `crash-report` or `diagnostics-bundle`. Treat the schema string as the
contract: bump it if a field changes meaning, so a tool reading an old file
does not misinterpret it.

## Trying it

`crates/diag/examples/crash.rs` panics on purpose after logging a few lines.
The panic hook is process-global and runs during unwinding, which the test
harness owns, so this is an example rather than a `#[test]`:

```bash
cargo run -p diag --example crash
```

Read the JSON it leaves behind — including that `api_token` came out as
`<redacted>`.
