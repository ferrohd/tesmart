//! The sans-IO request/response engine.
//!
//! One [`Engine`] drives one switch. It owns everything about the conversation except the bytes
//! moving: encoding, demultiplexing, command spacing, reply deadlines and retries. The caller
//! owns the socket and the clock.

use core::fmt;
use core::mem;
use core::num::NonZeroU32;
use core::time::Duration;

use crate::buf::Ring;
use crate::command::{Request, RequestKind, Response, ResponseKind};
use crate::port::Port;
use crate::profile::{Profile, Unsupported};
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

    /// The dialect and scope together, as the profiles take them.
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

    /// A well-formed frame arrived that no profile interprets.
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
    /// The active profile has no such command.
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
