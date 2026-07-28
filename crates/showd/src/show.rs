//! The show model — what turns CasparCG from a playout engine into a media
//! server.
//!
//! Caspar itself knows about channels and layers. A show needs *screens* placed
//! on a *canvas*, and *cues* that change several of them at once. Both are
//! built here and compiled down to AMCP:
//!
//! - A **screen** is a channel/layer pair with a rectangle on the show canvas
//!   and four corners. It maps to `MIXER FILL` (position and scale) and
//!   `MIXER PERSPECTIVE` (corner-pin keystone) — real output mapping, with no
//!   hardware beyond what Caspar already has.
//! - A **cue** is a list of actions fired as one `BEGIN`/`COMMIT` batch, so
//!   every screen changes on the same frame instead of drifting apart.
//!
//! Nothing here holds live state: the show is the *intent*, and the live state
//! comes back from the server over OSC. Keeping those separate is what lets two
//! consoles show the same thing.

use amcp::commands as c;
use amcp::{Anim, Command, Direction, Transition, TransitionKind};
use serde::{Deserialize, Serialize};

/// A rectangle in normalised canvas units, where 1.0 is the canvas width or
/// height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Default for Rect {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }
}

/// The show canvas. Pixel dimensions are carried so the console can show
/// "3840×1080" rather than unitless numbers; the mapping maths stays
/// normalised.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self { width: 1920, height: 1080 }
    }
}

/// One output: a Caspar layer, where it sits on the canvas, and its keystone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub id: String,
    pub name: String,
    pub channel: u32,
    pub layer: u32,
    /// Where this screen draws on the canvas.
    pub rect: Rect,
    /// Corner-pin, clockwise from top-left, in the layer's own space. The
    /// identity quad is the default and costs nothing to send.
    #[serde(default = "identity_corners")]
    pub corners: [(f64, f64); 4],
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "one")]
    pub opacity: f64,
}

fn identity_corners() -> [(f64, f64); 4] {
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
}
fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}

impl Screen {
    /// The commands that put this screen's geometry into the server.
    ///
    /// `PERSPECTIVE` is only sent when the corners are actually warped —
    /// an identity corner-pin still costs a transform on the layer, and
    /// sending it needlessly makes the server's state harder to read when
    /// debugging a rig.
    pub fn mapping_commands(&self) -> Vec<Command> {
        let mut out = vec![c::mixer_fill(
            self.channel,
            self.layer,
            self.rect.x,
            self.rect.y,
            self.rect.w,
            self.rect.h,
            &Anim::instant(),
        )];
        if self.corners != identity_corners() {
            out.push(c::mixer_perspective(self.channel, self.layer, &self.corners, &Anim::instant()));
        }
        let opacity = if self.enabled { self.opacity } else { 0.0 };
        out.push(c::mixer_opacity(self.channel, self.layer, opacity, &Anim::instant()));
        out
    }
}

/// How a cue action's transition is described in the show file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionSpec {
    /// `cut`, `mix`, `push`, `slide`, `wipe` or `sting`.
    #[serde(default = "cut")]
    pub kind: String,
    #[serde(default)]
    pub frames: u32,
    #[serde(default)]
    pub tween: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub sting: Option<String>,
}

fn cut() -> String {
    "cut".into()
}

impl TransitionSpec {
    fn to_transition(&self) -> Transition {
        let kind = match self.kind.to_lowercase().as_str() {
            "mix" => TransitionKind::Mix,
            "push" => TransitionKind::Push,
            "slide" => TransitionKind::Slide,
            "wipe" => TransitionKind::Wipe,
            "sting" => TransitionKind::Sting,
            _ => TransitionKind::Cut,
        };
        Transition {
            kind,
            duration: self.frames,
            tween: self.tween.clone(),
            direction: match self.direction.as_deref() {
                Some("left") => Some(Direction::Left),
                Some("right") => Some(Direction::Right),
                _ => None,
            },
            sting: self.sting.clone(),
        }
    }
}

