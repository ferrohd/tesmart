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
    /// # Errors
    ///
    /// [`OpError::Unsupported`] if the model cannot report its port (the 4x1 DVI series cannot,
    /// and the P23 series is unproven), or the underlying exchange's failure.
    #[tracing::instrument(level = "debug", skip(self), fields(switch = %self.ops.session))]
    pub async fn active_port(&mut self) -> Result<Port, OpError> {
        self.ops.require(RequestKind::ReadActivePort)?;
        self.read_active_port().await
    }

    /// Switches every output to a port, with the default options.
    ///
    /// # Errors
    ///
    /// See [`switch_with`](Self::switch_with).
    pub async fn switch(&mut self, port: Port) -> Result<SwitchOutcome, OpError> {
        self.switch_with(port, self.defaults).await
    }

    /// Switches every output to a port.
    ///
    /// # Errors
    ///
    /// [`OpError::PortOutOfRange`] if the switch has no such port,
    /// [`OpError::VerificationFailed`] if the read-back reports a different port, or the
    /// underlying exchange's failure. A read-back that stays silent is **not** an error: it
    /// produces [`SwitchOutcome::Unverified`].
    #[tracing::instrument(level = "debug", skip(self, options), fields(switch = %self.ops.session, port = port.get()))]
    pub async fn switch_with(
        &mut self,
        port: Port,
        options: SwitchOptions,
    ) -> Result<SwitchOutcome, OpError> {
        let port = self.ops.check_port(port)?;
        let skip_reason = self.plan_verification(options.verify)?;

        if options.skip_if_active && skip_reason.is_none() {
            // A pre-read is only worth anything if reading works at all.
            if let Ok(current) = self.read_active_port().await
                && current == port
            {
                tracing::debug!(target: "tesmart::op", "already on the requested port");
                return Ok(SwitchOutcome::AlreadyActive(port));
            }
        }

        self.ops.session.exchange(Request::Switch(port)).await?;

        if let Some(reason) = skip_reason {
            return Ok(SwitchOutcome::Unverified {
                port,
                reason,
                warnings: self.ops.warnings(RequestKind::Switch),
            });
        }

        // The switch needs a moment before it reports the new port.
        tokio::time::sleep(options.settle).await;

        let budget = options.attempts.max(1);
        for attempt in 1..=budget {
            match self.read_active_port().await {
                Ok(found) if found == port => {
                    return Ok(SwitchOutcome::Verified { port, attempts: attempt });
                }
                Ok(found) => {
                    if attempt == budget {
                        // The switch answered, and the answer is wrong. Fail loudly.
                        return Err(OpError::VerificationFailed { expected: port, found });
                    }
                    tracing::debug!(
                        target: "tesmart::op",
                        found = found.get(),
                        "the switch still reports the old port; reading again"
                    );
                }
                Err(OpError::Client(ClientError::NoReply { attempts: tries, .. })) => {
                    if attempt == budget {
                        return Ok(SwitchOutcome::Unverified {
                            port,
                            reason: Unverified::NoReply { attempts: tries },
                            warnings: self.ops.warnings(RequestKind::ReadActivePort),
                        });
                    }
                }
                Err(other) => return Err(other),
            }
        }

        // Unreachable: the loop returns on its last iteration.
        Ok(SwitchOutcome::Unverified {
            port,
            reason: Unverified::NoReply { attempts: budget },
            warnings: self.ops.warnings(RequestKind::ReadActivePort),
        })
    }

    /// Steps one output of a dual-monitor switch to the next or previous input.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a single-monitor switch, where the same opcode sets the
    /// display timeout instead.
    pub async fn step(&mut self, output: Output, step: Step) -> Result<Sent, OpError> {
        self.ops.fire(Request::StepOutput { output, step }).await
    }

    /// Moves the USB link between the two outputs of a dual-monitor switch.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a single-monitor switch.
    pub async fn move_usb(&mut self) -> Result<Sent, OpError> {
        self.ops.fire(Request::MoveUsb).await
    }

    /// Turns keyboard and mouse pass-through on or off, on a dual-monitor switch.
    ///
    /// The values are inverted on the wire, which the profile handles; `On` here means
    /// pass-through is enabled.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a single-monitor switch, which has no such command.
    pub async fn set_km_passthrough(&mut self, state: Toggle) -> Result<Sent, OpError> {
        self.ops.fire(Request::KmPassthrough(state)).await
    }

    /// Chooses which key starts a hotkey sequence, on a dual-monitor switch.
    ///
    /// Hotkeys only work on the dedicated keyboard and mouse port.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a single-monitor switch.
    pub async fn set_hotkey_trigger(&mut self, key: HotkeyTrigger) -> Result<Sent, OpError> {
        self.ops.fire(Request::HotkeyTrigger(key)).await
    }

    /// Turns auto-switching on or off, on a dual-monitor switch.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a single-monitor switch.
    pub async fn set_auto_switch(&mut self, state: Toggle) -> Result<Sent, OpError> {
        self.ops.fire(Request::AutoSwitch(state)).await
    }

    /// Sets the auto-switch dwell time, on a dual-monitor switch.
    ///
    /// The only command in the protocol with a two-byte payload, and the only one whose
    /// encoding is inferred rather than documented: see
    /// [`SwitchInterval`](tesmart_proto::value::SwitchInterval).
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a single-monitor switch.
    pub async fn set_auto_switch_interval(
        &mut self,
        interval: SwitchInterval,
    ) -> Result<Sent, OpError> {
        self.ops.fire(Request::AutoSwitchInterval(interval)).await
    }

    /// Mutes or unmutes the buzzer.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on models whose documentation has no buzzer command.
    pub async fn set_buzzer(&mut self, buzzer: Buzzer) -> Result<Sent, OpError> {
        self.ops.fire(Request::Buzzer(buzzer)).await
    }

    /// Sets how long the front panel display stays lit.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on models without the command.
    pub async fn set_display_timeout(&mut self, timeout: DisplayTimeout) -> Result<Sent, OpError> {
        self.ops.fire(Request::DisplayTimeout(timeout)).await
    }

    /// Turns automatic input detection on or off.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on models without the command.
    pub async fn set_auto_input_detect(&mut self, state: Toggle) -> Result<Sent, OpError> {
        self.ops.fire(Request::AutoInputDetect(state)).await
    }

    /// Starts following the active port.
    ///
    /// Holds one connection open: a switch only pushes port reports to a connected client. On a
    /// model that can be read, the watch also polls, because pushed reports are an undocumented
    /// behaviour that not every firmware has.
    pub fn watch(&mut self, options: WatchOptions) -> Watch<'_, C> {
        Watch::new(self, options)
    }

    /// Reads all four network settings.
    ///
    /// A field the switch does not answer for is reported in
    /// [`NetReport::missing`](crate::NetReport::missing) rather than failing the whole call: the
    /// other three are still worth having.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] if the model is not believed to have the ASCII side-channel, or
    /// a transport failure. A silent field is not an error.
    #[tracing::instrument(level = "debug", skip(self), fields(switch = %self.ops.session))]
    pub async fn network(&mut self) -> Result<NetReport, OpError> {
        self.ops.network().await
    }
}
