//! A client for CasparCG's `media-scanner`.
//!
//! The scanner is a separate Node service (default port 8000) that watches the
//! media, template and font directories, probes files with ffprobe and keeps
//! thumbnails. CasparCG 2.5 does not do this itself: `CLS`, `TLS`, `FLS`,
//! `CINF` and every `THUMBNAIL` command are HTTP-proxied straight to the
//! scanner (`AMCPCommandsImpl.cpp:1451`), and return `501 … FAILED` when it is
//! not running.
//!
//! So there are two ways to list media, and they are not equal:
//!
//! - **Through AMCP** — a flat text list, and only what the proxy passes on.
//! - **Directly over HTTP** — `/media` returns full ffprobe metadata as JSON,
//!   `/media/thumbnail/<id>` returns a real PNG rather than base64 wrapped in a
//!   status line, and `/templates` returns each HTML template's **GDD** schema,
//!   which is what lets a template's data form be generated instead of typed.
//!
//! This client takes the direct path, and the daemon degrades to AMCP listings
//! when the scanner is absent.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The port media-scanner listens on unless configured otherwise.
pub const DEFAULT_PORT: u16 = 8000;

/// Something went wrong talking to the scanner.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("media-scanner request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("media-scanner returned {0}")]
    Status(u16),
}

/// One entry from `/media`.
///
/// Deliberately loose: the scanner's `mediainfo` shape has changed across
/// versions, so the fields worth showing are typed and the rest is kept intact
/// under `extra` for the inspector to display without this needing an update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    /// The id AMCP uses — the path under the media root, upper-cased, without
    /// its extension. This is what goes in a `PLAY` command.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(rename = "mediaSize", default)]
    pub size: Option<u64>,
    #[serde(rename = "mediaTime", default)]
    pub modified: Option<i64>,
    #[serde(default)]
    pub format: Option<MediaFormat>,
    #[serde(default)]
    pub streams: Vec<MediaStream>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl MediaItem {
    /// Duration in seconds, from the container or the longest stream.
    pub fn duration(&self) -> Option<f64> {
        self.format
            .as_ref()
            .and_then(|f| f.duration.as_ref())
            .and_then(|d| d.parse::<f64>().ok())
            .or_else(|| {
                self.streams
                    .iter()
                    .filter_map(|s| s.duration.as_ref()?.parse::<f64>().ok())
                    .fold(None, |acc: Option<f64>, d| Some(acc.map_or(d, |a| a.max(d))))
            })
    }

    /// The first video stream's dimensions.
    pub fn resolution(&self) -> Option<(u32, u32)> {
        self.streams.iter().find_map(|s| Some((s.width?, s.height?)))
    }

    /// What kind of media this is, by the same rule the scanner uses for its
    /// `CINF` line: a video stream that is long enough is a movie, a short one
    /// or an attached picture is a still, and audio-only is audio.
    pub fn kind(&self) -> MediaKind {
        let has_video = self.streams.iter().any(|s| s.codec_type() == Some("video"));
        let has_audio = self.streams.iter().any(|s| s.codec_type() == Some("audio"));
        match (has_video, has_audio) {
            (true, _) if self.duration().unwrap_or(0.0) > 1.0 => MediaKind::Movie,
            (true, _) => MediaKind::Still,
            (false, true) => MediaKind::Audio,
            _ => MediaKind::Unknown,
        }
    }
}

/// The broad category of a media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Movie,
    Still,
    Audio,
    Unknown,
}

/// Container-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaFormat {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub long_name: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub bit_rate: Option<String>,
}

/// One ffprobe stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaStream {
    #[serde(default)]
    pub codec: Option<serde_json::Value>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub channels: Option<u32>,
    #[serde(default)]
    pub sample_rate: Option<String>,
}

impl MediaStream {
    /// `video` / `audio`, which the scanner nests under `codec.type`.
    pub fn codec_type(&self) -> Option<&str> {
        self.codec.as_ref()?.get("type")?.as_str()
    }
}

/// A template from `/templates`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    #[serde(default)]
    pub path: Option<String>,
    /// `html`, `ft`, `wt`, `ct` or `swf`.
    #[serde(default)]
    pub r#type: Option<String>,
    /// The Graphics Data Definition embedded in an HTML template: a JSON
    /// Schema describing the fields the template expects, which the console
    /// turns into a real form instead of a free-text JSON box.
    #[serde(default)]
    pub gdd: Option<serde_json::Value>,
    /// Set when the scanner could not parse the template's GDD.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TemplatesResponse {
    #[serde(default)]
    templates: Vec<Template>,
}

/// A media-scanner client.
#[derive(Debug, Clone)]
pub struct Scanner {
    base: String,
    http: reqwest::Client,
}

