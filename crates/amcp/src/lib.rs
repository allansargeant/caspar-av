//! AMCP — the CasparCG server's control protocol.
//!
//! A codec plus an async client. Everything here was written against the
//! CasparCG **2.5.0** server source rather than the community wiki, which is
//! behind on several points that matter to a bridge:
//!
//! - **`REQ <id>` / `RES <id>`.** Prefixing a command with `REQ <id>` makes the
//!   server prefix its reply `RES <id> `. This is not optional politeness: the
//!   server dispatches commands to one queue *per channel*, so replies can come
//!   back out of order and only the id makes them attributable.
//! - **Response framing is decided by the status code** — `200` runs until a
//!   blank line, `201`/`101` take exactly one line, everything else is a single
//!   line. See [`response`].
//! - **Batching is real.** `BEGIN` … `COMMIT` locks every touched channel and
//!   releases the commands together, so a cue lands on one frame. `BEGIN` is
//!   never answered; the batch answers once with `202 COMMIT OK` or
//!   `202 COMMIT PARTIAL`, *plus* an individual reply per command.
//! - **Escaping** follows the server's tokenizer: only `\\`, `\"` and `\n` are
//!   meaningful escapes and any other escaped character is silently dropped.
//!
//! ```no_run
//! use amcp::{Client, commands, Transition};
//!
//! # async fn demo() -> Result<(), amcp::Error> {
//! let client = Client::connect(("127.0.0.1", amcp::DEFAULT_PORT)).await?;
//! client.send(commands::play_clip(1, 10, "AMB", true, Some(&Transition::mix(25)))).await?;
//!
//! // One frame-accurate cue across three layers.
//! client.batch(vec![
//!     commands::play_clip(1, 10, "background", true, None),
//!     commands::play_clip(1, 20, "overlay", false, None),
//!     commands::mixer_opacity(1, 20, 0.8, &amcp::Anim::instant()),
//! ]).await?;
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod command;
pub mod commands;
pub mod response;

pub use client::{Client, Error, DEFAULT_PORT};
pub use command::{escape, Command, Target};
pub use commands::{Anim, Direction, Transition, TransitionKind};
pub use response::{Decoder, Response};
