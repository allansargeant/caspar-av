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

use serde::Serialize;
use serde_json::{json, Map, Value as Json};

use crate::decode::{Message, Value};

/// Everything the server has told us, as a nested tree plus a typed digest.
#[derive(Debug, Default)]
pub struct Telemetry {
    root: Json,
}

impl Telemetry {
    pub fn new() -> Self {
        Self { root: Json::Object(Map::new()) }
    }

    /// The raw nested tree.
    pub fn raw(&self) -> &Json {
        &self.root
    }

    /// Apply one message, creating intermediate nodes as needed.
    pub fn apply(&mut self, msg: &Message) {
        let segments: Vec<&str> = msg.address.trim_start_matches('/').split('/').collect();
        if segments.is_empty() || segments[0].is_empty() {
            return;
        }

        let value = match msg.args.len() {
            0 => Json::Null,
            1 => to_json(&msg.args[0]),
            _ => Json::Array(msg.args.iter().map(to_json).collect()),
        };

        let mut node = &mut self.root;
        for seg in &segments[..segments.len() - 1] {
            // A leaf can be replaced by a branch when the server starts
            // reporting sub-keys under a path it used to report a value for.
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
            .insert(segments[segments.len() - 1].to_string(), value);
    }

    /// Apply every message in a packet.
    pub fn apply_all(&mut self, msgs: &[Message]) {
        for m in msgs {
            self.apply(m);
        }
    }

    /// The typed view the console renders.
    pub fn digest(&self) -> Vec<ChannelState> {
        let Some(channels) = self.root.get("channel").and_then(Json::as_object) else {
            return Vec::new();
        };

        let mut out: Vec<ChannelState> = channels
            .iter()
            .filter_map(|(idx, ch)| {
                let index: u32 = idx.parse().ok()?;
                Some(ChannelState {
                    index,
                    format: ch.get("format").and_then(as_string),
                    framerate: ch.get("framerate").and_then(Json::as_f64),
                    profiler_time: ch
                        .pointer("/profiler/time")
                        .and_then(first_number),
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

fn slot_of(slot: Option<&Json>) -> Option<SlotState> {
    let slot = slot?;
    let producer = slot.get("producer").and_then(as_string);
    // An empty slot is reported as the `empty` producer rather than omitted.
    if producer.as_deref() == Some("empty") {
        return None;
    }
    Some(SlotState {
        producer,
        paused: slot.get("paused").and_then(Json::as_bool).unwrap_or(false),
        path: slot.pointer("/file/path").and_then(as_string),
        name: slot.pointer("/file/name").and_then(as_string),
        // `file/time` and `file/frame` are sent as (current, total) pairs.
        time: slot.pointer("/file/time").and_then(first_number),
        duration: slot.pointer("/file/time").and_then(second_number),
        frame: slot.pointer("/file/frame").and_then(first_number),
        frames: slot.pointer("/file/frame").and_then(second_number),
        fps: slot.pointer("/file/fps").and_then(first_number),
        loops: slot.get("loop").and_then(Json::as_bool).unwrap_or(false),
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
    /// Frame time in seconds, from the server's profiler — the honest measure
    /// of whether a channel is keeping up.
    pub profiler_time: Option<f64>,
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
    pub frame: Option<f64>,
    pub frames: Option<f64>,
    pub fps: Option<f64>,
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
        assert_eq!(t.raw().pointer("/channel/1/format").unwrap(), &json!("720p5000"));
    }

    #[test]
    fn multiple_args_become_an_array() {
        let mut t = Telemetry::new();
        t.apply(&msg(
            "/channel/1/stage/layer/10/foreground/file/time",
            vec![Value::Float(1.5), Value::Float(30.0)],
        ));
        let v = t.raw().pointer("/channel/1/stage/layer/10/foreground/file/time").unwrap();
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
            t.raw().pointer("/channel/1/stage/layer/1/foreground/file/path").unwrap(),
            &json!("x.mp4")
        );
    }
}
