//! The telemetry state tree.
//!
//! CasparCG pushes its whole monitor state as OSC every frame, one message per
//! leaf: `/channel/1/stage/layer/10/foreground/file/time` and so on. Two things
//! are built from that stream here:
//!
//! - a **raw tree**, a nested JSON mirror of every path the server has ever
//!   sent, which survives the server growing new keys without any change here;
//! - a **digest**, the small typed view the console actually renders — channels,
//!   their layers, and what each layer is playing.
//!
//! The raw tree is kept because Caspar's key set differs by version, producer
//! and consumer, and a bridge that only understood a fixed list would quietly
//! drop whatever it had not been taught.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Map, Value as Json};

use crate::decode::{Message, Value};

/// How many packets a key may go unreported before it is dropped.
///
/// The server re-sends its whole monitor state every frame, so a key that stops
/// arriving has genuinely gone away. Expiring them matters: without it, loading
/// a colour producer over a clip leaves the clip's `file/name` and `file/time`
/// behind, and the console cheerfully reports a colour producer playing a file
/// that stopped minutes ago. Observed on a live 2.5.0 server.
///
/// Counted in packets rather than seconds so it needs no clock and stays
/// deterministic under test. Generous enough that a state split across several
/// datagrams is never mistaken for a key disappearing.
const STALE_AFTER_PACKETS: u64 = 50;

/// Everything the server has told us, as a nested tree plus a typed digest.
///
/// Leaves are held flat, keyed by their full OSC address, because that is what
/// makes expiry tractable — pruning a nested tree in place is far more code for
/// the same result. The nested form is materialised on demand.
#[derive(Debug, Default)]
pub struct Telemetry {
    leaves: BTreeMap<String, Leaf>,
    generation: u64,
}

#[derive(Debug, Clone)]
struct Leaf {
    value: Json,
    seen: u64,
}

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The raw nested tree, built from the live leaves.
    pub fn raw(&self) -> Json {
        let mut root = Json::Object(Map::new());
        for (path, leaf) in &self.leaves {
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            let Some((last, parents)) = segments.split_last() else { continue };

            let mut node = &mut root;
            for seg in parents {
                // A leaf can become a branch when the server starts reporting
                // sub-keys under a path it used to report a value for.
                if !node.is_object() {
                    *node = Json::Object(Map::new());
                }
                node = node
                    .as_object_mut()
                    .expect("just made an object")
                    .entry((*seg).to_string())
                    .or_insert_with(|| Json::Object(Map::new()));
            }
            if !node.is_object() {
                *node = Json::Object(Map::new());
            }
            node.as_object_mut()
                .expect("just made an object")
                .insert((*last).to_string(), leaf.value.clone());
        }
        root
    }

    /// Apply one message.
    pub fn apply(&mut self, msg: &Message) {
        let path = msg.address.trim_start_matches('/');
        if path.is_empty() {
            return;
        }

        let value = match msg.args.len() {
            0 => Json::Null,
            1 => to_json(&msg.args[0]),
            _ => Json::Array(msg.args.iter().map(to_json).collect()),
        };

        self.leaves.insert(path.to_string(), Leaf { value, seen: self.generation });
    }

    /// Apply every message in a packet, then drop anything that has stopped
    /// being reported.
    pub fn apply_all(&mut self, msgs: &[Message]) {
        self.generation += 1;
        for m in msgs {
            self.apply(m);
        }
        let cutoff = self.generation.saturating_sub(STALE_AFTER_PACKETS);
        self.leaves.retain(|_, leaf| leaf.seen >= cutoff);
    }

    /// The typed view the console renders.
    pub fn digest(&self) -> Vec<ChannelState> {
        let root = self.raw();
        let Some(channels) = root.get("channel").and_then(Json::as_object) else {
            return Vec::new();
        };

        let mut out: Vec<ChannelState> = channels
            .iter()
            .filter_map(|(idx, ch)| {
                let index: u32 = idx.parse().ok()?;
                Some(ChannelState {
                    index,
                    format: ch.get("format").and_then(as_string),
                    framerate: ch.get("framerate").and_then(rational),
                    layers: layers_of(ch),
                })
            })
            .collect();
        out.sort_by_key(|c| c.index);
        out
    }
}

