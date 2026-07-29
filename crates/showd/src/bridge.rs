//! The bridge: one supervised connection to a CasparCG server, and the
//! snapshot the console mirrors.
//!
//! Everything the console shows comes from here, and the console holds no
//! authoritative state of its own — so two operators on two laptops see the
//! same thing, and a reconnecting browser is immediately correct.
//!
//! Telemetry deserves a note. Caspar pushes OSC over UDP, and by default it
//! pushes to the *IP* of every connected AMCP client on one shared port (6250).
//! On a machine also running the Caspar client, or a second copy of this
//! daemon, that port is contended. So on connect the bridge binds an ephemeral
//! port and asks for its own feed with `OSC SUBSCRIBE <port>`, which 2.5 added
//! for exactly this. If the server is older and ignores it, the shared port is
//! still bound as a fallback.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use amcp::{commands as c, Client, Command};
use casparosc::{ChannelState, Listener, Telemetry};
use scanner::{MediaItem, Scanner, Template};
use serde::{Deserialize, Serialize};

use crate::show::Show;

/// How many recent commands to keep for the console's log panel.
const LOG_CAPACITY: usize = 300;

/// How often to re-read the media library.
const LIBRARY_REFRESH: Duration = Duration::from_secs(15);

/// Where to find the server.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub amcp_port: u16,
    pub scanner_host: String,
    pub scanner_port: u16,
    /// The shared OSC port to bind as a fallback for pre-2.5 servers.
    pub osc_fallback_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            amcp_port: amcp::DEFAULT_PORT,
            scanner_host: "127.0.0.1".into(),
            scanner_port: scanner::DEFAULT_PORT,
            osc_fallback_port: casparosc::DEFAULT_PORT,
        }
    }
}

/// One line in the command log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: u64,
    pub command: String,
    pub code: Option<u16>,
    pub status: String,
    pub ok: bool,
}

/// The connection's health, as the console displays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Connecting,
    Connected,
    Down,
}

/// What the console mirrors.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub health: Health,
    pub server: ServerInfo,
    pub channels: Vec<ChannelState>,
    pub media: Vec<MediaItem>,
    pub templates: Vec<Template>,
    pub fonts: Vec<String>,
    pub scanner_up: bool,
    pub show: Show,
    /// Configuration problems worth surfacing before the show starts.
    pub warnings: Vec<String>,
    pub log: Vec<LogEntry>,
}

/// Static facts about the connected server.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
    pub version: Option<String>,
    /// The server's configured media/template/data/log paths, as `INFO PATHS`
    /// reports them — the fastest way to explain why a clip "isn't there".
    pub paths: Option<String>,
    /// The OSC port telemetry is actually arriving on.
    pub osc_port: Option<u16>,
}

#[derive(Default)]
struct Library {
    media: Vec<MediaItem>,
    templates: Vec<Template>,
    fonts: Vec<String>,
    up: bool,
}

struct Inner {
    config: Config,
    client: RwLock<Option<Client>>,
    connected: AtomicBool,
    /// Set once a connection attempt has completed, either way. Before that
    /// the honest answer is "connecting"; after it, a failure means down.
    attempted: AtomicBool,
    telemetry: Mutex<Telemetry>,
    library: RwLock<Library>,
    log: Mutex<VecDeque<LogEntry>>,
    info: RwLock<ServerInfo>,
    show: RwLock<Show>,
    scanner: Scanner,
}

/// A handle to the bridge. Cheap to clone.
#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

