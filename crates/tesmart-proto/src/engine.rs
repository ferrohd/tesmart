//! The sans-IO request/response engine.
//!
//! One [`Engine`] drives one switch. It owns everything about the conversation except the bytes
//! moving: encoding, demultiplexing, command spacing, reply deadlines and retries. The caller
//! owns the socket and the clock.
//!
//! # Contract
//!
//! ```text
//! loop {
//!     while let Some(event)    = engine.poll_event()      { handle(event) }
//!     if    let Some(transmit) = engine.poll_transmit(now) { write(&transmit); continue }
//!     match engine.poll_timeout() {
//!         Some(deadline) => select! { data = read() => engine.handle_input(data, now),
//!                                     _    = sleep_until(deadline) => engine.handle_timeout(now) },
//!         None => break,
//!     }
//! }
//! ```
//!
//! [`poll_transmit`](Engine::poll_transmit) assumes the bytes it returns reach the wire before
//! it is called again: command spacing is measured from the moment a transmission is handed
//! over.
//!
//! # What it protects you from
//!
//! | Hardware behaviour | What the engine does |
//! | --- | --- |
//! | The switch pushes a port report when someone presses a front button | [`Event::PortChanged`], never mistaken for the answer to a pending command |
//! | Read replies are sometimes missing or garbage | queries are retransmitted, up to [`Policy::max_retries`] |
//! | Commands sent less than a second apart are ignored | [`poll_transmit`](Engine::poll_transmit) withholds the bytes until the gap has passed |
//! | A retried read produces two replies | the late duplicate is dropped, not reported as a change |
//! | The stream contains garbage | [`Event::Desync`], and the decoder resynchronises |
//!
//! Sources for the timings: `bbeaudoin/bash` (1 s spacing), `pschmitt/tesmart.sh` (retry every
//! 200 ms), PiKVM `kvmd` (5 s socket timeout, one connection per command).

use core::fmt;
use core::mem;
use core::num::NonZeroU32;
use core::time::Duration;

use crate::buf::Ring;
use crate::command::{Request, RequestKind, Response, ResponseKind};
use crate::port::Port;
use crate::protocol::{CommandTable, Unsupported};
use crate::time::Timestamp;
use crate::wire::frame::OPCODE_PORT_REPORT;
use crate::wire::{
    AsciiDialect, AsciiScope, AsciiStyle, Chunk, Frame, StreamDecoder, Transmit, TransmitKind,
    WireBytes, WireError,
};

/// How many events are buffered before the oldest is dropped.
///
/// A conversation produces at most a handful of events per input chunk; a flood means the switch
/// is pushing unsolicited reports faster than the caller drains them, in which case the newest
/// state is the one worth keeping. Drops are counted by
/// [`dropped_events`](Engine::dropped_events).
const EVENT_CAPACITY: usize = 8;

/// Identifies one request/reply exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExchangeId(NonZeroU32);

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Timing and dialect choices.
///
/// The defaults are the values community tools converged on against real hardware. They are
/// deliberately conservative: this protocol has no checksum, no sequence numbers and no flow
/// control, so spacing commands out is the only thing keeping the conversation in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Minimum gap between two different commands.
    pub min_command_interval: Duration,
    /// How long to wait for a reply before retransmitting or giving up.
    pub reply_timeout: Duration,
    /// Gap before retransmitting a query. Not subject to [`Self::min_command_interval`].
    pub retry_backoff: Duration,
    /// How many times a query may be retransmitted.
    pub max_retries: u8,
    /// How long a port report is remembered, to recognise the late reply to a retried read.
    pub duplicate_window: Duration,
    /// Which spelling of the ASCII commands to send.
    pub ascii_dialect: AsciiDialect,
    /// Which transport's ASCII command set to send. Only matrices have two.
    pub ascii_scope: AsciiScope,
}

impl Policy {
    /// The field-tested defaults.
    pub const DEFAULT: Self = Self {
        min_command_interval: Duration::from_secs(1),
        reply_timeout: Duration::from_millis(500),
        retry_backoff: Duration::from_millis(200),
        max_retries: 5,
        duplicate_window: Duration::from_secs(1),
        ascii_dialect: AsciiDialect::Captured,
        ascii_scope: AsciiScope::Lan,
    };