/// One thing a cue does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    /// Play a clip on a screen.
    Play {
        screen: String,
        clip: String,
        #[serde(default)]
        looping: bool,
        #[serde(default)]
        transition: Option<TransitionSpec>,
    },
    /// Cue a clip into the background without showing it, ready for a later
    /// `Take`. This is how a show gets a clean, pre-rolled change.
    Load {
        screen: String,
        clip: String,
        #[serde(default)]
        looping: bool,
        #[serde(default)]
        transition: Option<TransitionSpec>,
    },
    /// Promote whatever `Load` cued into the foreground.
    Take { screen: String },
    Pause { screen: String },
    Resume { screen: String },
    Stop { screen: String },
    Clear { screen: String },
    /// Fade a screen's opacity over a number of frames.
    Opacity {
        screen: String,
        value: f64,
        #[serde(default)]
        frames: u32,
        #[serde(default)]
        tween: Option<String>,
    },
    /// Fade a screen's audio.
    Volume {
        screen: String,
        value: f64,
        #[serde(default)]
        frames: u32,
        #[serde(default)]
        tween: Option<String>,
    },
    /// Re-send a screen's mapping — used to snap a screen back after a manual
    /// nudge, or to apply a saved look.
    Remap { screen: String },
    /// Show a template on a screen's layer.
    Template {
        screen: String,
        template: String,
        #[serde(default)]
        cg_layer: u32,
        #[serde(default)]
        data: Option<String>,
    },
    /// Stop a template.
    TemplateStop {
        screen: String,
        #[serde(default)]
        cg_layer: u32,
    },
    /// An escape hatch: any AMCP command, verbatim. Every professional media
    /// server needs one, and without it the model's gaps become dead ends.
    Raw { command: String },
}

impl Action {
    /// Which screen this action needs resolved, if any.
    pub fn screen_id(&self) -> Option<&str> {
        match self {
            Action::Play { screen, .. }
            | Action::Load { screen, .. }
            | Action::Take { screen }
            | Action::Pause { screen }
            | Action::Resume { screen }
            | Action::Stop { screen }
            | Action::Clear { screen }
            | Action::Opacity { screen, .. }
            | Action::Volume { screen, .. }
            | Action::Remap { screen }
            | Action::Template { screen, .. }
            | Action::TemplateStop { screen, .. } => Some(screen),
            Action::Raw { .. } => None,
        }
    }
}

/// A cue: a named set of actions fired together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Seconds after this cue fires before the next one auto-follows.
    #[serde(default)]
    pub follow: Option<f64>,
    /// Colour tag for the cue list and the trigger grid.
    #[serde(default)]
    pub colour: Option<String>,
}

/// A pad on the trigger grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pad {
    /// Position in the grid, row-major from 0.
    pub index: u32,
    /// The cue this pad fires.
    pub cue: String,
}

/// The whole show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Show {
    #[serde(default = "untitled")]
    pub name: String,
    #[serde(default)]
    pub canvas: Canvas,
    #[serde(default)]
    pub screens: Vec<Screen>,
    #[serde(default)]
    pub cues: Vec<Cue>,
    #[serde(default)]
    pub pads: Vec<Pad>,
    /// Grid dimensions for the trigger page.
    #[serde(default = "default_grid")]
    pub grid: (u32, u32),
}

fn untitled() -> String {
    "Untitled show".into()
}
fn default_grid() -> (u32, u32) {
    (8, 4)
}

impl Default for Show {
    fn default() -> Self {
        Self {
            name: untitled(),
            canvas: Canvas::default(),
            screens: Vec::new(),
            cues: Vec::new(),
            pads: Vec::new(),
            grid: default_grid(),
        }
    }
}

/// A cue referenced an unknown screen.
#[derive(Debug, thiserror::Error, PartialEq)]
#[error("action refers to unknown screen '{0}'")]
pub struct UnknownScreen(pub String);

impl Show {
    /// Find a screen by id.
    pub fn screen(&self, id: &str) -> Option<&Screen> {
        self.screens.iter().find(|s| s.id == id)
    }

    /// Find a cue by id.
    pub fn cue(&self, id: &str) -> Option<&Cue> {
        self.cues.iter().find(|c| c.id == id)
    }

