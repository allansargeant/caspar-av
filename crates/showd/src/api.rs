//! The HTTP + WebSocket API the console talks to.
//!
//! Shaped the same way as the rest of this family of tools: `GET /api/state`
//! and `WS /ws/ui` carry the whole snapshot, everything else is a command that
//! changes something and returns nothing interesting. The console is a mirror,
//! not a second source of truth, so there is no partial-update protocol to keep
//! in step.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use amcp::{commands as c, Anim, Command};

use crate::bridge::{Bridge, Error};
use crate::show::{Action, Cue, Pad, Screen, Show};

/// Build the API router.
pub fn router(bridge: Bridge) -> Router {
    Router::new()
        .route("/api/state", get(state))
        .route("/api/telemetry", get(telemetry))
        .route("/api/command", post(raw_command))
        .route("/api/batch", post(raw_batch))
        .route("/api/library/refresh", post(refresh_library))
        .route("/api/media/:id/thumbnail", get(thumbnail))
        .route("/api/show", get(get_show).put(put_show))
        .route("/api/mapping/push", post(push_mapping))
        .route("/api/screens", post(add_screen))
        .route("/api/screens/:id", axum::routing::patch(patch_screen).delete(delete_screen))
        .route("/api/screens/:id/transport", post(transport))
        .route("/api/screens/:id/mixer", post(mixer))
        .route("/api/screens/:id/template", post(template))
        .route("/api/cues", post(add_cue))
        .route("/api/cues/:id", axum::routing::patch(patch_cue).delete(delete_cue))
        .route("/api/cues/:id/fire", post(fire_cue))
        .route("/api/pads", axum::routing::put(put_pads))
        .route("/ws/ui", get(ws_ui))
        .with_state(bridge)
}

// ------------------------------------------------------------------- errors

/// An API failure, rendered as JSON so the console can show the server's own
/// words rather than a generic "request failed".
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        let code = match e {
            // Offline is the server's fault, not the request's.
            Error::Offline => StatusCode::SERVICE_UNAVAILABLE,
            Error::Show(_) => StatusCode::BAD_REQUEST,
            Error::Amcp(_) => StatusCode::BAD_GATEWAY,
        };
        ApiError(code, e.to_string())
    }
}

impl From<crate::show::UnknownScreen> for ApiError {
    fn from(e: crate::show::UnknownScreen) -> Self {
        ApiError(StatusCode::BAD_REQUEST, e.to_string())
    }
}

fn not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("no such {what}"))
}

type ApiResult<T = Json<serde_json::Value>> = Result<T, ApiError>;

fn ok() -> ApiResult {
    Ok(Json(json!({ "ok": true })))
}

// -------------------------------------------------------------------- state

async fn state(State(b): State<Bridge>) -> impl IntoResponse {
    Json(b.snapshot())
}

async fn telemetry(State(b): State<Bridge>) -> impl IntoResponse {
    Json(b.telemetry_raw())
}

/// Push the snapshot whenever it changes.
///
/// Diffing the serialised form is crude but exactly right here: the snapshot is
/// small, and it means a channel sitting idle costs one comparison per tick
/// instead of a frame's worth of traffic.
async fn ws_ui(ws: WebSocketUpgrade, State(b): State<Bridge>) -> Response {
    ws.on_upgrade(|socket| ui_socket(socket, b))
}

async fn ui_socket(mut socket: WebSocket, bridge: Bridge) {
    let mut last = String::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    loop {
        ticker.tick().await;
        let Ok(json) = serde_json::to_string(&bridge.snapshot()) else {
            continue;
        };
        if json == last {
            continue;
        }
        if socket.send(Message::Text(json.clone())).await.is_err() {
            return;
        }
        last = json;
    }
}

// ----------------------------------------------------------- raw AMCP access

#[derive(Deserialize)]
struct RawCommand {
    command: String,
}

/// Send one arbitrary AMCP command.
///
/// A media server without a command line is a media server you cannot rescue at
/// 30 seconds to doors, so this is deliberately unrestricted.
async fn raw_command(State(b): State<Bridge>, Json(req): Json<RawCommand>) -> ApiResult {
    let resp = b.send(Command::new(req.command)).await?;
    Ok(Json(json!({ "code": resp.code, "status": resp.status, "lines": resp.lines })))
}

#[derive(Deserialize)]
struct RawBatch {
    commands: Vec<String>,
}

async fn raw_batch(State(b): State<Bridge>, Json(req): Json<RawBatch>) -> ApiResult {
    let cmds = req.commands.into_iter().map(Command::new).collect();
    let resp = b.batch(cmds).await?;
    Ok(Json(json!({ "code": resp.code, "status": resp.status })))
}

// ------------------------------------------------------------------ library

async fn refresh_library(State(b): State<Bridge>) -> ApiResult {
    b.refresh_library().await;
    ok()
}

