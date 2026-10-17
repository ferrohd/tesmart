//! Getting results onto the terminal, or into a pipe.
//!
//! Results go to standard output, everything else to standard error, so `--output json | jq`
//! works at any verbosity. Colour is written unconditionally and stripped by `anstream` when
//! the destination is not a terminal, when `NO_COLOR` is set, or when `--color never` was
//! given.

use std::io::{self, Write as _};

use anstream::AutoStream;
use owo_colors::OwoColorize as _;
use serde::Serialize;

use crate::cli::{ColorChoice, OutputFormat};
use crate::error::CliError;
use crate::report::{ChangeView, Report};

/// What a result belongs to.
#[derive(Debug, Clone)]
pub struct Context {
    /// The subcommand's name, as it appears in JSON.
    pub command: &'static str,
    /// The switch that was addressed, when there was one.
    pub target: Option<String>,
    /// The model it was addressed as.
    pub model: Option<String>,
}

impl Context {
    /// A context for a command that talks to no switch.
    pub fn local(command: &'static str) -> Self {
        Self { command, target: None, model: None }
    }
}

/// Writes results and failures.
#[derive(Debug, Clone, Copy)]
pub struct Printer {
    format: OutputFormat,
    color: ColorChoice,
    quiet: bool,
}

impl Printer {
    /// A printer configured from the global options.
    pub fn new(format: OutputFormat, color: ColorChoice, quiet: bool) -> Self {
        Self { format, color, quiet }
    }

    /// Whether output is machine readable.
    pub fn is_json(self) -> bool {
        matches!(self.format, OutputFormat::Json)
    }

    /// Prints a successful result.
    pub fn success(self, context: &Context, report: &Report) {
        match self.format {
            OutputFormat::Json => {
                self.write_json(&Envelope::success(context, report));
            }
            OutputFormat::Human => {
                if !self.quiet {
                    self.write_human(report);
                }
                self.warnings(report.warnings());
            }
        }
    }

    /// Prints one port change, as `watch` produces them.
    pub fn change(self, context: &Context, change: &ChangeView) {
        match self.format {
            OutputFormat::Json => {
                // Newline delimited: one envelope per change, flushed as it happens.
                self.write_json(&Envelope::change(context, change));
            }
            OutputFormat::Human => {
                if self.quiet {
                    return;
                }
                let line = match change.previous {
                    Some(previous) => {
                        format!("port {} -> {} ({})", previous, change.port.bold(), change.source)
                    }
                    None => format!("port {} ({})", change.port.bold(), change.source),
                };
                self.line(&line);
            }
        }
    }

    /// Prints a failure, and returns nothing: the caller owns the exit code.
    pub fn failure(self, context: &Context, error: &CliError) {
        match self.format {
            OutputFormat::Json => self.write_json(&Envelope::failure(context, error)),
            OutputFormat::Human => {
                let mut stderr = self.stderr();
                let _ignored = writeln!(stderr, "{}: {}", "error".red().bold(), error.chain());
            }
        }
    }

    /// Says something to the user that is not a result: what is about to happen, or where the
    /// emulator is listening. Goes to standard error, so it never pollutes a pipe.
    pub fn note(self, message: &str) {
        if self.quiet || self.is_json() {
            return;
        }
        let mut stderr = self.stderr();
        let _ignored = writeln!(stderr, "{}: {message}", "note".cyan().bold());
    }

    /// Prints caveats that came with a successful result.
    fn warnings(self, warnings: &[String]) {
        if warnings.is_empty() {
            return;
        }
        let mut stderr = self.stderr();
        for warning in warnings {
            let _ignored = writeln!(stderr, "{}: {warning}", "warning".yellow().bold());
        }
    }

    fn write_human(self, report: &Report) {
        let mut stdout = self.stdout();
        let _ignored = crate::render::human::write(&mut stdout, report);
        let _ignored = stdout.flush();
    }

    fn write_json<T: Serialize>(self, value: &T) {
        let mut stdout = self.stdout();
        let _ignored = match serde_json::to_string(value) {
            Ok(json) => writeln!(stdout, "{json}"),
            Err(error) => writeln!(stdout, r#"{{"ok":false,"error":{{"message":"{error}"}}}}"#),
        };
        let _ignored = stdout.flush();
    }

    fn line(self, text: &str) {
        let mut stdout = self.stdout();
        let _ignored = writeln!(stdout, "{text}");
        let _ignored = stdout.flush();
    }

    fn stdout(self) -> AutoStream<io::Stdout> {
        AutoStream::new(io::stdout(), self.choice())
    }

    fn stderr(self) -> AutoStream<io::Stderr> {
        AutoStream::new(io::stderr(), self.choice())
    }

    /// JSON is never coloured: it is for programs.
    fn choice(self) -> anstream::ColorChoice {
        if self.is_json() {
            return anstream::ColorChoice::Never;
        }
        match self.color {
            ColorChoice::Auto => anstream::ColorChoice::Auto,
            ColorChoice::Always => anstream::ColorChoice::Always,
            ColorChoice::Never => anstream::ColorChoice::Never,
        }
    }
}

/// The JSON contract: one object per result, one per line for `watch`.
#[derive(Debug, Serialize)]
struct Envelope<'a> {
    ok: bool,
    command: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'a Report>,
    #[serde(skip_serializing_if = "Option::is_none")]
    change: Option<&'a ChangeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<&'a str>,
}

