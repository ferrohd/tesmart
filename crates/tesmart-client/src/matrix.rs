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

/// An HDMI matrix, with operations gated on what the model documents.
pub struct Matrix<C: Connector> {
    ops: Ops<C>,
    defaults: RouteOptions,
}

impl<C: Connector> Matrix<C> {
    /// Starts building a matrix handle.
    pub fn builder(connector: C) -> MatrixBuilder<C> {
        MatrixBuilder::new(connector)
    }

    /// A handle to a catalogued matrix.
    ///
    /// # Errors
    ///
    /// [`BuildError::WrongFamily`] if the model is not a matrix.
    pub fn from_model(connector: C, model: &'static ModelInfo) -> Result<Self, BuildError> {
        MatrixBuilder::new(connector).model(model).build()
    }

    /// What the matrix is believed to be.
    pub fn spec(&self) -> &ModelSpec {
        &self.ops.spec
    }

    /// How many inputs it has.
    pub fn inputs(&self) -> PortCount {
        self.ops.spec.ports
    }

    /// How many outputs it has.
    pub fn outputs(&self) -> PortCount {
        self.ops.spec.outputs
    }

    /// The underlying session, for raw exchanges and statistics.
    pub fn session(&self) -> &Session<C, Family> {
        &self.ops.session
    }

    /// The underlying session.
    pub fn session_mut(&mut self) -> &mut Session<C, Family> {
        &mut self.ops.session
    }

    /// Reads which input feeds each output.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] if the model is not believed to answer `MT00RD0000NT`, or the
    /// underlying exchange's failure, including [`ClientError::NoReply`] when the matrix stays
    /// silent.
    #[tracing::instrument(level = "debug", skip(self), fields(matrix = %self.ops.session))]
    pub async fn routing(&mut self) -> Result<RoutingReport, OpError> {
        self.ops.require(RequestKind::MatrixReadRouting)?;
        self.read_routing().await
    }

    /// Routes one input to one output, with the default options.
    ///
    /// # Errors
    ///
    /// See [`route_with`](Self::route_with).
    pub async fn route(&mut self, input: Port, output: Port) -> Result<RouteOutcome, OpError> {
        self.route_with(input, output, self.defaults).await
    }