    /// No command spacing and short deadlines: for the emulator and for tests.
    ///
    /// Do not use against hardware. Real switches drop commands sent back to back.
    pub const IMMEDIATE: Self = Self {
        min_command_interval: Duration::ZERO,
        reply_timeout: Duration::from_millis(100),
        retry_backoff: Duration::ZERO,
        max_retries: 2,
        duplicate_window: Duration::from_millis(100),
        ascii_dialect: AsciiDialect::Captured,
        ascii_scope: AsciiScope::Lan,
    };

    /// Sets the minimum gap between commands.
    #[must_use]
    pub const fn with_min_command_interval(mut self, interval: Duration) -> Self {
        self.min_command_interval = interval;
        self
    }

    /// Sets the reply deadline.
    #[must_use]
    pub const fn with_reply_timeout(mut self, timeout: Duration) -> Self {
        self.reply_timeout = timeout;
        self
    }

    /// Sets how many times a query may be retransmitted.
    #[must_use]
    pub const fn with_max_retries(mut self, retries: u8) -> Self {
        self.max_retries = retries;
        self
    }

    /// Sets the ASCII dialect.
    #[must_use]
    pub const fn with_ascii_dialect(mut self, dialect: AsciiDialect) -> Self {
        self.ascii_dialect = dialect;
        self
    }

    /// Sets which transport's ASCII command set to send.
    ///
    /// A driver should take this from the transport rather than from the user: the wrapped
    /// matrix forms are documented "for RS232 connection only".
    #[must_use]
    pub const fn with_ascii_scope(mut self, scope: AsciiScope) -> Self {
        self.ascii_scope = scope;
        self
    }

    /// The dialect and scope together, as the command tables take them.
    pub const fn ascii_style(self) -> AsciiStyle {
        AsciiStyle { dialect: self.ascii_dialect, scope: self.ascii_scope }
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Something that happened, in the order it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// An exchange finished.
    Completed {
        /// The exchange that finished.
        id: ExchangeId,
        /// Its result. [`Response::Accepted`] means "sent, and this command has no reply".
        response: Response,
    },

    /// An exchange gave up.
    Failed {
        /// The exchange that failed.
        id: ExchangeId,
        /// Why.
        error: ExchangeError,
    },

    /// The switch reported a port nobody asked about: someone used the front panel, a hotkey or
    /// the remote.
    PortChanged(Port),

    /// A reply arrived that nothing was waiting for.
    UnexpectedReply {
        /// What kind of reply it was.
        kind: ResponseKind,
    },

    /// A well-formed frame arrived that no command table interprets.
    UnexpectedFrame {
        /// Its opcode.
        opcode: u8,
    },

    /// A port report carried an impossible port index. Known to happen; the read is retried.
    InvalidPortReport {
        /// The byte that was in the report.
        raw: u8,
    },

    /// Bytes were dropped because they could not be part of anything valid.
    Desync {
        /// How many bytes were lost.
        dropped: usize,
    },
}

/// Why an exchange did not produce an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ExchangeError {
    /// No usable reply arrived, after this many transmissions.
    #[error("no reply after {attempts} attempt(s)")]
    Timeout {
        /// How many times the request was sent.
        attempts: u8,
    },

    /// The engine was reset, typically because the connection was lost.
    #[error("exchange abandoned")]
    Aborted,
}

/// Why a request could not be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SubmitError {
    /// The active protocol has no such command.
    #[error(transparent)]
    Unsupported(#[from] Unsupported),

    /// Another exchange is still in flight. This protocol cannot multiplex.
    #[error("exchange {pending} is still in flight")]
    Busy {
        /// The exchange that is holding the engine.
        pending: ExchangeId,
    },

    /// Raw bytes that do not fit one protocol unit.
    #[error(transparent)]
    Wire(#[from] WireError),
}

/// What a pending exchange will accept as its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expectation {
    /// The protocol defines no reply; the exchange is done once the bytes are handed over.
    None,
    /// Exactly this kind of reply.
    Kind(ResponseKind),
    /// Anything at all, used by raw exchanges.
    Any,
}

impl Expectation {
    fn matches(self, kind: ResponseKind) -> bool {
        match self {
            Self::None => false,
            Self::Kind(expected) => expected == kind,
            Self::Any => true,
        }
    }

    const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy)]
struct Exchange {
    id: ExchangeId,
    kind: Option<RequestKind>,
    expectation: Expectation,
    bytes: WireBytes,
    transmit_kind: TransmitKind,
    attempt: u8,
    retriable: bool,
}

