//! Working out which transport a target string means, and driving whichever one it is.
//!
//! The rules are fixed and in this order, so that a target always resolves to the same
//! transport whatever else is going on:
//!
//! | Input | Transport |
//! | --- | --- |
//! | `tcp://…` | TCP, and a malformed remainder is an error rather than a fallback |
//! | `serial://…` | serial |
//! | `sim`, `sim://MODEL` | the in-process emulator |
//! | starts with `/`, `./`, `\\`, or looks like `COM3` | serial |
//! | anything else | TCP |
//!
//! A target naming a transport this build was not compiled with is an error, not a fallback:
//! silently connecting somewhere else would be worse than refusing.

use core::fmt;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;

use tesmart_client::{ConnectionPolicy, Connector, TransportError, TransportKind};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// How long a connection attempt may take, unless [`TargetOptions`] says otherwise.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The line rate every TESmart document specifies.
pub const DEFAULT_BAUD: u32 = 9600;

/// The model `sim` emulates when the target does not name one.
#[cfg(feature = "sim")]
pub const DEFAULT_SIM_MODEL: &str = "HKS1601-EB23";

/// Transport settings for targets that do not spell them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetOptions {
    /// How long a connection attempt may take.
    pub connect_timeout: Duration,
    /// Baud rate for a serial target that does not name one, as `/dev/ttyUSB0@19200` would.
    pub baud: u32,
}

impl Default for TargetOptions {
    fn default() -> Self {
        Self { connect_timeout: DEFAULT_CONNECT_TIMEOUT, baud: DEFAULT_BAUD }
    }
}

impl TargetOptions {
    /// Sets how long a connection attempt may take.
    #[must_use]
    pub const fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Sets the baud rate used when the target does not name one.
    #[must_use]
    pub const fn with_baud(mut self, baud: u32) -> Self {
        self.baud = baud;
        self
    }
}

/// A target string that names no transport this build can reach.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TargetError {
    /// The target was empty or only whitespace.
    #[error("empty target")]
    Empty,

    /// The target named a scheme that means nothing here.
    #[error("unknown target scheme {scheme:?}; expected tcp://, serial:// or sim")]
    UnknownScheme {
        /// The scheme that was found.
        scheme: String,
    },

    /// The target named TCP, and the rest of it does not parse.
    #[cfg(feature = "tcp")]
    #[error("{target:?}: {source}")]
    Tcp {
        /// The target as given.
        target: String,
        /// Why it does not parse.
        #[source]
        source: tesmart_tcp::ParseTargetError,
    },

    /// The target named a serial port, and the rest of it does not parse.
    #[cfg(feature = "serial")]
    #[error("{target:?}: {source}")]
    Serial {
        /// The target as given.
        target: String,
        /// Why it does not parse.
        #[source]
        source: tesmart_serial::ParseTargetError,
    },

    /// The emulator was asked to be a model that is not in the registry.
    #[cfg(feature = "sim")]
    #[error("unknown model {name:?} for the emulator")]
    UnknownModel {
        /// The name that was asked for.
        name: String,
    },

    /// The target names a transport this build was compiled without.
    #[error("{target:?} needs the `{feature}` feature, which this build of tesmart does not have")]
    MissingFeature {
        /// The target as given.
        target: String,
        /// The Cargo feature that would enable it.
        feature: &'static str,
    },
}

/// Resolves a target string with the default [`TargetOptions`].
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use tesmart::{Connector, TransportKind};
///
/// let switch = tesmart::resolve("192.168.1.10")?;
/// assert_eq!(switch.kind(), TransportKind::Tcp);
/// assert_eq!(switch.to_string(), "tcp://192.168.1.10:5000");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`TargetError`] if the string names no transport this build supports, or names one and then
/// fails to parse.
pub fn resolve(target: &str) -> Result<AnyConnector, TargetError> {
    resolve_with(target, TargetOptions::default())
}