    /// Compile one action into AMCP commands.
    pub fn compile_action(&self, action: &Action) -> Result<Vec<Command>, UnknownScreen> {
        // Every screen-bound action needs its screen; resolve once.
        let screen = match action.screen_id() {
            Some(id) => Some(self.screen(id).ok_or_else(|| UnknownScreen(id.to_string()))?),
            None => None,
        };
        let (ch, ly) = screen.map(|s| (s.channel, s.layer)).unwrap_or((1, 1));

        Ok(match action {
            Action::Play { clip, looping, transition, .. } => {
                let t = transition.as_ref().map(TransitionSpec::to_transition);
                vec![c::play_clip(ch, ly, clip, *looping, t.as_ref())]
            }
            Action::Load { clip, looping, transition, .. } => {
                let t = transition.as_ref().map(TransitionSpec::to_transition);
                vec![c::loadbg(ch, ly, clip, *looping, t.as_ref())]
            }
            Action::Take { .. } => vec![c::play(ch, ly)],
            Action::Pause { .. } => vec![c::pause(ch, ly)],
            Action::Resume { .. } => vec![c::resume(ch, ly)],
            Action::Stop { .. } => vec![c::stop(ch, ly)],
            Action::Clear { .. } => vec![c::clear_layer(ch, ly)],
            Action::Opacity { value, frames, tween, .. } => {
                vec![c::mixer_opacity(ch, ly, *value, &anim(*frames, tween))]
            }
            Action::Volume { value, frames, tween, .. } => {
                vec![c::mixer_volume(ch, ly, *value, &anim(*frames, tween))]
            }
            Action::Remap { .. } => screen.map(Screen::mapping_commands).unwrap_or_default(),
            Action::Template { template, cg_layer, data, .. } => {
                vec![c::cg_add(ch, ly, *cg_layer, template, true, data.as_deref())]
            }
            Action::TemplateStop { cg_layer, .. } => vec![c::cg_stop(ch, ly, *cg_layer)],
            Action::Raw { command } => vec![Command::new(command.clone())],
        })
    }

    /// Compile a whole cue.
    ///
    /// An unknown screen fails the *cue*, rather than firing the actions that
    /// happen to resolve: half a cue on stage is worse than none, and the
    /// operator gets told which screen is missing.
    pub fn compile_cue(&self, cue: &Cue) -> Result<Vec<Command>, UnknownScreen> {
        let mut out = Vec::new();
        for a in &cue.actions {
            out.extend(self.compile_action(a)?);
        }
        Ok(out)
    }

    /// Every screen's mapping, for pushing the whole layout at once — on
    /// connect, or after the server restarts.
    pub fn mapping_commands(&self) -> Vec<Command> {
        self.screens.iter().flat_map(Screen::mapping_commands).collect()
    }

    /// Problems worth telling the operator about before the show, rather than
    /// during it.
    ///
    /// Two screens on the same channel *and* layer is the one that bites: the
    /// second silently replaces the first, so one of the outputs simply never
    /// shows anything and nothing reports an error.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, a) in self.screens.iter().enumerate() {
            for b in &self.screens[i + 1..] {
                if a.channel == b.channel && a.layer == b.layer {
                    out.push(format!(
                        "'{}' and '{}' both use channel {} layer {} — one will replace the other",
                        a.name, b.name, a.channel, a.layer
                    ));
                }
            }
        }
        for pad in &self.pads {
            if self.cue(&pad.cue).is_none() {
                out.push(format!("pad {} fires missing cue '{}'", pad.index, pad.cue));
            }
        }
        out
    }
}

