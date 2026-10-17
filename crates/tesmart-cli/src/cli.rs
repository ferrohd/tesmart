//! The command line surface: every flag, subcommand and value this tool accepts.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tesmart::proto::value::{Buzzer, DisplayTimeout, HotkeyTrigger, Output, Step, Toggle};
use tesmart::proto::wire::AsciiDialect;

/// Where to look for a switch when nothing says otherwise.
///
/// Every TESmart switch leaves the factory on this address and none of them do DHCP, so it is
/// the right guess far more often than not. It is printed in `--help`, and `net show`/`net set`
/// name the target they are talking to.
pub const DEFAULT_TARGET: &str = "tcp://192.168.1.10:5000";

const EXIT_CODES: &str = "\
Exit codes:
  0  success, including a command that was sent but cannot be verified
  1  something unexpected
  2  usage: bad arguments, no model, a port this switch does not have
  3  transport: cannot connect, or the link died
  4  timeout: nothing came back in time
  5  unsupported: this switch cannot do that
  6  verification: the switch reports something else, or --require-verified was not satisfied
  7  protocol: the switch said something that makes no sense";

/// Control TESmart KVM switches over TCP or RS232.
#[derive(Debug, Parser)]
#[command(
    name = "tesmart",
    version,
    about,
    after_help = EXIT_CODES,
    max_term_width = 100,
)]
pub struct Cli {
    /// Shared options.
    #[command(flatten)]
    pub global: Global,

    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Options every subcommand shares.
#[derive(Debug, Args)]
pub struct Global {
    /// Where the switch is: `tcp://host:port`, `serial:///dev/ttyUSB0`, or `sim`.
    #[arg(
        short, long, global = true,
        env = "TESMART_TARGET",
        default_value = DEFAULT_TARGET,
        value_name = "TARGET",
    )]
    pub target: String,

    /// Which model, from the built-in registry. See `tesmart models list`.
    #[arg(short, long, global = true, env = "TESMART_MODEL", value_name = "MODEL")]
    pub model: Option<String>,

    /// Which protocol the switch speaks, for a switch that is not in the registry.
    #[arg(short, long, global = true, env = "TESMART_PROTOCOL", value_name = "PROTOCOL")]
    pub protocol: Option<ProtocolArg>,

    /// How many inputs the switch has, for a switch that is not in the registry.
    #[arg(long, global = true, env = "TESMART_PORTS", value_name = "N")]
    pub ports: Option<u8>,

    /// Override what the registry claims, after testing it: read-port=yes
    #[arg(long, global = true, value_name = "KIND=SUPPORT")]
    pub assume: Vec<String>,

    /// How to print results.
    #[arg(short, long, global = true, env = "TESMART_OUTPUT", default_value = "human")]
    pub output: OutputFormat,

    /// When to colour output.
    #[arg(long, global = true, default_value = "auto", value_name = "WHEN")]
    pub color: ColorChoice,

    /// Log to stderr: -v info, -vv the bytes on the wire, -vvv everything.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Print nothing on success; the exit code carries the result.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Fail when a result cannot be confirmed, instead of reporting it as sent.
    #[arg(long, global = true)]
    pub require_verified: bool,

    /// How long one whole operation may take.
    #[arg(
        long, global = true,
        value_name = "DURATION",
        default_value = "10s",
        value_parser = parse_duration,
        help_heading = "Tuning",
    )]
    pub timeout: Duration,

    /// How long to wait for a connection.
    #[arg(
        long, global = true,
        value_name = "DURATION",
        default_value = "5s",
        value_parser = parse_duration,
        help_heading = "Tuning",
    )]
    pub connect_timeout: Duration,

    /// How many times to resend a query that goes unanswered.
    #[arg(long, global = true, value_name = "N", default_value_t = 5, help_heading = "Tuning")]
    pub retries: u8,

    /// Minimum gap between commands. Real switches need a second.
    #[arg(
        long, global = true,
        value_name = "DURATION",
        default_value = "1s",
        value_parser = parse_duration,
        help_heading = "Tuning",
    )]
    pub rate_limit: Duration,

    /// Which spelling of the ASCII network commands to send.
    #[arg(long, global = true, default_value = "captured", help_heading = "Tuning")]
    pub ascii_dialect: DialectArg,

    /// Baud rate, for a serial target.
    #[arg(
        long, global = true,
        value_name = "N",
        default_value_t = crate::target::DEFAULT_BAUD,
        help_heading = "Tuning",
    )]
    pub baud: u32,

    /// Keep one connection open instead of opening one per command.
    #[arg(long, global = true, help_heading = "Tuning")]
    pub persistent: bool,
}

