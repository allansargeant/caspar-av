//! An async AMCP client.
//!
//! Every command is sent as `REQ <id> <COMMAND>` and matched to its reply by
//! the `RES <id>` prefix. That is not belt-and-braces: the server dispatches to
//! one queue per channel, so a reply for channel 2 can overtake a reply for
//! channel 1, and pairing replies by arrival order would silently mis-attribute
//! them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::command::Command;
use crate::response::{Decoder, Response};

/// The port CasparCG listens on for AMCP by default.
pub const DEFAULT_PORT: u16 = 5250;

/// How long a command waits for its reply before giving up. Generous, because
/// `THUMBNAIL GENERATE_ALL` and a cold `CLS` over a large media directory are
/// legitimately slow.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Something that went wrong talking to the server.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connection to the server was lost")]
    Disconnected,
    #[error("timed out waiting for a reply to {0}")]
    Timeout(String),
    #[error("{command} failed: {code} {status}")]
    Amcp { command: String, code: u16, status: String },
}

/// Reply slots awaiting a `RES <id>`.
///
/// A plain (non-async) mutex on purpose: every critical section is a single map
/// operation with no await inside it, so this can never block the runtime, and
/// it removes the possibility of a reply being misrouted because the lock
/// happened to be held when it arrived.
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>;

/// Lock helper — the guarded sections cannot panic, so poisoning is not a
/// state we need to model.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Outgoing {
    wire: String,
    /// `None` for fire-and-forget commands that the server never answers
    /// (`BEGIN`), so the writer does not register a reply slot that would
    /// never be filled.
    slot: Option<(String, oneshot::Sender<Response>)>,
}

/// A connected AMCP client. Cheap to clone; all clones share one connection.
#[derive(Clone)]
pub struct Client {
    tx: mpsc::UnboundedSender<Outgoing>,
    notifications: broadcast::Sender<Response>,
    next_id: Arc<AtomicU64>,
    pending: Pending,
    /// Set as soon as either half of the connection ends.
    ///
    /// Not derivable from the command channel: that only closes once a *write*
    /// has failed, so a server that goes away while nobody is sending looks
    /// alive indefinitely — and a supervisor watching for the drop never
    /// reconnects. The reader notices EOF immediately, so it is what sets this.
    closed: Arc<AtomicBool>,
}

impl Client {
    /// Connect to a CasparCG server's AMCP port.
    pub async fn connect(addr: impl ToSocketAddrs) -> Result<Self, Error> {
        let stream = TcpStream::connect(addr).await?;
        // Control traffic is small and latency-critical; Nagle would coalesce a
        // cue's commands into one delayed write.
        stream.set_nodelay(true)?;
        Ok(Self::from_stream(stream))
    }

