//! Operations for HDMI matrices: family B.
//!
//! A matrix is not a KVM with more outputs. It has no active port, no switch opcode and no hex
//! protocol at all; it routes any input to any output with ASCII strings. It also has something
//! no KVM has:
//!
//! > **it can be read back.**
//!
//! `MT00RD0000NT` answers with the whole routing table, so a route is the one write in this
//! entire protocol family that can be confirmed rather than merely sent. That is why
//! [`Matrix::route`] returns a [`RouteOutcome`] with a real [`Verified`](RouteOutcome::Verified)
//! case and [`Kvm::set_buzzer`](crate::Kvm::set_buzzer) can never do better than [`Sent`].
//!
//! What a given matrix supports varies by model rather than by family: the 4x4 series has a
//! display timeout and no presets, the 8x8 and 16x16 have presets and a buzzer and no display
//! timeout, and the RS232-only 4x4 models have none of them. Each operation is gated on the
//! registry, so asking an 8x8 for a display timeout is refused before a byte is sent.

use core::time::Duration;

use tesmart_proto::engine::Policy;
use tesmart_proto::model::ModelInfo;
use tesmart_proto::value::{Buzzer, MatrixTimeout, NetSetting, Preset};
use tesmart_proto::{Family, Port, PortCount, Request, RequestKind, Response, Support};

use crate::error::{ClientError, OpError};
use crate::kvm::BuildError;
use crate::ops::{Ops, Parts, Wanted};
use crate::outcome::{
    Confirmed, NetOutcome, NetReport, PresetOutcome, RouteOutcome, RoutingReport, Sent, Unverified,
};
use crate::session::Session;
use crate::spec::ModelSpec;
use crate::transport::{ConnectionPolicy, Connector};

/// How long to wait after a routing command before reading the routing back.
///
/// The same one second the KVM operations wait after a switch, for the same reason: this
/// hardware drops commands that arrive back to back.
const DEFAULT_SETTLE: Duration = Duration::from_secs(1);

/// Whether to confirm a routing change by reading the routing back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerifyRouting {
    /// Read the routing back and check it. The default, because a matrix can.
    #[default]
    Yes,
    /// Send and return.
    No,
}

/// How to route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteOptions {
    /// Whether to confirm the result.
    pub verify: VerifyRouting,
    /// How long to wait between the command and the read-back.
    pub settle: Duration,
}

impl Default for RouteOptions {
    fn default() -> Self {
        Self { verify: VerifyRouting::Yes, settle: DEFAULT_SETTLE }
    }
}

impl RouteOptions {
    /// Sets whether to confirm the result.
    #[must_use]
    pub const fn verify(mut self, verify: VerifyRouting) -> Self {
        self.verify = verify;
        self
    }

    /// Sets the wait between the command and the read-back.
    #[must_use]
    pub const fn settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }
}
