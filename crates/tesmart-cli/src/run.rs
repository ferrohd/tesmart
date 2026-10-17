//! Running a command against a switch.
//!
//! The transport is resolved once, into [`crate::target::AnyConnector`], so everything below
//! this point is generic over one concrete type rather than monomorphised per transport.

use tesmart::client::{
    Confirmed, Connector, ErrorClass, Kvm, OpError, SwitchOptions, Verify, WatchOptions,
};
use tesmart::proto::value::{MatrixTimeout, NetSetting, Preset, SwitchInterval};
use tesmart::proto::{Port, Protocol, RequestKind, Support, model};

use crate::cli::{
    Cli, Command, DualCommand, MatrixCommand, ModelsCommand, NetCommand, NetSetArgs, PresetCommand,
    RawArgs, StepArg, SwitchArgs, WatchArgs,
};
use crate::error::CliError;
use crate::generate;
use crate::handle;
use crate::render::{Context, Printer};
use crate::report::{
    ChangeView, DoneView, ModelDetailView, ModelSummaryView, PortProbeView, PortView, PresetView,
    ProbeView, ProbedCapability, RawView, Report, RouteView, RoutingView, SwitchView,
};

/// Runs whatever the command line asked for.
///
/// Returns the context alongside the result so a failure can be reported against the same
/// target and model as a success would have been.
pub async fn execute(cli: &Cli, printer: &Printer) -> (Context, Result<Report, CliError>) {
    let name = cli.command.name();

    if !cli.command.needs_a_switch() {
        return (Context::local(name), local(cli, printer).await);
    }

    let options = handle::target_options(&cli.global);
    let connector = match crate::target::resolve_with(&cli.global.target, options) {
        Ok(connector) => connector,
        Err(error) => {
            // Report against what the user typed, since nothing was resolved.
            let context = Context {
                command: name,
                target: Some(cli.global.target.clone()),
                model: cli.global.model.clone(),
            };
            return (context, Err(CliError::usage(error.to_string())));
        }
    };

    let context = Context {
        command: name,
        target: Some(connector.to_string()),
        model: cli.global.model.clone(),
    };

    let result = with_transport(connector, cli, printer, &context).await;
    (context, result)
}

/// Everything that needs a switch, for one transport.
async fn with_transport<C: Connector>(
    connector: C,
    cli: &Cli,
    printer: &Printer,
    context: &Context,
) -> Result<Report, CliError> {
    // A matrix is a different protocol with a different handle, so it forks before anything is
    // built: a `Kvm` cannot route and a `Matrix` has no active port.
    if let Command::Matrix(command) = &cli.command {
        return matrix(connector, cli, command).await;
    }

    // The ASCII network channel is the one thing both protocols have, and TESmart documents it
    // for matrices over both transports, so `net` works against either handle.
    if let Command::Net(command) = &cli.command
        && handle::is_matrix(&cli.global)?
    {
        let mut matrix = handle::build_matrix(connector, &cli.global)?;
        return net(&mut matrix, command, printer, context).await;
    }

    // `raw` is the one command that does not consult the opcode tables, so it is also the one
    // that does not need a model: the protocol only decides how a reply is read back.
    let protocol_default = matches!(cli.command, Command::Raw(_)).then_some(Protocol::Kvm);
    let mut kvm = handle::build(connector, &cli.global, protocol_default)?;

    match &cli.command {
        Command::Switch(args) => switch(&mut kvm, args).await,
        Command::Get => Ok(Report::Port(PortView::from(kvm.active_port().await?))),
        Command::Next(args) => step(&mut kvm, StepArg::Next, args.monitor).await,
        Command::Prev(args) => step(&mut kvm, StepArg::Prev, args.monitor).await,

        Command::Buzzer { state } => Ok(sent(kvm.set_buzzer((*state).into()).await?)),
        Command::Timeout { value } => Ok(sent(kvm.set_display_timeout((*value).into()).await?)),
        Command::Autodetect { state } => {
            Ok(sent(kvm.set_auto_input_detect((*state).into()).await?))
        }

        Command::Dual(command) => dual(&mut kvm, command).await,

        Command::Net(command) => net(&mut kvm, command, printer, context).await,

        Command::Watch(args) => watch(&mut kvm, args, printer, context).await,
        Command::Probe(args) => probe(&mut kvm, args, printer).await,
        Command::Raw(args) => raw(&mut kvm, args).await,

        // Forked above, before the handle was built.
        Command::Matrix(_) => Err(CliError::usage("matrix commands need a matrix handle")),

        // Handled before a transport is ever opened.
        Command::Models(_)
        | Command::Completions { .. }
        | Command::Manpage { .. }
        | Command::Ports
        | Command::Sim(_) => Err(CliError::usage("this command does not talk to a switch")),
    }
}

