//! The operations: what you actually want to do to a KVM switch.
//!
//! Each one is written against the model's [capabilities](tesmart_proto::Capabilities) rather
//! than against a model name, and each returns what it managed to establish. The protocol
//! acknowledges so little that "sent" and "confirmed" have to be different answers.

use core::time::Duration;

use tesmart_proto::engine::Policy;
use tesmart_proto::model::ModelInfo;
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, NetSetting, Output, Step, SwitchInterval, Toggle,
};
use tesmart_proto::{Family, Port, PortCount, Request, RequestKind, Response, Support};

use crate::error::{ClientError, OpError};
use crate::ops::{Ops, Parts, Wanted};
use crate::outcome::{
    Confirmed, NetOutcome, NetReport, PortCountProbe, Sent, SwitchOutcome, Unverified,
};
use crate::session::Session;
use crate::spec::ModelSpec;
use crate::transport::ConnectionPolicy;
use crate::transport::Connector;
use crate::watch::{Watch, WatchOptions};

/// How long to wait after a switch command before reading the port back.
///
/// PiKVM waits a full second after each switch; the protocol notes recommend the same.
const DEFAULT_SETTLE: Duration = Duration::from_secs(1);

/// Whether to confirm a switch by reading the port back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verify {
    /// Verify when the model is believed to support reading its port. The default.
    #[default]
    Auto,
    /// Always try, even on a model where reading is unproven.
    Always,
    /// Never read back.
    Never,
}

/// How to perform a switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchOptions {
    /// Whether to confirm the result.
    pub verify: Verify,

    /// Read the active port first and do nothing if it already matches.
    ///
    /// Off by default: it makes every switch cost an extra command gap, for a case that rarely
    /// matters.
    pub skip_if_active: bool,

    /// How long to wait between the command and the read-back.
    pub settle: Duration,

    /// How many read-backs to try before giving up on confirming.
    pub attempts: u8,
}

impl Default for SwitchOptions {
    fn default() -> Self {
        Self { verify: Verify::Auto, skip_if_active: false, settle: DEFAULT_SETTLE, attempts: 2 }
    }
}

impl SwitchOptions {
    /// Sets whether to confirm the result.
    #[must_use]
    pub const fn verify(mut self, verify: Verify) -> Self {
        self.verify = verify;
        self
    }

    /// Sets whether to read the port first and skip a redundant switch.
    #[must_use]
    pub const fn skip_if_active(mut self, skip: bool) -> Self {
        self.skip_if_active = skip;
        self
    }

    /// Sets the wait between the command and the read-back.
    #[must_use]
    pub const fn settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }

    /// Sets how many read-backs to try.
    #[must_use]
    pub const fn attempts(mut self, attempts: u8) -> Self {
        self.attempts = attempts;
        self
    }
}

/// A KVM switch, with operations that know what it can confirm.
pub struct Kvm<C: Connector> {
    ops: Ops<C>,
    defaults: SwitchOptions,
}

impl<C: Connector> Kvm<C> {
    /// Starts building a switch handle.
    pub fn builder(connector: C) -> KvmBuilder<C> {
        KvmBuilder::new(connector)
    }

    /// A handle to a catalogued model.
    ///
    /// # Errors
    ///
    /// [`BuildError::WrongFamily`] if the model is an HDMI matrix; use
    /// [`Matrix`](crate::Matrix) for those.
    pub fn from_model(connector: C, model: &'static ModelInfo) -> Result<Self, BuildError> {
        KvmBuilder::new(connector).model(model).build()
    }

    /// What the switch is believed to be.
    pub fn spec(&self) -> &ModelSpec {
        &self.ops.spec
    }

    /// How many inputs it has.
    pub fn ports(&self) -> PortCount {
        self.ops.spec.ports
    }

    /// Which protocol family it speaks.
    pub fn family(&self) -> Family {
        self.ops.spec.family
    }

    /// The underlying session, for raw exchanges and statistics.
    pub fn session(&self) -> &Session<C, Family> {
        &self.ops.session
    }

    /// The underlying session.
    pub fn session_mut(&mut self) -> &mut Session<C, Family> {
        &mut self.ops.session
    }

    /// Reads the active port.
    ///
}
