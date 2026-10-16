//! What a command produced, in a shape this tool owns.
//!
//! These are deliberately *not* the library's outcome types. `--output json` is a contract with
//! whoever is scripting this tool, so it is defined here: a rename inside `tesmart-client`
//! then breaks this file's mapping at compile time instead of breaking somebody's `jq` in
//! silence.

use serde::Serialize;
use tesmart::client::{
    NetOutcome, NetReport, PortChange, PortCountProbe, PresetOutcome, RouteOutcome, RoutingReport,
    Sent, SwitchOutcome, Unverified, Warning,
};
use tesmart::proto::model::ModelInfo;
use tesmart::proto::value::{NetConfig, NetField};
use tesmart::proto::{Port, Profile as _, RequestKind, Response};

/// Everything a command can produce.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Report {
    /// A switch attempt.
    Switch(SwitchView),
    /// The active port.
    Port(PortView),
    /// A command the protocol defines no reply to.
    Sent(SentView),
    /// A matrix's routing, as read.
    Routing(RoutingView),
    /// A matrix routing change.
    Route(RouteView),
    /// A matrix preset that was loaded.
    Preset(PresetView),
    /// The network settings, as read.
    Net(NetView),
    /// The network settings, after writing.
    NetSet(NetSetView),
    /// What the unverified capabilities actually did.
    Probe(ProbeView),
    /// The answer to raw bytes.
    Raw(RawView),
    /// The registry.
    Models(Vec<ModelSummaryView>),
    /// One registry entry in full.
    Model(Box<ModelDetailView>),
    /// This machine's serial ports.
    Ports(Vec<SerialPortView>),
    /// A command whose output is the side effect: generated files, a finished watch.
    Done(DoneView),
}

impl Report {
    /// Caveats that belong with the result, hoisted into the envelope.
    pub fn warnings(&self) -> &[String] {
        match self {
            Self::Switch(view) => &view.warnings,
            Self::Sent(view) => &view.warnings,
            Self::Routing(view) => &view.warnings,
            Self::Route(view) => &view.warnings,
            Self::Preset(view) => &view.warnings,
            Self::Net(view) => &view.warnings,
            Self::NetSet(view) => &view.warnings,
            // Nothing else carries caveats: they are reports about the tool, not the switch.
            _ => &[],
        }
    }

    /// Whether the switch confirmed what happened.
    ///
    /// `None` means confirmation does not apply to this command.
    pub fn confirmed(&self) -> Option<bool> {
        match self {
            Self::Switch(view) => Some(view.verified),
            Self::Route(view) => Some(view.verified),
            Self::NetSet(view) => Some(view.consistent),
            _ => None,
        }
    }

    /// Why it could not be confirmed, for `--require-verified`.
    pub fn unverified_reason(&self) -> Option<&str> {
        match self {
            Self::Switch(view) => view.reason.as_deref(),
            Self::Route(view) => view.reason.as_deref(),
            Self::NetSet(view) if !view.consistent => Some("a value did not read back"),
            _ => None,
        }
    }
}

/// A switch attempt.
#[derive(Debug, Serialize)]
pub struct SwitchView {
    /// The port that was asked for.
    pub port: u8,
    /// Whether the switch confirmed it.
    pub verified: bool,
    /// Whether the switch was already there and nothing was sent.
    pub already_active: bool,
    /// How many read-backs it took.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u8>,
    /// Why it could not be confirmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl From<SwitchOutcome> for SwitchView {
    fn from(outcome: SwitchOutcome) -> Self {
        match outcome {
            SwitchOutcome::AlreadyActive(port) => Self {
                port: port.get(),
                verified: true,
                already_active: true,
                attempts: None,
                reason: None,
                warnings: Vec::new(),
            },
            SwitchOutcome::Verified { port, attempts } => Self {
                port: port.get(),
                verified: true,
                already_active: false,
                attempts: Some(attempts),
                reason: None,
                warnings: Vec::new(),
            },
            SwitchOutcome::Unverified { port, reason, warnings } => Self {
                port: port.get(),
                verified: false,
                already_active: false,
                attempts: None,
                reason: Some(describe_unverified(reason)),
                warnings: warnings_to_strings(&warnings),
            },
            // `SwitchOutcome` is non-exhaustive: anything new is reported as unconfirmed until
            // this mapping learns about it, which is the safe direction to be wrong in.
            other => Self {
                port: other.port().get(),
                verified: other.is_confirmed(),
                already_active: false,
                attempts: None,
                reason: (!other.is_confirmed()).then(|| "not verified".to_owned()),
                warnings: Vec::new(),
            },
        }
    }
}

fn describe_unverified(reason: Unverified) -> String {
    reason.to_string()
}

/// The active port.
#[derive(Debug, Serialize)]
pub struct PortView {
    /// The port, as the front panel counts them.
    pub port: u8,
}

impl From<Port> for PortView {
    fn from(port: Port) -> Self {
        Self { port: port.get() }
    }
}

