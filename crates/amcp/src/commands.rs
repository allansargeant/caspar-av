//! Typed builders for the AMCP command surface.
//!
//! One function per command, named after it. The set and the parameter order
//! are taken from `register_commands()` in the 2.5.0 server
//! (`src/protocol/amcp/AMCPCommandsImpl.cpp:1739`) rather than from the wiki,
//! which lags the server — `CALLBG`, `APPLY`, `MIXER INVERT` and
//! `OSC SUBSCRIBE` are all 2.5-era and missing or stale there.
//!
//! Anything not covered here can still be sent with [`Command::new`].

use crate::command::Command;
use std::fmt;

// ---------------------------------------------------------------- transitions

/// How a layer's foreground is replaced when a background is played in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransitionKind {
    #[default]
    Cut,
    Mix,
    Push,
    Slide,
    Wipe,
    /// A sting transition driven by a clip, named by [`Transition::sting`].
    Sting,
}

impl fmt::Display for TransitionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TransitionKind::Cut => "CUT",
            TransitionKind::Mix => "MIX",
            TransitionKind::Push => "PUSH",
            TransitionKind::Slide => "SLIDE",
            TransitionKind::Wipe => "WIPE",
            TransitionKind::Sting => "STING",
        })
    }
}

/// Which way a directional transition travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    Left,
    #[default]
    Right,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Direction::Left => "LEFT",
            Direction::Right => "RIGHT",
        })
    }
}

/// A transition applied by `LOADBG` / `PLAY`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Length in frames. Ignored for `CUT`.
    pub duration: u32,
    pub tween: Option<String>,
    pub direction: Option<Direction>,
    /// The sting clip, for [`TransitionKind::Sting`].
    pub sting: Option<String>,
}

impl Transition {
    /// An immediate cut — the default when no transition is given.
    pub fn cut() -> Self {
        Self::default()
    }

    /// A dissolve over `frames`.
    pub fn mix(frames: u32) -> Self {
        Self { kind: TransitionKind::Mix, duration: frames, ..Default::default() }
    }

    /// A push over `frames`.
    pub fn push(frames: u32, direction: Direction) -> Self {
        Self {
            kind: TransitionKind::Push,
            duration: frames,
            direction: Some(direction),
            ..Default::default()
        }
    }

    /// A sting driven by a clip.
    pub fn sting(clip: impl Into<String>) -> Self {
        Self { kind: TransitionKind::Sting, sting: Some(clip.into()), ..Default::default() }
    }

    /// Set the easing curve (`easeinoutsine`, `linear`, …).
    pub fn tween(mut self, tween: impl Into<String>) -> Self {
        self.tween = Some(tween.into());
        self
    }

    fn apply(&self, mut cmd: Command) -> Command {
        if self.kind == TransitionKind::Sting {
            cmd = cmd.arg("STING").arg(self.sting.clone().unwrap_or_default());
            return cmd;
        }
        cmd = cmd.arg(self.kind).arg(self.duration);
        if let Some(t) = &self.tween {
            cmd = cmd.arg(t);
        }
        if let Some(d) = self.direction {
            cmd = cmd.arg(d);
        }
        cmd
    }
}

/// The duration + easing tail shared by most `MIXER` commands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Anim {
    /// Length in frames; 0 applies the change immediately.
    pub duration: u32,
    pub tween: Option<String>,
}

impl Anim {
    /// Apply instantly.
    pub fn instant() -> Self {
        Self::default()
    }

    /// Ramp over `frames`, linearly.
    pub fn frames(frames: u32) -> Self {
        Self { duration: frames, tween: None }
    }

    /// Ramp over `frames` with an easing curve.
    pub fn eased(frames: u32, tween: impl Into<String>) -> Self {
        Self { duration: frames, tween: Some(tween.into()) }
    }

    fn apply(&self, mut cmd: Command) -> Command {
        if self.duration == 0 && self.tween.is_none() {
            return cmd;
        }
        cmd = cmd.arg(self.duration);
        if let Some(t) = &self.tween {
            cmd = cmd.arg(t);
        }
        cmd
    }
}

// --------------------------------------------------------------------- basic

/// `LOADBG` — cue a producer into the layer's background.
pub fn loadbg(ch: u32, layer: u32, clip: &str, looping: bool, tr: Option<&Transition>) -> Command {
    let mut c = Command::new("LOADBG").layer(ch, layer).arg(clip);
    if looping {
        c = c.arg("LOOP");
    }
    if let Some(t) = tr {
        c = t.apply(c);
    }
    c
}