#[derive(Debug, Clone, Copy)]
enum State<T> {
    Idle,
    /// Waiting for the command-spacing or retry gap to pass.
    Queued {
        exchange: Exchange,
        ready_at: T,
    },
    /// Sent, waiting for a reply until `deadline`.
    Awaiting {
        exchange: Exchange,
        deadline: T,
    },
}

/// One decoded chunk, turned into something that does not borrow the decoder.
#[derive(Debug, Clone, Copy)]
enum Incoming {
    Decoded(Response),
    Frame(Frame),
    Text(WireBytes),
    BadPort { raw: u8 },
    Dropped { len: usize },
}

/// The protocol engine: bytes in, events out, no IO.
#[derive(Debug)]
pub struct Engine<P: CommandTable, T: Timestamp> {
    table: P,
    policy: Policy,
    decoder: StreamDecoder,
    events: Ring<Event, EVENT_CAPACITY>,
    dropped_events: u32,
    state: State<T>,
    next_id: NonZeroU32,
    last_transmit: Option<T>,
    last_port_report: Option<(Port, T)>,
}

impl<P: CommandTable, T: Timestamp> Engine<P, T> {
    /// An engine for `table`, with the default [`Policy`].
    pub fn new(table: P) -> Self {
        Self::with_policy(table, Policy::DEFAULT)
    }

    /// An engine for `table` with an explicit policy.
    pub fn with_policy(table: P, policy: Policy) -> Self {
        Self {
            table,
            policy,
            decoder: StreamDecoder::new(),
            events: Ring::new(),
            dropped_events: 0,
            state: State::Idle,
            next_id: NonZeroU32::MIN,
            last_transmit: None,
            last_port_report: None,
        }
    }

    /// The command table in use.
    pub fn table(&self) -> P {
        self.table
    }

