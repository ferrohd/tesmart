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