/// Everything that needs no switch.
async fn local(cli: &Cli, printer: &Printer) -> Result<Report, CliError> {
    match &cli.command {
        Command::Models(ModelsCommand::List) => {
            Ok(Report::Models(model::MODELS.iter().map(ModelSummaryView::from).collect()))
        }
        Command::Models(ModelsCommand::Show { model: name }) => {
            let found = model::lookup(name).ok_or_else(|| {
                CliError::usage(format!("unknown model {name:?}; see `tesmart models list`"))
            })?;
            Ok(Report::Model(Box::new(ModelDetailView::new(found))))
        }

        Command::Ports => ports(),

        Command::Sim(crate::cli::SimCommand::Serve(args)) => {
            crate::simulate::serve(args, printer).await
        }

        Command::Completions { shell } => Ok(generate::completions(*shell)),
        Command::Manpage { out } => generate::manpage(out.as_deref()),

        _ => Err(CliError::usage("this command needs a switch")),
    }
}

async fn switch<C: Connector>(kvm: &mut Kvm<C>, args: &SwitchArgs) -> Result<Report, CliError> {
    let port = port(args.port)?;

    let mut options = SwitchOptions::default();
    if args.no_verify {
        options.verify = Verify::Never;
    }
    if args.verify_always {
        options.verify = Verify::Always;
    }
    options.skip_if_active = args.if_needed;
    if let Some(settle) = args.settle {
        options.settle = settle;
    }
    if let Some(attempts) = args.attempts {
        options.attempts = attempts;
    }

    let outcome = kvm.switch_with(port, options).await?;
    Ok(Report::Switch(SwitchView::from(outcome)))
}

/// `next` and `prev`, which mean different things per protocol.
///
/// On a single-monitor switch there is no relative command, so this reads the active port and
/// switches to its neighbour, wrapping, and the result can be confirmed. On a dual-monitor
/// switch the ±1 step *is* the command, and nothing about it can be confirmed.
async fn step<C: Connector>(
    kvm: &mut Kvm<C>,
    direction: StepArg,
    monitor: crate::cli::OutputArg,
) -> Result<Report, CliError> {
    if kvm.protocol() == Protocol::KvmDual {
        return Ok(sent(kvm.step(monitor.into(), direction.into()).await?));
    }

    let current = kvm.active_port().await?;
    let last = kvm.ports().get();
    let wanted = match direction {
        StepArg::Next if current.get() >= last => 1,
        StepArg::Next => current.get() + 1,
        StepArg::Prev if current.get() <= 1 => last,
        StepArg::Prev => current.get() - 1,
    };

    let outcome = kvm.switch(port(wanted)?).await?;
    Ok(Report::Switch(SwitchView::from(outcome)))
}

async fn dual<C: Connector>(kvm: &mut Kvm<C>, command: &DualCommand) -> Result<Report, CliError> {
    match command {
        DualCommand::Step { monitor, direction } => {
            Ok(sent(kvm.step((*monitor).into(), (*direction).into()).await?))
        }
        DualCommand::Usb => Ok(sent(kvm.move_usb().await?)),
        DualCommand::Passthrough { state } => {
            Ok(sent(kvm.set_km_passthrough((*state).into()).await?))
        }
        DualCommand::Hotkey { key } => Ok(sent(kvm.set_hotkey_trigger((*key).into()).await?)),
        DualCommand::Autoswitch { state } => Ok(sent(kvm.set_auto_switch((*state).into()).await?)),
        DualCommand::Interval { duration } => {
            let seconds = u16::try_from(duration.as_secs()).map_err(|_| {
                CliError::usage("that interval is far longer than the switch can store")
            })?;
            let interval = SwitchInterval::from_secs(seconds)
                .map_err(|error| CliError::usage(format!("--interval: {error}")))?;
            Ok(sent(kvm.set_auto_switch_interval(interval).await?))
        }
    }
}

