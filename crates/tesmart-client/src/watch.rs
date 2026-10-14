//! Following the active port, however it changes.
//!
//! Two mechanisms, because neither is sufficient on its own:
//!
//! - while a client is connected, the switch pushes `AA BB 03 11 XX ..` when the port changes by
//!   any means: front panel, hotkey, remote. This is undocumented, observed in packet captures,
//!   and stops as soon as the connection closes;
//! - polling the active port, which works on any model that can be read at all. PiKVM polls
//!   every 10 seconds.
//!
//! A [`Watch`] does both and de-duplicates, so a change is reported once whichever way it was
//! noticed, and a poll that finds nothing new says nothing.

use core::time::Duration;

use tesmart_proto::engine::Event;
use tesmart_proto::{Port, Request, RequestKind, Response};
use tokio::time::Instant;

use crate::error::{ClientError, OpError};
use crate::kvm::Kvm;
use crate::transport::ConnectionPolicy;
use crate::transport::Connector;

/// How often to poll, when polling is possible.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How to watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchOptions {
    /// How often to read the active port, in addition to listening.
    ///
    /// `None` listens only, which is silent on firmware that never volunteers anything.
    pub poll_interval: Option<Duration>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self { poll_interval: Some(DEFAULT_POLL_INTERVAL) }
    }
}

impl WatchOptions {
    /// Listen only, with no polling.
    pub const fn listen_only() -> Self {
        Self { poll_interval: None }
    }

    /// Sets how often to poll.
    #[must_use]
    pub const fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = Some(interval);
        self
    }
}

/// How a change came to light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeSource {
    /// The switch volunteered it.
    Reported,
    /// A poll found it.
    Polled,
}

impl core::fmt::Display for ChangeSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Reported => "reported",
            Self::Polled => "polled",
        })
    }
}

/// The active port changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortChange {
    /// The port now active.
    pub port: Port,
    /// The port before, when it was known.
    pub previous: Option<Port>,
    /// How it was noticed.
    pub source: ChangeSource,
}

impl core::fmt::Display for PortChange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.previous {
            Some(previous) => write!(f, "port {previous} -> {} ({})", self.port, self.source),
            None => write!(f, "port {} ({})", self.port, self.source),
        }
    }
}