fn layers_of(ch: &Json) -> Vec<LayerState> {
    let Some(layers) = ch.pointer("/stage/layer").and_then(Json::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<LayerState> = layers
        .iter()
        .filter_map(|(idx, layer)| {
            let index: u32 = idx.parse().ok()?;
            Some(LayerState {
                index,
                foreground: slot_of(layer.get("foreground")),
                background: slot_of(layer.get("background")),
            })
        })
        .collect();
    out.sort_by_key(|l| l.index);
    out
}

/// Build a slot's view from what the server actually publishes.
///
/// The key set here was corrected against a live 2.5.0 server rather than read
/// off the header. What the ffmpeg producer emits per frame is only
/// `file/time`, `file/clip` and `loop` (`av_producer.cpp:985`); stream frame
/// rate is published once at open, under `file/streams/<n>/fps`, as a
/// *rational*. In particular there is **no** `file/frame` and no
/// `file/video/width` on the wire, however plausible those look in the source.
fn slot_of(slot: Option<&Json>) -> Option<SlotState> {
    let slot = slot?;
    let producer = slot.get("producer").and_then(as_string);
    // An empty slot is reported as the `empty` producer rather than omitted.
    if producer.as_deref() == Some("empty") {
        return None;
    }

    // `file/streams/0/fps` is `,ii` — numerator and denominator. `file/fps` is
    // accepted as a fallback for producers that publish a plain scalar.
    let fps = slot
        .pointer("/file/streams/0/fps")
        .and_then(rational)
        .or_else(|| slot.pointer("/file/fps").and_then(rational));

    // `file/time` is (position, duration) in seconds.
    let time = slot.pointer("/file/time").and_then(first_number);
    let duration = slot.pointer("/file/time").and_then(second_number);

    // Frame numbers are *derived*, not reported: operators think in frames, and
    // position × rate is exact here because both figures come from the server.
    let frame = match (time, fps) {
        (Some(t), Some(f)) => Some((t * f).round()),
        _ => None,
    };
    let frames = match (duration, fps) {
        (Some(d), Some(f)) => Some((d * f).round()),
        _ => None,
    };

    Some(SlotState {
        producer,
        paused: slot.get("paused").and_then(Json::as_bool).unwrap_or(false),
        path: slot.pointer("/file/path").and_then(as_string),
        name: slot.pointer("/file/name").and_then(as_string),
        time,
        duration,
        frame,
        frames,
        fps,
        loops: slot.get("loop").and_then(Json::as_bool).unwrap_or(false),
        // Trim points, when the clip was loaded with an in/out.
        clip_in: slot.pointer("/file/clip").and_then(first_number),
        clip_duration: slot.pointer("/file/clip").and_then(second_number),
        // Not published by the ffmpeg producer; other producers may.
        width: slot.pointer("/file/video/width").and_then(first_number),
        height: slot.pointer("/file/video/height").and_then(first_number),
    })
}

/// One channel's live state.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChannelState {
    pub index: u32,
    pub format: Option<String>,
    pub framerate: Option<f64>,
    pub layers: Vec<LayerState>,
}

/// One layer, and what is in its two slots.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LayerState {
    pub index: u32,
    pub foreground: Option<SlotState>,
    pub background: Option<SlotState>,
}

/// What is loaded in a layer's foreground or background.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SlotState {
    pub producer: Option<String>,
    pub paused: bool,
    pub path: Option<String>,
    pub name: Option<String>,
    pub time: Option<f64>,
    pub duration: Option<f64>,
    /// Derived as `time × fps` — the server publishes seconds, not frames.
    pub frame: Option<f64>,
    /// Derived as `duration × fps`.
    pub frames: Option<f64>,
    pub fps: Option<f64>,
    /// In-point in seconds, from `file/clip`.
    pub clip_in: Option<f64>,
    /// Trimmed duration in seconds, from `file/clip`.
    pub clip_duration: Option<f64>,
    #[serde(rename = "loop")]
    pub loops: bool,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

