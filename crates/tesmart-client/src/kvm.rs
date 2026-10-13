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

    /// Writes network settings, then reads them back.
    ///
    /// Three things are true of this operation and of no other:
    ///
    /// - the settings persist when the switch answers `OK` but take effect only after a power
    ///   cycle;
    /// - there is no factory reset, so a wrong address leaves RS232 as the only way back in;
    /// - over RS232 the commands themselves are disputed.
    ///
    /// All three come back in [`NetOutcome::warnings`](crate::NetOutcome::warnings). The
    /// [`Confirmed`] argument is there to make the call site say that a human asked for this.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] if the model is not believed to have the side-channel, or a
    /// transport failure; nothing is written after the first failure. A field that reads back as
    /// something else is **not** an error: it lands in
    /// [`NetOutcome::mismatched`](crate::NetOutcome::mismatched), because seeing all four values
    /// is what tells you how bad the mistake is before you power cycle.
    #[tracing::instrument(level = "debug", skip(self, changes, confirmed), fields(switch = %self.ops.session))]
    pub async fn set_network(
        &mut self,
        changes: &[NetSetting],
        confirmed: Confirmed,
    ) -> Result<NetOutcome, OpError> {
        self.ops.set_network(changes, confirmed).await
    }

    /// Finds out how many inputs the switch really has, by trying them.
    ///
    /// **This moves the active input.** There is no command that reports the port count, but an
    /// out-of-range switch command is ignored, so switching to 16, 15, 14 ... and reading back
    /// finds the first one that takes. The original port is restored afterwards.
    ///
    /// Borrowed from `krohrbaugh/teeheesmart`, which detects port counts the same way. Only
    /// worth running on a model the registry does not know, and only when the video
    /// interruption is acceptable.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] unless the switch can report its active port: without a
    /// read-back there is nothing to learn. Transport failures propagate; a switch that goes
    /// silent mid-probe ends the probe with what it found so far.
    #[tracing::instrument(level = "debug", skip(self), fields(switch = %self.ops.session))]
    pub async fn probe_port_count(&mut self) -> Result<PortCountProbe, OpError> {
        self.ops.require(RequestKind::ReadActivePort)?;
        self.ops.require(RequestKind::Switch)?;

        // Every read here skips the range check: the whole point is that the registry's port
        // count may be wrong, so a read-back above it is the discovery, not an error.
        let original = self.read_port_unchecked().await?;
        let settle = self.defaults.settle;
        let mut found = None;

        for candidate in (1..=Port::MAX).rev() {
            let Ok(port) = Port::new(candidate) else { continue };
            self.ops.session.exchange(Request::Switch(port)).await?;
            tokio::time::sleep(settle).await;

            match self.read_port_unchecked().await {
                // The switch took it, so this port exists and nothing above it does.
                Ok(active) if active == port => {
                    found = Some(port);
                    break;
                }
                // Ignored: the switch does not have this port.
                Ok(_) => {}
                Err(OpError::Client(ClientError::NoReply { .. })) => {
                    tracing::debug!(
                        target: "tesmart::op",
                        candidate,
                        "no reply while probing; stopping"
                    );
                    break;
                }
                Err(other) => return Err(other),
            }
        }

        // Put the user's input back, whatever happened.
        self.ops.session.exchange(Request::Switch(original)).await?;
        tokio::time::sleep(settle).await;
        let restored = matches!(self.read_port_unchecked().await, Ok(port) if port == original);

        Ok(PortCountProbe { found, claimed: self.ops.spec.ports, original, restored })
    }

    /// Reads the port without the capability check, for internal use by `switch`.
    async fn read_active_port(&mut self) -> Result<Port, OpError> {
        let port = self.read_port_unchecked().await?;
        self.ops.check_port(port)
    }

    /// Reads the port without checking it against the switch's claimed port count.
    async fn read_port_unchecked(&mut self) -> Result<Port, OpError> {
        match self.ops.session.exchange(Request::ReadActivePort).await? {
            Response::ActivePort(port) => Ok(port),
            other => Err(OpError::UnexpectedResponse {
                kind: other.kind(),
                asked: RequestKind::ReadActivePort,
            }),
        }
    }

    /// Decides whether to verify a switch, and why not if not.
    fn plan_verification(&self, verify: Verify) -> Result<Option<Unverified>, OpError> {
        let support = self.ops.spec.support(RequestKind::ReadActivePort);
        match verify {
            Verify::Never => Ok(Some(Unverified::Disabled)),
            Verify::Auto if support.is_available() => Ok(None),
            Verify::Auto => Ok(Some(Unverified::ModelCannotRead { support })),
            Verify::Always if support.is_worth_trying() => Ok(None),
            // Asked to verify on a switch that provably cannot: say so rather than pretend.
            Verify::Always => Err(self.ops.unsupported(RequestKind::ReadActivePort)),
        }
    }
}