/// What to do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Switch every output to a port.
    Switch(SwitchArgs),

    /// Print the active port.
    Get,

    /// Move one input up.
    Next(StepArgs),

    /// Move one input down.
    Prev(StepArgs),

    /// Mute or unmute the buzzer.
    Buzzer {
        /// Whether the buzzer should sound.
        state: OnOff,
    },

    /// Set how long the front panel display stays lit.
    Timeout {
        /// How long, or never.
        value: TimeoutArg,
    },

    /// Turn automatic input detection on or off.
    Autodetect {
        /// Whether to detect inputs automatically.
        state: OnOff,
    },

    /// Commands that only exist on dual-monitor switches.
    #[command(subcommand)]
    Dual(DualCommand),

    /// Route inputs to outputs on an HDMI matrix.
    #[command(subcommand)]
    Matrix(MatrixCommand),

    /// Read or write the switch's network settings.
    #[command(subcommand)]
    Net(NetCommand),

    /// Follow the active port until interrupted.
    Watch(WatchArgs),

    /// Try the capabilities the registry is unsure about and report what worked.
    Probe(ProbeArgs),

    /// Send raw bytes and print whatever comes back.
    Raw(RawArgs),

    /// Inspect the built-in model registry.
    #[command(subcommand)]
    Models(ModelsCommand),

    /// List the serial ports this machine has.
    Ports,

    /// Run an emulated switch, for testing without hardware.
    #[command(subcommand)]
    Sim(SimCommand),

    /// Print a shell completion script.
    Completions {
        /// Which shell.
        shell: clap_complete::Shell,
    },

    /// Print the manual page in roff format.
    Manpage {
        /// Write to this directory instead of standard output.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
}

impl Command {
    /// The name used in JSON output and in error envelopes.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Switch(_) => "switch",
            Self::Get => "get",
            Self::Next(_) => "next",
            Self::Prev(_) => "prev",
            Self::Buzzer { .. } => "buzzer",
            Self::Timeout { .. } => "timeout",
            Self::Autodetect { .. } => "autodetect",
            Self::Dual(command) => command.name(),
            Self::Matrix(command) => command.name(),
            Self::Net(NetCommand::Show) => "net-show",
            Self::Net(NetCommand::Set(_)) => "net-set",
            Self::Watch(_) => "watch",
            Self::Probe(_) => "probe",
            Self::Raw(_) => "raw",
            Self::Models(ModelsCommand::List) => "models-list",
            Self::Models(ModelsCommand::Show { .. }) => "models-show",
            Self::Ports => "ports",
            Self::Sim(SimCommand::Serve(_)) => "sim-serve",
            Self::Completions { .. } => "completions",
            Self::Manpage { .. } => "manpage",
        }
    }

    /// Whether this command talks to a switch.
    pub fn needs_a_switch(&self) -> bool {
        !matches!(
            self,
            Self::Models(_)
                | Self::Completions { .. }
                | Self::Manpage { .. }
                | Self::Ports
                | Self::Sim(_)
        )
    }
}

/// How to switch.
#[derive(Debug, Args)]
pub struct SwitchArgs {
    /// The port to switch to, as the front panel counts them.
    pub port: u8,

    /// Do not read the port back afterwards.
    #[arg(long, conflicts_with = "verify_always")]
    pub no_verify: bool,

    /// Read the port back even on a model where that is unproven.
    #[arg(long)]
    pub verify_always: bool,

    /// Read the port first and do nothing if it already matches.
    #[arg(long)]
    pub if_needed: bool,

    /// How long to wait before reading the port back.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    pub settle: Option<Duration>,

    /// How many times to read the port back before giving up.
    #[arg(long, value_name = "N")]
    pub attempts: Option<u8>,
}

/// How to step.
#[derive(Debug, Args)]
pub struct StepArgs {
    /// Which output to steer, on a dual-monitor switch. Ignored on single-monitor ones.
    ///
    /// Named `--monitor` rather than `--output` so it cannot be confused with `--output json`.
    #[arg(long, default_value = "a", value_name = "A|B")]
    pub monitor: OutputArg,
}

