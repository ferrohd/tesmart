//! RS232 transport for TESmart KVM switches.
//!
//! One implementation of [`Connector`], plus port enumeration so a tool can help the user find
//! the right device file. No protocol and no operations: those live in `tesmart-proto` and
//! `tesmart-client`.
//!
//! ```no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use tesmart_client::Kvm;
//! use tesmart_proto::{Port, model};
//! use tesmart_serial::SerialConnector;
//!
//! // "/dev/ttyUSB0", "serial:///dev/ttyUSB0" and "/dev/ttyUSB0@9600" all parse.
//! let connector: SerialConnector = "/dev/ttyUSB0".parse()?;
//! let mut kvm = Kvm::from_model(connector, model::lookup("HKS1601-EB23").unwrap())?;
//!
//! // Nothing on a dual-monitor or 4x1 DVI switch can be verified, but an EB23 can.
//! println!("{}", kvm.switch(Port::new(2)?).await?);
//! # Ok(()) }
//! ```
//!
//! # What to know about the serial port
//!
//! - **9600 baud, 8 data bits, no parity, one stop bit, no flow control.** Every TESmart
//!   document specifies this and none mentions flow control, so only the baud rate is
//!   configurable here; the rest are protocol constants.
//! - The switch exposes a **3-pin Phoenix connector**, not a DB9. An adapter is needed, and its
//!   wiring is the first thing to check when nothing answers.
//! - Ports are opened **exclusively** (`TIOCEXCL` plus `flock`). A second `tesmart` invocation
//!   fails cleanly instead of interleaving its bytes into someone else's six-byte frame, which
//!   is the one failure mode that would corrupt a command rather than lose it.
//! - Connections are [`Persistent`](tesmart_client::ConnectionPolicy::Persistent) by default,
//!   unlike TCP: opening a tty costs a port-settings round trip and toggles DTR and RTS on most
//!   USB adapters.
//! - **Network configuration over RS232 is disputed.** TESmart's own guide says KVM firmware
//!   does not support it; `bbeaudoin` and `luborpetr` report doing it successfully. The
//!   operations layer attaches a warning, because [`TransportKind::Serial`] says which
//!   transport is in use.
//!
//! # Platforms
//!
//! Tested on Linux, including an end-to-end test over a pseudo terminal. macOS should work (a
//! PTY there needs `with_baud(0)`, which `tokio-serial` documents). Windows is **untested**:
//! `tokio-serial` supports it, nothing here exercises it.
#![allow(clippy::must_use_candidate)]

use core::fmt;
use core::future::Future;
use core::str::FromStr;
use core::time::Duration;
use std::io;

use tesmart_client::{ConnectionPolicy, Connector, TransportError, TransportKind};
use tokio_serial::{DataBits, FlowControl, Parity, SerialStream, StopBits};

/// The baud rate every TESmart document specifies.
pub const DEFAULT_BAUD: u32 = 9600;

/// How long to wait for a port to open before giving up.
pub const DEFAULT_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// The URL scheme this crate answers to.
pub const SCHEME: &str = "serial";

/// Opens a serial port to a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConnector {
    path: String,
    baud: u32,
    exclusive: bool,
    open_timeout: Duration,
}