/// Builds a [`Kvm`].
///
/// The family and port count have to come from somewhere: a registry entry, or explicitly. There
/// is deliberately no default, because the same opcode means different things in different
/// families and guessing from a model name is how you change the wrong setting.
pub struct KvmBuilder<C: Connector> {
    connector: C,
    model: Option<&'static ModelInfo>,
    family: Option<Family>,
    ports: Option<PortCount>,
    overrides: Vec<(RequestKind, Support)>,
    protocol: Policy,
    connection: Option<ConnectionPolicy>,
    operation_timeout: Option<Duration>,
    defaults: SwitchOptions,
}

impl<C: Connector> KvmBuilder<C> {
    fn new(connector: C) -> Self {
        Self {
            connector,
            model: None,
            family: None,
            ports: None,
            overrides: Vec::new(),
            protocol: Policy::DEFAULT,
            connection: None,
            operation_timeout: None,
            defaults: SwitchOptions::default(),
        }
    }

    /// Takes family, ports and capabilities from a registry entry.
    #[must_use]
    pub fn model(mut self, model: &'static ModelInfo) -> Self {
        self.model = Some(model);
        self
    }

    /// Sets the protocol family, overriding the model's.
    #[must_use]
    pub fn family(mut self, family: Family) -> Self {
        self.family = Some(family);
        self
    }

    /// Sets the port count, overriding the model's.
    #[must_use]
    pub fn ports(mut self, ports: PortCount) -> Self {
        self.ports = Some(ports);
        self
    }

    /// Records something you have tested on your own unit, such as reading the port on a P23.
    #[must_use]
    pub fn assume(mut self, kind: RequestKind, support: Support) -> Self {
        self.overrides.push((kind, support));
        self
    }

    /// Sets the protocol timings and ASCII dialect.
    #[must_use]
    pub fn protocol_policy(mut self, policy: Policy) -> Self {
        self.protocol = policy;
        self
    }

    /// Sets when connections are opened and closed.
    ///
    /// Without this, the transport's own [preference](Connector::preferred_policy) is used:
    /// per command over TCP, persistent over a serial line.
    #[must_use]
    pub fn connection_policy(mut self, policy: ConnectionPolicy) -> Self {
        self.connection = Some(policy);
        self
    }

    /// Sets how long a single operation may take.
    #[must_use]
    pub fn operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = Some(timeout);
        self
    }

    /// Sets the options [`Kvm::switch`] uses.
    #[must_use]
    pub fn switch_defaults(mut self, options: SwitchOptions) -> Self {
        self.defaults = options;
        self
    }

    /// Builds the handle.
    ///
    /// # Errors
    ///
    /// [`BuildError::NoFamily`] or [`BuildError::NoPortCount`] if neither a model nor explicit
    /// values were given, and [`BuildError::WrongFamily`] for an HDMI matrix, which needs
    /// [`Matrix`](crate::Matrix) instead.
    pub fn build(self) -> Result<Kvm<C>, BuildError> {
        let Parts { mut spec } =
            Parts::resolve(self.model, self.family, self.ports, None, Wanted::Kvm)?;
        for (kind, support) in self.overrides {
            spec = spec.with_capability(kind, support);
        }

        let mut session = Session::with_policy(self.connector, spec.family, self.protocol);
        // Only override what the transport asked for if the caller said something.
        if let Some(policy) = self.connection {
            session.set_connection_policy(policy);
        }
        if let Some(timeout) = self.operation_timeout {
            session.set_operation_timeout(timeout);
        }

        Ok(Kvm { ops: Ops::new(session, spec), defaults: self.defaults })
    }
}

/// The handle could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BuildError {
    /// No protocol family was given, and none could be inferred.
    ///
    /// Not inferred from a model name on purpose: `0x03` sets the display timeout on a
    /// single-monitor switch and steps output A's input on a dual-monitor one.
    #[error("no protocol family: give a model from the registry, or a family explicitly")]
    NoFamily,

    /// No port count was given, and none could be inferred.
    #[error("no port count: give a model from the registry, or a port count explicitly")]
    NoPortCount,

    /// The model speaks a family this operation type cannot drive.
    ///
    /// Matrices and KVM switches share a default address and nothing else, so each has its own
    /// operation type: [`Kvm`] and [`Matrix`](crate::Matrix).
    #[error("expected {expected}, but this model speaks the {actual} protocol")]
    WrongFamily {
        /// What the operation type drives.
        expected: &'static str,
        /// What the model actually speaks.
        actual: Family,
    },
}

impl<C: Connector> core::fmt::Debug for Kvm<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Kvm")
            .field("spec", &self.ops.spec)
            .field("session", &self.ops.session)
            .finish_non_exhaustive()
    }
}

impl<C: Connector> core::fmt::Debug for KvmBuilder<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KvmBuilder")
            .field("switch", &format_args!("{}", self.connector))
            .field("model", &self.model.map(|model| model.key))
            .field("family", &self.family)
            .field("ports", &self.ports)
            .field("overrides", &self.overrides)
            .finish_non_exhaustive()
    }
}