/// Why a command could not be run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not connected to a CasparCG server")]
    Offline,
    #[error(transparent)]
    Amcp(#[from] amcp::Error),
    #[error(transparent)]
    Show(#[from] crate::show::UnknownScreen),
}

impl Bridge {
    /// Start the bridge and its supervisor.
    pub fn spawn(config: Config, show: Show) -> Self {
        let scanner = Scanner::new(&config.scanner_host, config.scanner_port);
        let info = ServerInfo {
            host: config.host.clone(),
            port: config.amcp_port,
            ..Default::default()
        };
        let inner = Arc::new(Inner {
            config,
            client: RwLock::new(None),
            connected: AtomicBool::new(false),
            attempted: AtomicBool::new(false),
            telemetry: Mutex::new(Telemetry::new()),
            library: RwLock::new(Library::default()),
            log: Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)),
            info: RwLock::new(info),
            show: RwLock::new(show),
            scanner,
        });

        let bridge = Bridge { inner };
        tokio::spawn(bridge.clone().supervise());
        tokio::spawn(bridge.clone().refresh_library_forever());
        bridge
    }

    // ------------------------------------------------------------- accessors

    /// The current show.
    pub fn show(&self) -> Show {
        self.inner.show.read().unwrap().clone()
    }

    /// Replace the show.
    pub fn set_show(&self, show: Show) {
        *self.inner.show.write().unwrap() = show;
    }

    /// Mutate the show in place, returning whatever the closure returns.
    pub fn edit_show<T>(&self, f: impl FnOnce(&mut Show) -> T) -> T {
        f(&mut self.inner.show.write().unwrap())
    }

    /// True while an AMCP connection is up.
    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Relaxed)
    }

    /// Build the console's snapshot.
    pub fn snapshot(&self) -> Snapshot {
        let library = self.inner.library.read().unwrap();
        let show = self.inner.show.read().unwrap().clone();
        Snapshot {
            health: if self.is_connected() {
                Health::Connected
            } else if self.inner.attempted.load(Ordering::Relaxed) {
                Health::Down
            } else {
                Health::Connecting
            },
            server: self.inner.info.read().unwrap().clone(),
            channels: self.inner.telemetry.lock().unwrap().digest(),
            media: library.media.clone(),
            templates: library.templates.clone(),
            fonts: library.fonts.clone(),
            scanner_up: library.up,
            warnings: show.warnings(),
            show,
            log: self.inner.log.lock().unwrap().iter().cloned().collect(),
        }
    }

    /// The raw OSC tree, for the diagnostics view. Everything the server has
    /// said, including keys this build does not model.
    pub fn telemetry_raw(&self) -> serde_json::Value {
        self.inner.telemetry.lock().unwrap().raw()
    }

    // -------------------------------------------------------------- commands

    fn client(&self) -> Option<Client> {
        self.inner.client.read().unwrap().clone()
    }

    /// Send one command.
    pub async fn send(&self, command: Command) -> Result<amcp::Response, Error> {
        let client = self.client().ok_or(Error::Offline)?;
        let text = command.to_string();
        let result = client.send(command).await;
        self.log_result(&text, &result);
        Ok(result?)
    }

    /// Send several commands as one frame-accurate batch.
    pub async fn batch(&self, commands: Vec<Command>) -> Result<amcp::Response, Error> {
        if commands.is_empty() {
            return Err(Error::Offline);
        }
        let client = self.client().ok_or(Error::Offline)?;
        let text = commands.iter().map(Command::to_string).collect::<Vec<_>>().join(" · ");
        let result = client.batch(commands).await;
        self.log_result(&text, &result);
        Ok(result?)
    }

    /// Fire a cue by id.
    pub async fn fire_cue(&self, cue_id: &str) -> Result<amcp::Response, Error> {
        let commands = {
            let show = self.inner.show.read().unwrap();
            let cue = show
                .cue(cue_id)
                .ok_or_else(|| crate::show::UnknownScreen(cue_id.to_string()))?;
            show.compile_cue(cue)?
        };
        self.batch(commands).await
    }

    /// Push every screen's mapping to the server.
    pub async fn push_mapping(&self) -> Result<amcp::Response, Error> {
        let commands = self.inner.show.read().unwrap().mapping_commands();
        if commands.is_empty() {
            return Err(Error::Offline);
        }
        self.batch(commands).await
    }

    /// A thumbnail for a media id, straight from the scanner.
    pub async fn thumbnail(&self, id: &str) -> Option<Vec<u8>> {
        self.inner.scanner.thumbnail(id).await.ok().flatten()
    }

    fn log_result(&self, command: &str, result: &Result<amcp::Response, amcp::Error>) {
        let entry = match result {
            Ok(r) => LogEntry {
                at: now_ms(),
                command: command.to_string(),
                code: Some(r.code),
                status: r.status.clone(),
                ok: r.is_ok(),
            },
            Err(e) => LogEntry {
                at: now_ms(),
                command: command.to_string(),
                code: None,
                status: e.to_string(),
                ok: false,
            },
        };
        let mut log = self.inner.log.lock().unwrap();
        if log.len() == LOG_CAPACITY {
            log.pop_front();
        }
        log.push_back(entry);
    }

    // ------------------------------------------------------------ supervisor

    /// Connect, stay connected, and reconnect with backoff when the server
    /// goes away — which it does, every time someone restarts it mid-rig.
    async fn supervise(self) {
        let addr = format!("{}:{}", self.inner.config.host, self.inner.config.amcp_port);
        let mut backoff = Duration::from_millis(500);

        loop {
            match Client::connect(&addr).await {
                Ok(client) => {
                    backoff = Duration::from_millis(500);
                    self.inner.connected.store(true, Ordering::Relaxed);
                    self.inner.attempted.store(true, Ordering::Relaxed);
                    *self.inner.client.write().unwrap() = Some(client.clone());
                    tracing::info!(%addr, "connected to CasparCG");

                    self.on_connect(&client).await;

                    // Hold until the connection drops.
                    while !client.is_closed() {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }

                    tracing::warn!(%addr, "lost connection to CasparCG");
                    self.inner.connected.store(false, Ordering::Relaxed);
                    *self.inner.client.write().unwrap() = None;
                }
                Err(e) => {
                    tracing::debug!(%addr, error = %e, "connect failed");
                    self.inner.connected.store(false, Ordering::Relaxed);
                    self.inner.attempted.store(true, Ordering::Relaxed);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            }
        }
    }

    /// Identify the server and get telemetry flowing.
    async fn on_connect(&self, client: &Client) {
        if let Ok(r) = client.send(c::version()).await {
            self.inner.info.write().unwrap().version = r.single().map(str::to_string);
        }
        if let Ok(r) = client.send(c::info_paths()).await {
            self.inner.info.write().unwrap().paths = Some(r.lines.join("\n"));
        }

        // Ask for a private telemetry feed; fall back to the shared port.
        let bound = match Listener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await {
            Ok(listener) => match listener.port() {
                Ok(port) => match client.send(c::osc_subscribe(port)).await {
                    Ok(_) => Some((listener, port)),
                    Err(e) => {
                        tracing::info!(error = %e, "OSC SUBSCRIBE refused; using the shared port");
                        None
                    }
                },
                Err(_) => None,
            },
            Err(e) => {
                tracing::warn!(error = %e, "could not bind an OSC port");
                None
            }
        };

        let listener = match bound {
            Some((l, port)) => {
                self.inner.info.write().unwrap().osc_port = Some(port);
                Some(l)
            }
            None => {
                let port = self.inner.config.osc_fallback_port;
                match Listener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
                    Ok(l) => {
                        self.inner.info.write().unwrap().osc_port = Some(port);
                        Some(l)
                    }
                    Err(e) => {
                        // Not fatal: commands still work, the console just has
                        // no live position or fps to show.
                        tracing::warn!(port, error = %e, "no OSC telemetry — port unavailable");
                        None
                    }
                }
            }
        };

        if let Some(listener) = listener {
            tokio::spawn(self.clone().receive_telemetry(listener, client.clone()));
        }
    }

    /// Feed OSC into the state tree until the AMCP connection goes away.
    async fn receive_telemetry(self, mut listener: Listener, client: Client) {
        loop {
            let recv = tokio::time::timeout(Duration::from_secs(1), listener.recv()).await;
            match recv {
                Ok(Ok(msgs)) => self.inner.telemetry.lock().unwrap().apply_all(&msgs),
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "OSC socket failed");
                    return;
                }
                // A quiet second is normal when nothing is playing; the timeout
                // is only here so a dead connection ends this task.
                Err(_) => {}
            }
            if client.is_closed() {
                return;
            }
        }
    }

    /// Keep the media library current.
    async fn refresh_library_forever(self) {
        loop {
            self.refresh_library().await;
            tokio::time::sleep(LIBRARY_REFRESH).await;
        }
    }

    /// Re-read media, templates and fonts from the scanner.
    pub async fn refresh_library(&self) {
        let s = &self.inner.scanner;
        let media = s.media().await;
        let up = media.is_ok();
        let templates = s.templates().await.unwrap_or_default();
        let fonts = s.fonts().await.unwrap_or_default();

        let mut library = self.inner.library.write().unwrap();
        if let Ok(media) = media {
            library.media = media;
        }
        if up {
            library.templates = templates;
            library.fonts = fonts;
        }
        library.up = up;
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