impl SerialConnector {
    /// A connector for a device file, at 9600 baud, opened exclusively.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            baud: DEFAULT_BAUD,
            exclusive: true,
            open_timeout: DEFAULT_OPEN_TIMEOUT,
        }
    }

    /// Sets the baud rate.
    ///
    /// Only useful for a switch that has been reconfigured, or for a pseudo terminal on macOS,
    /// where `0` tells the driver to skip setting a speed at all.
    #[must_use]
    pub const fn with_baud(mut self, baud: u32) -> Self {
        self.baud = baud;
        self
    }

    /// Sets whether to claim the port exclusively. On by default.
    ///
    /// Turning this off lets two processes write to the same switch, which can split a six-byte
    /// frame down the middle.
    #[must_use]
    pub const fn with_exclusive(mut self, exclusive: bool) -> Self {
        self.exclusive = exclusive;
        self
    }

    /// Sets how long opening the port may take.
    #[must_use]
    pub const fn with_open_timeout(mut self, timeout: Duration) -> Self {
        self.open_timeout = timeout;
        self
    }

    /// The device file.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The baud rate.
    pub const fn baud(&self) -> u32 {
        self.baud
    }

    /// Whether the port is claimed exclusively.
    pub const fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    /// How long opening the port may take.
    pub const fn open_timeout(&self) -> Duration {
        self.open_timeout
    }

    /// Opens the port, inside the open timeout.
    ///
    /// `SerialStream::open` is a synchronous call, so it runs on the blocking pool: a wedged
    /// driver then costs one blocking thread rather than stalling the executor. Registering the
    /// file descriptor with the reactor works from there because tokio's blocking threads hold
    /// the runtime context.
    async fn open_within_timeout(&self) -> Result<SerialStream, TransportError> {
        let settings = self.settings();
        let exclusive = self.exclusive;
        let path = self.path.clone();

        let opening = tokio::task::spawn_blocking(move || {
            let mut stream = SerialStream::open(&settings)?;
            stream.set_exclusive(exclusive)?;
            Ok::<SerialStream, tokio_serial::Error>(stream)
        });

        match tokio::time::timeout(self.open_timeout, opening).await {
            Ok(Ok(Ok(stream))) => {
                tracing::debug!(
                    target: "tesmart::serial",
                    %path, baud = self.baud, exclusive,
                    "port opened"
                );
                Ok(stream)
            }
            Ok(Ok(Err(error))) => Err(TransportError::connect(self, into_io_error(error))),
            // The blocking task panicked or was cancelled.
            Ok(Err(join)) => Err(TransportError::connect(self, io::Error::other(join))),
            Err(_elapsed) => Err(TransportError::connect_timeout(self, self.open_timeout)),
        }
    }

    /// The line settings: the baud rate, and four protocol constants.
    fn settings(&self) -> tokio_serial::SerialPortBuilder {
        tokio_serial::new(&self.path, self.baud)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
    }
}

impl fmt::Display for SerialConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}://{}@{}", self.path, self.baud)
    }
}

impl Connector for SerialConnector {
    type Io = SerialStream;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        self.open_within_timeout()
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Serial
    }

    /// Persistent: opening a tty is not free, and on most USB adapters it toggles DTR and RTS.
    fn preferred_policy(&self) -> ConnectionPolicy {
        ConnectionPolicy::Persistent
    }
}

/// Accepts `serial://<path>[@baud]` and a bare `<path>[@baud]`.
///
/// | Input | Result |
/// | --- | --- |
/// | `/dev/ttyUSB0`, `serial:///dev/ttyUSB0` | that port at 9600 baud |
/// | `/dev/ttyUSB0@19200`, `serial:///dev/ttyUSB0@19200` | that port at 19200 baud |
/// | `COM3` | that port at 9600 baud |
/// | `tcp://192.168.1.10` | [`ParseTargetError::WrongScheme`], so a caller can try another transport |
impl FromStr for SerialConnector {
    type Err = ParseTargetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let target = s.trim();
        if target.is_empty() {
            return Err(ParseTargetError::Empty);
        }

        // `serial:///dev/ttyUSB0` leaves a leading slash, which is part of the path.
        let rest = match target.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case(SCHEME) => rest,
            Some((scheme, _)) => {
                return Err(ParseTargetError::WrongScheme { scheme: scheme.to_owned() });
            }
            None => target,
        };

        let (path, baud) = match rest.rsplit_once('@') {
            Some((path, baud)) => (path, Some(baud)),
            None => (rest, None),
        };
        if path.is_empty() {
            return Err(ParseTargetError::Empty);
        }

        let connector = Self::new(path);
        match baud {
            Some(baud) => {
                let baud: u32 = baud
                    .trim()
                    .parse()
                    .map_err(|_| ParseTargetError::Baud { baud: baud.to_owned() })?;
                Ok(connector.with_baud(baud))
            }
            None => Ok(connector),
        }
    }
}