/// Proxy a thumbnail from media-scanner so the console stays single-origin.
async fn thumbnail(State(b): State<Bridge>, Path(id): Path<String>) -> Response {
    match b.thumbnail(&id).await {
        Some(png) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                // Thumbnails only change when the file does, and the id is the
                // path, so a short cache saves a request per tile per redraw.
                (header::CACHE_CONTROL, "public, max-age=60"),
            ],
            png,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// --------------------------------------------------------------------- show

async fn get_show(State(b): State<Bridge>) -> impl IntoResponse {
    Json(b.show())
}

async fn put_show(State(b): State<Bridge>, Json(show): Json<Show>) -> ApiResult {
    b.set_show(show);
    ok()
}

async fn push_mapping(State(b): State<Bridge>) -> ApiResult {
    b.push_mapping().await?;
    ok()
}

// ------------------------------------------------------------------ screens

async fn add_screen(State(b): State<Bridge>, Json(screen): Json<Screen>) -> ApiResult {
    b.edit_show(|s| s.screens.push(screen));
    ok()
}

async fn patch_screen(
    State(b): State<Bridge>,
    Path(id): Path<String>,
    Json(screen): Json<Screen>,
) -> ApiResult {
    let found = b.edit_show(|s| match s.screens.iter_mut().find(|s| s.id == id) {
        Some(slot) => {
            *slot = screen;
            true
        }
        None => false,
    });
    if !found {
        return Err(not_found("screen"));
    }
    // Geometry edits are meant to be seen while dragging, so push immediately
    // rather than waiting for an explicit apply.
    let _ = b.push_mapping().await;
    ok()
}

async fn delete_screen(State(b): State<Bridge>, Path(id): Path<String>) -> ApiResult {
    b.edit_show(|s| s.screens.retain(|s| s.id != id));
    ok()
}

/// What to do to a screen's transport.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum TransportRequest {
    Play {
        clip: String,
        #[serde(default)]
        looping: bool,
        #[serde(default)]
        frames: u32,
    },
    Load {
        clip: String,
        #[serde(default)]
        looping: bool,
        #[serde(default)]
        frames: u32,
    },
    Take,
    Pause,
    Resume,
    Stop,
    Clear,
}

async fn transport(
    State(b): State<Bridge>,
    Path(id): Path<String>,
    Json(req): Json<TransportRequest>,
) -> ApiResult {
    let action = match req {
        TransportRequest::Play { clip, looping, frames } => Action::Play {
            screen: id,
            clip,
            looping,
            transition: mix_of(frames),
        },
        TransportRequest::Load { clip, looping, frames } => Action::Load {
            screen: id,
            clip,
            looping,
            transition: mix_of(frames),
        },
        TransportRequest::Take => Action::Take { screen: id },
        TransportRequest::Pause => Action::Pause { screen: id },
        TransportRequest::Resume => Action::Resume { screen: id },
        TransportRequest::Stop => Action::Stop { screen: id },
        TransportRequest::Clear => Action::Clear { screen: id },
    };

    let commands = b.edit_show(|s| s.compile_action(&action))?;
    b.batch(commands).await?;
    ok()
}

fn mix_of(frames: u32) -> Option<crate::show::TransitionSpec> {
    (frames > 0).then(|| crate::show::TransitionSpec {
        kind: "mix".into(),
        frames,
        tween: None,
        direction: None,
        sting: None,
    })
}

/// A mixer change on a screen's layer.
#[derive(Deserialize)]
struct MixerRequest {
    /// `opacity`, `volume`, `brightness`, `saturation`, `contrast`, `rotation`,
    /// `fill`, `clip`, `crop`, `anchor`, `perspective`, `blend`, `keyer`.
    property: String,
    #[serde(default)]
    values: Vec<f64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    frames: u32,
    #[serde(default)]
    tween: Option<String>,
}