/// `LOAD` — load a producer straight into the foreground, paused on frame one.
pub fn load(ch: u32, layer: u32, clip: &str) -> Command {
    Command::new("LOAD").layer(ch, layer).arg(clip)
}

/// `PLAY` with a clip — load and start it in one step.
pub fn play_clip(ch: u32, layer: u32, clip: &str, looping: bool, tr: Option<&Transition>) -> Command {
    let mut c = Command::new("PLAY").layer(ch, layer).arg(clip);
    if looping {
        c = c.arg("LOOP");
    }
    if let Some(t) = tr {
        c = t.apply(c);
    }
    c
}

/// `PLAY` with no clip — promote whatever `LOADBG` cued into the foreground.
pub fn play(ch: u32, layer: u32) -> Command {
    Command::new("PLAY").layer(ch, layer)
}

/// `PAUSE`.
pub fn pause(ch: u32, layer: u32) -> Command {
    Command::new("PAUSE").layer(ch, layer)
}

/// `RESUME`.
pub fn resume(ch: u32, layer: u32) -> Command {
    Command::new("RESUME").layer(ch, layer)
}

/// `STOP` — remove the foreground producer.
pub fn stop(ch: u32, layer: u32) -> Command {
    Command::new("STOP").layer(ch, layer)
}

/// `CLEAR` a layer — foreground *and* background.
pub fn clear_layer(ch: u32, layer: u32) -> Command {
    Command::new("CLEAR").layer(ch, layer)
}

/// `CLEAR` a whole channel.
pub fn clear_channel(ch: u32) -> Command {
    Command::new("CLEAR").channel(ch)
}

/// `CLEAR ALL` — every channel.
pub fn clear_all() -> Command {
    Command::new("CLEAR ALL")
}

/// `CALL` — invoke a parameter on the foreground producer, e.g. `SEEK 200`.
pub fn call(ch: u32, layer: u32, params: &[&str]) -> Command {
    Command::new("CALL").layer(ch, layer).args(params)
}

/// `CALLBG` — as `CALL`, but on the background producer (2.5+).
pub fn callbg(ch: u32, layer: u32, params: &[&str]) -> Command {
    Command::new("CALLBG").layer(ch, layer).args(params)
}

/// `SWAP` — exchange the contents of two layers, optionally with their
/// transforms.
pub fn swap(ch: u32, layer: u32, other: &str, with_transforms: bool) -> Command {
    let c = Command::new("SWAP").layer(ch, layer).arg(other);
    if with_transforms {
        c.arg("TRANSFORMS")
    } else {
        c
    }
}

/// `ADD` a consumer to a channel, e.g. `ADD 1 SCREEN` or `ADD 1 FILE out.mp4`.
pub fn add_consumer(ch: u32, params: &[&str]) -> Command {
    Command::new("ADD").channel(ch).args(params)
}

/// `REMOVE` a consumer from a channel.
pub fn remove_consumer(ch: u32, params: &[&str]) -> Command {
    Command::new("REMOVE").channel(ch).args(params)
}

/// `PRINT` — write a PNG snapshot of the channel output.
pub fn print(ch: u32) -> Command {
    Command::new("PRINT").channel(ch)
}

/// `SET` a channel property, e.g. `MODE 1080p5000`.
pub fn set(ch: u32, key: &str, value: &str) -> Command {
    Command::new("SET").channel(ch).arg(key).arg(value)
}

// ---------------------------------------------------------------------- data

/// `DATA STORE`.
pub fn data_store(name: &str, data: &str) -> Command {
    Command::new("DATA STORE").arg(name).arg(data)
}

/// `DATA RETRIEVE`.
pub fn data_retrieve(name: &str) -> Command {
    Command::new("DATA RETRIEVE").arg(name)
}

/// `DATA LIST`.
pub fn data_list() -> Command {
    Command::new("DATA LIST")
}

/// `DATA REMOVE`.
pub fn data_remove(name: &str) -> Command {
    Command::new("DATA REMOVE").arg(name)
}

// ------------------------------------------------------------------ template

/// `CG ADD` — load a template into a CG layer and optionally play it at once.
pub fn cg_add(
    ch: u32,
    layer: u32,
    cg_layer: u32,
    template: &str,
    play_on_load: bool,
    data: Option<&str>,
) -> Command {
    Command::new("CG ADD")
        .layer(ch, layer)
        .arg(cg_layer)
        .arg(template)
        .arg(if play_on_load { 1 } else { 0 })
        .opt(data)
}