impl Scanner {
    /// A client for a scanner at `host:port`.
    pub fn new(host: &str, port: u16) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { base: format!("http://{host}:{port}"), http }
    }

    /// The base URL being used.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Whether the scanner is reachable. Used to decide between the rich HTTP
    /// listing and the AMCP fallback, and to tell the operator *why* their
    /// media list is empty — the single most common CasparCG setup complaint.
    pub async fn is_up(&self) -> bool {
        matches!(self.http.get(format!("{}/media", self.base)).send().await, Ok(r) if r.status().is_success())
    }

    /// Every scanned media file, with metadata.
    pub async fn media(&self) -> Result<Vec<MediaItem>, Error> {
        self.get_json("/media").await
    }

    /// Metadata for one media id.
    pub async fn media_info(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.get_json(&format!("/media/info/{}", enc(&id.to_uppercase()))).await
    }

    /// Every template, with GDD schemas where the scanner could extract them.
    pub async fn templates(&self) -> Result<Vec<Template>, Error> {
        let r: TemplatesResponse = self.get_json("/templates").await?;
        Ok(r.templates)
    }

    /// Font names, parsed out of the AMCP-shaped `/fls` text response.
    pub async fn fonts(&self) -> Result<Vec<String>, Error> {
        let text = self.get_text("/fls").await?;
        Ok(parse_amcp_list(&text))
    }

    /// A thumbnail PNG. Returns `None` when the scanner has not made one.
    pub async fn thumbnail(&self, id: &str) -> Result<Option<Vec<u8>>, Error> {
        let url = format!("{}/media/thumbnail/{}", self.base, enc(&id.to_uppercase()));
        let res = self.http.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(Error::Status(res.status().as_u16()));
        }
        Ok(Some(res.bytes().await?.to_vec()))
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, Error> {
        let res = self.http.get(format!("{}{path}", self.base)).send().await?;
        if !res.status().is_success() {
            return Err(Error::Status(res.status().as_u16()));
        }
        Ok(res.json().await?)
    }

    async fn get_text(&self, path: &str) -> Result<String, Error> {
        let res = self.http.get(format!("{}{path}", self.base)).send().await?;
        if !res.status().is_success() {
            return Err(Error::Status(res.status().as_u16()));
        }
        Ok(res.text().await?)
    }
}

/// Percent-encode a path segment. Media ids are upper-cased paths and can
/// contain spaces and other characters that must not go into a URL raw.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Strip the `200 X OK` status line the scanner wraps its text listings in and
/// return the remaining non-empty lines.
fn parse_amcp_list(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| l.starts_with("20"))
        .map(|l| l.trim().trim_matches('"').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_amcp_wrapped_text_listings() {
        let text = "200 FLS OK\r\n\"ARIAL\"\r\n\"HELVETICA\"\r\n\r\n";
        assert_eq!(parse_amcp_list(text), vec!["ARIAL", "HELVETICA"]);
    }

    #[test]
    fn encodes_path_segments() {
        assert_eq!(enc("SHOW/MY CLIP"), "SHOW%2FMY%20CLIP");
        assert_eq!(enc("AMB"), "AMB");
    }

    #[test]
    fn media_kind_follows_the_streams() {
        let movie: MediaItem = serde_json::from_str(
            r#"{"name":"AMB","format":{"duration":"30.0"},
                "streams":[{"codec":{"type":"video"},"width":1920,"height":1080}]}"#,
        )
        .unwrap();
        assert_eq!(movie.kind(), MediaKind::Movie);
        assert_eq!(movie.duration(), Some(30.0));
        assert_eq!(movie.resolution(), Some((1920, 1080)));

        let still: MediaItem = serde_json::from_str(
            r#"{"name":"LOGO","format":{"duration":"0.04"},
                "streams":[{"codec":{"type":"video"},"width":512,"height":512}]}"#,
        )
        .unwrap();
        assert_eq!(still.kind(), MediaKind::Still);

        let audio: MediaItem = serde_json::from_str(
            r#"{"name":"BED","format":{"duration":"120"},
                "streams":[{"codec":{"type":"audio"},"channels":2}]}"#,
        )
        .unwrap();
        assert_eq!(audio.kind(), MediaKind::Audio);
    }

    #[test]
    fn unknown_mediainfo_fields_survive_round_trip() {
        let item: MediaItem =
            serde_json::from_str(r#"{"name":"AMB","field_order":"progressive"}"#).unwrap();
        assert!(item.extra.contains_key("field_order"));
    }

    #[test]
    fn duration_falls_back_to_the_longest_stream() {
        let item: MediaItem = serde_json::from_str(
            r#"{"name":"X","streams":[{"duration":"5.0"},{"duration":"12.5"}]}"#,
        )
        .unwrap();
        assert_eq!(item.duration(), Some(12.5));
    }
}