/// Commands that only exist on dual-monitor switches.
#[derive(Debug, Subcommand)]
pub enum DualCommand {
    /// Step one output to the next or previous input.
    Step {
        /// Which output to steer: a or b.
        monitor: OutputArg,
        /// Which way to go.
        direction: StepArg,
    },

    /// Move the USB link to the other output.
    Usb,

    /// Turn keyboard and mouse pass-through on or off.
    Passthrough {
        /// Whether pass-through should be on.
        state: OnOff,
    },

    /// Choose which key starts a hotkey sequence.
    Hotkey {
        /// Which key.
        key: HotkeyArg,
    },

    /// Turn auto-switching on or off.
    Autoswitch {
        /// Whether to switch automatically.
        state: OnOff,
    },

    /// Set the auto-switch dwell time.
    Interval {
        /// How long to stay on each input, at least 5s.
        #[arg(value_name = "DURATION", value_parser = parse_duration)]
        duration: Duration,
    },
}

impl DualCommand {
    fn name(&self) -> &'static str {
        match self {
            Self::Step { .. } => "dual-step",
            Self::Usb => "dual-usb",
            Self::Passthrough { .. } => "dual-passthrough",
            Self::Hotkey { .. } => "dual-hotkey",
            Self::Autoswitch { .. } => "dual-autoswitch",
            Self::Interval { .. } => "dual-interval",
        }
    }
}

/// Commands that only exist on HDMI matrices.
///
/// A matrix shares the default address with a KVM and none of its commands. Which of these a
/// given matrix has depends on the model: the 4x4 series has the display timeout, the 8x8 and
/// 16x16 have presets and a buzzer, and the RS232-only 4x4 models have only routing.
#[derive(Debug, Subcommand)]
pub enum MatrixCommand {
    /// Print which input feeds each output.
    Show,

    /// Route one input to one output.
    Route {
        /// The input to take.
        input: u8,
        /// The output to feed.
        ///
        /// The argument id is not `output`, so that it cannot collide with the global
        /// `--output json`.
        #[arg(id = "route-output", value_name = "OUTPUT")]
        output: u8,
    },

    /// Route one input to every output.
    RouteAll {
        /// The input to take.
        input: u8,
    },

    /// Mirror input 1 to output 1, 2 to 2, and so on.
    Mirror,

    /// Save the current routing into a preset, or load one back.
    #[command(subcommand)]
    Preset(PresetCommand),

    /// Set how long the front display stays lit. 4x4 matrices only.
    Timeout {
        /// How many seconds, 0 to never turn it off.
        #[arg(value_name = "SECONDS")]
        seconds: u16,
    },

    /// Mute or unmute the buzzer. 8x8 and 16x16 matrices only.
    Buzzer {
        /// Whether the buzzer should sound.
        state: OnOff,
    },
}

impl MatrixCommand {
    fn name(&self) -> &'static str {
        match self {
            Self::Show => "matrix-show",
            Self::Route { .. } => "matrix-route",
            Self::RouteAll { .. } => "matrix-route-all",
            Self::Mirror => "matrix-mirror",
            Self::Preset(PresetCommand::Save { .. }) => "matrix-preset-save",
            Self::Preset(PresetCommand::Load { .. }) => "matrix-preset-load",
            Self::Timeout { .. } => "matrix-timeout",
            Self::Buzzer { .. } => "matrix-buzzer",
        }
    }
}

/// Matrix routing presets.
#[derive(Debug, Subcommand)]
pub enum PresetCommand {
    /// Save the current routing into a preset.
    Save {
        /// Which slot, 1 to 8.
        slot: u8,
    },

    /// Load a preset and apply it.
    Load {
        /// Which slot, 1 to 8.
        slot: u8,
    },
}

/// How to probe.
#[derive(Debug, Args)]
pub struct ProbeArgs {
    /// Also find the real input count, by switching to every input in turn.
    ///
    /// This changes which computer you are looking at while it runs, then puts it back. No
    /// command reports the input count, so trying them is the only way to find out.
    ///
    /// Named `--input-count` rather than `--ports` so it cannot be confused with the global
    /// `--ports N`, which *declares* the count rather than discovering it.
    #[arg(long)]
    pub input_count: bool,
}

/// Network settings.
#[derive(Debug, Subcommand)]
pub enum NetCommand {
    /// Read all four network settings.
    Show,