impl<'a> Envelope<'a> {
    fn base(context: &'a Context, ok: bool) -> Self {
        Self {
            ok,
            command: context.command,
            target: context.target.as_deref(),
            model: context.model.as_deref(),
            result: None,
            change: None,
            error: None,
            warnings: Vec::new(),
        }
    }

    fn success(context: &'a Context, report: &'a Report) -> Self {
        Self {
            result: Some(report),
            warnings: report.warnings().iter().map(String::as_str).collect(),
            ..Self::base(context, true)
        }
    }

    fn change(context: &'a Context, change: &'a ChangeView) -> Self {
        Self { change: Some(change), ..Self::base(context, true) }
    }

    fn failure(context: &'a Context, error: &CliError) -> Self {
        Self {
            error: Some(ErrorView { class: error.class().name(), message: error.chain() }),
            ..Self::base(context, false)
        }
    }
}

/// A failure, as a program sees it.
#[derive(Debug, Serialize)]
struct ErrorView {
    class: &'static str,
    message: String,
}

/// Human rendering, one module so the formatting lives in one place.
mod human {
    use std::io::{self, Write};

    use owo_colors::OwoColorize as _;

    use crate::report::Report;

    /// Dispatches to one small writer per kind of result.
    pub fn write(out: &mut impl Write, report: &Report) -> io::Result<()> {
        match report {
            Report::Switch(view) => switch(out, view),
            Report::Port(view) => writeln!(out, "port {}", view.port.bold()),
            Report::Sent(view) => {
                writeln!(out, "{} sent, state cannot be read back", view.command)
            }
            Report::Routing(view) => routing(out, view),
            Report::Route(view) => route(out, view),
            Report::Preset(view) => preset(out, view),
            Report::Net(view) => net(out, view),
            Report::NetSet(view) => net_set(out, view),
            Report::Probe(view) => probe(out, view),
            Report::Raw(view) => raw(out, view),
            Report::Models(models) => model_list(out, models),
            Report::Model(view) => model_detail(out, view),
            Report::Ports(ports) => serial_ports(out, ports),
            Report::Done(view) => writeln!(out, "{}", view.message),
        }
    }

    fn switch(out: &mut impl Write, view: &crate::report::SwitchView) -> io::Result<()> {
        if view.already_active {
            return writeln!(out, "already on port {}", view.port.bold());
        }

        if !view.verified {
            let reason = view.reason.as_deref().unwrap_or("not verified");
            return writeln!(
                out,
                "port {} requested, {}: {reason}",
                view.port.bold(),
                "not verified".yellow()
            );
        }

        match view.attempts {
            // More than one read means the first one disagreed, which is worth seeing.
            Some(attempts) if attempts > 1 => writeln!(
                out,
                "switched to port {} (confirmed after {attempts} reads)",
                view.port.bold()
            ),
            _ => writeln!(out, "switched to port {}", view.port.bold()),
        }
    }

    fn routing(out: &mut impl Write, view: &crate::report::RoutingView) -> io::Result<()> {
        if view.links.is_empty() {
            return writeln!(out, "{}", "the matrix reported no routing".yellow());
        }
        writeln!(out, "{:<7} INPUT", "OUTPUT")?;
        for link in &view.links {
            writeln!(out, "{:<7} {}", link.output, link.input.bold())?;
        }
        if view.mirrored {
            writeln!(out, "(mirrored)")?;
        }
        Ok(())
    }

    fn route(out: &mut impl Write, view: &crate::report::RouteView) -> io::Result<()> {
        if !view.verified {
            let reason = view.reason.as_deref().unwrap_or("not verified");
            return writeln!(out, "routing requested, {}: {reason}", "not verified".yellow());
        }
        writeln!(out, "routed, confirmed by the matrix")?;
        match &view.routing {
            Some(routing) => routing_table(out, routing),
            None => Ok(()),
        }
    }

    fn preset(out: &mut impl Write, view: &crate::report::PresetView) -> io::Result<()> {
        match &view.routing {
            Some(routing) => {
                writeln!(out, "preset {} applied", view.preset.bold())?;
                routing_table(out, routing)
            }
            None => writeln!(out, "preset {} sent, routing not read back", view.preset.bold()),
        }
    }

    /// The routing table, indented, as a detail under something else.
    fn routing_table(out: &mut impl Write, view: &crate::report::RoutingView) -> io::Result<()> {
        for link in &view.links {
            writeln!(out, "  output {} <- input {}", link.output, link.input.bold())?;
        }
        Ok(())
    }

