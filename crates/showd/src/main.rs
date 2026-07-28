//! `caspar-avd` — the caspar-AV bridge daemon.
//!
//! CasparCG speaks AMCP over raw TCP and pushes telemetry over UDP OSC. A
//! browser can do neither. This daemon sits between them: it holds the
//! connection, mirrors the server's state, owns the show, and serves the
//! console over plain HTTP.

mod api;
mod bridge;
mod show;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::http::{header, HeaderValue};
use clap::Parser;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use bridge::{Bridge, Config};
use show::Show;

#[derive(Parser, Debug)]
#[command(name = "caspar-avd", version, about = "caspar-AV bridge daemon")]
struct Args {
    /// CasparCG server host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// CasparCG AMCP port.
    #[arg(long, default_value_t = amcp::DEFAULT_PORT)]
    port: u16,

    /// media-scanner host.
    #[arg(long, default_value = "127.0.0.1")]
    scanner_host: String,

    /// media-scanner port.
    #[arg(long, default_value_t = scanner::DEFAULT_PORT)]
    scanner_port: u16,

    /// Shared OSC port to fall back to when the server is older than 2.5 and
    /// does not honour `OSC SUBSCRIBE`.
    #[arg(long, default_value_t = casparosc::DEFAULT_PORT)]
    osc_port: u16,

    /// Address to serve the console and API on.
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: SocketAddr,

    /// Show file to load at start and save to.
    #[arg(long)]
    show: Option<PathBuf>,

    /// Directory holding the built console.
    #[arg(long, default_value = "web")]
    web: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=warn".into()),
        )
        .init();

    let args = Args::parse();

    let show = match &args.show {
        Some(path) if path.exists() => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading show file {}", path.display()))?;
            let show: Show = serde_json::from_str(&text)
                .with_context(|| format!("parsing show file {}", path.display()))?;
            tracing::info!(path = %path.display(), screens = show.screens.len(), cues = show.cues.len(), "loaded show");
            show
        }
        Some(path) => {
            tracing::info!(path = %path.display(), "show file does not exist yet; starting empty");
            Show::default()
        }
        None => Show::default(),
    };

    let config = Config {
        host: args.host.clone(),
        amcp_port: args.port,
        scanner_host: args.scanner_host.clone(),
        scanner_port: args.scanner_port,
        osc_fallback_port: args.osc_port,
    };

    let bridge = Bridge::spawn(config, show);

    if let Some(path) = args.show.clone() {
        tokio::spawn(autosave(bridge.clone(), path));
    }

    // The console is a built SPA: hashed assets are safe to cache, but
    // index.html must not be, or an upgraded daemon can be driven by a stale
    // bundle against a changed API.
    let index = args.web.join("index.html");
    if !index.exists() {
        tracing::warn!(
            dir = %args.web.display(),
            "no console build found — run `npm ci && npm run build` in console/"
        );
    }
    let static_files = ServeDir::new(&args.web).fallback(ServeFile::new(&index));

    let app = api::router(bridge)
        .fallback_service(static_files)
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!(
        console = %format!("http://{}", args.bind),
        caspar = %format!("{}:{}", args.host, args.port),
        "caspar-avd running"
    );

    axum::serve(listener, app).await.context("serving")?;
    Ok(())
}

/// Persist the show when it changes.
///
/// Polling rather than write-through: edits arrive in bursts while someone
/// drags a screen around, and a show file is not worth an fsync per frame.
async fn autosave(bridge: Bridge, path: PathBuf) {
    let mut last = String::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let Ok(json) = serde_json::to_string_pretty(&bridge.show()) else {
            continue;
        };
        if json == last {
            continue;
        }
        // Write to a temporary file and rename, so an interrupted save cannot
        // leave a half-written show behind.
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &json).and_then(|_| std::fs::rename(&tmp, &path)) {
            tracing::warn!(path = %path.display(), error = %e, "could not save show");
            continue;
        }
        last = json;
    }
}