/// Everything under `tesmart matrix`.
///
/// Routing is the only write in the protocol that can be confirmed, so `route`, `route-all` and
/// `mirror` all come back with the matrix's own report of what it did.
async fn matrix<C: Connector>(
    connector: C,
    cli: &Cli,
    command: &MatrixCommand,
) -> Result<Report, CliError> {
    let mut matrix = handle::build_matrix(connector, &cli.global)?;

    match command {
        MatrixCommand::Show => Ok(Report::Routing(RoutingView::from(&matrix.routing().await?))),
        MatrixCommand::Route { input, output } => {
            let outcome = matrix.route(port(*input)?, port(*output)?).await?;
            Ok(Report::Route(RouteView::from(outcome)))
        }
        MatrixCommand::RouteAll { input } => {
            let outcome = matrix.route_all(port(*input)?).await?;
            Ok(Report::Route(RouteView::from(outcome)))
        }
        MatrixCommand::Mirror => Ok(Report::Route(RouteView::from(matrix.mirror().await?))),
        MatrixCommand::Preset(PresetCommand::Save { slot }) => {
            Ok(sent(matrix.save_preset(preset(*slot)?).await?))
        }
        MatrixCommand::Preset(PresetCommand::Load { slot }) => {
            let outcome = matrix.load_preset(preset(*slot)?).await?;
            Ok(Report::Preset(PresetView::from(outcome)))
        }
        MatrixCommand::Timeout { seconds } => {
            let timeout = MatrixTimeout::from_secs(*seconds)
                .map_err(|error| CliError::usage(error.to_string()))?;
            Ok(sent(matrix.set_display_timeout(timeout).await?))
        }
        MatrixCommand::Buzzer { state } => Ok(sent(matrix.set_buzzer((*state).into()).await?)),
    }
}

/// The network side-channel, which both KVMs and matrices have.
///
/// Written against [`NetOps`] rather than a concrete handle, because this is the one place the
/// two protocols overlap.
async fn net<D: NetOps>(
    device: &mut D,
    command: &NetCommand,
    printer: &Printer,
    context: &Context,
) -> Result<Report, CliError> {
    match command {
        NetCommand::Show => Ok(Report::Net(device.read_net().await?.into())),
        NetCommand::Set(args) => net_set(device, args, printer, context).await,
    }
}

/// Reading and writing network settings, whichever protocol is attached.
trait NetOps {
    /// Reads all four settings.
    async fn read_net(&mut self) -> Result<tesmart::client::NetReport, OpError>;

    /// Writes settings, then reads them back.
    async fn write_net(
        &mut self,
        changes: &[NetSetting],
        confirmed: Confirmed,
    ) -> Result<tesmart::client::NetOutcome, OpError>;
}

impl<C: Connector> NetOps for Kvm<C> {
    async fn read_net(&mut self) -> Result<tesmart::client::NetReport, OpError> {
        self.network().await
    }

    async fn write_net(
        &mut self,
        changes: &[NetSetting],
        confirmed: Confirmed,
    ) -> Result<tesmart::client::NetOutcome, OpError> {
        self.set_network(changes, confirmed).await
    }
}

impl<C: Connector> NetOps for tesmart::client::Matrix<C> {
    async fn read_net(&mut self) -> Result<tesmart::client::NetReport, OpError> {
        self.network().await
    }

    async fn write_net(
        &mut self,
        changes: &[NetSetting],
        confirmed: Confirmed,
    ) -> Result<tesmart::client::NetOutcome, OpError> {
        self.set_network(changes, confirmed).await
    }
}

