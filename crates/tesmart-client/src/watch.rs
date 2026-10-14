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

/// An open watch on a switch.
///
/// Holds the connection open for as long as it lives, because a switch only pushes reports to a
/// connected client. Dropping it restores the session's previous connection policy; call
/// [`close`](Watch::close) to also close the connection, which is what makes the switch stop
/// pushing.
#[derive(Debug)]
pub struct Watch<'a, C: Connector> {
    kvm: &'a mut Kvm<C>,
    options: WatchOptions,
    previous_policy: ConnectionPolicy,
    last_known: Option<Port>,
    next_poll: Option<Instant>,
}

impl<'a, C: Connector> Watch<'a, C> {
    pub(crate) fn new(kvm: &'a mut Kvm<C>, options: WatchOptions) -> Self {
        let previous_policy = kvm.session().connection_policy();
        kvm.session_mut().set_connection_policy(ConnectionPolicy::Persistent);

        // Polling needs a read command; without one, listening is all there is.
        let can_poll = kvm.spec().may_attempt(RequestKind::ReadActivePort);
        let poll_interval = options.poll_interval.filter(|_| can_poll);
        if options.poll_interval.is_some() && !can_poll {
            tracing::debug!(
                target: "tesmart::op",
                "polling disabled: this model cannot report its active port"
            );
        }

        Self {
            kvm,
            options: WatchOptions { poll_interval },
            previous_policy,
            last_known: None,
            next_poll: poll_interval.map(|interval| Instant::now() + interval),
        }
    }

    /// The last port this watch knows about.
    pub fn last_known(&self) -> Option<Port> {
        self.last_known
    }

    /// The options actually in use, after capability checks.
    pub fn options(&self) -> WatchOptions {
        self.options
    }

    /// Records a starting point, so the first change is reported relative to it.
    ///
    /// # Errors
    ///
    /// The read's failure. Harmless to skip: without it, the first change simply has no
    /// `previous`.
    pub async fn prime(&mut self) -> Result<Port, OpError> {
        let port = self.kvm.active_port().await?;
        self.last_known = Some(port);
        Ok(port)
    }

    /// Waits for the next change.
    ///
    /// # Errors
    ///
    /// A transport failure, or a poll that fails for a reason other than silence.
    pub async fn next_change(&mut self) -> Result<PortChange, OpError> {
        loop {
            // A poll that is already due goes first: it is the cheaper way to catch up after a
            // long gap.
            if let Some(due) = self.next_poll
                && Instant::now() >= due
            {
                self.next_poll = self.options.poll_interval.map(|interval| due + interval);
                if let Some(change) = self.poll().await? {
                    return Ok(change);
                }
                continue;
            }

            let event = match self.next_poll {
                Some(due) => {
                    tokio::select! {
                        event = self.kvm.session_mut().next_event() => Some(event?),
                        () = tokio::time::sleep_until(due) => None,
                    }
                }
                None => Some(self.kvm.session_mut().next_event().await?),
            };

            if let Some(Event::PortChanged(port)) = event
                && let Some(change) = self.accept(port, ChangeSource::Reported)
            {
                return Ok(change);
            }
        }
    }

    /// Closes the connection, which is what stops the switch pushing reports.
    ///
    /// Dropping the watch cannot do this, because closing needs to await.
    pub async fn close(self) {
        self.kvm.session_mut().disconnect().await;
        // `Drop` restores the connection policy.
    }

    async fn poll(&mut self) -> Result<Option<PortChange>, OpError> {
        match self.kvm.session_mut().exchange(Request::ReadActivePort).await {
            Ok(Response::ActivePort(port)) => Ok(self.accept(port, ChangeSource::Polled)),
            Ok(_) => Ok(None),
            // A silent read is normal on this hardware; keep watching.
            Err(ClientError::NoReply { .. }) => {
                tracing::debug!(target: "tesmart::op", "poll went unanswered");
                Ok(None)
            }
            Err(other) => Err(other.into()),
        }
    }

    /// Turns an observation into a change, unless it is what we already knew.
    fn accept(&mut self, port: Port, source: ChangeSource) -> Option<PortChange> {
        if self.last_known == Some(port) {
            return None;
        }
        let previous = self.last_known.replace(port);
        Some(PortChange { port, previous, source })
    }
}

impl<C: Connector> Drop for Watch<'_, C> {
    fn drop(&mut self) {
        // The connection cannot be closed from `drop` (that needs an await), but the policy can
        // be put back, so the next command behaves as the caller configured it.
        self.kvm.session_mut().set_connection_policy(self.previous_policy);
    }
}