/// `CG PLAY`.
pub fn cg_play(ch: u32, layer: u32, cg_layer: u32) -> Command {
    Command::new("CG PLAY").layer(ch, layer).arg(cg_layer)
}

/// `CG STOP`.
pub fn cg_stop(ch: u32, layer: u32, cg_layer: u32) -> Command {
    Command::new("CG STOP").layer(ch, layer).arg(cg_layer)
}

/// `CG NEXT`.
pub fn cg_next(ch: u32, layer: u32, cg_layer: u32) -> Command {
    Command::new("CG NEXT").layer(ch, layer).arg(cg_layer)
}

/// `CG REMOVE`.
pub fn cg_remove(ch: u32, layer: u32, cg_layer: u32) -> Command {
    Command::new("CG REMOVE").layer(ch, layer).arg(cg_layer)
}

/// `CG CLEAR` — every CG layer on the layer.
pub fn cg_clear(ch: u32, layer: u32) -> Command {
    Command::new("CG CLEAR").layer(ch, layer)
}

/// `CG UPDATE` — push new data into a running template.
pub fn cg_update(ch: u32, layer: u32, cg_layer: u32, data: &str) -> Command {
    Command::new("CG UPDATE").layer(ch, layer).arg(cg_layer).arg(data)
}

/// `CG INVOKE` — call a method on a running template.
pub fn cg_invoke(ch: u32, layer: u32, cg_layer: u32, method: &str) -> Command {
    Command::new("CG INVOKE").layer(ch, layer).arg(cg_layer).arg(method)
}

// --------------------------------------------------------------------- mixer

/// `MIXER OPACITY`.
pub fn mixer_opacity(ch: u32, layer: u32, value: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER OPACITY").layer(ch, layer).arg(value))
}

/// `MIXER BRIGHTNESS`.
pub fn mixer_brightness(ch: u32, layer: u32, value: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER BRIGHTNESS").layer(ch, layer).arg(value))
}

/// `MIXER SATURATION`.
pub fn mixer_saturation(ch: u32, layer: u32, value: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER SATURATION").layer(ch, layer).arg(value))
}

/// `MIXER CONTRAST`.
pub fn mixer_contrast(ch: u32, layer: u32, value: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER CONTRAST").layer(ch, layer).arg(value))
}

/// `MIXER VOLUME`.
pub fn mixer_volume(ch: u32, layer: u32, value: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER VOLUME").layer(ch, layer).arg(value))
}

/// `MIXER MASTERVOLUME` — channel-wide.
pub fn mixer_mastervolume(ch: u32, value: f64) -> Command {
    Command::new("MIXER MASTERVOLUME").channel(ch).arg(value)
}

/// `MIXER LEVELS` — min/max in, gamma, min/max out.
///
/// The parameter list mirrors the AMCP command exactly; collapsing it into a
/// struct would make call sites harder to check against the protocol docs,
/// which is the thing that actually goes wrong here.
#[allow(clippy::too_many_arguments)]
pub fn mixer_levels(
    ch: u32,
    layer: u32,
    min_in: f64,
    max_in: f64,
    gamma: f64,
    min_out: f64,
    max_out: f64,
    anim: &Anim,
) -> Command {
    anim.apply(
        Command::new("MIXER LEVELS")
            .layer(ch, layer)
            .arg(min_in)
            .arg(max_in)
            .arg(gamma)
            .arg(min_out)
            .arg(max_out),
    )
}

/// `MIXER FILL` — position and scale the layer within the channel, in
/// normalised units where 1.0 is the channel's width or height.
///
/// This is the command that makes Caspar usable as a show output: a screen's
/// content is placed on the canvas by its fill rect.
pub fn mixer_fill(ch: u32, layer: u32, x: f64, y: f64, w: f64, h: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER FILL").layer(ch, layer).arg(x).arg(y).arg(w).arg(h))
}

/// `MIXER CLIP` — the rectangle outside which the layer is masked away.
pub fn mixer_clip(ch: u32, layer: u32, x: f64, y: f64, w: f64, h: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER CLIP").layer(ch, layer).arg(x).arg(y).arg(w).arg(h))
}

/// `MIXER CROP` — trim the source edges.
pub fn mixer_crop(
    ch: u32,
    layer: u32,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    anim: &Anim,
) -> Command {
    anim.apply(
        Command::new("MIXER CROP").layer(ch, layer).arg(left).arg(top).arg(right).arg(bottom),
    )
}