    /// Write network settings, then read them back.
    Set(NetSetArgs),
}

/// What to write.
#[derive(Debug, Args)]
pub struct NetSetArgs {
    /// New IP address.
    #[arg(long, value_name = "ADDR")]
    pub ip: Option<std::net::Ipv4Addr>,

    /// New TCP port.
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,

    /// New default gateway.
    #[arg(long, value_name = "ADDR")]
    pub gateway: Option<std::net::Ipv4Addr>,

    /// New subnet mask.
    #[arg(long, value_name = "ADDR")]
    pub mask: Option<std::net::Ipv4Addr>,

    /// Confirm. There is no factory reset: a wrong address leaves RS232 as the only way back.
    #[arg(long)]
    pub yes: bool,
}

/// How to watch.
#[derive(Debug, Args)]
pub struct WatchArgs {
    /// How often to read the port, in addition to listening.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration, conflicts_with = "no_poll")]
    pub poll: Option<Duration>,

    /// Only listen; never ask.
    #[arg(long)]
    pub no_poll: bool,
}

/// What to send.
#[derive(Debug, Args)]
pub struct RawArgs {
    /// The bytes, in hex: AA BB 03 10 00 EE, or aabb03100 0ee.
    #[arg(required = true, value_name = "HEX")]
    pub bytes: Vec<String>,

    /// Wait for a reply and print it.
    #[arg(long)]
    pub expect_reply: bool,
}

/// Registry queries.
#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// List every model this tool knows about.
    List,

    /// Show what one model can do, and on what evidence.
    Show {
        /// The model name or alias.
        model: String,
    },
}

/// The emulator.
#[derive(Debug, Subcommand)]
pub enum SimCommand {
    /// Serve an emulated switch over TCP until interrupted.
    Serve(SimServeArgs),
}

/// How to emulate.
#[derive(Debug, Args)]
pub struct SimServeArgs {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:5000", value_name = "ADDR")]
    pub listen: std::net::SocketAddr,

    /// Which model to pretend to be.
    #[arg(long, default_value = crate::target::DEFAULT_SIM_MODEL, value_name = "MODEL")]
    pub model: String,

    /// Behave perfectly, instead of the way hardware does.
    #[arg(long)]
    pub perfect: bool,

    /// Drop every n-th reply.
    #[arg(long, value_name = "N")]
    pub drop_every: Option<u8>,

    /// Terminate frames with this byte instead of 0xEE.
    #[arg(long, value_name = "HEX")]
    pub bad_terminator: Option<String>,

    /// Accept switch commands and do nothing.
    #[arg(long)]
    pub ignore_switch: bool,
}

/// How to print results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Lines meant for a person.
    Human,
    /// One JSON object, or one per line for `watch`.
    Json,
}

/// When to colour output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    /// Colour when writing to a terminal.
    Auto,
    /// Always colour.
    Always,
    /// Never colour.
    Never,
}

/// One of the four TESmart control protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProtocolArg {
    /// Single-monitor KVM switches.
    Kvm,
    /// Dual-monitor KVM switches.
    KvmDual,
    /// HDMI matrices. Use the `matrix` subcommands with these.
    Matrix,
    /// The 4x1 SDI switch, which only understands `switch`.
    Sdi,
}

impl From<ProtocolArg> for tesmart::proto::Protocol {
    fn from(protocol: ProtocolArg) -> Self {
        match protocol {
            ProtocolArg::Kvm => Self::Kvm,
            ProtocolArg::KvmDual => Self::KvmDual,
            ProtocolArg::Matrix => Self::Matrix,
            ProtocolArg::Sdi => Self::Sdi,
        }
    }
}

/// An on/off setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    /// Enabled.
    On,
    /// Disabled.
    Off,
}

impl From<OnOff> for Toggle {
    fn from(state: OnOff) -> Self {
        match state {
            OnOff::On => Self::On,
            OnOff::Off => Self::Off,
        }
    }
}

impl From<OnOff> for Buzzer {
    fn from(state: OnOff) -> Self {
        match state {
            OnOff::On => Self::Audible,
            OnOff::Off => Self::Muted,
        }
    }
}

/// A front display timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TimeoutArg {
    /// Ten seconds.
    #[value(name = "10s")]
    Ten,
    /// Thirty seconds.
    #[value(name = "30s")]
    Thirty,
    /// Leave the display on.
    Never,
}