    fn net(out: &mut impl Write, view: &crate::report::NetView) -> io::Result<()> {
        write_net(out, view)?;
        for field in &view.missing {
            writeln!(out, "{field:<9} {}", "no answer".yellow())?;
        }
        Ok(())
    }

    fn net_set(out: &mut impl Write, view: &crate::report::NetSetView) -> io::Result<()> {
        write_net(out, &view.applied)?;
        for field in &view.mismatched {
            writeln!(out, "{}: {field} did not read back as written", "mismatch".red().bold())?;
        }
        Ok(())
    }

    fn probe(out: &mut impl Write, view: &crate::report::ProbeView) -> io::Result<()> {
        writeln!(
            out,
            "{:<18} {:<12} {:<11} {:<28} SUGGESTION",
            "CAPABILITY", "REGISTRY", "PROBE", "DETAIL"
        )?;

        for row in &view.capabilities {
            // Pad before colouring: escape codes have no width but `{:<11}` counts them.
            let probe = colour_probe(row.probe, format!("{:<11}", row.probe));
            let suggestion = row
                .suggestion
                .as_deref()
                .map(|flag| format!("suggest: {flag}"))
                .unwrap_or_default();
            let line = format!(
                "{:<18} {:<12} {probe} {:<28} {suggestion}",
                row.command,
                row.registry,
                row.detail.as_deref().unwrap_or(""),
            );
            writeln!(out, "{}", line.trim_end())?;
        }

        if let Some(ports) = &view.ports {
            writeln!(out)?;
            match ports.found {
                Some(found) if ports.agrees => {
                    writeln!(out, "inputs: {}, as expected", found.bold())?;
                }
                Some(found) => writeln!(
                    out,
                    "inputs: {} found, but {} claimed; {}",
                    found.bold(),
                    ports.claimed,
                    format_args!("suggest: --ports {found}")
                )?,
                None => writeln!(out, "inputs: {}", "none could be confirmed".yellow())?,
            }
            if !ports.restored {
                writeln!(out, "{}", "the original input was not confirmed restored".red())?;
            }
        }
        Ok(())
    }

    fn raw(out: &mut impl Write, view: &crate::report::RawView) -> io::Result<()> {
        writeln!(out, "sent  {}", view.sent)?;
        match &view.reply {
            Some(reply) => writeln!(out, "reply {reply}"),
            None => Ok(()),
        }
    }

    fn model_list(
        out: &mut impl Write,
        models: &[crate::report::ModelSummaryView],
    ) -> io::Result<()> {
        writeln!(out, "{:<14} {:<9} {:>5}  {:<13} NET", "MODEL", "PROTOCOL", "PORTS", "READ-PORT")?;
        for model in models {
            writeln!(
                out,
                "{:<14} {:<9} {:>5}  {:<13} {}",
                model.model, model.protocol, model.ports, model.read_port, model.net_config
            )?;
        }
        Ok(())
    }

    fn model_detail(out: &mut impl Write, view: &crate::report::ModelDetailView) -> io::Result<()> {
        writeln!(out, "{} ({}, {} ports)", view.model.bold(), view.protocol, view.ports)?;
        if !view.aliases.is_empty() {
            writeln!(out, "aliases: {}", view.aliases.join(", "))?;
        }
        writeln!(out)?;
        writeln!(out, "{:<22} {:<7} {:<13} EVIDENCE", "COMMAND", "OPCODE", "SUPPORT")?;

        for capability in &view.capabilities {
            writeln!(
                out,
                "{:<22} {:<7} {:<13} {}",
                capability.command,
                capability.opcode.as_deref().unwrap_or("ascii"),
                capability.support,
                capability.evidence
            )?;
            // The protocol tables carry caveats worth repeating here, where someone is looking.
            if let Some(note) = capability.note {
                writeln!(out, "    {note}")?;
            }
        }

        if !view.notes.is_empty() {
            writeln!(out)?;
            writeln!(out, "{}", view.notes)?;
        }
        Ok(())
    }

    fn serial_ports(
        out: &mut impl Write,
        ports: &[crate::report::SerialPortView],
    ) -> io::Result<()> {
        if ports.is_empty() {
            return writeln!(out, "no serial ports found");
        }
        for port in ports {
            writeln!(out, "{}  {}", port.path, port.kind)?;
        }
        Ok(())
    }

    fn write_net(out: &mut impl Write, view: &crate::report::NetView) -> io::Result<()> {
        for (label, value) in [
            ("ip", view.ip.clone()),
            ("port", view.port.map(|port| port.to_string())),
            ("gateway", view.gateway.clone()),
            ("mask", view.mask.clone()),
        ] {
            if let Some(value) = value {
                writeln!(out, "{label:<9} {value}")?;
            }
        }
        Ok(())
    }

    fn colour_probe(probe: &str, padded: String) -> String {
        match probe {
            "works" => padded.green().to_string(),
            "no reply" | "unsupported" | "partial" => padded.yellow().to_string(),
            _ => padded,
        }
    }
}