/// A target string that is not a serial target.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseTargetError {
    /// Nothing to parse.
    #[error("empty serial target")]
    Empty,

    /// Another transport's scheme.
    ///
    /// Distinct from the other variants so that a tool accepting several transports can try the
    /// next parser instead of reporting this as a failure.
    #[error("{scheme}:// is not a serial target")]
    WrongScheme {
        /// The scheme that was found.
        scheme: String,
    },

    /// Not a baud rate.
    #[error("{baud:?} is not a baud rate")]
    Baud {
        /// The text that was rejected.
        baud: String,
    },
}

/// A serial port the operating system knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// The device file to open.
    pub path: String,
    /// What the port is attached to, as far as the system can tell.
    pub kind: PortKind,
}

impl PortInfo {
    /// A connector for this port, with the protocol's defaults.
    pub fn connector(&self) -> SerialConnector {
        SerialConnector::new(self.path.clone())
    }
}

impl fmt::Display for PortInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}  {}", self.path, self.kind)
    }
}

/// What a serial port is attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortKind {
    /// A USB serial adapter, which is what a TESmart switch is usually reached through.
    Usb(UsbInfo),
    /// A port on the PCI bus.
    Pci,
    /// A Bluetooth serial profile.
    Bluetooth,
    /// Anything else, including on-board UARTs.
    Unknown,
}

impl fmt::Display for PortKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb(info) => write!(f, "USB {info}"),
            Self::Pci => f.write_str("PCI"),
            Self::Bluetooth => f.write_str("Bluetooth"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

/// What the system knows about a USB serial adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbInfo {
    /// USB vendor id.
    pub vid: u16,
    /// USB product id.
    pub pid: u16,
    /// Manufacturer string, when the device or the system provides one.
    pub manufacturer: Option<String>,
    /// Product string, when the device or the system provides one.
    pub product: Option<String>,
    /// Serial number, when the device provides one.
    pub serial_number: Option<String>,
}

impl fmt::Display for UsbInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}:{:04x}", self.vid, self.pid)?;
        for text in [self.manufacturer.as_deref(), self.product.as_deref()].into_iter().flatten() {
            write!(f, " {text}")?;
        }
        Ok(())
    }
}

/// Lists the serial ports the system knows about.
///
/// Useful for finding which `/dev/ttyUSB*` the adapter landed on. Enumeration is best effort:
/// a port that appears here may be in use or may not be a switch, and a port missing from here
/// can still be opened by path.
///
/// # Errors
///
/// [`EnumerateError`] if the platform cannot enumerate ports.
///
/// # Panics
///
/// On Linux without `libudev`, the underlying crate reads `/sys/class/tty` and panics if that
/// directory does not exist, which happens only in an environment with no `/sys` mounted.
pub fn available_ports() -> Result<Vec<PortInfo>, EnumerateError> {
    let ports = tokio_serial::available_ports()
        .map_err(|error| EnumerateError { message: error.to_string() })?;

    Ok(ports
        .into_iter()
        .map(|port| PortInfo {
            path: port.port_name,
            kind: match port.port_type {
                tokio_serial::SerialPortType::UsbPort(usb) => PortKind::Usb(UsbInfo {
                    vid: usb.vid,
                    pid: usb.pid,
                    manufacturer: usb.manufacturer,
                    product: usb.product,
                    serial_number: usb.serial_number,
                }),
                tokio_serial::SerialPortType::PciPort => PortKind::Pci,
                tokio_serial::SerialPortType::BluetoothPort => PortKind::Bluetooth,
                tokio_serial::SerialPortType::Unknown => PortKind::Unknown,
            },
        })
        .collect())
}

/// The system's serial ports could not be listed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot list serial ports: {message}")]
pub struct EnumerateError {
    message: String,
}