/// Resolves a target string.
///
/// # Errors
///
/// As [`resolve`].
pub fn resolve_with(target: &str, options: TargetOptions) -> Result<AnyConnector, TargetError> {
    let text = target.trim();
    if text.is_empty() {
        return Err(TargetError::Empty);
    }

    if let Some(rest) = scheme(text, "sim") {
        return simulated(rest);
    }
    if scheme(text, "serial").is_some() || looks_like_a_device_path(text) {
        return serial(text, options);
    }
    if let Some((found, _)) = text.split_once("://")
        && !found.eq_ignore_ascii_case("tcp")
    {
        return Err(TargetError::UnknownScheme { scheme: found.to_owned() });
    }
    tcp(text, options)
}

#[cfg(feature = "tcp")]
fn tcp(text: &str, options: TargetOptions) -> Result<AnyConnector, TargetError> {
    let connector: tesmart_tcp::TcpConnector =
        text.parse().map_err(|source| TargetError::Tcp { target: text.to_owned(), source })?;
    Ok(AnyConnector::Tcp(connector.with_connect_timeout(options.connect_timeout)))
}

#[cfg(not(feature = "tcp"))]
fn tcp(text: &str, _options: TargetOptions) -> Result<AnyConnector, TargetError> {
    Err(TargetError::MissingFeature { target: text.to_owned(), feature: "tcp" })
}

#[cfg(feature = "serial")]
fn serial(text: &str, options: TargetOptions) -> Result<AnyConnector, TargetError> {
    let connector: tesmart_serial::SerialConnector =
        text.parse().map_err(|source| TargetError::Serial { target: text.to_owned(), source })?;
    // An explicit `@baud` in the target wins over the option.
    let connector = if text.contains('@') { connector } else { connector.with_baud(options.baud) };
    Ok(AnyConnector::Serial(connector.with_open_timeout(options.connect_timeout)))
}

#[cfg(not(feature = "serial"))]
fn serial(text: &str, _options: TargetOptions) -> Result<AnyConnector, TargetError> {
    Err(TargetError::MissingFeature { target: text.to_owned(), feature: "serial" })
}

#[cfg(feature = "sim")]
fn simulated(rest: &str) -> Result<AnyConnector, TargetError> {
    let name = if rest.is_empty() { DEFAULT_SIM_MODEL } else { rest };
    let model = tesmart_proto::model::lookup(name)
        .ok_or_else(|| TargetError::UnknownModel { name: name.to_owned() })?;
    Ok(AnyConnector::Sim(tesmart_sim::Simulator::from_model(model).connector()))
}

#[cfg(not(feature = "sim"))]
fn simulated(rest: &str) -> Result<AnyConnector, TargetError> {
    let target = if rest.is_empty() { "sim".to_owned() } else { std::format!("sim://{rest}") };
    Err(TargetError::MissingFeature { target, feature: "sim" })
}

/// Returns the remainder after `scheme:` or `scheme://`, or `None` if it is a different scheme.
///
/// A bare `sim` counts as the `sim` scheme with an empty remainder.
fn scheme<'a>(text: &'a str, wanted: &str) -> Option<&'a str> {
    if text.eq_ignore_ascii_case(wanted) {
        return Some("");
    }
    let (found, rest) = text.split_once("://")?;
    found.eq_ignore_ascii_case(wanted).then_some(rest)
}

/// Whether a target looks like a device file rather than a host.
fn looks_like_a_device_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with("./")
        || text.starts_with(r"\\")
        || is_windows_com_port(text)
}

/// `COM3`, `COM12@9600`, but not `computer.local`.
fn is_windows_com_port(text: &str) -> bool {
    let name = text.split('@').next().unwrap_or(text);
    let Some(number) = name.strip_prefix("COM").or_else(|| name.strip_prefix("com")) else {
        return false;
    };
    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}

/// Any transport this build supports, chosen at run time.
///
/// Exists so that a target string can become one concrete type: `Kvm<AnyConnector>` is a single
/// handle whatever the user typed, instead of three generic instantiations the caller has to
/// match over. The cost is one enum branch per read and write, which is nothing next to the
/// syscall underneath.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AnyConnector {
    /// A switch's LAN port.
    #[cfg(feature = "tcp")]
    Tcp(tesmart_tcp::TcpConnector),

    /// A switch's RS232 port.
    #[cfg(feature = "serial")]
    Serial(tesmart_serial::SerialConnector),

    /// An emulated switch inside this process.
    #[cfg(feature = "sim")]
    Sim(tesmart_sim::InProcess),
}

