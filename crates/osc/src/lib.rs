//! CasparCG telemetry: an OSC decoder, a UDP listener, and the state tree the
//! console renders.
//!
//! CasparCG does not answer status questions — it *pushes* them. Every frame,
//! the server sends a `#bundle` of OSC messages over UDP describing the whole
//! monitor state. Two ways to receive it, both handled here:
//!
//! - **Implicitly.** Connecting an AMCP client makes the server subscribe that
//!   client's *IP address* on the configured `default-port` (6250), unless
//!   `<disable-send-to-amcp-clients>` is set.
//! - **Explicitly**, and much better for a bridge: `OSC SUBSCRIBE <port>` over
//!   the AMCP session directs telemetry to a port this process chose, so it does
//!   not have to share 6250 with every other Caspar client on the machine.
//!
//! [`Listener`] binds a port; feed what it receives into [`Telemetry`].

pub mod decode;
pub mod state;

pub use decode::{decode_packet, Message, Value};
pub use state::{ChannelState, LayerState, SlotState, Telemetry};

use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// The port CasparCG sends OSC to unless configured otherwise.
pub const DEFAULT_PORT: u16 = 6250;

/// A bound UDP socket receiving OSC telemetry.
pub struct Listener {
    socket: UdpSocket,
    buf: Vec<u8>,
}

impl Listener {
    /// Bind a port. Pass port `0` to let the OS choose one — then read it back
    /// with [`Listener::port`] and hand it to `OSC SUBSCRIBE`.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr).await?;
        // Caspar's per-frame bundle can carry a lot of layers; 64 KiB is the
        // largest a UDP datagram can be anyway.
        Ok(Self { socket, buf: vec![0u8; 65_536] })
    }

    /// The port actually bound.
    pub fn port(&self) -> std::io::Result<u16> {
        Ok(self.socket.local_addr()?.port())
    }

    /// Await one packet and decode it.
    ///
    /// A packet that fails to decode is reported rather than returned; the
    /// caller should keep listening, since one bad datagram says nothing about
    /// the next.
    pub async fn recv(&mut self) -> std::io::Result<Vec<Message>> {
        loop {
            let (n, from) = self.socket.recv_from(&mut self.buf).await?;
            match decode_packet(&self.buf[..n]) {
                Ok(msgs) => return Ok(msgs),
                Err(e) => tracing::debug!(%from, error = %e, "discarding malformed OSC packet"),
            }
        }
    }
}
