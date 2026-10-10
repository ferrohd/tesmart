//! What an operation actually achieved.
//!
//! The protocol acknowledges almost nothing: of the ten commands a single-monitor KVM
//! understands, two can be read back and the rest are fire and forget. Operations therefore
//! return outcomes rather than `()`, so "the switch is on port 3" and "port 3 was requested and
//! nobody can say what happened" stay distinguishable all the way to the user.

use core::fmt;

use tesmart_proto::value::{NetConfig, NetField, NetSetting, Preset};
use tesmart_proto::wire::RoutingReply;
use tesmart_proto::{Port, PortCount, RequestKind, Support};

/// The result of asking the switch to change input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwitchOutcome {
    /// The switch was already on that port, and nothing was sent.
    ///
    /// Only possible when the pre-read was requested; it is off by default because it costs a
    /// full command gap.
    AlreadyActive(Port),

    /// The command was sent and the switch confirmed the new port.
    Verified {
        /// The port now active.
        port: Port,
        /// How many reads it took to confirm.
        attempts: u8,
    },

    /// The command was sent and nobody can confirm what happened.
    Unverified {
        /// The port that was requested.
        port: Port,
        /// Why it could not be confirmed.
        reason: Unverified,
        /// Anything worth telling the user.
        warnings: Vec<Warning>,
    },
}

impl SwitchOutcome {
    /// The port the switch is on, or was asked to move to.
    pub fn port(&self) -> Port {
        match self {
            Self::AlreadyActive(port)
            | Self::Verified { port, .. }
            | Self::Unverified { port, .. } => *port,
        }
    }

    /// Whether the switch confirmed the result.
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::AlreadyActive(_) | Self::Verified { .. })
    }
}

impl fmt::Display for SwitchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive(port) => write!(f, "already on port {port}"),
            Self::Verified { port, attempts: 1 } => write!(f, "switched to port {port}"),
            Self::Verified { port, attempts } => {
                write!(f, "switched to port {port} (confirmed after {attempts} reads)")
            }
            Self::Unverified { port, reason, .. } => {
                write!(f, "port {port} requested, not verified: {reason}")
            }
        }
    }
}

/// Why an operation's effect could not be confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unverified {
    /// This model cannot report its active port.
    ///
    /// True of the 4x1 DVI series, and unproven on the P23 series.
    ModelCannotRead {
        /// What the registry says about reading the port on this model.
        support: Support,
    },

    /// Verification was switched off by the caller.
    Disabled,

    /// The switch was asked and stayed silent.
    ///
    /// Not evidence of failure: read replies are known to go missing on this hardware.
    NoReply {
        /// How many times the read was sent.
        attempts: u8,
    },
}

impl fmt::Display for Unverified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelCannotRead { support } => {
                write!(f, "reading the active port is {support} on this model")
            }
            Self::Disabled => f.write_str("verification was disabled"),
            Self::NoReply { attempts } => {
                write!(f, "no reply to the read-back after {attempts} attempt(s)")
            }
        }
    }
}

/// A command that the protocol defines no reply to.
///
/// The bytes reached the transport. The switch does not acknowledge buzzer, display timeout or
/// auto-detect changes, and none of them can be read back on any series, so this is the most
/// that can honestly be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// What was sent.
    pub kind: RequestKind,
    /// What is known about this command on this model.
    pub support: Support,
    /// Anything worth telling the user.
    pub warnings: Vec<Warning>,
}

impl fmt::Display for Sent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} sent, state cannot be read back", self.kind)
    }
}

/// What trying every input discovered about how many there are.
///
/// Produced by [`Kvm::probe_port_count`](crate::Kvm::probe_port_count), which switches inputs to
/// find out, so this carries whether the original input was put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortCountProbe {
    /// The highest input the switch accepted, if any did.
    ///
    /// `None` means no candidate read back as itself: either the switch ignores switching
    /// altogether or it stopped answering partway through.
    pub found: Option<Port>,
    /// How many inputs the registry or the caller claimed.
    pub claimed: PortCount,
    /// The input that was active before probing started.
    pub original: Port,
    /// Whether the original input was confirmed restored afterwards.
    pub restored: bool,
}

impl PortCountProbe {
    /// Whether the probe agrees with what was claimed.
    pub fn agrees(&self) -> bool {
        self.found.is_some_and(|found| found.get() == self.claimed.get())
    }
}

impl fmt::Display for PortCountProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.found {
            Some(found) if self.agrees() => write!(f, "{found} inputs, as expected"),
            Some(found) => write!(f, "{found} inputs, but {} were expected", self.claimed),
            None => f.write_str("no input could be confirmed"),
        }?;
        if !self.restored {
            write!(f, "; the original input {} could not be confirmed restored", self.original)?;
        }
        Ok(())
    }
}

/// The switch's network settings, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetReport {
    /// The values that came back.
    pub config: NetConfig,
    /// Fields the switch did not answer for.
    pub missing: Vec<NetField>,
    /// Anything worth telling the user.
    pub warnings: Vec<Warning>,
}

impl NetReport {
    /// Whether all four fields were read.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty() && self.config.is_complete()
    }
}

/// The result of writing network settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetOutcome {
    /// What was asked for.
    pub requested: Vec<NetSetting>,
    /// What the switch reported afterwards.
    pub applied: NetConfig,
    /// Fields whose read-back does not match what was written.
    pub mismatched: Vec<NetField>,
    /// Anything worth telling the user. Always includes the power-cycle warning.
    pub warnings: Vec<Warning>,
}

impl NetOutcome {
    /// Whether every written field read back as written.
    pub fn is_consistent(&self) -> bool {
        self.mismatched.is_empty()
    }
}
