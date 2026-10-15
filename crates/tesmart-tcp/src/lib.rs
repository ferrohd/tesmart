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