/// `MIXER ANCHOR` — the origin that rotation and fill scale around.
pub fn mixer_anchor(ch: u32, layer: u32, x: f64, y: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER ANCHOR").layer(ch, layer).arg(x).arg(y))
}

/// `MIXER ROTATION`, in degrees.
pub fn mixer_rotation(ch: u32, layer: u32, degrees: f64, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER ROTATION").layer(ch, layer).arg(degrees))
}

/// `MIXER PERSPECTIVE` — corner-pin the layer by its four corners, clockwise
/// from top-left. This is real keystone correction, and it is what lets a
/// projector output be aligned without external hardware.
pub fn mixer_perspective(ch: u32, layer: u32, corners: &[(f64, f64); 4], anim: &Anim) -> Command {
    let mut c = Command::new("MIXER PERSPECTIVE").layer(ch, layer);
    for (x, y) in corners {
        c = c.arg(x).arg(y);
    }
    anim.apply(c)
}

/// `MIXER KEYER` — use the layer as an alpha key for the one below.
pub fn mixer_keyer(ch: u32, layer: u32, on: bool) -> Command {
    Command::new("MIXER KEYER").layer(ch, layer).arg(if on { 1 } else { 0 })
}

/// `MIXER INVERT` — invert the layer's alpha (2.5+).
pub fn mixer_invert(ch: u32, layer: u32, on: bool) -> Command {
    Command::new("MIXER INVERT").layer(ch, layer).arg(if on { 1 } else { 0 })
}

/// `MIXER BLEND` — set the layer's blend mode, e.g. `screen`, `add`.
pub fn mixer_blend(ch: u32, layer: u32, mode: &str) -> Command {
    Command::new("MIXER BLEND").layer(ch, layer).arg(mode)
}

/// `MIXER CHROMA` — chroma key.
pub fn mixer_chroma(ch: u32, layer: u32, params: &[&str]) -> Command {
    Command::new("MIXER CHROMA").layer(ch, layer).args(params)
}

/// `MIXER GRID` — lay every layer of the channel out in an n×n grid.
pub fn mixer_grid(ch: u32, n: u32, anim: &Anim) -> Command {
    anim.apply(Command::new("MIXER GRID").channel(ch).arg(n))
}

/// `MIXER CLEAR` — drop every transform on a layer.
pub fn mixer_clear_layer(ch: u32, layer: u32) -> Command {
    Command::new("MIXER CLEAR").layer(ch, layer)
}

/// `MIXER CLEAR` — drop every transform on a channel.
pub fn mixer_clear_channel(ch: u32) -> Command {
    Command::new("MIXER CLEAR").channel(ch)
}

/// `CHANNEL_GRID` — route every channel into a monitoring grid on the last one.
pub fn channel_grid() -> Command {
    Command::new("CHANNEL_GRID")
}

// --------------------------------------------------------------------- query

/// `INFO` — one line per channel.
pub fn info() -> Command {
    Command::new("INFO")
}

/// `INFO <channel>` — the channel's full state as XML.
pub fn info_channel(ch: u32) -> Command {
    Command::new("INFO").channel(ch)
}

/// `INFO <channel>-<layer>` — one layer's state as XML.
pub fn info_layer(ch: u32, layer: u32) -> Command {
    Command::new("INFO").layer(ch, layer)
}

/// `INFO CONFIG` — the server's running configuration as XML.
pub fn info_config() -> Command {
    Command::new("INFO CONFIG")
}

/// `INFO PATHS` — the configured media/template/data/log paths.
pub fn info_paths() -> Command {
    Command::new("INFO PATHS")
}

/// `VERSION`.
pub fn version() -> Command {
    Command::new("VERSION")
}

/// `CLS` — list media. Proxied by the server to media-scanner, so it fails
/// with `501` when the scanner is not running.
pub fn cls() -> Command {
    Command::new("CLS")
}

/// `TLS` — list templates. Also proxied to media-scanner.
pub fn tls() -> Command {
    Command::new("TLS")
}

/// `FLS` — list fonts. Also proxied to media-scanner.
pub fn fls() -> Command {
    Command::new("FLS")
}

/// `CINF` — info for one media file. Also proxied to media-scanner.
pub fn cinf(name: &str) -> Command {
    Command::new("CINF").arg(name)
}

/// `GL INFO` — OpenGL device and texture pool state.
pub fn gl_info() -> Command {
    Command::new("GL INFO")
}

/// `DIAG` — open the server's diagnostics window.
pub fn diag() -> Command {
    Command::new("DIAG")
}