fn to_json(v: &Value) -> Json {
    match v {
        Value::Int(i) => json!(i),
        Value::Long(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::Double(d) => json!(d),
        Value::String(s) => json!(s),
        Value::Bool(b) => json!(b),
        Value::Blob(b) => json!(b.len()), // size only; the bytes are not useful here
        Value::Null | Value::Impulse => Json::Null,
    }
}

fn as_string(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Array(a) => a.first().and_then(as_string),
        _ => None,
    }
}

/// A rational sent as two numbers.
///
/// The server reports a channel's frame rate as `,ii` — numerator and
/// denominator — not as a single float. Observed on a live 2.5.0 server:
/// `/channel/1/framerate ,ii [25, 1]`. Reading it as a scalar yields nothing at
/// all, which is why this is a function and not a plain `as_f64`.
///
/// A single number is still accepted, so a server that reports it that way, or
/// a future one that changes, keeps working.
fn rational(v: &Json) -> Option<f64> {
    match v {
        Json::Array(a) => {
            let num = a.first()?.as_f64()?;
            match a.get(1).and_then(Json::as_f64) {
                Some(den) if den != 0.0 => Some(num / den),
                _ => Some(num),
            }
        }
        other => other.as_f64(),
    }
}

fn first_number(v: &Json) -> Option<f64> {
    match v {
        Json::Array(a) => a.first().and_then(Json::as_f64),
        other => other.as_f64(),
    }
}

