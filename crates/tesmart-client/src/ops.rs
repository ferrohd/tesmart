//! What [`Kvm`](crate::Kvm) and [`Matrix`](crate::Matrix) have in common.
//!
//! The two operation types drive different families, but they sit on the same session, consult
//! the same capability flags, and share one command: the ASCII network side-channel, which is
//! the same four strings on a KVM and on a matrix.
//!
//! This type holds that shared half. It is deliberately not public: a caller reaches it through
//! whichever operation type matches the switch, so a matrix cannot be asked for its active port
//! by going around the front door.

use tesmart_proto::model::ModelInfo;
use tesmart_proto::value::{NetConfig, NetField, NetSetting};
use tesmart_proto::{Family, Port, Request, RequestKind, Response, Support};

use crate::error::{ClientError, OpError};
use crate::outcome::{Confirmed, NetOutcome, NetReport, Sent, Warning};
use crate::session::Session;
use crate::spec::ModelSpec;
use crate::transport::Connector;

/// A session plus what the switch on the other end is believed to be.
pub(crate) struct Ops<C: Connector> {
    pub(crate) session: Session<C, Family>,
    pub(crate) spec: ModelSpec,
}

impl<C: Connector> Ops<C> {
    pub(crate) const fn new(session: Session<C, Family>, spec: ModelSpec) -> Self {
        Self { session, spec }
    }

    /// Reads all four network settings.
    ///
    /// A field the switch does not answer for lands in [`NetReport::missing`] rather than
    /// failing the whole call: the other three are still worth having.
    pub(crate) async fn network(&mut self) -> Result<NetReport, OpError> {
        self.require(RequestKind::NetQuery)?;

        let mut config = NetConfig::default();
        let mut missing = Vec::new();

        for field in NetField::ALL {
            match self.session.exchange(Request::NetQuery(field)).await {
                Ok(Response::Net(setting)) => config.set(setting),
                Ok(other) => {
                    return Err(OpError::UnexpectedResponse {
                        kind: other.kind(),
                        asked: RequestKind::NetQuery,
                    });
                }
                Err(ClientError::NoReply { .. }) => {
                    tracing::debug!(target: "tesmart::op", %field, "no answer");
                    missing.push(field);
                }
                Err(other) => return Err(other.into()),
            }
        }

        Ok(NetReport { config, missing, warnings: self.net_warnings(RequestKind::NetQuery) })
    }

    /// Writes network settings, then reads them back.
    pub(crate) async fn set_network(
        &mut self,
        changes: &[NetSetting],
        _confirmed: Confirmed,
    ) -> Result<NetOutcome, OpError> {
        self.require(RequestKind::NetSet)?;

        let mut warnings = self.net_warnings(RequestKind::NetSet);
        warnings.push(Warning::NoFactoryReset);
        warnings.push(Warning::PowerCycleRequired);

        for setting in changes {
            tracing::info!(
                target: "tesmart::op",
                field = %setting.field(),
                value = %setting,
                "writing"
            );
            match self.session.exchange(Request::NetSet(*setting)).await? {
                Response::Ack => {}
                other => {
                    return Err(OpError::UnexpectedResponse {
                        kind: other.kind(),
                        asked: RequestKind::NetSet,
                    });
                }
            }
        }

        // Read everything back before telling the user to power cycle: this is the last chance
        // to notice a typo while the old address still works.
        let report = self.network().await?;
        let mut mismatched = Vec::new();
        for setting in changes {
            let field = setting.field();
            if report.config.get(field) != Some(*setting) {
                mismatched.push(field);
            }
        }

        Ok(NetOutcome { requested: changes.to_vec(), applied: report.config, mismatched, warnings })
    }

    /// Sends a command the protocol defines no reply to.
    pub(crate) async fn fire(&mut self, request: Request) -> Result<Sent, OpError> {
        let kind = request.kind();
        self.require(kind)?;
        self.session.exchange(request).await?;
        Ok(Sent { kind, support: self.spec.support(kind), warnings: self.warnings(kind) })
    }

    /// Refuses a command this switch is known not to have.
    pub(crate) fn require(&self, kind: RequestKind) -> Result<(), OpError> {
        if self.spec.may_attempt(kind) { Ok(()) } else { Err(self.unsupported(kind)) }
    }

    pub(crate) fn unsupported(&self, kind: RequestKind) -> OpError {
        OpError::Unsupported { kind, support: self.spec.support(kind), model: self.spec.name }
    }

    /// Checks an input against the switch's input count.
    pub(crate) fn check_port(&self, port: Port) -> Result<Port, OpError> {
        self.spec
            .ports
            .check(port)
            .map_err(|_| OpError::PortOutOfRange { port, ports: self.spec.ports })
    }

    /// Checks an output against the switch's output count.
    pub(crate) fn check_output(&self, output: Port) -> Result<Port, OpError> {
        self.spec
            .outputs
            .check(output)
            .map_err(|_| OpError::OutputOutOfRange { output, outputs: self.spec.outputs })
    }

    /// Caveats worth passing to the user alongside a successful result.
    pub(crate) fn warnings(&self, kind: RequestKind) -> Vec<Warning> {
        let mut warnings = Vec::new();
        if self.spec.support(kind) == Support::Unverified {
            warnings.push(Warning::CapabilityUnverified { kind, support: self.spec.support(kind) });
        }
        warnings
    }

    /// As [`warnings`](Self::warnings), plus the transport caveat for the ASCII side-channel.
    ///
    /// The caveat is a KVM one. TESmart documents the matrix network commands for RS232
    /// explicitly, in their own wrapped spelling, so there is nothing disputed about using them
    /// over a serial line.
    pub(crate) fn net_warnings(&self, kind: RequestKind) -> Vec<Warning> {
        let mut warnings = self.warnings(kind);
        if self.spec.family != Family::Matrix
            && !self.session.transport_kind().net_config_is_documented()
        {
            warnings.push(Warning::NetConfigOverSerial);
        }
        warnings
    }
}

/// Which operation type is being built, and therefore which families it accepts.
///
/// One value rather than a predicate plus a label, so the test and the error message it
/// produces cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wanted {
    /// [`Kvm`](crate::Kvm): anything that is not a matrix.
    Kvm,
    /// [`Matrix`](crate::Matrix): matrices only.
    Matrix,
}

impl Wanted {
    const fn accepts(self, family: Family) -> bool {
        match self {
            Self::Kvm => !matches!(family, Family::Matrix),
            Self::Matrix => matches!(family, Family::Matrix),
        }
    }

    /// What [`BuildError::WrongFamily`](crate::BuildError::WrongFamily) reports as expected.
    const fn describe(self) -> &'static str {
        match self {
            Self::Kvm => "a KVM switch",
            Self::Matrix => "an HDMI matrix",
        }
    }
}
