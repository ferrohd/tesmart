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