fn anim(frames: u32, tween: &Option<String>) -> Anim {
    match tween {
        Some(t) => Anim::eased(frames, t.clone()),
        None => Anim::frames(frames),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(cmds: &[Command]) -> Vec<String> {
        cmds.iter().map(|c| c.to_string()).collect()
    }

    fn show() -> Show {
        Show {
            screens: vec![
                Screen {
                    id: "left".into(),
                    name: "Left projector".into(),
                    channel: 1,
                    layer: 10,
                    rect: Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 },
                    corners: identity_corners(),
                    enabled: true,
                    opacity: 1.0,
                },
                Screen {
                    id: "right".into(),
                    name: "Right projector".into(),
                    channel: 2,
                    layer: 10,
                    rect: Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 },
                    corners: identity_corners(),
                    enabled: true,
                    opacity: 1.0,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_screen_maps_to_fill_and_opacity() {
        let s = show();
        let cmds = wire(&s.screen("left").unwrap().mapping_commands());
        assert_eq!(cmds, vec!["MIXER FILL 1-10 0 0 0.5 1", "MIXER OPACITY 1-10 1"]);
    }

    #[test]
    fn a_warped_screen_also_sends_perspective() {
        let mut s = show();
        s.screens[0].corners = [(0.0, 0.02), (1.0, 0.0), (1.0, 1.0), (0.0, 0.98)];
        let cmds = wire(&s.screen("left").unwrap().mapping_commands());
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[1], "MIXER PERSPECTIVE 1-10 0 0.02 1 0 1 1 0 0.98");
    }

    #[test]
    fn a_disabled_screen_maps_to_zero_opacity() {
        let mut s = show();
        s.screens[0].enabled = false;
        let cmds = wire(&s.screen("left").unwrap().mapping_commands());
        assert!(cmds.contains(&"MIXER OPACITY 1-10 0".to_string()));
    }

    #[test]
    fn a_cue_compiles_across_screens() {
        let mut s = show();
        s.cues.push(Cue {
            id: "c1".into(),
            name: "Opening".into(),
            actions: vec![
                Action::Play {
                    screen: "left".into(),
                    clip: "opener_l".into(),
                    looping: false,
                    transition: Some(TransitionSpec {
                        kind: "mix".into(),
                        frames: 25,
                        tween: None,
                        direction: None,
                        sting: None,
                    }),
                },
                Action::Play {
                    screen: "right".into(),
                    clip: "opener_r".into(),
                    looping: false,
                    transition: None,
                },
            ],
            follow: None,
            colour: None,
        });

        let cmds = wire(&s.compile_cue(s.cue("c1").unwrap()).unwrap());
        assert_eq!(
            cmds,
            vec!["PLAY 1-10 opener_l MIX 25", "PLAY 2-10 opener_r"]
        );
    }

    #[test]
    fn an_unknown_screen_fails_the_whole_cue() {
        let s = show();
        let cue = Cue {
            id: "c".into(),
            name: "bad".into(),
            actions: vec![
                Action::Stop { screen: "left".into() },
                Action::Stop { screen: "ghost".into() },
            ],
            follow: None,
            colour: None,
        };
        assert_eq!(s.compile_cue(&cue), Err(UnknownScreen("ghost".into())));
    }

    #[test]
    fn a_raw_action_passes_straight_through() {
        let s = show();
        let cmds = wire(&s
            .compile_action(&Action::Raw { command: "CLEAR ALL".into() })
            .unwrap());
        assert_eq!(cmds, vec!["CLEAR ALL"]);
    }

    #[test]
    fn opacity_actions_carry_their_fade() {
        let s = show();
        let cmds = wire(&s
            .compile_action(&Action::Opacity {
                screen: "left".into(),
                value: 0.0,
                frames: 50,
                tween: Some("easeoutquad".into()),
            })
            .unwrap());
        assert_eq!(cmds, vec!["MIXER OPACITY 1-10 0 50 easeoutquad"]);
    }

    #[test]
    fn load_then_take_is_a_two_step_change() {
        let s = show();
        let load = wire(&s
            .compile_action(&Action::Load {
                screen: "left".into(),
                clip: "next".into(),
                looping: false,
                transition: None,
            })
            .unwrap());
        let take = wire(&s.compile_action(&Action::Take { screen: "left".into() }).unwrap());
        assert_eq!(load, vec!["LOADBG 1-10 next"]);
        assert_eq!(take, vec!["PLAY 1-10"]);
    }

    #[test]
    fn screens_sharing_a_layer_are_warned_about() {
        let mut s = show();
        assert!(s.warnings().is_empty());
        s.screens[1].channel = 1;
        s.screens[1].layer = 10;
        assert_eq!(s.warnings().len(), 1);
        assert!(s.warnings()[0].contains("channel 1 layer 10"));
    }

    #[test]
    fn a_pad_pointing_at_a_missing_cue_is_warned_about() {
        let mut s = show();
        s.pads.push(Pad { index: 0, cue: "ghost".into() });
        assert_eq!(s.warnings().len(), 1);
        assert!(s.warnings()[0].contains("ghost"));
    }

    #[test]
    fn a_show_round_trips_through_json() {
        let s = show();
        let json = serde_json::to_string(&s).unwrap();
        let back: Show = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn a_minimal_show_file_fills_in_defaults() {
        let s: Show = serde_json::from_str(r#"{"screens":[]}"#).unwrap();
        assert_eq!(s.canvas, Canvas::default());
        assert_eq!(s.grid, (8, 4));
        assert_eq!(s.name, "Untitled show");
    }
}
