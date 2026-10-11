//! The async driver: the only place in the workspace that awaits anything.
//!
//! A [`Session`] owns a [`Engine`](tesmart_proto::Engine) and a connection, and turns the
//! engine's poll-based interface into `async fn`s. It is generic over the byte stream, so TCP,
//! RS232 and the emulator all run the same loop.

use core::fmt;
use core::time::Duration;
use std::collections::VecDeque;

use tesmart_proto::engine::{Event, ExchangeError, ExchangeId, Policy};
use tesmart_proto::wire::{AsciiScope, Hex, Transmit};
use tesmart_proto::{Engine, Family, Profile, Request, RequestKind, Response};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use crate::error::ClientError;
use crate::transport::{ConnectionPolicy, Connector, TransportError, TransportKind};

/// Read buffer. The switch's LAN bridge uses segments of about 48 bytes.
const RX_CAPACITY: usize = 512;

/// How many unsolicited events are kept before the oldest is dropped.
const UNSOLICITED_CAPACITY: usize = 32;

/// How long a whole operation may take, including connecting and command spacing.
const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Which spelling of the ASCII commands a transport calls for.
///
/// Matrices document their network commands twice, once "for LAN connection only" and once "for
/// RS232 connection only". The emulator is driven over in-process pipes, so it is treated as a
/// LAN connection, which is also what its own parser accepts either way.
const fn scope_for(kind: TransportKind) -> AsciiScope {
    match kind {
        TransportKind::Serial => AsciiScope::Serial,
        TransportKind::Tcp | TransportKind::Simulated => AsciiScope::Lan,
    }
}

/// Counters, for `-v` output and for asserting in tests what actually happened on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Stats {
    /// Exchanges started.
    pub exchanges: u64,
    /// Writes handed to the transport, retransmissions included.
    pub writes: u64,
    /// Writes that were retransmissions.
    pub retries: u64,
    /// Connections opened.
    pub connections: u64,
    /// Connections closed.
    pub disconnections: u64,
    /// Bytes written.
    pub bytes_tx: u64,
    /// Bytes read.
    pub bytes_rx: u64,
    /// Port reports nobody asked for.
    pub unsolicited: u64,
    /// Times the decoder had to throw bytes away.
    pub desyncs: u64,
    /// Replies that were malformed, such as a port index that is not a port.
    pub bad_replies: u64,
}