    /// Routes one input to one output.
    ///
    /// # Errors
    ///
    /// [`OpError::PortOutOfRange`] or [`OpError::OutputOutOfRange`] if the matrix has no such
    /// input or output, [`OpError::VerificationFailed`] if the read-back shows a different
    /// input on that output, or the underlying exchange's failure. A silent read-back is not an
    /// error: it produces [`RouteOutcome::Unverified`].
    #[tracing::instrument(
        level = "debug",
        skip(self, options),
        fields(matrix = %self.ops.session, input = input.get(), output = output.get())
    )]
    pub async fn route_with(
        &mut self,
        input: Port,
        output: Port,
        options: RouteOptions,
    ) -> Result<RouteOutcome, OpError> {
        let input = self.ops.check_port(input)?;
        let output = self.ops.check_output(output)?;

        self.ops.require(RequestKind::MatrixRoute)?;
        self.ops.session.exchange(Request::MatrixRoute { input, output }).await?;

        self.confirm(options, |routing| match routing.input_for(output) {
            Some(found) if found == input => Ok(()),
            // The matrix answered, and it disagrees. Fail loudly, as a KVM does.
            Some(found) => Err(OpError::VerificationFailed { expected: input, found }),
            None => Ok(()),
        })
        .await
    }

    /// Routes one input to every output.
    ///
    /// # Errors
    ///
    /// As [`route_with`](Self::route_with), except that every output is checked.
    #[tracing::instrument(level = "debug", skip(self), fields(matrix = %self.ops.session, input = input.get()))]
    pub async fn route_all(&mut self, input: Port) -> Result<RouteOutcome, OpError> {
        let input = self.ops.check_port(input)?;
        self.ops.require(RequestKind::MatrixRouteAll)?;
        self.ops.session.exchange(Request::MatrixRouteAll(input)).await?;

        let options = self.defaults;
        self.confirm(options, |routing| match routing.links.iter().find(|(_, fed)| *fed != input) {
            None => Ok(()),
            Some((_, found)) => Err(OpError::VerificationFailed { expected: input, found: *found }),
        })
        .await
    }

    /// Mirrors input 1 to output 1, 2 to 2, and so on.
    ///
    /// # Errors
    ///
    /// As [`route_with`](Self::route_with).
    #[tracing::instrument(level = "debug", skip(self), fields(matrix = %self.ops.session))]
    pub async fn mirror(&mut self) -> Result<RouteOutcome, OpError> {
        self.ops.require(RequestKind::MatrixMirror)?;
        self.ops.session.exchange(Request::MatrixMirror).await?;

        let options = self.defaults;
        self.confirm(options, |routing| {
            match routing.links.iter().find(|(output, input)| output != input) {
                None => Ok(()),
                Some((output, input)) => {
                    Err(OpError::VerificationFailed { expected: *output, found: *input })
                }
            }
        })
        .await
    }

    /// Loads a stored routing preset.
    ///
    /// Documented for the 8x8 and 16x16 series only. The routing is read back afterwards where
    /// possible, which is the only way to see what the preset contained.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a model without presets, or the underlying exchange's
    /// failure.
    #[tracing::instrument(level = "debug", skip(self), fields(matrix = %self.ops.session, preset = preset.get()))]
    pub async fn load_preset(&mut self, preset: Preset) -> Result<PresetOutcome, OpError> {
        self.ops.require(RequestKind::MatrixLoadPreset)?;
        self.ops.session.exchange(Request::MatrixLoadPreset(preset)).await?;

        let warnings = self.ops.warnings(RequestKind::MatrixLoadPreset);
        let routing = match self.defaults.verify {
            VerifyRouting::No => None,
            VerifyRouting::Yes if !self.can_verify() => None,
            VerifyRouting::Yes => {
                tokio::time::sleep(self.defaults.settle).await;
                self.read_routing().await.ok()
            }
        };

        Ok(PresetOutcome { preset, routing, warnings })
    }

    /// Saves the current routing into a preset.
    ///
    /// Write only: nothing reports what a preset holds, so this cannot be confirmed. Documented
    /// for the 8x8 and 16x16 series only.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a model without presets.
    pub async fn save_preset(&mut self, preset: Preset) -> Result<Sent, OpError> {
        self.ops.fire(Request::MatrixSavePreset(preset)).await
    }

    /// Sets how long the front display stays lit.
    ///
    /// Documented for the 4x4 series only; the 8x8 and 16x16 documents have no such command.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a model without the command.
    pub async fn set_display_timeout(&mut self, timeout: MatrixTimeout) -> Result<Sent, OpError> {
        self.ops.fire(Request::MatrixDisplayTimeout(timeout)).await
    }

    /// Mutes or unmutes the buzzer.
    ///
    /// Documented for the 8x8 and 16x16 series only. The wire values are inverted relative to
    /// the KVM buzzer command, which the profile handles; [`Buzzer::Muted`] means muted here as
    /// everywhere else.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a model without the command.
    pub async fn set_buzzer(&mut self, buzzer: Buzzer) -> Result<Sent, OpError> {
        self.ops.fire(Request::MatrixBuzzer(buzzer)).await
    }

    /// Reads all four network settings.
    ///
    /// # Errors
    ///
    /// [`OpError::Unsupported`] on a matrix with no LAN port, such as the RS232-only 4x4
    /// models, or a transport failure.
    #[tracing::instrument(level = "debug", skip(self), fields(matrix = %self.ops.session))]
    pub async fn network(&mut self) -> Result<NetReport, OpError> {
        self.ops.network().await
    }

    /// Writes network settings, then reads them back.
    ///
    /// Unlike the KVM case, this is documented over both transports: a matrix takes the bare
    /// strings over LAN and the `MT80xx` wrapped ones over RS232, and the session picks the
    /// right spelling from the transport. The power-cycle and no-factory-reset warnings still
    /// apply.
    ///
    /// # Errors
    ///
    /// As [`network`](Self::network); a field that reads back wrong lands in
    /// [`NetOutcome::mismatched`] rather than failing.
    #[tracing::instrument(level = "debug", skip(self, changes, confirmed), fields(matrix = %self.ops.session))]
    pub async fn set_network(
        &mut self,
        changes: &[NetSetting],
        confirmed: Confirmed,
    ) -> Result<NetOutcome, OpError> {
        self.ops.set_network(changes, confirmed).await
    }

    /// Reads the routing without the capability check.
    async fn read_routing(&mut self) -> Result<RoutingReport, OpError> {
        match self.ops.session.exchange(Request::MatrixReadRouting).await? {
            Response::Routing(reply) => {
                Ok(RoutingReport::new(&reply, self.ops.warnings(RequestKind::MatrixReadRouting)))
            }
            other => Err(OpError::UnexpectedResponse {
                kind: other.kind(),
                asked: RequestKind::MatrixReadRouting,
            }),
        }
    }

    /// Reads the routing back and checks it, where that is possible and wanted.
    async fn confirm(
        &mut self,
        options: RouteOptions,
        check: impl FnOnce(&RoutingReport) -> Result<(), OpError>,
    ) -> Result<RouteOutcome, OpError> {
        let reason = match options.verify {
            VerifyRouting::No => Some(Unverified::Disabled),
            VerifyRouting::Yes if !self.can_verify() => Some(Unverified::ModelCannotRead {
                support: self.ops.spec.support(RequestKind::MatrixReadRouting),
            }),
            VerifyRouting::Yes => None,
        };
        if let Some(reason) = reason {
            return Ok(RouteOutcome::Unverified {
                reason,
                warnings: self.ops.warnings(RequestKind::MatrixRoute),
            });
        }

        tokio::time::sleep(options.settle).await;
        match self.read_routing().await {
            Ok(routing) => {
                check(&routing)?;
                Ok(RouteOutcome::Verified { routing })
            }
            Err(OpError::Client(ClientError::NoReply { attempts, .. })) => {
                Ok(RouteOutcome::Unverified {
                    reason: Unverified::NoReply { attempts },
                    warnings: self.ops.warnings(RequestKind::MatrixReadRouting),
                })
            }
            Err(other) => Err(other),
        }
    }

    fn can_verify(&self) -> bool {
        self.ops.spec.support(RequestKind::MatrixReadRouting).is_available()
    }
}