/// `LOG LEVEL`.
pub fn log_level(level: &str) -> Command {
    Command::new("LOG LEVEL").arg(level)
}

/// `BYE` — close the connection politely.
pub fn bye() -> Command {
    Command::new("BYE")
}

// ----------------------------------------------------------------- thumbnail

/// `THUMBNAIL LIST`.
pub fn thumbnail_list() -> Command {
    Command::new("THUMBNAIL LIST")
}

/// `THUMBNAIL RETRIEVE` — base64 PNG for one media file.
pub fn thumbnail_retrieve(name: &str) -> Command {
    Command::new("THUMBNAIL RETRIEVE").arg(name)
}

/// `THUMBNAIL GENERATE`.
pub fn thumbnail_generate(name: &str) -> Command {
    Command::new("THUMBNAIL GENERATE").arg(name)
}

/// `THUMBNAIL GENERATE_ALL`.
pub fn thumbnail_generate_all() -> Command {
    Command::new("THUMBNAIL GENERATE_ALL")
}

// ----------------------------------------------------------------------- osc

/// `OSC SUBSCRIBE <port>` (2.5+) — ask the server to send this client's OSC
/// telemetry to `port` on the connecting address, instead of relying on the
/// default port that every other client is also listening on.
pub fn osc_subscribe(port: u16) -> Command {
    Command::new("OSC SUBSCRIBE").arg(port)
}

/// `OSC UNSUBSCRIBE <port>`.
pub fn osc_unsubscribe(port: u16) -> Command {
    Command::new("OSC UNSUBSCRIBE").arg(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(c: Command) -> String {
        c.to_wire().trim_end_matches("\r\n").to_string()
    }

    #[test]
    fn play_with_loop_and_transition() {
        assert_eq!(
            wire(play_clip(1, 10, "AMB", true, Some(&Transition::mix(25)))),
            "PLAY 1-10 AMB LOOP MIX 25"
        );
    }

    #[test]
    fn transitions_carry_tween_and_direction() {
        let t = Transition::push(12, Direction::Left).tween("easeinoutsine");
        assert_eq!(wire(loadbg(2, 1, "clip", false, Some(&t))), "LOADBG 2-1 clip PUSH 12 easeinoutsine LEFT");
    }

    #[test]
    fn a_sting_names_its_clip_instead_of_a_duration() {
        let t = Transition::sting("stings/wipe");
        assert_eq!(wire(play_clip(1, 1, "next", false, Some(&t))), "PLAY 1-1 next STING stings/wipe");
    }

    #[test]
    fn instant_anim_adds_nothing() {
        assert_eq!(wire(mixer_opacity(1, 10, 0.5, &Anim::instant())), "MIXER 1-10 OPACITY 0.5");
    }

    #[test]
    fn eased_anim_appends_duration_then_tween() {
        assert_eq!(
            wire(mixer_opacity(1, 10, 0.5, &Anim::eased(25, "easeoutquad"))),
            "MIXER 1-10 OPACITY 0.5 25 easeoutquad"
        );
    }

    #[test]
    fn fill_and_perspective_carry_their_geometry() {
        assert_eq!(
            wire(mixer_fill(1, 10, 0.0, 0.0, 0.5, 0.5, &Anim::instant())),
            "MIXER 1-10 FILL 0 0 0.5 0.5"
        );
        let corners = [(0.0, 0.0), (1.0, 0.05), (1.0, 1.0), (0.0, 0.95)];
        assert_eq!(
            wire(mixer_perspective(1, 10, &corners, &Anim::instant())),
            "MIXER 1-10 PERSPECTIVE 0 0 1 0.05 1 1 0 0.95"
        );
    }

    #[test]
    fn cg_add_encodes_play_on_load_as_a_flag() {
        assert_eq!(
            wire(cg_add(1, 20, 1, "lower-third", true, Some(r#"{"f0":"Hello"}"#))),
            r#"CG 1-20 ADD 1 lower-third 1 "{\"f0\":\"Hello\"}""#
        );
    }

    #[test]
    fn clip_names_with_spaces_are_quoted() {
        assert_eq!(wire(load(1, 1, "my clip")), "LOAD 1-1 \"my clip\"");
    }

    #[test]
    fn channel_and_layer_scopes_differ() {
        assert_eq!(wire(clear_channel(1)), "CLEAR 1");
        assert_eq!(wire(clear_layer(1, 10)), "CLEAR 1-10");
        assert_eq!(wire(clear_all()), "CLEAR ALL");
    }
}