/// Turns a `tokio_serial::Error` into an `io::Error`, keeping the kind where there is one.
///
/// "No such device" and "already in use" are both worth telling apart: the second one usually
/// means another copy of the tool is running.
fn into_io_error(error: tokio_serial::Error) -> io::Error {
    let kind = match error.kind() {
        tokio_serial::ErrorKind::NoDevice => io::ErrorKind::NotFound,
        tokio_serial::ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        tokio_serial::ErrorKind::Io(kind) => kind,
        tokio_serial::ErrorKind::Unknown => io::ErrorKind::Other,
    };
    io::Error::new(kind, error.description)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(target: &str) -> SerialConnector {
        target.parse().unwrap_or_else(|error| panic!("{target:?}: {error}"))
    }

    #[test]
    fn the_defaults_are_the_documented_line_settings() {
        let connector = SerialConnector::new("/dev/ttyUSB0");

        assert_eq!(connector.baud(), 9600);
        assert!(connector.is_exclusive());
        assert_eq!(connector.open_timeout(), Duration::from_secs(5));
        assert_eq!(connector.to_string(), "serial:///dev/ttyUSB0@9600");
        assert_eq!(connector.kind(), TransportKind::Serial);
        // Opening a tty is not free, so unlike TCP this transport wants one connection.
        assert_eq!(connector.preferred_policy(), ConnectionPolicy::Persistent);
    }

    #[test]
    fn parses_paths_with_and_without_a_scheme_or_baud() {
        for target in ["/dev/ttyUSB0", "serial:///dev/ttyUSB0", "SERIAL:///dev/ttyUSB0"] {
            let connector = parse(target);
            assert_eq!(connector.path(), "/dev/ttyUSB0", "{target}");
            assert_eq!(connector.baud(), DEFAULT_BAUD, "{target}");
        }

        for target in ["/dev/ttyUSB0@19200", "serial:///dev/ttyUSB0@19200"] {
            let connector = parse(target);
            assert_eq!(connector.path(), "/dev/ttyUSB0");
            assert_eq!(connector.baud(), 19200);
        }

        // Windows names and a PTY's baud 0 both have to survive the parser.
        assert_eq!(parse("COM3").path(), "COM3");
        assert_eq!(parse("/dev/pts/7@0").baud(), 0);
    }

    #[test]
    fn display_round_trips_through_from_str() {
        for target in ["/dev/ttyUSB0@9600", "/dev/ttyS0@115200", "COM3@9600"] {
            let connector = parse(target);
            assert_eq!(connector.to_string(), format!("serial://{target}"));
            assert_eq!(parse(&connector.to_string()), connector);
        }
    }

    #[test]
    fn another_transports_scheme_is_a_distinct_error() {
        // The CLI relies on this to try the TCP parser rather than reporting a failure.
        assert_eq!(
            "tcp://192.168.1.10:5000".parse::<SerialConnector>(),
            Err(ParseTargetError::WrongScheme { scheme: "tcp".to_owned() })
        );
    }

    #[test]
    fn rejects_what_is_not_a_target() {
        assert_eq!("".parse::<SerialConnector>(), Err(ParseTargetError::Empty));
        assert_eq!("   ".parse::<SerialConnector>(), Err(ParseTargetError::Empty));
        assert_eq!("serial://".parse::<SerialConnector>(), Err(ParseTargetError::Empty));
        assert_eq!(
            "/dev/ttyUSB0@fast".parse::<SerialConnector>(),
            Err(ParseTargetError::Baud { baud: "fast".to_owned() })
        );
    }

    #[test]
    fn the_line_settings_are_protocol_constants() {
        let settings = SerialConnector::new("/dev/ttyUSB0").with_baud(9600).settings();
        // Nothing to assert against but the builder itself; what matters is that the four
        // non-baud settings are fixed and cannot be configured away.
        let built = format!("{settings:?}");
        assert!(built.contains("Eight"), "{built}");
        assert!(built.contains("None"), "{built}");
        assert!(built.contains("One"), "{built}");
    }

    #[test]
    fn enumeration_does_not_fail_on_this_machine() {
        // A smoke test: the list may well be empty, but listing must work.
        let ports = available_ports().expect("ports can be listed");
        for port in &ports {
            assert_ne!(port.path, "");
            assert_ne!(port.to_string(), "");
            assert_eq!(port.connector().path(), port.path);
        }
    }

    #[tokio::test]
    async fn a_port_that_does_not_exist_names_the_target() {
        let connector = SerialConnector::new("/dev/tesmart-does-not-exist");

        let error = connector.connect().await.expect_err("there is no such device");

        let message = error.to_string();
        assert!(message.contains("serial:///dev/tesmart-does-not-exist@9600"), "{message}");
    }
}