async fn mixer(
    State(b): State<Bridge>,
    Path(id): Path<String>,
    Json(req): Json<MixerRequest>,
) -> ApiResult {
    let (ch, ly) = b
        .edit_show(|s| s.screen(&id).map(|s| (s.channel, s.layer)))
        .ok_or_else(|| not_found("screen"))?;

    let anim = match &req.tween {
        Some(t) => Anim::eased(req.frames, t.clone()),
        None => Anim::frames(req.frames),
    };
    let v = |i: usize| req.values.get(i).copied().unwrap_or(0.0);

    let need = |n: usize| -> Result<(), ApiError> {
        if req.values.len() < n {
            Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("{} needs {n} values, got {}", req.property, req.values.len()),
            ))
        } else {
            Ok(())
        }
    };

    let command = match req.property.to_lowercase().as_str() {
        "opacity" => {
            need(1)?;
            c::mixer_opacity(ch, ly, v(0), &anim)
        }
        "volume" => {
            need(1)?;
            c::mixer_volume(ch, ly, v(0), &anim)
        }
        "brightness" => {
            need(1)?;
            c::mixer_brightness(ch, ly, v(0), &anim)
        }
        "saturation" => {
            need(1)?;
            c::mixer_saturation(ch, ly, v(0), &anim)
        }
        "contrast" => {
            need(1)?;
            c::mixer_contrast(ch, ly, v(0), &anim)
        }
        "rotation" => {
            need(1)?;
            c::mixer_rotation(ch, ly, v(0), &anim)
        }
        "fill" => {
            need(4)?;
            c::mixer_fill(ch, ly, v(0), v(1), v(2), v(3), &anim)
        }
        "clip" => {
            need(4)?;
            c::mixer_clip(ch, ly, v(0), v(1), v(2), v(3), &anim)
        }
        "crop" => {
            need(4)?;
            c::mixer_crop(ch, ly, v(0), v(1), v(2), v(3), &anim)
        }
        "anchor" => {
            need(2)?;
            c::mixer_anchor(ch, ly, v(0), v(1), &anim)
        }
        "levels" => {
            need(5)?;
            c::mixer_levels(ch, ly, v(0), v(1), v(2), v(3), v(4), &anim)
        }
        "perspective" => {
            need(8)?;
            let corners = [(v(0), v(1)), (v(2), v(3)), (v(4), v(5)), (v(6), v(7))];
            c::mixer_perspective(ch, ly, &corners, &anim)
        }
        "keyer" => c::mixer_keyer(ch, ly, v(0) != 0.0),
        "invert" => c::mixer_invert(ch, ly, v(0) != 0.0),
        "blend" => {
            let mode = req.text.clone().ok_or_else(|| {
                ApiError(StatusCode::BAD_REQUEST, "blend needs a mode in `text`".into())
            })?;
            c::mixer_blend(ch, ly, &mode)
        }
        "clear" => c::mixer_clear_layer(ch, ly),
        other => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("unknown mixer property '{other}'"),
            ))
        }
    };

    b.send(command).await?;
    ok()
}

#[derive(Deserialize)]
struct TemplateRequest {
    template: String,
    #[serde(default)]
    cg_layer: u32,
    #[serde(default)]
    data: Option<String>,
    /// `add` (load and play), `update`, `stop`, `next`, `invoke`.
    #[serde(default = "add")]
    action: String,
    #[serde(default)]
    method: Option<String>,
}

fn add() -> String {
    "add".into()
}

async fn template(
    State(b): State<Bridge>,
    Path(id): Path<String>,
    Json(req): Json<TemplateRequest>,
) -> ApiResult {
    let (ch, ly) = b
        .edit_show(|s| s.screen(&id).map(|s| (s.channel, s.layer)))
        .ok_or_else(|| not_found("screen"))?;

    let command = match req.action.as_str() {
        "add" => c::cg_add(ch, ly, req.cg_layer, &req.template, true, req.data.as_deref()),
        "update" => c::cg_update(ch, ly, req.cg_layer, req.data.as_deref().unwrap_or("{}")),
        "stop" => c::cg_stop(ch, ly, req.cg_layer),
        "next" => c::cg_next(ch, ly, req.cg_layer),
        "invoke" => c::cg_invoke(
            ch,
            ly,
            req.cg_layer,
            req.method.as_deref().unwrap_or_default(),
        ),
        other => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("unknown template action '{other}'"),
            ))
        }
    };

    b.send(command).await?;
    ok()
}

// --------------------------------------------------------------------- cues

async fn add_cue(State(b): State<Bridge>, Json(cue): Json<Cue>) -> ApiResult {
    b.edit_show(|s| s.cues.push(cue));
    ok()
}

async fn patch_cue(
    State(b): State<Bridge>,
    Path(id): Path<String>,
    Json(cue): Json<Cue>,
) -> ApiResult {
    let found = b.edit_show(|s| match s.cues.iter_mut().find(|c| c.id == id) {
        Some(slot) => {
            *slot = cue;
            true
        }
        None => false,
    });
    if found {
        ok()
    } else {
        Err(not_found("cue"))
    }
}

async fn delete_cue(State(b): State<Bridge>, Path(id): Path<String>) -> ApiResult {
    b.edit_show(|s| {
        s.cues.retain(|c| c.id != id);
        // A pad pointing at a deleted cue would silently do nothing when
        // pressed, which is the worst way to find out.
        s.pads.retain(|p| p.cue != id);
    });
    ok()
}

async fn fire_cue(State(b): State<Bridge>, Path(id): Path<String>) -> ApiResult {
    b.fire_cue(&id).await?;
    ok()
}

async fn put_pads(State(b): State<Bridge>, Json(pads): Json<Vec<Pad>>) -> ApiResult {
    b.edit_show(|s| s.pads = pads);
    ok()
}
