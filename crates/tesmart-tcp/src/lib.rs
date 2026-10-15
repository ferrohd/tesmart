//! TCP transport for TESmart KVM switches.
//!
//! One implementation of [`Connector`] and nothing else: no protocol, no retries, no operations.
//! All of that lives in `tesmart-proto` and `tesmart-client`, which is what makes this crate
//! about 200 lines.
//!
//! ```no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use tesmart_client::Kvm;
//! use tesmart_proto::{Port, model};
//! use tesmart_tcp::TcpConnector;
//!
//! // "192.168.1.10", "192.168.1.10:5000", "tcp://kvm.local:5000" and "[fd00::1]" all parse.
//! let connector: TcpConnector = "192.168.1.10".parse()?;
//! let mut kvm = Kvm::from_model(connector, model::lookup("HKS1601-EB23").unwrap())?;
//!
//! let outcome = kvm.switch(Port::new(4)?).await?;
//! println!("{outcome}");
//! # Ok(()) }
//! ```
//!
//! # What to know about the LAN port
//!
//! - The factory address is **192.168.1.10:5000** with gateway 192.168.1.1 and mask
//!   255.255.255.0. There is **no DHCP**: a switch straight out of the box is only reachable
//!   from that subnet.
//! - It is a serial-to-Ethernet bridge, not a network service. Expect serial behaviour over
//!   TCP: split replies, unsolicited bytes, segments of about 48 bytes. The decoder in
//!   `tesmart-proto` is built for exactly that.
//! - **There is no authentication.** Anyone who can reach port 5000 can switch the inputs of
//!   every machine attached. Keep it off the internet.
//! - Newer units shipped a 10 Mbps-only Ethernet controller that may fail to link with
//!   2.5/5/10 GbE switch ports. If a connection times out and the cable is right, try a 1 GbE
//!   port. Source: `bbeaudoin/bash` field notes.
//! - Nagle's algorithm is disabled by default: every command is a 6 byte frame, which is
//!   exactly what Nagle would sit on waiting for more.
#![allow(clippy::must_use_candidate)]

use core::fmt;
use core::future::Future;
use core::str::FromStr;
use core::time::Duration;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tesmart_client::{Connector, TransportError, TransportKind};
use tokio::net::TcpStream;

/// The address every TESmart switch leaves the factory with.
pub const DEFAULT_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 10);

/// The port every TESmart switch leaves the factory with.
pub const DEFAULT_PORT: u16 = 5000;

/// How long to wait for a connection before giving up.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The URL scheme this crate answers to.
pub const SCHEME: &str = "tcp";

/// Where the switch is: an address, or a name to resolve.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    /// An address, used as is.
    Ip(IpAddr),
    /// A name, resolved on every connection attempt.
    Name(String),
}

impl Host {
    /// The address, if this host does not need resolving.
    pub fn ip(&self) -> Option<IpAddr> {
        match self {
            Self::Ip(ip) => Some(*ip),
            Self::Name(_) => None,
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // IPv6 needs brackets to be unambiguous next to a port.
            Self::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]"),
            Self::Ip(IpAddr::V4(ip)) => write!(f, "{ip}"),
            Self::Name(name) => f.write_str(name),
        }
    }
}

impl From<IpAddr> for Host {
    fn from(ip: IpAddr) -> Self {
        Self::Ip(ip)
    }
}

impl From<Ipv4Addr> for Host {
    fn from(ip: Ipv4Addr) -> Self {
        Self::Ip(IpAddr::V4(ip))
    }
}

impl FromStr for Host {
    type Err = ParseTargetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let host = s.trim();
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(Self::Ip(ip));
        }
        if let Some(ip) = host
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|inner| inner.parse::<IpAddr>().ok())
        {
            return Ok(Self::Ip(ip));
        }
        if is_plausible_name(host) {
            Ok(Self::Name(host.to_owned()))
        } else {
            Err(ParseTargetError::Host { host: host.to_owned() })
        }
    }
}

