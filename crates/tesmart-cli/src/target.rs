//! Working out which transport a target string means, and driving whichever one it is.
//!
//! This is the tool's job rather than the library's: `tesmart::tcp::TcpConnector` and
//! `tesmart::serial::SerialConnector` each parse their own address, and only something with a
//! user to answer to has to guess which of them `/dev/ttyUSB0` was meant for.
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
//! Every transport is always compiled in, so a target can only fail by being malformed, never
//! by naming something this build cannot reach.

use core::fmt;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;

use tesmart::client::{ConnectionPolicy, Connector, TransportError, TransportKind};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// How long a connection attempt may take, unless [`TargetOptions`] says otherwise.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The line rate every TESmart document specifies.
pub const DEFAULT_BAUD: u32 = 9600;

/// The model `sim` emulates when the target does not name one.
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

/// A target string that names no transport this build can reach.
#[derive(Debug, thiserror::Error)]
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
    #[error("{target:?}: {source}")]
    Tcp {
        /// The target as given.
        target: String,
        /// Why it does not parse.
        #[source]
        source: tesmart::tcp::ParseTargetError,
    },

    /// The target named a serial port, and the rest of it does not parse.
    #[error("{target:?}: {source}")]
    Serial {
        /// The target as given.
        target: String,
        /// Why it does not parse.
        #[source]
        source: tesmart::serial::ParseTargetError,
    },

    /// The emulator was asked to be a model that is not in the registry.
    #[error("unknown model {name:?} for the emulator")]
    UnknownModel {
        /// The name that was asked for.
        name: String,
    },
}

/// Resolves a target string into whichever transport it names.
///
/// # Errors
///
/// [`TargetError`] if the string names no transport this build supports, or names one and then
/// fails to parse.
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

fn tcp(text: &str, options: TargetOptions) -> Result<AnyConnector, TargetError> {
    let connector: tesmart::tcp::TcpConnector =
        text.parse().map_err(|source| TargetError::Tcp { target: text.to_owned(), source })?;
    Ok(AnyConnector::Tcp(connector.with_connect_timeout(options.connect_timeout)))
}

fn serial(text: &str, options: TargetOptions) -> Result<AnyConnector, TargetError> {
    let connector: tesmart::serial::SerialConnector =
        text.parse().map_err(|source| TargetError::Serial { target: text.to_owned(), source })?;
    // An explicit `@baud` in the target wins over the option.
    let connector = if text.contains('@') { connector } else { connector.with_baud(options.baud) };
    Ok(AnyConnector::Serial(connector.with_open_timeout(options.connect_timeout)))
}

fn simulated(rest: &str) -> Result<AnyConnector, TargetError> {
    let name = if rest.is_empty() { DEFAULT_SIM_MODEL } else { rest };
    let model = tesmart::proto::model::lookup(name)
        .ok_or_else(|| TargetError::UnknownModel { name: name.to_owned() })?;
    Ok(AnyConnector::Sim(tesmart::sim::Simulator::from_model(model).connector()))
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

/// Any transport the tool speaks, chosen at run time.
///
/// Exists so that a target string can become one concrete type: `Kvm<AnyConnector>` is a single
/// handle whatever the user typed, instead of three generic instantiations of the whole command
/// dispatch. The cost is one enum branch per read and write, which is nothing next to the
/// syscall underneath.
#[derive(Debug, Clone)]
pub enum AnyConnector {
    /// A switch's LAN port.
    Tcp(tesmart::tcp::TcpConnector),

    /// A switch's RS232 port.
    Serial(tesmart::serial::SerialConnector),

    /// An emulated switch inside this process.
    Sim(tesmart::sim::InProcess),
}

impl fmt::Display for AnyConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(connector) => connector.fmt(f),
            Self::Serial(connector) => connector.fmt(f),
            Self::Sim(connector) => connector.fmt(f),
        }
    }
}

impl Connector for AnyConnector {
    type Io = AnyIo;

    async fn connect(&self) -> Result<Self::Io, TransportError> {
        match self {
            Self::Tcp(connector) => connector.connect().await.map(AnyIo::Tcp),
            Self::Serial(connector) => connector.connect().await.map(AnyIo::Serial),
            Self::Sim(connector) => connector.connect().await.map(AnyIo::Sim),
        }
    }

    /// Delegated, and it matters: the session derives the ASCII command scope from this, so a
    /// matrix reached over RS232 only gets its `MT80xx` commands if this reports `Serial`.
    fn kind(&self) -> TransportKind {
        match self {
            Self::Tcp(connector) => connector.kind(),
            Self::Serial(connector) => connector.kind(),
            Self::Sim(connector) => connector.kind(),
        }
    }