    /// The policy in use.
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Replaces the policy. Takes effect on the next exchange.
    pub const fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
    }

    /// Queues a request.
    ///
    /// Validation against the command table happens here, before any byte is produced: a
    /// command that does not exist in this protocol can never reach the switch.
    ///
    /// # Errors
    ///
    /// [`SubmitError::Unsupported`] if the protocol has no such command, or
    /// [`SubmitError::Busy`] if an exchange is already in flight.
    pub fn submit(&mut self, request: Request, now: T) -> Result<ExchangeId, SubmitError> {
        if let Some(pending) = self.pending() {
            return Err(SubmitError::Busy { pending });
        }

        let transmit = self.table.encode(request, self.policy.ascii_style())?;
        let kind = request.kind();
        let expectation = match kind.expects() {
            Some(response) => Expectation::Kind(response),
            None => Expectation::None,
        };

        let id = self.take_id();
        Ok(self.queue(
            Exchange {
                id,
                kind: Some(kind),
                expectation,
                bytes: transmit.bytes(),
                transmit_kind: transmit.kind(),
                attempt: 1,
                // Repeating a read is harmless; repeating a write is not.
                retriable: kind.is_query(),
            },
            now,
        ))
    }

    /// Queues raw bytes, bypassing the command tables.
    ///
    /// The escape hatch for probing undocumented models. With `expect_reply` the exchange
    /// completes on whatever comes back first, as [`Response::Frame`] or [`Response::Text`];
    /// nothing is ever retransmitted.
    ///
    /// # Errors
    ///
    /// [`SubmitError::Wire`] if `bytes` is longer than one protocol unit, or
    /// [`SubmitError::Busy`] if an exchange is already in flight.
    pub fn submit_raw(
        &mut self,
        bytes: &[u8],
        expect_reply: bool,
        now: T,
    ) -> Result<ExchangeId, SubmitError> {
        if let Some(pending) = self.pending() {
            return Err(SubmitError::Busy { pending });
        }

        let wire = WireBytes::from_slice(bytes)?;
        let id = self.take_id();
        Ok(self.queue(
            Exchange {
                id,
                kind: None,
                expectation: if expect_reply { Expectation::Any } else { Expectation::None },
                bytes: wire,
                transmit_kind: TransmitKind::Raw,
                attempt: 1,
                retriable: false,
            },
            now,
        ))
    }

    /// The bytes to write, if the switch is ready to be written to.
    ///
    /// `None` means either nothing is queued, or the command-spacing gap has not passed yet;
    /// [`poll_timeout`](Self::poll_timeout) says when to try again.
    pub fn poll_transmit(&mut self, now: T) -> Option<Transmit> {
        match &self.state {
            State::Queued { ready_at, .. } if now >= *ready_at => {}
            _ => return None,
        }

        let State::Queued { exchange, .. } = mem::replace(&mut self.state, State::Idle) else {
            return None;
        };

        let transmit =
            Transmit::new(exchange.bytes, exchange.transmit_kind).with_attempt(exchange.attempt);
        self.last_transmit = Some(now);

        if exchange.expectation.is_none() {
            // Nothing will ever come back for this command, by design.
            self.push(Event::Completed { id: exchange.id, response: Response::Accepted });
        } else {
            let deadline = now + self.policy.reply_timeout;
            self.state = State::Awaiting { exchange, deadline };
        }

        Some(transmit)
    }

    /// When [`handle_timeout`](Self::handle_timeout) or
    /// [`poll_transmit`](Self::poll_transmit) next needs to be called.
    pub fn poll_timeout(&self) -> Option<T> {
        match &self.state {
            State::Idle => None,
            State::Queued { ready_at, .. } => Some(*ready_at),
            State::Awaiting { deadline, .. } => Some(*deadline),
        }
    }

    /// Feeds bytes read from the switch.
    pub fn handle_input(&mut self, data: &[u8], now: T) {
        let mut rest = data;
        loop {
            let accepted = self.decoder.feed(rest);
            rest = &rest[accepted..];
            self.drain(now);

            if rest.is_empty() {
                break;
            }
            if accepted == 0 {
                // The buffer is full of bytes that cannot start anything valid.
                let dropped = self.decoder.buffered();
                self.decoder.reset();
                self.push(Event::Desync { dropped });
            }
        }
    }

    /// Acts on an expired deadline: retransmits a query, or fails the exchange.
    pub fn handle_timeout(&mut self, now: T) {
        match &self.state {
            State::Awaiting { deadline, .. } if now >= *deadline => {}
            _ => return,
        }

        // A reply with no terminator is still a reply. `OK` has been seen arriving bare.
        let table = self.table;
        let recovered =
            self.decoder.flush_ascii().and_then(|text| table.decode(&Chunk::Ascii(text)));
        if let Some(response) = recovered {
            self.deliver(response, now);
            if !matches!(self.state, State::Awaiting { .. }) {
                return;
            }
        }

        let State::Awaiting { mut exchange, .. } = mem::replace(&mut self.state, State::Idle)
        else {
            return;
        };

        if exchange.retriable && exchange.attempt <= self.policy.max_retries {
            exchange.attempt += 1;
            let ready_at = now + self.policy.retry_backoff;
            self.state = State::Queued { exchange, ready_at };
        } else {
            self.push(Event::Failed {
                id: exchange.id,
                error: ExchangeError::Timeout { attempts: exchange.attempt },
            });
        }
    }

    /// The next event, oldest first.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop()
    }

    /// Whether no exchange is in flight.
    pub const fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    /// The exchange in flight, if any.
    pub const fn pending(&self) -> Option<ExchangeId> {
        match &self.state {
            State::Idle => None,
            State::Queued { exchange, .. } | State::Awaiting { exchange, .. } => Some(exchange.id),
        }
    }

    /// The kind of request in flight, if any. `None` for raw exchanges.
    pub const fn pending_kind(&self) -> Option<RequestKind> {
        match &self.state {
            State::Idle => None,
            State::Queued { exchange, .. } | State::Awaiting { exchange, .. } => exchange.kind,
        }
    }

    /// How many events were dropped because the caller did not drain them.
    pub const fn dropped_events(&self) -> u32 {
        self.dropped_events
    }

    /// How many events are waiting to be polled.
    pub const fn queued_events(&self) -> usize {
        self.events.len()
    }

    /// Abandons the in-flight exchange and empties the decode buffer.
    ///
    /// Call this after a reconnect. The time of the last transmission is deliberately kept: the
    /// switch needs its gap between commands whether or not the connection was replaced.
    pub fn reset(&mut self) {
        self.decoder.reset();
        self.last_port_report = None;
        if let State::Queued { exchange, .. } | State::Awaiting { exchange, .. } =
            mem::replace(&mut self.state, State::Idle)
        {
            self.push(Event::Failed { id: exchange.id, error: ExchangeError::Aborted });
        }
    }

    /// Queues an exchange, delaying it until the switch is ready for another command.
    fn queue(&mut self, exchange: Exchange, now: T) -> ExchangeId {
        let id = exchange.id;
        let ready_at = match self.last_transmit {
            Some(last) => {
                let gate = last + self.policy.min_command_interval;
                if gate > now { gate } else { now }
            }
            // Nothing has been sent yet, so there is nothing to wait for.
            None => now,
        };
        self.state = State::Queued { exchange, ready_at };
        id
    }

    fn drain(&mut self, now: T) {
        loop {
            let table = self.table;
            let incoming = match self.decoder.poll_chunk() {
                None => break,
                Some(chunk) => classify(table, &chunk),
            };
            self.apply(incoming, now);
        }
    }

    fn apply(&mut self, incoming: Incoming, now: T) {
        match incoming {
            Incoming::Decoded(response) => self.deliver(response, now),
            Incoming::Frame(frame) => {
                if self.expects_any() {
                    self.complete(Response::Frame(frame), now);
                } else {
                    self.push(Event::UnexpectedFrame { opcode: frame.opcode });
                }
            }
            Incoming::Text(bytes) => {
                if self.expects_any() {
                    self.complete(Response::Text(bytes), now);
                } else {
                    self.push(Event::UnexpectedReply { kind: ResponseKind::Text });
                }
            }
            Incoming::BadPort { raw } => self.push(Event::InvalidPortReport { raw }),
            Incoming::Dropped { len } => self.push(Event::Desync { dropped: len }),
        }
    }

    /// Routes a response either to the exchange that asked for it, or to an event.
    fn deliver(&mut self, response: Response, now: T) {
        if self.expects(response.kind()) {
            self.complete(response, now);
            return;
        }

        match response {
            Response::ActivePort(port) => {
                if self.is_duplicate_report(port, now) {
                    // The late reply to a read that was already retried and answered.
                    return;
                }
                self.last_port_report = Some((port, now));
                self.push(Event::PortChanged(port));
            }
            other => self.push(Event::UnexpectedReply { kind: other.kind() }),
        }
    }

    fn complete(&mut self, response: Response, now: T) {
        let State::Awaiting { exchange, .. } = mem::replace(&mut self.state, State::Idle) else {
            return;
        };
        if let Response::ActivePort(port) = response {
            self.last_port_report = Some((port, now));
        }
        self.push(Event::Completed { id: exchange.id, response });
    }

    fn expects(&self, kind: ResponseKind) -> bool {
        match &self.state {
            State::Awaiting { exchange, .. } => exchange.expectation.matches(kind),
            _ => false,
        }
    }

    fn expects_any(&self) -> bool {
        matches!(
            &self.state,
            State::Awaiting { exchange, .. } if matches!(exchange.expectation, Expectation::Any)
        )
    }

    fn is_duplicate_report(&self, port: Port, now: T) -> bool {
        match self.last_port_report {
            Some((previous, at)) => previous == port && now <= at + self.policy.duplicate_window,
            None => false,
        }
    }

    fn push(&mut self, event: Event) {
        if self.events.push(event) {
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
    }

    fn take_id(&mut self) -> ExchangeId {
        let id = ExchangeId(self.next_id);
        self.next_id = self.next_id.checked_add(1).unwrap_or(NonZeroU32::MIN);
        id
    }
}

fn classify<P: CommandTable>(table: P, chunk: &Chunk<'_>) -> Incoming {
    if let Some(response) = table.decode(chunk) {
        return Incoming::Decoded(response);
    }
    match chunk {
        Chunk::Frame(frame) => Incoming::Frame(*frame),
        // Only reachable when the active protocol ignores port reports, i.e. a matrix answering
        // with a KVM frame: the wrong protocol was selected.
        Chunk::PortReport(port) => {
            Incoming::Frame(Frame::one(OPCODE_PORT_REPORT, port.to_read_byte()))
        }
        Chunk::BadPortReport { raw } => Incoming::BadPort { raw: *raw },
        Chunk::Ascii(text) => Incoming::Text(WireBytes::from_slice_truncating(text.as_bytes()).0),
        Chunk::Discarded(bytes) => Incoming::Dropped { len: bytes.len() },
    }
}