/// Opens TCP connections to a switch's LAN port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpConnector {
    host: Host,
    port: u16,
    connect_timeout: Duration,
    nodelay: bool,
}

impl TcpConnector {
    /// A connector for `host` and `port`.
    pub fn new(host: impl Into<Host>, port: u16) -> Self {
        Self { host: host.into(), port, connect_timeout: DEFAULT_CONNECT_TIMEOUT, nodelay: true }
    }

    /// A connector for an address that is already resolved.
    pub fn from_addr(addr: SocketAddr) -> Self {
        Self::new(addr.ip(), addr.port())
    }

    /// Sets how long to wait for a connection.
    #[must_use]
    pub const fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Sets whether to disable Nagle's algorithm. On by default.
    #[must_use]
    pub const fn with_nodelay(mut self, nodelay: bool) -> Self {
        self.nodelay = nodelay;
        self
    }

    /// Where the switch is.
    pub const fn host(&self) -> &Host {
        &self.host
    }

    /// Which port it listens on.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// How long a connection attempt may take.
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// The socket address, when no resolution is needed.
    pub fn socket_addr(&self) -> Option<SocketAddr> {
        self.host.ip().map(|ip| SocketAddr::new(ip, self.port))
    }

    /// Connects, inside the connect timeout.
    async fn connect_within_timeout(&self) -> Result<TcpStream, TransportError> {
        match tokio::time::timeout(self.connect_timeout, self.attempt()).await {
            Ok(result) => result,
            Err(_elapsed) => Err(TransportError::connect_timeout(self, self.connect_timeout)),
        }
    }

    /// Tries every address the host resolves to, in order.
    async fn attempt(&self) -> Result<TcpStream, TransportError> {
        let mut last: Option<io::Error> = None;

        for addr in self.addresses().await? {
            tracing::debug!(target: "tesmart::tcp", %addr, "connecting");
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    self.configure(&stream);
                    return Ok(stream);
                }
                Err(error) => {
                    tracing::debug!(target: "tesmart::tcp", %addr, %error, "connect failed");
                    last = Some(error);
                }
            }
        }

        Err(match last {
            Some(error) => TransportError::connect(self, error),
            // Resolution succeeded and returned nothing at all.
            None => TransportError::Target(format!("{self} resolved to no addresses")),
        })
    }

    /// Resolves the host. An address is returned as is; a name goes through the resolver on
    /// every attempt, so a switch that moved is found again on reconnect.
    async fn addresses(&self) -> Result<Vec<SocketAddr>, TransportError> {
        match &self.host {
            Host::Ip(ip) => Ok(vec![SocketAddr::new(*ip, self.port)]),
            Host::Name(name) => {
                let resolved: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), self.port))
                    .await
                    .map_err(|error| TransportError::connect(self, error))?
                    .collect();
                tracing::debug!(
                    target: "tesmart::tcp",
                    %name, addresses = resolved.len(),
                    "resolved"
                );
                Ok(resolved)
            }
        }
    }

    fn configure(&self, stream: &TcpStream) {
        if !self.nodelay {
            return;
        }
        // A failed socket option is not worth throwing a working connection away for.
        if let Err(error) = stream.set_nodelay(true) {
            tracing::debug!(
                target: "tesmart::tcp",
                %error,
                "could not disable Nagle; 6 byte commands may be delayed"
            );
        }
    }
}

impl Default for TcpConnector {
    /// The factory address: `192.168.1.10:5000`.
    fn default() -> Self {
        Self::new(DEFAULT_ADDRESS, DEFAULT_PORT)
    }
}

impl fmt::Display for TcpConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}://{}:{}", self.host, self.port)
    }
}

impl Connector for TcpConnector {
    type Io = TcpStream;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        self.connect_within_timeout()
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Tcp
    }
}