impl fmt::Display for AnyConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "tcp")]
            Self::Tcp(connector) => connector.fmt(f),
            #[cfg(feature = "serial")]
            Self::Serial(connector) => connector.fmt(f),
            #[cfg(feature = "sim")]
            Self::Sim(connector) => connector.fmt(f),
        }
    }
}

impl Connector for AnyConnector {
    type Io = AnyIo;

    async fn connect(&self) -> Result<Self::Io, TransportError> {
        match self {
            #[cfg(feature = "tcp")]
            Self::Tcp(connector) => connector.connect().await.map(AnyIo::Tcp),
            #[cfg(feature = "serial")]
            Self::Serial(connector) => connector.connect().await.map(AnyIo::Serial),
            #[cfg(feature = "sim")]
            Self::Sim(connector) => connector.connect().await.map(AnyIo::Sim),
        }
    }

    /// Delegated, and it matters: the session derives the ASCII command scope from this, so a
    /// matrix reached over RS232 only gets its `MT80xx` commands if this reports `Serial`.
    fn kind(&self) -> TransportKind {
        match self {
            #[cfg(feature = "tcp")]
            Self::Tcp(connector) => connector.kind(),
            #[cfg(feature = "serial")]
            Self::Serial(connector) => connector.kind(),
            #[cfg(feature = "sim")]
            Self::Sim(connector) => connector.kind(),
        }
    }

    /// Delegated, and it matters: a serial line wants one connection held open, a TCP socket
    /// wants one per command.
    fn preferred_policy(&self) -> ConnectionPolicy {
        match self {
            #[cfg(feature = "tcp")]
            Self::Tcp(connector) => connector.preferred_policy(),
            #[cfg(feature = "serial")]
            Self::Serial(connector) => connector.preferred_policy(),
            #[cfg(feature = "sim")]
            Self::Sim(connector) => connector.preferred_policy(),
        }
    }
}

/// The byte stream an [`AnyConnector`] opens.
///
/// Each variant is spelled as the transport's own associated type rather than as a concrete
/// stream, so this follows along if a transport ever changes what it opens, and so this crate
/// needs no direct dependency on `tokio::net` or `tokio-serial`.
///
/// Every variant is [`Unpin`], which is what lets the delegation below be plain `Pin::new`
/// with no `unsafe`.
#[derive(Debug)]
#[non_exhaustive]
pub enum AnyIo {
    /// A TCP connection.
    #[cfg(feature = "tcp")]
    Tcp(<tesmart_tcp::TcpConnector as Connector>::Io),

    /// An open serial port.
    #[cfg(feature = "serial")]
    Serial(<tesmart_serial::SerialConnector as Connector>::Io),

    /// An in-process pipe to an emulated switch.
    #[cfg(feature = "sim")]
    Sim(<tesmart_sim::InProcess as Connector>::Io),
}

impl AsyncRead for AnyIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(feature = "tcp")]
            Self::Tcp(io) => Pin::new(io).poll_read(cx, buf),
            #[cfg(feature = "serial")]
            Self::Serial(io) => Pin::new(io).poll_read(cx, buf),
            #[cfg(feature = "sim")]
            Self::Sim(io) => Pin::new(io).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for AnyIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            #[cfg(feature = "tcp")]
            Self::Tcp(io) => Pin::new(io).poll_write(cx, buf),
            #[cfg(feature = "serial")]
            Self::Serial(io) => Pin::new(io).poll_write(cx, buf),
            #[cfg(feature = "sim")]
            Self::Sim(io) => Pin::new(io).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(feature = "tcp")]
            Self::Tcp(io) => Pin::new(io).poll_flush(cx),
            #[cfg(feature = "serial")]
            Self::Serial(io) => Pin::new(io).poll_flush(cx),
            #[cfg(feature = "sim")]
            Self::Sim(io) => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(feature = "tcp")]
            Self::Tcp(io) => Pin::new(io).poll_shutdown(cx),
            #[cfg(feature = "serial")]
            Self::Serial(io) => Pin::new(io).poll_shutdown(cx),
            #[cfg(feature = "sim")]
            Self::Sim(io) => Pin::new(io).poll_shutdown(cx),
        }
    }
}