impl From<TimeoutArg> for DisplayTimeout {
    fn from(value: TimeoutArg) -> Self {
        match value {
            TimeoutArg::Ten => Self::Seconds10,
            TimeoutArg::Thirty => Self::Seconds30,
            TimeoutArg::Never => Self::Never,
        }
    }
}

/// One of a dual-monitor switch's outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputArg {
    /// Output A.
    A,
    /// Output B.
    B,
}

impl From<OutputArg> for Output {
    fn from(output: OutputArg) -> Self {
        match output {
            OutputArg::A => Self::A,
            OutputArg::B => Self::B,
        }
    }
}

/// Which way to step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StepArg {
    /// One input up.
    Next,
    /// One input down.
    Prev,
}

impl From<StepArg> for Step {
    fn from(step: StepArg) -> Self {
        match step {
            StepArg::Next => Self::Next,
            StepArg::Prev => Self::Previous,
        }
    }
}

/// Which key starts a hotkey sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HotkeyArg {
    /// Right Ctrl, pressed twice.
    RightCtrl,
    /// Scroll Lock, pressed twice.
    ScrollLock,
}

impl From<HotkeyArg> for HotkeyTrigger {
    fn from(key: HotkeyArg) -> Self {
        match key {
            HotkeyArg::RightCtrl => Self::RightCtrl,
            HotkeyArg::ScrollLock => Self::ScrollLock,
        }
    }
}

/// Which spelling of the ASCII commands to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DialectArg {
    /// What TESmart's own Windows controller sends.
    Captured,
    /// What tesmart.sh sends.
    TesmartSh,
}

impl From<DialectArg> for AsciiDialect {
    fn from(dialect: DialectArg) -> Self {
        match dialect {
            DialectArg::Captured => Self::Captured,
            DialectArg::TesmartSh => Self::TesmartSh,
        }
    }
}

fn parse_duration(text: &str) -> Result<Duration, String> {
    humantime::parse_duration(text).map_err(|error| format!("{error} (try 500ms, 2s, 1m)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_default_target_is_the_factory_address() {
        let cli = Cli::try_parse_from(["tesmart", "get"]).unwrap();
        assert_eq!(cli.global.target, "tcp://192.168.1.10:5000");
        assert_eq!(cli.command.name(), "get");
        assert!(cli.command.needs_a_switch());
    }

    #[test]
    fn global_options_may_come_after_the_subcommand() {
        let cli = Cli::try_parse_from(["tesmart", "switch", "3", "-m", "HKS801-EB23", "-vv"])
            .expect("global options are accepted anywhere");
        assert_eq!(cli.global.model.as_deref(), Some("HKS801-EB23"));
        assert_eq!(cli.global.verbose, 2);
    }

    #[test]
    fn registry_commands_need_no_switch() {
        for args in [
            vec!["tesmart", "models", "list"],
            vec!["tesmart", "completions", "bash"],
            vec!["tesmart", "manpage"],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap();
            assert!(!cli.command.needs_a_switch(), "{args:?}");
        }
    }

    #[test]
    fn durations_accept_human_units() {
        let cli = Cli::try_parse_from(["tesmart", "--timeout", "2500ms", "get"]).unwrap();
        assert_eq!(cli.global.timeout, Duration::from_millis(2500));

        let error = Cli::try_parse_from(["tesmart", "--timeout", "soon", "get"]).unwrap_err();
        assert!(error.to_string().contains("try 500ms"), "{error}");
    }

    #[test]
    fn the_display_timeout_values_are_the_documented_ones() {
        for (text, expected) in [
            ("10s", DisplayTimeout::Seconds10),
            ("30s", DisplayTimeout::Seconds30),
            ("never", DisplayTimeout::Never),
        ] {
            let cli = Cli::try_parse_from(["tesmart", "timeout", text]).unwrap();
            let Command::Timeout { value } = cli.command else {
                panic!("expected the timeout command")
            };
            assert_eq!(DisplayTimeout::from(value), expected);
        }

        // 20s is not one of them, and the switch has no way to express it.
        assert!(Cli::try_parse_from(["tesmart", "timeout", "20s"]).is_err());
    }

    #[test]
    fn switch_verification_flags_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["tesmart", "switch", "3", "--no-verify", "--verify-always"])
                .is_err()
        );
    }
}