fn second_number(v: &Json) -> Option<f64> {
    v.as_array()?.get(1).and_then(Json::as_f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(addr: &str, args: Vec<Value>) -> Message {
        Message { address: addr.into(), args }
    }

    #[test]
    fn builds_a_nested_tree() {
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/1/format", vec![Value::String("720p5000".into())]));
        assert_eq!(t.raw().pointer("/channel/1/format"), Some(&json!("720p5000")));
    }

    #[test]
    fn multiple_args_become_an_array() {
        let mut t = Telemetry::new();
        t.apply(&msg(
            "/channel/1/stage/layer/10/foreground/file/time",
            vec![Value::Float(1.5), Value::Float(30.0)],
        ));
        let raw = t.raw();
        let v = raw.pointer("/channel/1/stage/layer/10/foreground/file/time").unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn digest_pulls_out_channels_and_layers() {
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/1/format", vec![Value::String("1080p5000".into())]));
        t.apply(&msg("/channel/1/framerate", vec![Value::Double(50.0)]));
        t.apply(&msg(
            "/channel/1/stage/layer/10/foreground/producer",
            vec![Value::String("ffmpeg".into())],
        ));
        t.apply(&msg("/channel/1/stage/layer/10/foreground/paused", vec![Value::Bool(false)]));
        t.apply(&msg(
            "/channel/1/stage/layer/10/foreground/file/time",
            vec![Value::Float(1.5), Value::Float(30.0)],
        ));
        t.apply(&msg(
            "/channel/1/stage/layer/10/foreground/file/path",
            vec![Value::String("AMB.mp4".into())],
        ));

        let d = t.digest();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].index, 1);
        assert_eq!(d[0].format.as_deref(), Some("1080p5000"));
        assert_eq!(d[0].layers.len(), 1);
        let fg = d[0].layers[0].foreground.as_ref().unwrap();
        assert_eq!(fg.producer.as_deref(), Some("ffmpeg"));
        assert_eq!(fg.path.as_deref(), Some("AMB.mp4"));
        assert_eq!(fg.time, Some(1.5));
        assert_eq!(fg.duration, Some(30.0));
        assert!(!fg.paused);
    }

    #[test]
    fn a_playing_clip_matches_what_a_live_server_sends() {
        // This is a transcript, not an invention: every address, type tag and
        // value below was captured from CasparCG 2.5.0 playing an 8s clip.
        let mut t = Telemetry::new();
        let fg = "/channel/1/stage/layer/10/foreground";
        t.apply(&msg(&format!("{fg}/producer"), vec![Value::String("ffmpeg".into())]));
        t.apply(&msg(&format!("{fg}/paused"), vec![Value::Bool(false)]));
        t.apply(&msg(&format!("{fg}/loop"), vec![Value::Bool(true)]));
        t.apply(&msg(&format!("{fg}/file/name"), vec![Value::String("TESTCLIP".into())]));
        t.apply(&msg(
            &format!("{fg}/file/path"),
            vec![Value::String("/opt/caspar/media/TESTCLIP.mp4".into())],
        ));
        t.apply(&msg(&format!("{fg}/file/time"), vec![Value::Float(1.28), Value::Float(8.0)]));
        t.apply(&msg(&format!("{fg}/file/clip"), vec![Value::Float(0.0), Value::Float(8.0)]));
        t.apply(&msg(
            &format!("{fg}/file/streams/0/fps"),
            vec![Value::Int(25), Value::Int(1)],
        ));

        let s = t.digest()[0].layers[0].foreground.clone().unwrap();
        assert_eq!(s.producer.as_deref(), Some("ffmpeg"));
        assert_eq!(s.name.as_deref(), Some("TESTCLIP"));
        // OSC floats are 32-bit on the wire, so these are compared with a
        // tolerance rather than exactly — 1.28f32 widens to 1.2799999713897705.
        assert!((s.time.unwrap() - 1.28).abs() < 1e-6);
        assert_eq!(s.duration, Some(8.0));
        assert_eq!(s.fps, Some(25.0), "fps lives under file/streams/0/fps as a rational");
        assert_eq!(s.frame, Some(32.0), "1.28s × 25fps");
        assert_eq!(s.frames, Some(200.0));
        assert_eq!(s.clip_in, Some(0.0));
        assert_eq!(s.clip_duration, Some(8.0));
        assert!(s.loops);
        assert!(!s.paused);
    }

    #[test]
    fn keys_that_stop_being_reported_are_dropped() {
        // Observed live: playing a colour producer over a clip leaves the
        // clip's file/* keys in place, because the server simply stops sending
        // them rather than sending a tombstone. Without expiry the digest
        // reports a colour producer playing a file that finished long ago.
        let fg = "/channel/1/stage/layer/10/foreground";
        let mut t = Telemetry::new();
        t.apply_all(&[
            msg(&format!("{fg}/producer"), vec![Value::String("ffmpeg".into())]),
            msg(&format!("{fg}/file/name"), vec![Value::String("TESTCLIP".into())]),
            msg(&format!("{fg}/file/time"), vec![Value::Float(4.4), Value::Float(8.0)]),
        ]);
        assert_eq!(t.digest()[0].layers[0].foreground.as_ref().unwrap().name.as_deref(), Some("TESTCLIP"));

        // The colour producer reports only these; file/* simply stop arriving.
        for _ in 0..STALE_AFTER_PACKETS + 1 {
            t.apply_all(&[
                msg(&format!("{fg}/producer"), vec![Value::String("color".into())]),
                msg(&format!("{fg}/color"), vec![Value::String("#FF808080".into())]),
            ]);
        }

        let s = t.digest()[0].layers[0].foreground.clone().unwrap();
        assert_eq!(s.producer.as_deref(), Some("color"));
        assert_eq!(s.name, None, "the previous clip's name must not linger");
        assert_eq!(s.time, None, "nor its position");
    }

    #[test]
    fn a_key_is_kept_while_it_keeps_arriving() {
        let mut t = Telemetry::new();
        for _ in 0..STALE_AFTER_PACKETS * 3 {
            t.apply_all(&[msg("/channel/1/format", vec![Value::String("720p5000".into())])]);
        }
        assert_eq!(t.digest()[0].format.as_deref(), Some("720p5000"));
    }

    #[test]
    fn a_state_split_across_packets_is_not_mistaken_for_removal() {
        // Alternating halves of the state, as a large rig split over several
        // datagrams would look. Both must survive.
        let mut t = Telemetry::new();
        for i in 0..STALE_AFTER_PACKETS * 2 {
            if i % 2 == 0 {
                t.apply_all(&[msg("/channel/1/format", vec![Value::String("720p5000".into())])]);
            } else {
                t.apply_all(&[msg("/channel/1/framerate", vec![Value::Int(50), Value::Int(1)])]);
            }
        }
        let d = t.digest();
        assert_eq!(d[0].format.as_deref(), Some("720p5000"));
        assert_eq!(d[0].framerate, Some(50.0));
    }

    #[test]
    fn framerate_is_a_rational_pair_not_a_float() {
        // Exactly what a live 2.5.0 server sends: `/channel/1/framerate ,ii [25, 1]`.
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/1/framerate", vec![Value::Int(25), Value::Int(1)]));
        assert_eq!(t.digest()[0].framerate, Some(25.0));

        // 30000/1001 — the shape that makes the rational form necessary.
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/2/framerate", vec![Value::Int(30000), Value::Int(1001)]));
        assert!((t.digest()[0].framerate.unwrap() - 29.97).abs() < 0.01);
    }

    #[test]
    fn a_scalar_framerate_still_works() {
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/1/framerate", vec![Value::Double(50.0)]));
        assert_eq!(t.digest()[0].framerate, Some(50.0));
    }

    #[test]
    fn a_colour_producer_reports_no_file_fields() {
        // Observed live: the colour producer sends producer/color/paused and no
        // file/* keys at all, so every timing field must stay None rather than
        // defaulting to zero and rendering as a stopped clip.
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/1/stage/layer/10/foreground/producer", vec![Value::String("color".into())]));
        t.apply(&msg("/channel/1/stage/layer/10/foreground/color", vec![Value::String("#3050FFFF".into())]));
        t.apply(&msg("/channel/1/stage/layer/10/foreground/paused", vec![Value::Bool(false)]));

        let fg = t.digest()[0].layers[0].foreground.clone().unwrap();
        assert_eq!(fg.producer.as_deref(), Some("color"));
        assert_eq!(fg.time, None);
        assert_eq!(fg.duration, None);
        assert_eq!(fg.frame, None);
    }

    #[test]
    fn an_empty_producer_is_reported_as_no_slot() {
        let mut t = Telemetry::new();
        t.apply(&msg(
            "/channel/1/stage/layer/1/foreground/producer",
            vec![Value::String("empty".into())],
        ));
        let d = t.digest();
        assert!(d[0].layers[0].foreground.is_none());
    }

    #[test]
    fn channels_and_layers_come_out_in_numeric_order() {
        let mut t = Telemetry::new();
        for ch in [10u32, 2, 1] {
            t.apply(&msg(&format!("/channel/{ch}/format"), vec![Value::String("PAL".into())]));
            for layer in [20u32, 3] {
                t.apply(&msg(
                    &format!("/channel/{ch}/stage/layer/{layer}/foreground/producer"),
                    vec![Value::String("ffmpeg".into())],
                ));
            }
        }
        let d = t.digest();
        assert_eq!(d.iter().map(|c| c.index).collect::<Vec<_>>(), vec![1, 2, 10]);
        assert_eq!(d[0].layers.iter().map(|l| l.index).collect::<Vec<_>>(), vec![3, 20]);
    }

    #[test]
    fn a_leaf_can_become_a_branch() {
        // The server reports `file` as a value in some states and a subtree in
        // others; the later message must win rather than be dropped.
        let mut t = Telemetry::new();
        t.apply(&msg("/channel/1/stage/layer/1/foreground/file", vec![Value::Int(0)]));
        t.apply(&msg(
            "/channel/1/stage/layer/1/foreground/file/path",
            vec![Value::String("x.mp4".into())],
        ));
        assert_eq!(
            t.raw().pointer("/channel/1/stage/layer/1/foreground/file/path"),
            Some(&json!("x.mp4"))
        );
    }
}