/// Accepts `tcp://host[:port]`, `host:port` and a bare `host`, with IPv6 in brackets.
///
/// | Input | Result |
/// | --- | --- |
/// | `tcp://192.168.1.10:5000`, `192.168.1.10:5000` | that address |
/// | `tcp://192.168.1.10`, `192.168.1.10` | port 5000 |
/// | `kvm.local:5000`, `kvm.local` | a name, resolved at connect time |
/// | `[fd00::1]:5000`, `[fd00::1]`, `fd00::1` | IPv6, with or without a port |
/// | `serial:///dev/ttyUSB0` | [`ParseTargetError::WrongScheme`], so a caller can try another transport |
impl FromStr for TcpConnector {
    type Err = ParseTargetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let target = s.trim();
        if target.is_empty() {
            return Err(ParseTargetError::Empty);
        }

        let rest = match target.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case(SCHEME) => rest,
            Some((scheme, _)) => {
                return Err(ParseTargetError::WrongScheme { scheme: scheme.to_owned() });
            }
            None => target,
        };
        let rest = rest.trim_end_matches('/');
        if rest.is_empty() {
            return Err(ParseTargetError::Empty);
        }

        let (host, port) = split_host_port(rest)?;
        Ok(Self::new(host, port))
    }
}

/// Splits `host:port`, being careful that a bare IPv6 literal is full of colons.
fn split_host_port(rest: &str) -> Result<(Host, u16), ParseTargetError> {
    // Anything that parses whole as an address or a socket address is unambiguous.
    if let Ok(addr) = rest.parse::<SocketAddr>() {
        return Ok((Host::Ip(addr.ip()), checked_port(addr.port(), rest)?));
    }
    if let Ok(ip) = rest.parse::<IpAddr>() {
        return Ok((Host::Ip(ip), DEFAULT_PORT));
    }

    // A bracketed host may carry a port: `[fd00::1]:5000`.
    if let Some(end) = rest.strip_prefix('[').and_then(|_| rest.find(']')) {
        let inside = &rest[1..end];
        let host: Host = inside.parse()?;
        let tail = &rest[end + 1..];
        let port = match tail.strip_prefix(':') {
            Some(port) => parse_port(port)?,
            None if tail.is_empty() => DEFAULT_PORT,
            None => return Err(ParseTargetError::Host { host: rest.to_owned() }),
        };
        return Ok((host, port));
    }

    match rest.rsplit_once(':') {
        Some((host, port)) => Ok((host.parse()?, parse_port(port)?)),
        None => Ok((rest.parse()?, DEFAULT_PORT)),
    }
}

fn parse_port(port: &str) -> Result<u16, ParseTargetError> {
    let parsed: u16 =
        port.trim().parse().map_err(|_| ParseTargetError::Port { port: port.to_owned() })?;
    checked_port(parsed, port)
}

fn checked_port(port: u16, text: &str) -> Result<u16, ParseTargetError> {
    if port == 0 {
        // Port 0 means "any port" to a listener and nothing at all to a client.
        return Err(ParseTargetError::Port { port: text.to_owned() });
    }
    Ok(port)
}

/// Whether a string could be a host name. Deliberately permissive: the resolver has the final
/// say, this only rejects what is obviously a different kind of target.
fn is_plausible_name(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && !host.starts_with('-')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

/// A target string that is not a TCP target.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseTargetError {
    /// Nothing to parse.
    #[error("empty TCP target")]
    Empty,

    /// Another transport's scheme.
    ///
    /// Distinct from the other variants so that a tool accepting several transports can try the
    /// next parser instead of reporting this as a failure.
    #[error("{scheme}:// is not a TCP target")]
    WrongScheme {
        /// The scheme that was found.
        scheme: String,
    },

    /// Not a host name or address.
    #[error("{host:?} is not a host name or IP address")]
    Host {
        /// The text that was rejected.
        host: String,
    },

    /// Not a usable TCP port.
    #[error("{port:?} is not a TCP port (expected 1..=65535)")]
    Port {
        /// The text that was rejected.
        port: String,
    },
}
