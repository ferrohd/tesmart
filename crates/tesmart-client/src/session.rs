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

/// One conversation with one switch.
pub struct Session<C: Connector, P: Profile = Family> {
    connector: C,
    engine: Engine<P, Instant>,
    conn: Option<C::Io>,
    policy: ConnectionPolicy,
    operation_timeout: Duration,
    rx: Box<[u8; RX_CAPACITY]>,
    unsolicited: VecDeque<Event>,
    stats: Stats,
}

impl<C: Connector, P: Profile> Session<C, P> {
    /// A session for `profile`, with the default protocol [`Policy`] and connection policy.
    pub fn new(connector: C, profile: P) -> Self {
        Self::with_policy(connector, profile, Policy::DEFAULT)
    }

    /// A session with an explicit protocol policy (timings and ASCII dialect).
    ///
    /// The connection policy starts at whatever the transport
    /// [prefers](Connector::preferred_policy); override it with
    /// [`set_connection_policy`](Self::set_connection_policy).
    ///
    /// The policy's [`ascii_scope`](Policy::ascii_scope) is taken from the transport rather than
    /// from the caller: a matrix reached over RS232 needs the `MT80xx` wrapped network commands,
    /// and which transport is in use is not something a caller should have to restate.
    pub fn with_policy(connector: C, profile: P, policy: Policy) -> Self {
        let connection_policy = connector.preferred_policy();
        let policy = policy.with_ascii_scope(scope_for(connector.kind()));
        Self {
            connector,
            engine: Engine::with_policy(profile, policy),
            conn: None,
            policy: connection_policy,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
            rx: Box::new([0; RX_CAPACITY]),
            unsolicited: VecDeque::new(),
            stats: Stats::default(),
        }
    }

    /// The connector in use.
    pub fn connector(&self) -> &C {
        &self.connector
    }

    /// What kind of link this session runs over.
    pub fn transport_kind(&self) -> TransportKind {
        self.connector.kind()
    }

    /// The protocol family in use.
    pub fn family(&self) -> Family {
        self.engine.profile().family()
    }

    /// The protocol timings in use.
    pub fn protocol_policy(&self) -> &Policy {
        self.engine.policy()
    }

    /// Replaces the protocol timings. Takes effect on the next exchange.
    ///
    /// The ASCII scope is re-derived from the transport, for the same reason
    /// [`with_policy`](Self::with_policy) derives it.
    pub fn set_protocol_policy(&mut self, policy: Policy) {
        let scope = scope_for(self.connector.kind());
        self.engine.set_policy(policy.with_ascii_scope(scope));
    }

    /// When connections are opened and closed.
    pub fn connection_policy(&self) -> ConnectionPolicy {
        self.policy
    }

    /// Changes when connections are opened and closed.
    pub fn set_connection_policy(&mut self, policy: ConnectionPolicy) {
        self.policy = policy;
    }

    /// How long a whole operation may take.
    pub fn operation_timeout(&self) -> Duration {
        self.operation_timeout
    }

    /// Sets how long a whole operation may take.
    ///
    /// Must comfortably exceed the protocol policy's retry budget, or reads will be cut off
    /// before the engine has finished retrying them.
    pub fn set_operation_timeout(&mut self, timeout: Duration) {
        self.operation_timeout = timeout;
    }

    /// Whether a connection is currently open.
    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Counters since the session was created.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Port reports that arrived while something else was going on, oldest first.
    pub fn take_unsolicited(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.unsolicited.drain(..)
    }

    /// Sends a request and waits for its answer.
    ///
    /// For a command the protocol defines no reply to, the answer is
    /// [`Response::Accepted`]: the bytes reached the transport, and nothing more can be known.
    ///
    /// # Errors
    ///
    /// [`ClientError::Rejected`] if the active family has no such command (nothing is sent),
    /// [`ClientError::NoReply`] if the switch stayed silent through every retry,
    /// [`ClientError::Transport`] if the link failed, [`ClientError::Timeout`] if the operation
    /// budget ran out.
    pub async fn exchange(&mut self, request: Request) -> Result<Response, ClientError> {
        let kind = request.kind();
        self.clear_stale_exchange();

        let id = self.engine.submit(request, Instant::now())?;
        self.stats.exchanges += 1;
        tracing::debug!(
            target: "tesmart::session",
            %id, request = %request, switch = %self.connector,
            "exchange started"
        );

        let result = self.pump(id, Some(kind)).await;
        self.settle(&result).await;
        result
    }