    /// Drive an already-open stream. Split out so tests can run the whole
    /// client against an in-process fake server.
    pub fn from_stream(stream: TcpStream) -> Self {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::unbounded_channel::<Outgoing>();
        let (notifications, _) = broadcast::channel(256);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        // Writer: serialise commands, registering the reply slot *before* the
        // bytes go out so a fast reply can never arrive before its slot exists.
        {
            let pending = pending.clone();
            let closed = closed.clone();
            tokio::spawn(async move {
                while let Some(out) = rx.recv().await {
                    if let Some((id, slot)) = out.slot {
                        lock(&pending).insert(id, slot);
                    }
                    if writer.write_all(out.wire.as_bytes()).await.is_err() {
                        closed.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                // Dropping `pending` senders here would be wrong — the reader
                // task owns the same map and will clear it on close.
            });
        }

        // Reader: decode responses, route by id, broadcast the rest.
        {
            let pending = pending.clone();
            let notifications = notifications.clone();
            let closed = closed.clone();
            tokio::spawn(async move {
                let mut decoder = Decoder::new();
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    let n = match reader.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    for resp in decoder.feed(&buf[..n]) {
                        if resp.is_notification() {
                            let _ = notifications.send(resp);
                            continue;
                        }
                        let slot = resp.id.as_ref().and_then(|id| lock(&pending).remove(id));
                        match slot {
                            Some(slot) => {
                                let _ = slot.send(resp);
                            }
                            None => {
                                // Untagged, or a reply whose caller gave up.
                                let _ = notifications.send(resp);
                            }
                        }
                    }
                }
                // Connection closed: mark it so a supervisor notices without
                // having to send anything, and drop every slot so waiters see
                // `Disconnected` instead of hanging until the timeout.
                closed.store(true, Ordering::Relaxed);
                lock(&pending).clear();
            });
        }

        Self { tx, notifications, next_id: Arc::new(AtomicU64::new(1)), pending, closed }
    }

    fn next_id(&self) -> String {
        self.next_id.fetch_add(1, Ordering::Relaxed).to_string()
    }

    /// Send a command and wait for its reply.
    ///
    /// A 4xx/5xx reply is returned as [`Error::Amcp`] — the command reached the
    /// server and was refused, which callers nearly always want to treat as a
    /// failure rather than inspect.
    pub async fn send(&self, command: Command) -> Result<Response, Error> {
        let resp = self.send_raw(command.clone()).await?;
        if resp.is_error() {
            return Err(Error::Amcp {
                command: command.to_string(),
                code: resp.code,
                status: resp.status,
            });
        }
        Ok(resp)
    }

    /// Send a command and return its reply even when the status is an error.
    pub async fn send_raw(&self, command: Command) -> Result<Response, Error> {
        // PING is answered `PONG …` with no status code and no RES prefix, so
        // it can only be matched off the notification stream. Subscribing
        // *before* sending is what stops the reply racing past the subscriber.
        if command.name().eq_ignore_ascii_case("PING") {
            return self.ping(command).await;
        }

        if self.is_closed() {
            return Err(Error::Disconnected);
        }

        let id = self.next_id();
        let (slot, wait) = oneshot::channel();
        let wire = format!("REQ {id} {}", command.to_wire());

        self.tx
            .send(Outgoing { wire, slot: Some((id.clone(), slot)) })
            .map_err(|_| Error::Disconnected)?;

        match tokio::time::timeout(REPLY_TIMEOUT, wait).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(Error::Timeout(command.to_string()))
            }
        }
    }

    /// Execute several commands as one atomic batch.
    ///
    /// The server locks every touched channel, queues all the commands, then
    /// releases them together — so a multi-layer cue takes effect on a single
    /// frame instead of smeared across several. This is the mechanism a show
    /// cue is built on.
    ///
    /// Returns the batch's own reply: `202 COMMIT OK`, or `202 COMMIT PARTIAL`
    /// when at least one command in the batch failed. The individual replies
    /// arrive too and are surfaced on [`Client::notifications`].
    pub async fn batch(&self, commands: Vec<Command>) -> Result<Response, Error> {
        if commands.is_empty() {
            return Err(Error::Amcp {
                command: "BEGIN".into(),
                code: 400,
                status: "empty batch".into(),
            });
        }
        // A single command in a BEGIN/COMMIT is pointless overhead, and the
        // server shortcuts it anyway.
        if commands.len() == 1 {
            return self.send(commands.into_iter().next().unwrap()).await;
        }
        if self.is_closed() {
            return Err(Error::Disconnected);
        }

        let id = self.next_id();
        let (slot, wait) = oneshot::channel();

        // BEGIN carries the id the COMMIT reply will be tagged with, and is
        // itself never answered — hence `slot: None`.
        self.tx
            .send(Outgoing { wire: format!("REQ {id} BEGIN\r\n"), slot: None })
            .map_err(|_| Error::Disconnected)?;

        for c in &commands {
            let inner = self.next_id();
            self.tx
                .send(Outgoing { wire: format!("REQ {inner} {}", c.to_wire()), slot: None })
                .map_err(|_| Error::Disconnected)?;
        }

        self.tx
            .send(Outgoing { wire: "COMMIT\r\n".into(), slot: Some((id.clone(), slot)) })
            .map_err(|_| Error::Disconnected)?;

        match tokio::time::timeout(REPLY_TIMEOUT, wait).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(Error::Timeout("COMMIT".into()))
            }
        }
    }

    /// `PING`, matched off the notification stream.
    async fn ping(&self, command: Command) -> Result<Response, Error> {
        let mut notes = self.notifications.subscribe();
        // Sent without a REQ id: the server discards it for PING anyway.
        self.tx
            .send(Outgoing { wire: command.to_wire(), slot: None })
            .map_err(|_| Error::Disconnected)?;

        let wait = async {
            loop {
                match notes.recv().await {
                    Ok(r) if r.status.starts_with("PONG") => return Ok(r),
                    Ok(_) => continue,
                    Err(_) => return Err(Error::Disconnected),
                }
            }
        };

        match tokio::time::timeout(REPLY_TIMEOUT, wait).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout(command.to_string())),
        }
    }

    /// Subscribe to unsolicited server messages: 1xx notifications, and any
    /// reply that could not be matched to a waiting command.
    pub fn notifications(&self) -> broadcast::Receiver<Response> {
        self.notifications.subscribe()
    }

    /// True once the connection has gone away, in either direction.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed) || self.tx.is_closed()
    }
}
