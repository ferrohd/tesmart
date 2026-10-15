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