    /// Sends bytes that bypass the profile tables, for probing an undocumented model.
    ///
    /// With `expect_reply`, the first thing that comes back is returned whatever it is; with
    /// `false`, the call returns as soon as the bytes are written.
    ///
    /// # Errors
    ///
    /// As [`exchange`](Self::exchange), plus [`ClientError::Rejected`] if `bytes` is longer than
    /// one protocol unit.
    pub async fn exchange_raw(
        &mut self,
        bytes: &[u8],
        expect_reply: bool,
    ) -> Result<Option<Response>, ClientError> {
        self.clear_stale_exchange();

        let id = self.engine.submit_raw(bytes, expect_reply, Instant::now())?;
        self.stats.exchanges += 1;
        tracing::debug!(
            target: "tesmart::session",
            %id, bytes = %Hex(bytes), expect_reply,
            "raw exchange started"
        );

        let result = self.pump(id, None).await;
        self.settle(&result).await;

        match result {
            Ok(Response::Accepted) if !expect_reply => Ok(None),
            Ok(response) => Ok(Some(response)),
            Err(error) => Err(error),
        }
    }

    /// Waits for the switch to say something on its own.
    ///
    /// Only [`Event::PortChanged`] is returned; diagnostics are logged and counted. Requires a
    /// [`ConnectionPolicy::Persistent`] session, since a switch only pushes reports to a
    /// connected client.
    ///
    /// # Errors
    ///
    /// [`ClientError::Transport`] if the link fails or the switch closes it.
    pub async fn next_event(&mut self) -> Result<Event, ClientError> {
        loop {
            if let Some(event) = self.unsolicited.pop_front() {
                return Ok(event);
            }

            let mut drained = false;
            while let Some(event) = self.engine.poll_event() {
                self.record(event);
                drained = true;
            }
            if drained {
                continue;
            }

            self.connect().await?;
            self.wait(self.engine.poll_timeout(), None).await?;
        }
    }

    /// Closes the connection, if one is open.
    ///
    /// The close is clean (a shutdown, not a drop), because the protocol notes say unsolicited
    /// reports stop after a clean close.
    pub async fn disconnect(&mut self) {
        if let Some(mut io) = self.conn.take() {
            let _shutdown = io.shutdown().await;
            self.stats.disconnections += 1;
            tracing::debug!(target: "tesmart::session", switch = %self.connector, "closed");
        }
    }

    /// Runs one exchange to completion.
    async fn pump(
        &mut self,
        id: ExchangeId,
        kind: Option<RequestKind>,
    ) -> Result<Response, ClientError> {
        let deadline = Instant::now() + self.operation_timeout;
        let mut written = false;

        loop {
            if let Some(outcome) = self.collect(id, kind) {
                return outcome;
            }

            if let Some(transmit) = self.engine.poll_transmit(Instant::now()) {
                self.write(&transmit).await?;
                written = true;
                continue;
            }

            let timer = self.engine.poll_timeout();

            if !written {
                // The engine is holding the command gap. Wait for it before opening a
                // connection, so a per-command session does not sit on an idle socket.
                let Some(ready_at) = timer else {
                    // The engine went idle without finishing our exchange: it was reset.
                    return Err(ClientError::Aborted);
                };
                tracing::trace!(
                    target: "tesmart::session",
                    wait = ?ready_at.saturating_duration_since(Instant::now()),
                    "waiting for the command gap"
                );
                if !sleep_until_bounded(ready_at, deadline).await {
                    return Err(self.timed_out());
                }
                continue;
            }

            self.connect().await?;
            if !self.wait(timer, Some(deadline)).await? {
                return Err(self.timed_out());
            }
        }
    }

    /// Waits for a read, the engine's deadline, or the operation budget.
    ///
    /// Returns `false` if the operation budget expired.
    async fn wait(
        &mut self,
        timer: Option<Instant>,
        deadline: Option<Instant>,
    ) -> Result<bool, ClientError> {
        // Disjoint field borrows: the read holds `conn` and `rx` while the engine is updated.
        let Self { engine, conn, rx, stats, connector, .. } = self;
        let Some(io) = conn.as_mut() else {
            return Err(ClientError::Transport(TransportError::Closed));
        };

        let timer_at = timer.unwrap_or_else(Instant::now);
        let deadline_at = deadline.unwrap_or_else(Instant::now);

        tokio::select! {
            biased;

            () = tokio::time::sleep_until(deadline_at), if deadline.is_some() => Ok(false),

            read = io.read(rx.as_mut_slice()) => {
                let count = read.map_err(TransportError::read)?;
                if count == 0 {
                    tracing::debug!(target: "tesmart::session", switch = %connector, "switch closed the connection");
                    *conn = None;
                    return Err(ClientError::Transport(TransportError::Closed));
                }
                stats.bytes_rx += count as u64;
                tracing::debug!(
                    target: "tesmart::wire",
                    dir = "rx", bytes = %Hex(&rx[..count]),
                    "read"
                );
                engine.handle_input(&rx[..count], Instant::now());
                Ok(true)
            }

            () = tokio::time::sleep_until(timer_at), if timer.is_some() => {
                engine.handle_timeout(Instant::now());
                Ok(true)
            }
        }
    }

    /// Pulls events out of the engine, returning the outcome of exchange `id` if it finished.
}
