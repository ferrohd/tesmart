//! The boundary between this crate and anything that moves bytes.
//!
//! A transport is one trait with one method. Everything else in the workspace, the driver loop
//! included, is written against [`Connector`], which is why adding RS232 did not change a line
//! of protocol or operation code.

use core::fmt;
use core::future::Future;
use core::time::Duration;
use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

/// Opens connections to a switch.
///
/// Implementors are cheap, reusable descriptions of *where* the switch is, not open handles:
/// [`connect`](Connector::connect) takes `&self` and may be called many times, because the
/// default connection policy opens a connection per command the way PiKVM does.
///
/// The [`Display`](fmt::Display) bound is what puts a usable target in every log line and error
/// message (`tcp://192.168.1.10:5000`, `serial:///dev/ttyUSB0@9600`).
pub trait Connector: fmt::Display {
    /// The byte stream this connector produces.
    type Io: AsyncRead + AsyncWrite + Unpin + Send;

    /// Opens a connection, applying the transport's own connect timeout.
    ///
    /// # Errors
    ///
    /// [`TransportError::Connect`] or [`TransportError::ConnectTimeout`] if the switch cannot
    /// be reached.
    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send;

    /// What kind of link this is.
    ///
    /// Protocol behaviour differs by transport in one documented way: TESmart states that KVM
    /// firmware does not support IP configuration over RS232, while two community tools report
    /// doing exactly that. Operations use this to attach a warning instead of guessing.
    fn kind(&self) -> TransportKind;

    /// How this transport prefers its connections to be managed.
    ///
    /// A TCP connection is cheap to open and closing it stops the switch pushing unsolicited
    /// reports, so the default is [`ConnectionPolicy::PerCommand`], which is also what PiKVM
    /// does. Opening a serial port costs a port-settings round trip and toggles DTR and RTS on
    /// most USB adapters, so `tesmart-serial` overrides this to
    /// [`ConnectionPolicy::Persistent`].
    ///
    /// A policy set explicitly by the caller always wins over this.
    fn preferred_policy(&self) -> ConnectionPolicy {
        ConnectionPolicy::PerCommand
    }
}

/// When connections are opened and closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionPolicy {
    /// Open before each command, close after it.
    ///
    /// The default, and what PiKVM does. A closed connection also stops the switch pushing
    /// unsolicited port reports, which keeps the line quiet between commands.
    #[default]
    PerCommand,

    /// Keep one connection open.
    ///
    /// Needed to observe port changes made from the front panel, and cheaper for a burst of
    /// commands or for a transport whose connections are expensive to establish.
    Persistent,
}

/// Every reference to a connector is a connector.
///
/// Lets a caller keep ownership of its connector and still hand one to a session.
impl<C: Connector + ?Sized + Sync> Connector for &C {
    type Io = C::Io;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        (**self).connect()
    }

    fn kind(&self) -> TransportKind {
        (**self).kind()
    }

    fn preferred_policy(&self) -> ConnectionPolicy {
        (**self).preferred_policy()
    }
}

/// What sort of link a [`Connector`] produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportKind {
    /// A TCP connection to the switch's LAN port, which is a serial-to-Ethernet bridge.
    Tcp,
    /// A serial line, 9600 8N1.
    Serial,
    /// An emulated switch.
    Simulated,
}

impl TransportKind {
    /// Whether the ASCII network configuration commands are known to work on this transport.
    ///
    /// Over RS232 they are disputed: TESmart's own IP-change guide says KVM firmware "does not
    /// support IP configuration via RS232", while `bbeaudoin` and `luborpetr` report sending
    /// them over serial successfully.
    pub const fn net_config_is_documented(self) -> bool {
        matches!(self, Self::Tcp | Self::Simulated)
    }

    /// A stable, hyphenated name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Serial => "serial",
            Self::Simulated => "simulated",
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Something went wrong with the link, as opposed to with the protocol.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The connection attempt failed.
    #[error("cannot connect to {target}")]
    Connect {
        /// The connector that failed, as it describes itself.
        target: String,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// The connection attempt ran out of time.
    #[error("connecting to {target} timed out after {after:?}")]
    ConnectTimeout {
        /// The connector that timed out.
        target: String,
        /// How long was allowed.
        after: Duration,
    },

    /// The switch closed the connection.
    ///
    /// Expected at the end of a conversation, a failure in the middle of one.
    #[error("the switch closed the connection")]
    Closed,

    /// A read or write failed.
    #[error("{operation} failed")]
    Io {
        /// What was being attempted: `read` or `write`.
        operation: &'static str,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// The target could not be understood, before any connection was attempted.
    #[error("{0}")]
    Target(String),
}

impl TransportError {
    /// Wraps a connection failure, naming the target.
    pub fn connect(target: &dyn fmt::Display, source: io::Error) -> Self {
        Self::Connect { target: target.to_string(), source }
    }

    /// Reports a connection attempt that ran out of time.
    pub fn connect_timeout(target: &dyn fmt::Display, after: Duration) -> Self {
        Self::ConnectTimeout { target: target.to_string(), after }
    }

    /// Wraps a read failure.
    pub fn read(source: io::Error) -> Self {
        Self::io("read", source)
    }

    /// Wraps a write failure.
    pub fn write(source: io::Error) -> Self {
        Self::io("write", source)
    }

    /// Wraps an IO failure, recognising the ones that only mean "the link is gone".
    ///
    /// A switch that hangs up mid-command surfaces as a broken pipe on the next write or a
    /// reset on the next read; "the switch closed the connection" is more useful to a user than
    /// either of those.
    fn io(operation: &'static str, source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io { operation, source },
        }
    }
}