async fn net_set<D: NetOps>(
    device: &mut D,
    args: &NetSetArgs,
    printer: &Printer,
    context: &Context,
) -> Result<Report, CliError> {
    let mut changes = Vec::new();
    if let Some(ip) = args.ip {
        changes.push(NetSetting::Ip(ip));
    }
    if let Some(port) = args.port {
        changes.push(NetSetting::Port(port));
    }
    if let Some(gateway) = args.gateway {
        changes.push(NetSetting::Gateway(gateway));
    }
    if let Some(mask) = args.mask {
        changes.push(NetSetting::Mask(mask));
    }

    if changes.is_empty() {
        return Err(CliError::usage("nothing to set; pass --ip, --port, --gateway or --mask"));
    }
    if !args.yes {
        return Err(CliError::usage(
            "refusing to write network settings without --yes: there is no factory reset, and a \
             wrong address leaves RS232 as the only way back",
        ));
    }

    // Say out loud which switch is about to be reconfigured, before doing it.
    let target = context.target.as_deref().unwrap_or("the switch");
    printer.note(&format!("writing network settings to {target}"));

    let outcome = device.write_net(&changes, Confirmed::yes()).await?;
    Ok(Report::NetSet(outcome.into()))
}

async fn watch<C: Connector>(
    kvm: &mut Kvm<C>,
    args: &WatchArgs,
    printer: &Printer,
    context: &Context,
) -> Result<Report, CliError> {
    let mut options = WatchOptions::default();
    if args.no_poll {
        options.poll_interval = None;
    } else if let Some(interval) = args.poll {
        options.poll_interval = Some(interval);
    }

    let mut watch = kvm.watch(options);
    // A starting point makes the first change read as "4 -> 7" instead of just "7". Failing to
    // get one is not a reason to stop watching.
    let _ignored = watch.prime().await;

    let mut changes = 0u64;
    loop {
        tokio::select! {
            change = watch.next_change() => {
                printer.change(context, &ChangeView::from(change?));
                changes += 1;
            }
            _interrupted = tokio::signal::ctrl_c() => break,
        }
    }

    // A clean close is what makes the switch stop pushing reports.
    watch.close().await;
    Ok(Report::Done(DoneView::new(format!("watch stopped after {changes} change(s)"))))
}

/// Tries the capabilities the registry is unsure about, and reports what actually happened.
async fn probe<C: Connector>(
    kvm: &mut Kvm<C>,
    args: &crate::cli::ProbeArgs,
    printer: &Printer,
) -> Result<Report, CliError> {
    let mut capabilities = Vec::new();
    let mut ports = None;

    // Reading the active port: the one capability that matters most and is least documented.
    let registry = kvm.spec().support(RequestKind::ReadActivePort);
    capabilities.push(match registry {
        Support::Unsupported => {
            not_probed(RequestKind::ReadActivePort, registry, "no such command")
        }
        _ => match kvm.active_port().await {
            Ok(port) => probed(
                RequestKind::ReadActivePort,
                registry,
                "works",
                Some(format!("port {port}")),
                suggestion(registry, "read-port"),
            ),
            Err(error) => failed_probe(RequestKind::ReadActivePort, registry, &error)?,
        },
    });

    // The ASCII side-channel.
    let registry = kvm.spec().support(RequestKind::NetQuery);
    capabilities.push(match registry {
        Support::Unsupported => not_probed(RequestKind::NetQuery, registry, "no such command"),
        _ => match kvm.network().await {
            Ok(report) if report.is_complete() => probed(
                RequestKind::NetQuery,
                registry,
                "works",
                report.config.ip.map(|ip| ip.to_string()),
                suggestion(registry, "net"),
            ),
            Ok(report) => probed(
                RequestKind::NetQuery,
                registry,
                "partial",
                Some(format!("{} field(s) unanswered", report.missing.len())),
                None,
            ),
            Err(error) => failed_probe(RequestKind::NetQuery, registry, &error)?,
        },
    });

    // The write-only settings cannot be probed at all: no series can read them back, so
    // sending one would change the switch without telling us anything.
    for kind in [RequestKind::Buzzer, RequestKind::DisplayTimeout, RequestKind::AutoInputDetect] {
        let registry = kvm.spec().support(kind);
        capabilities.push(not_probed(kind, registry, "write only, cannot be read back"));
    }

    // The input count, which is the one thing no command reports. Opt in, because finding out
    // means switching inputs.
    if args.input_count {
        printer.note("probing the input count: this changes the active input, then restores it");
        match kvm.probe_port_count().await {
            Ok(probe) => ports = Some(PortProbeView::from(probe)),
            Err(error) if error.class() == ErrorClass::Unsupported => {
                printer.note("skipping the input count: this switch cannot report its port");
            }
            Err(error) => return Err(CliError::usage(format!("probe stopped: {error}"))),
        }
    }

    Ok(Report::Probe(ProbeView { capabilities, ports }))
}