/// Builds a [`Matrix`].
pub struct MatrixBuilder<C: Connector> {
    connector: C,
    model: Option<&'static ModelInfo>,
    inputs: Option<PortCount>,
    outputs: Option<PortCount>,
    overrides: Vec<(RequestKind, Support)>,
    protocol: Policy,
    connection: Option<ConnectionPolicy>,
    operation_timeout: Option<Duration>,
    defaults: RouteOptions,
}

impl<C: Connector> MatrixBuilder<C> {
    fn new(connector: C) -> Self {
        Self {
            connector,
            model: None,
            inputs: None,
            outputs: None,
            overrides: Vec::new(),
            protocol: Policy::DEFAULT,
            connection: None,
            operation_timeout: None,
            defaults: RouteOptions::default(),
        }
    }

    /// Takes the port counts and capabilities from a registry entry.
    #[must_use]
    pub fn model(mut self, model: &'static ModelInfo) -> Self {
        self.model = Some(model);
        self
    }

    /// Sets the input count, overriding the model's.
    #[must_use]
    pub fn inputs(mut self, inputs: PortCount) -> Self {
        self.inputs = Some(inputs);
        self
    }

    /// Sets the output count, overriding the model's.
    #[must_use]
    pub fn outputs(mut self, outputs: PortCount) -> Self {
        self.outputs = Some(outputs);
        self
    }

    /// Records something you have tested on your own unit.
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

    /// Sets the options [`Matrix::route`] uses.
    #[must_use]
    pub fn route_defaults(mut self, options: RouteOptions) -> Self {
        self.defaults = options;
        self
    }

    /// Builds the handle.
    ///
    /// # Errors
    ///
    /// [`BuildError::NoPortCount`] if neither a model nor an input count was given, and
    /// [`BuildError::WrongFamily`] if the model is not a matrix.
    pub fn build(self) -> Result<Matrix<C>, BuildError> {
        // The model's own family wins, so pointing this at a KVM entry is refused rather than
        // overridden. Without a model there is only one family this builder can mean.
        let family = self.model.map_or(Family::Matrix, |model| model.family);
        let Parts { mut spec } =
            Parts::resolve(self.model, Some(family), self.inputs, self.outputs, Wanted::Matrix)?;
        for (kind, support) in self.overrides {
            spec = spec.with_capability(kind, support);
        }

        let mut session = Session::with_policy(self.connector, spec.family, self.protocol);
        if let Some(policy) = self.connection {
            session.set_connection_policy(policy);
        }
        if let Some(timeout) = self.operation_timeout {
            session.set_operation_timeout(timeout);
        }

        Ok(Matrix { ops: Ops::new(session, spec), defaults: self.defaults })
    }
}

impl<C: Connector> core::fmt::Debug for Matrix<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Matrix")
            .field("spec", &self.ops.spec)
            .field("session", &self.ops.session)
            .finish_non_exhaustive()
    }
}

impl<C: Connector> core::fmt::Debug for MatrixBuilder<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MatrixBuilder")
            .field("matrix", &format_args!("{}", self.connector))
            .field("model", &self.model.map(|model| model.key))
            .field("inputs", &self.inputs)
            .field("outputs", &self.outputs)
            .finish_non_exhaustive()
    }
}