/// A command the protocol defines no reply to.
#[derive(Debug, Serialize)]
pub struct SentView {
    /// What was sent.
    pub command: &'static str,
    /// What the registry says about it on this model.
    pub support: &'static str,
    /// Always false: these commands cannot be read back on any series.
    pub verifiable: bool,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl From<Sent> for SentView {
    fn from(sent: Sent) -> Self {
        Self {
            command: sent.kind.name(),
            support: sent.support.name(),
            verifiable: false,
            warnings: warnings_to_strings(&sent.warnings),
        }
    }
}

/// A matrix's routing, as read.
#[derive(Debug, Serialize)]
pub struct RoutingView {
    /// One entry per output the matrix reported.
    pub links: Vec<LinkView>,
    /// Whether every output is fed by the input with the same number.
    pub mirrored: bool,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// One output and the input feeding it.
#[derive(Debug, Serialize)]
pub struct LinkView {
    /// The output.
    pub output: u8,
    /// The input feeding it.
    pub input: u8,
}

impl From<&RoutingReport> for RoutingView {
    fn from(report: &RoutingReport) -> Self {
        Self {
            links: report
                .links
                .iter()
                .map(|(output, input)| LinkView { output: output.get(), input: input.get() })
                .collect(),
            mirrored: report.is_mirrored(),
            warnings: warnings_to_strings(&report.warnings),
        }
    }
}

/// A matrix routing change.
///
/// Unlike [`SwitchView`], `verified` here is usually true: a matrix reports its routing, so a
/// route can actually be checked.
#[derive(Debug, Serialize)]
pub struct RouteView {
    /// Whether the matrix confirmed the change.
    pub verified: bool,
    /// The routing afterwards, when the matrix reported it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingView>,
    /// Why it could not be confirmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl From<RouteOutcome> for RouteView {
    fn from(outcome: RouteOutcome) -> Self {
        match outcome {
            RouteOutcome::Verified { routing } => Self {
                verified: true,
                routing: Some(RoutingView::from(&routing)),
                reason: None,
                warnings: Vec::new(),
            },
            RouteOutcome::Unverified { reason, warnings } => Self {
                verified: false,
                routing: None,
                reason: Some(reason.to_string()),
                warnings: warnings_to_strings(&warnings),
            },
            // `RouteOutcome` is non-exhaustive: anything new reads as unconfirmed until this
            // mapping learns about it.
            other => Self {
                verified: other.is_confirmed(),
                routing: other.routing().map(RoutingView::from),
                reason: (!other.is_confirmed()).then(|| "not verified".to_owned()),
                warnings: Vec::new(),
            },
        }
    }
}

/// A matrix preset that was loaded.
#[derive(Debug, Serialize)]
pub struct PresetView {
    /// Which slot.
    pub preset: u8,
    /// The routing the preset applied, when the matrix reported it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingView>,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl From<PresetOutcome> for PresetView {
    fn from(outcome: PresetOutcome) -> Self {
        Self {
            preset: outcome.preset.get(),
            routing: outcome.routing.as_ref().map(RoutingView::from),
            warnings: warnings_to_strings(&outcome.warnings),
        }
    }
}

/// The network settings, as read.
#[derive(Debug, Serialize)]
pub struct NetView {
    /// IP address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// TCP port.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Default gateway.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    /// Subnet mask.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    /// Fields the switch did not answer for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<&'static str>,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl From<NetReport> for NetView {
    fn from(report: NetReport) -> Self {
        let mut view = Self::from(&report.config);
        view.missing = report.missing.iter().copied().map(field_name).collect();
        view.warnings = warnings_to_strings(&report.warnings);
        view
    }
}

impl From<&NetConfig> for NetView {
    fn from(config: &NetConfig) -> Self {
        Self {
            ip: config.ip.map(|ip| ip.to_string()),
            port: config.port,
            gateway: config.gateway.map(|ip| ip.to_string()),
            mask: config.mask.map(|ip| ip.to_string()),
            missing: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// The network settings, after writing.
#[derive(Debug, Serialize)]
pub struct NetSetView {
    /// What the switch reports now.
    pub applied: NetView,
    /// Fields that did not read back as written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mismatched: Vec<&'static str>,
    /// Whether every written field read back as written.
    pub consistent: bool,
    /// Always true after a successful write.
    pub power_cycle_required: bool,
    /// Caveats.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl From<NetOutcome> for NetSetView {
    fn from(outcome: NetOutcome) -> Self {
        Self {
            applied: NetView::from(&outcome.applied),
            mismatched: outcome.mismatched.iter().copied().map(field_name).collect(),
            consistent: outcome.mismatched.is_empty(),
            power_cycle_required: true,
            warnings: warnings_to_strings(&outcome.warnings),
        }
    }
}

/// What the unverified capabilities actually did.
#[derive(Debug, Serialize)]
pub struct ProbeView {
    /// One row per capability that was worth trying.
    pub capabilities: Vec<ProbedCapability>,
    /// What trying every input found, when `--ports` asked for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ports: Option<PortProbeView>,
}

/// What trying every input discovered about how many there are.
#[derive(Debug, Serialize)]
pub struct PortProbeView {
    /// The highest input the switch accepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<u8>,
    /// How many the registry claimed.
    pub claimed: u8,
    /// Whether the two agree.
    pub agrees: bool,
    /// Whether the original input was confirmed restored.
    pub restored: bool,
    /// What happened, in words.
    pub summary: String,
}

impl From<PortCountProbe> for PortProbeView {
    fn from(probe: PortCountProbe) -> Self {
        Self {
            found: probe.found.map(Port::get),
            claimed: probe.claimed.get(),
            agrees: probe.agrees(),
            restored: probe.restored,
            summary: probe.to_string(),
        }
    }
}

/// One probed capability.
#[derive(Debug, Serialize)]
pub struct ProbedCapability {
    /// The command.
    pub command: &'static str,
    /// What the registry claims.
    pub registry: &'static str,
    /// What happened: works, no-reply, unsupported, or not-probed.
    pub probe: &'static str,
    /// What the switch said, when it said anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The flag that would record this result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// The answer to raw bytes.
#[derive(Debug, Serialize)]
pub struct RawView {
    /// What was sent, in hex.
    pub sent: String,
    /// What came back, interpreted if possible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

impl RawView {
    /// Builds a view from the bytes that were sent and whatever answered.
    pub fn new(sent: &[u8], reply: Option<&Response>) -> Self {
        Self {
            sent: tesmart::proto::wire::Hex(sent).to_string(),
            reply: reply.map(ToString::to_string),
        }
    }
}

/// One line of `models list`.
#[derive(Debug, Serialize)]
pub struct ModelSummaryView {
    /// The registry key.
    pub model: &'static str,
    /// Which protocol family it speaks.
    pub family: &'static str,
    /// How many inputs it has.
    pub ports: u8,
    /// Whether it can report its active port, and on what evidence.
    pub read_port: &'static str,
    /// Whether it has the ASCII network side-channel.
    pub net_config: &'static str,
}

impl From<&'static ModelInfo> for ModelSummaryView {
    fn from(model: &'static ModelInfo) -> Self {
        Self {
            model: model.key,
            family: model.family.name(),
            ports: model.ports.get(),
            read_port: model.support(RequestKind::ReadActivePort).name(),
            net_config: model.support(RequestKind::NetQuery).name(),
        }
    }
}

/// One registry entry in full.
#[derive(Debug, Serialize)]
pub struct ModelDetailView {
    /// The registry key.
    pub model: &'static str,
    /// Other names that resolve to it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<&'static str>,
    /// Which protocol family it speaks.
    pub family: &'static str,
    /// How many inputs it has.
    pub ports: u8,
    /// What it can do, and on what evidence.
    pub capabilities: Vec<CapabilityView>,
    /// Anything a user should know before trusting the entry.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub notes: &'static str,
}

/// One capability of one model.
#[derive(Debug, Serialize)]
pub struct CapabilityView {
    /// The command.
    pub command: &'static str,
    /// The opcode, for the commands that have one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opcode: Option<String>,
    /// What is known, and on what evidence.
    pub support: &'static str,
    /// What that means in words.
    pub evidence: &'static str,
    /// A caveat from the protocol tables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

impl ModelDetailView {
    /// Describes a registry entry, including the family's command table.
    pub fn new(model: &'static ModelInfo) -> Self {
        let capabilities = model
            .family
            .commands()
            .iter()
            .map(|entry| {
                let support = model.support(entry.kind);
                CapabilityView {
                    command: entry.kind.name(),
                    opcode: entry.opcode.map(|opcode| format!("0x{opcode:02X}")),
                    support: support.name(),
                    evidence: support.describe(),
                    note: entry.note,
                }
            })
            .collect();

        Self {
            model: model.key,
            aliases: model.aliases.to_vec(),
            family: model.family.name(),
            ports: model.ports.get(),
            capabilities,
            notes: model.notes,
        }
    }
}

/// One of this machine's serial ports.
#[derive(Debug, Serialize)]
pub struct SerialPortView {
    /// The device file.
    pub path: String,
    /// What it is attached to.
    pub kind: String,
}

/// A command whose result is its side effect.
#[derive(Debug, Serialize)]
pub struct DoneView {
    /// What happened, in words.
    pub message: String,
}

impl DoneView {
    /// A one-line report.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

/// One port change, for `watch`.
#[derive(Debug, Serialize)]
pub struct ChangeView {
    /// The port now active.
    pub port: u8,
    /// The port before, when it was known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<u8>,
    /// Whether the switch volunteered it or a poll found it.
    pub source: &'static str,
}

impl From<PortChange> for ChangeView {
    fn from(change: PortChange) -> Self {
        Self {
            port: change.port.get(),
            previous: change.previous.map(Port::get),
            source: match change.source {
                tesmart::client::ChangeSource::Reported => "reported",
                tesmart::client::ChangeSource::Polled => "polled",
            },
        }
    }
}

fn field_name(field: NetField) -> &'static str {
    match field {
        NetField::Ip => "ip",
        NetField::Port => "port",
        NetField::Gateway => "gateway",
        NetField::Mask => "mask",
    }
}

fn warnings_to_strings(warnings: &[Warning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}