fn probed(
    kind: RequestKind,
    registry: Support,
    outcome: &'static str,
    detail: Option<String>,
    suggestion: Option<String>,
) -> ProbedCapability {
    ProbedCapability {
        command: kind.name(),
        registry: registry.name(),
        probe: outcome,
        detail,
        suggestion,
    }
}

fn not_probed(kind: RequestKind, registry: Support, why: &'static str) -> ProbedCapability {
    ProbedCapability {
        command: kind.name(),
        registry: registry.name(),
        probe: "not probed",
        detail: Some(why.to_owned()),
        suggestion: None,
    }
}

/// A capability that answered badly is a result; a link that failed is not.
fn failed_probe(
    kind: RequestKind,
    registry: Support,
    error: &OpError,
) -> Result<ProbedCapability, CliError> {
    match error.class() {
        ErrorClass::Timeout => Ok(probed(kind, registry, "no reply", None, None)),
        ErrorClass::Unsupported => Ok(probed(kind, registry, "unsupported", None, None)),
        // A dead link says nothing about the switch's capabilities: stop and report it.
        _ => Err(CliError::usage(format!("probe stopped: {error}"))),
    }
}

/// What to pass to make a working-but-unverified capability stick.
fn suggestion(registry: Support, flag: &str) -> Option<String> {
    (registry == Support::Unverified).then(|| format!("--assume {flag}=yes"))
}

async fn raw<C: Connector>(kvm: &mut Kvm<C>, args: &RawArgs) -> Result<Report, CliError> {
    let bytes = parse_hex(&args.bytes)?;
    let reply =
        kvm.session_mut().exchange_raw(&bytes, args.expect_reply).await.map_err(OpError::from)?;

    Ok(Report::Raw(RawView::new(&bytes, reply.as_ref())))
}

fn ports() -> Result<Report, CliError> {
    let ports = tesmart::serial::available_ports()
        .map_err(|error| CliError::usage(error.to_string()))?
        .into_iter()
        .map(|port| crate::report::SerialPortView { path: port.path, kind: port.kind.to_string() })
        .collect();

    Ok(Report::Ports(ports))
}

fn sent(sent: tesmart::client::Sent) -> Report {
    Report::Sent(sent.into())
}

fn port(number: u8) -> Result<Port, CliError> {
    Port::new(number).map_err(|error| CliError::usage(error.to_string()))
}

fn preset(slot: u8) -> Result<Preset, CliError> {
    Preset::new(slot).map_err(|error| CliError::usage(error.to_string()))
}

/// Accepts `AA BB 03`, `aabb03`, `AA:BB:03` and `AA,BB,03`.
fn parse_hex(parts: &[String]) -> Result<Vec<u8>, CliError> {
    let digits: String = parts
        .join("")
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ':' | ',' | '-' | '_'))
        .collect();

    if digits.is_empty() {
        return Err(CliError::usage("no bytes given"));
    }
    if !digits.len().is_multiple_of(2) {
        return Err(CliError::usage(format!(
            "{digits:?} is half a byte short: hex comes in pairs"
        )));
    }

    (0..digits.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&digits[at..at + 2], 16)
                .map_err(|_| CliError::usage(format!("{:?} is not hex", &digits[at..at + 2])))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(parts: &[&str]) -> Result<Vec<u8>, CliError> {
        parse_hex(&parts.iter().map(|part| (*part).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn hex_is_accepted_however_it_is_typed() {
        let expected = vec![0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE];
        assert_eq!(hex(&["AA", "BB", "03", "10", "00", "EE"]).unwrap(), expected);
        assert_eq!(hex(&["aabb031000ee"]).unwrap(), expected);
        assert_eq!(hex(&["AA:BB:03", "10,00-EE"]).unwrap(), expected);
    }

    #[test]
    fn bad_hex_says_what_is_wrong() {
        assert!(hex(&["AAB"]).unwrap_err().to_string().contains("pairs"));
        assert!(hex(&["ZZ"]).unwrap_err().to_string().contains("not hex"));
        assert!(hex(&[""]).unwrap_err().to_string().contains("no bytes"));
    }
}