    /// Delegated, and it matters: a serial line wants one connection held open, a TCP socket
    /// wants one per command.
    fn preferred_policy(&self) -> ConnectionPolicy {
        match self {
            Self::Tcp(connector) => connector.preferred_policy(),
            Self::Serial(connector) => connector.preferred_policy(),
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
pub enum AnyIo {
    /// A TCP connection.
    Tcp(<tesmart::tcp::TcpConnector as Connector>::Io),

    /// An open serial port.
    Serial(<tesmart::serial::SerialConnector as Connector>::Io),

    /// An in-process pipe to an emulated switch.
    Sim(<tesmart::sim::InProcess as Connector>::Io),
}

impl AsyncRead for AnyIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(io) => Pin::new(io).poll_read(cx, buf),
            Self::Serial(io) => Pin::new(io).poll_read(cx, buf),
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
            Self::Tcp(io) => Pin::new(io).poll_write(cx, buf),
            Self::Serial(io) => Pin::new(io).poll_write(cx, buf),
            Self::Sim(io) => Pin::new(io).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(io) => Pin::new(io).poll_flush(cx),
            Self::Serial(io) => Pin::new(io).poll_flush(cx),
            Self::Sim(io) => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(io) => Pin::new(io).poll_shutdown(cx),
            Self::Serial(io) => Pin::new(io).poll_shutdown(cx),
            Self::Sim(io) => Pin::new(io).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(target: &str) -> Result<AnyConnector, TargetError> {
        resolve_with(target, TargetOptions::default())
    }

    fn resolved(target: &str) -> AnyConnector {
        resolve(target).unwrap_or_else(|error| panic!("{target:?}: {error}"))
    }

    #[test]
    fn addresses_and_names_are_tcp() {
        for target in [
            "tcp://192.168.1.10:5000",
            "192.168.1.10",
            "192.168.1.10:5000",
            "kvm.local",
            "[fd00::1]:5000",
        ] {
            assert!(matches!(resolved(target), AnyConnector::Tcp(_)), "{target}");
        }
        assert_eq!(resolved("192.168.1.10").to_string(), "tcp://192.168.1.10:5000");
    }

    #[test]
    fn device_paths_are_serial() {
        for target in ["/dev/ttyUSB0", "serial:///dev/ttyUSB0", "./tty", "COM3", "com12"] {
            assert!(matches!(resolved(target), AnyConnector::Serial(_)), "{target}");
        }
        assert_eq!(resolved("/dev/ttyUSB0").to_string(), "serial:///dev/ttyUSB0@9600");
    }

    #[test]
    fn the_baud_option_applies_unless_the_target_names_one() {
        let options = TargetOptions { baud: 19200, ..TargetOptions::default() };
        let AnyConnector::Serial(connector) = resolve_with("/dev/ttyUSB0", options).unwrap() else {
            panic!("expected a serial target")
        };
        assert_eq!(connector.baud(), 19200);

        let AnyConnector::Serial(connector) = resolve_with("/dev/ttyUSB0@115200", options).unwrap()
        else {
            panic!("expected a serial target")
        };
        assert_eq!(connector.baud(), 115_200, "the target is more specific than the option");
    }

    #[test]
    fn sim_resolves_with_and_without_a_model() {
        assert_eq!(resolved("sim").to_string(), "sim://HKS1601-EB23");
        assert_eq!(resolved("sim://DKS0401A30").to_string(), "sim://DKS0401A30");
        assert!(resolve("sim://nonsense").is_err());
    }

    #[test]
    fn an_unknown_scheme_says_what_is_expected() {
        let error = resolve("udp://192.168.1.10").unwrap_err();
        assert!(error.to_string().contains("expected tcp://, serial:// or sim"), "{error}");
    }

    #[test]
    fn an_empty_target_is_refused() {
        assert!(matches!(resolve("   "), Err(TargetError::Empty)));
    }

    #[test]
    fn a_malformed_tcp_target_does_not_fall_back_to_serial() {
        let error = resolve("tcp://192.168.1.10:0").unwrap_err();
        assert!(error.to_string().contains("not a TCP port"), "{error}");
    }

    #[test]
    fn com_ports_are_told_apart_from_host_names() {
        assert!(is_windows_com_port("COM3"));
        assert!(is_windows_com_port("com12@9600"));
        assert!(!is_windows_com_port("computer.local"));
        assert!(!is_windows_com_port("COM"));
    }

    /// The whole point of the enum: these two must survive the indirection. `kind()` decides
    /// which spelling of the matrix network commands goes out, and `preferred_policy()` decides
    /// whether the connection is held open.
    #[test]
    fn the_transport_is_not_lost_behind_the_enum() {
        let tcp = resolved("192.168.1.10");
        assert_eq!(tcp.kind(), TransportKind::Tcp);
        assert_eq!(tcp.preferred_policy(), ConnectionPolicy::PerCommand);

        let serial = resolved("/dev/ttyUSB0");
        assert_eq!(serial.kind(), TransportKind::Serial);
        assert_eq!(
            serial.preferred_policy(),
            ConnectionPolicy::Persistent,
            "a serial line must stay open, or every command pays a port-settings round trip"
        );

        assert_eq!(resolved("sim").kind(), TransportKind::Simulated);
    }

    /// And the bytes have to actually flow through `AnyIo`.
    #[tokio::test]
    async fn bytes_reach_the_switch_through_the_enum() {
        use tesmart::client::Kvm;
        use tesmart::proto::{Port, engine::Policy, model};

        let connector = resolved("sim://HKS1601-EB23");
        let info = model::lookup("HKS1601-EB23").expect("a known model");
        let mut kvm = Kvm::from_model(connector, info).expect("a KVM");
        kvm.session_mut().set_policy(Policy::IMMEDIATE);

        let outcome = kvm.switch(Port::new(7).unwrap()).await.unwrap();
        assert!(outcome.is_confirmed(), "{outcome}");
        assert_eq!(kvm.active_port().await.unwrap().get(), 7);
    }
}
