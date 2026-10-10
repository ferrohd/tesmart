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
