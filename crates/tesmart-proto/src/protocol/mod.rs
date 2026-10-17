//! Protocols, as declarative command tables.
//!
//! TESmart uses four incompatible control protocols and **the same opcode byte means different
//! things across them**. `0x03` sets the front display timeout on a single-monitor KVM and steps
//! output A's input on a dual-monitor one. A command table is therefore not a formality: it is the
//! thing that stops a display-timeout command from silently changing which computer you are
//! looking at.
//!
//! | [`Protocol`] | Table | Devices | Wire format |
//! | --- | --- | --- | --- |
//! | `kvm` | [`KvmTable`] | single-monitor 4/8/16-port KVMs with RS232 or LAN | `AA BB` hex frames |
//! | `kvm-dual` | [`KvmDualTable`] | 4-port dual-monitor KVMs (community documented) | the same frames, redefined from `0x03` up |
//! | `matrix` | [`MatrixTable`] | HDMI matrices | `MT00…NT` ASCII strings |
//! | `sdi` | [`SdiTable`] | the 4x1 SDI switch | four-byte `AA CC`, write-only |
//!
//! The names in the first column are the ones [`Protocol`] parses, prints and serialises, and
//! the ones the command line takes for `--protocol`. There is no shorthand to learn.
//!
//! A request that is not in the active protocol is refused by [`CommandTable::encode`] before
//! anything is sent, and the error says what the opcode would have done instead.
//!
//! The trait is sealed: the set of protocols is closed, which is what lets every table be an
//! exhaustive `match` and makes a new [`Request`] variant a compile error until all four
//! protocols have decided what to do with it. Unknown models are handled by choosing the protocol
//! that fits, not by writing a fifth command table.

pub mod base;
pub mod dual;
pub mod kvm;
pub mod matrix;
pub mod sdi;

use core::fmt;
use core::str::FromStr;

pub use crate::protocol::dual::KvmDualTable;
pub use crate::protocol::kvm::KvmTable;
pub use crate::protocol::matrix::MatrixTable;
pub use crate::protocol::sdi::SdiTable;

use crate::command::{CommandInfo, Request, RequestKind, Response};
use crate::wire::{AsciiStyle, Chunk, Transmit};

mod sealed {
    pub trait Sealed {}
}

/// The command table of one protocol.
pub trait CommandTable: Copy + sealed::Sealed {
    /// Which protocol this is.
    fn protocol(self) -> Protocol;

    /// Encodes a request, or refuses it.
    ///
    /// # Errors
    ///
    /// Returns [`Unsupported`] if the request has no meaning in this protocol.
    fn encode(self, request: Request, style: AsciiStyle) -> Result<Transmit, Unsupported>;

    /// Interprets one decoded chunk, if it means anything in this protocol.
    fn decode(self, chunk: &Chunk<'_>) -> Option<Response>;

    /// Every command this protocol understands, for help output and capability reporting.
    fn commands(self) -> &'static [CommandInfo];

    /// Whether this protocol understands a kind of request.
    fn supports(self, kind: RequestKind) -> bool {
        self.command(kind).is_some()
    }

    /// The table entry for a kind of request.
    fn command(self, kind: RequestKind) -> Option<&'static CommandInfo> {
        self.commands().iter().find(|entry| entry.kind == kind)
    }
}

/// A protocol, chosen at run time.
///
/// Deliberately has no `Default`: the protocol must come from the model registry or from an
/// explicit choice by the user. Guessing it from a model name is how you end up sending
/// `0x03` to the wrong kind of switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Protocol {
    /// Single-monitor KVMs, over `AA BB` hex frames. See [`KvmTable`].
    Kvm,
    /// Dual-monitor KVMs: the same framing, redefined from `0x03` up. See [`KvmDualTable`].
    KvmDual,
    /// HDMI matrices, over ASCII strings. See [`MatrixTable`].
    Matrix,
    /// The 4x1 SDI switch, over four-byte `AA CC` commands. See [`SdiTable`].
    Sdi,
}

impl Protocol {
    /// Every protocol, in documentation order.
    pub const ALL: [Self; 4] = [Self::Kvm, Self::KvmDual, Self::Matrix, Self::Sdi];

    /// The name used on the command line and in configuration.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kvm => "kvm",
            Self::KvmDual => "kvm-dual",
            Self::Matrix => "matrix",
            Self::Sdi => "sdi",
        }
    }

    /// Whether this protocol uses the `AA BB` hex frame codec.
    ///
    /// False for [`Protocol::Sdi`], which sends binary but not *that* binary: its four-byte
    /// `AA CC 01 0n` commands are not frames this crate's codec can read or write, and it never
    /// replies, so there is nothing to decode either.
    pub const fn uses_frames(self) -> bool {
        matches!(self, Self::Kvm | Self::KvmDual)
    }

    /// Whether a switch in this protocol can report anything at all.
    ///
    /// False for [`Protocol::Sdi`]: no read command, no acknowledgement, nothing. A driver can
    /// use this to skip waiting for replies that are never coming.
    pub const fn can_reply(self) -> bool {
        !matches!(self, Self::Sdi)
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Protocol {
    type Err = UnknownProtocol;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|protocol| s.eq_ignore_ascii_case(protocol.name()))
            .ok_or(UnknownProtocol)
    }
}

/// Text that does not name a protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("expected one of kvm, kvm-dual, matrix, sdi")]
pub struct UnknownProtocol;

impl sealed::Sealed for Protocol {}

/// Runtime dispatch over the four protocols.
impl CommandTable for Protocol {
    fn protocol(self) -> Protocol {
        self
    }

    fn encode(self, request: Request, style: AsciiStyle) -> Result<Transmit, Unsupported> {
        match self {
            Self::Kvm => KvmTable.encode(request, style),
            Self::KvmDual => KvmDualTable.encode(request, style),
            Self::Matrix => MatrixTable.encode(request, style),
            Self::Sdi => SdiTable.encode(request, style),
        }
    }

    fn decode(self, chunk: &Chunk<'_>) -> Option<Response> {
        match self {
            Self::Kvm => KvmTable.decode(chunk),
            Self::KvmDual => KvmDualTable.decode(chunk),
            Self::Matrix => MatrixTable.decode(chunk),
            Self::Sdi => SdiTable.decode(chunk),
        }
    }

    fn commands(self) -> &'static [CommandInfo] {
        match self {
            Self::Kvm => KvmTable.commands(),
            Self::KvmDual => KvmDualTable.commands(),
            Self::Matrix => MatrixTable.commands(),
            Self::Sdi => SdiTable.commands(),
        }
    }
}

/// A request that has no meaning in the active protocol.
///
/// The [`Display`](fmt::Display) implementation explains what the opcode would have done
/// instead, where the protocols clash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    /// The protocol that refused the request.
    pub protocol: Protocol,
    /// The request that was refused.
    pub request: RequestKind,
}

impl Unsupported {
    pub(crate) const fn new(protocol: Protocol, request: RequestKind) -> Self {
        Self { protocol, request }
    }

    /// Why this combination is refused, when there is something interesting to say.
    pub const fn note(self) -> Option<&'static str> {
        clash_note(self.protocol, self.request)
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not available on the {} protocol", self.request, self.protocol)?;
        match self.note() {
            Some(note) => write!(f, ": {note}"),
            None => Ok(()),
        }
    }
}

impl core::error::Error for Unsupported {}

/// The interesting half of an [`Unsupported`] message.
const fn clash_note(protocol: Protocol, request: RequestKind) -> Option<&'static str> {
    match (protocol, request) {
        (Protocol::KvmDual, RequestKind::DisplayTimeout) => Some(
            "on dual-monitor switches opcode 0x03 steps output A's input instead of setting the \
             display timeout",
        ),
        (Protocol::Kvm, RequestKind::StepOutput) => Some(
            "on single-monitor switches opcode 0x03 sets the front display timeout; use switch \
             to choose a port",
        ),
        (Protocol::Kvm, RequestKind::MoveUsb | RequestKind::KmPassthrough) => {
            Some("this command only exists on dual-monitor switches")
        }
        (
            Protocol::Kvm,
            RequestKind::HotkeyTrigger | RequestKind::AutoSwitch | RequestKind::AutoSwitchInterval,
        ) => Some("this command is only documented for dual-monitor switches"),
        (Protocol::KvmDual, RequestKind::ReadActivePort) => {
            Some("no read command is documented for dual-monitor switches")
        }
        (Protocol::KvmDual, RequestKind::NetQuery | RequestKind::NetSet) => {
            Some("network configuration is not documented for dual-monitor switches")
        }
        (Protocol::KvmDual, RequestKind::AutoInputDetect) => {
            Some("auto input detect is not documented for dual-monitor switches")
        }
        (Protocol::Matrix, RequestKind::Switch) => {
            Some("matrices route an input to an output; use matrix-route")
        }
        (Protocol::Matrix, RequestKind::DisplayTimeout) => {
            Some("matrices use their own display timeout command; use matrix-display-timeout")
        }
        (Protocol::Matrix, RequestKind::Buzzer) => {
            Some("matrices use their own buzzer command, with inverted values; use matrix-buzzer")
        }
        (Protocol::Matrix, RequestKind::ReadActivePort) => {
            Some("a matrix has no single active port; read its routing instead")
        }
        (Protocol::Matrix, _) => Some("matrices understand only the ASCII matrix commands"),
        (Protocol::Sdi, RequestKind::ReadActivePort) => {
            Some("the SDI switch does not report anything: it has no read command and never acks")
        }
        // Catches the matrix commands too: the SDI switch has one command, full stop.
        (Protocol::Sdi, _) => {
            Some("the SDI switch has exactly one command, AA CC 01 <port>; use switch")
        }
        (
            Protocol::Kvm | Protocol::KvmDual,
            RequestKind::MatrixRoute
            | RequestKind::MatrixRouteAll
            | RequestKind::MatrixMirror
            | RequestKind::MatrixReadRouting
            | RequestKind::MatrixDisplayTimeout
            | RequestKind::MatrixBuzzer
            | RequestKind::MatrixLoadPreset
            | RequestKind::MatrixSavePreset,
        ) => Some("matrix commands only work on HDMI matrices, not on switches"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Port;
    use crate::value::{DisplayTimeout, Output, Step};

    #[test]
    fn protocols_parse_and_print() {
        assert_eq!("kvm".parse(), Ok(Protocol::Kvm));
        assert_eq!("KVM-DUAL".parse(), Ok(Protocol::KvmDual));
        assert_eq!(" matrix ".parse(), Ok(Protocol::Matrix));
        assert_eq!("sdi".parse(), Ok(Protocol::Sdi));
        assert_eq!("hdmi".parse::<Protocol>(), Err(UnknownProtocol));
        assert_eq!(std::format!("{}", Protocol::KvmDual), "kvm-dual");

        // every protocol round-trips through its own name
        for protocol in Protocol::ALL {
            assert_eq!(protocol.name().parse(), Ok(protocol));
        }
    }

    #[test]
    fn only_the_sdi_protocol_is_write_only() {
        assert!(Protocol::Kvm.can_reply());
        assert!(Protocol::Matrix.can_reply());
        assert!(!Protocol::Sdi.can_reply());
        assert!(!Protocol::Sdi.uses_frames(), "AA CC is not the AA BB codec");
    }

    #[test]
    fn the_0x03_clash_is_explained_in_both_directions() {
        let dual = KvmTable
            .encode(
                Request::StepOutput { output: Output::A, step: Step::Next },
                AsciiStyle::DEFAULT,
            )
            .expect_err("stepping an output is a dual-monitor command");
        let message = std::format!("{dual}");
        assert!(message.contains("step-output is not available on the kvm protocol"), "{message}");
        assert!(message.contains("0x03 sets the front display timeout"), "{message}");

        let single = KvmDualTable
            .encode(Request::DisplayTimeout(DisplayTimeout::Never), AsciiStyle::DEFAULT)
            .expect_err("display timeout is a single-monitor command");
        let message = std::format!("{single}");
        assert!(message.contains("0x03 steps output A's input"), "{message}");
    }

    #[test]
    fn matrix_refuses_kvm_commands_with_a_hint() {
        let error = MatrixTable
            .encode(Request::Switch(Port::FIRST), AsciiStyle::DEFAULT)
            .expect_err("matrices have no switch opcode");
        assert!(std::format!("{error}").contains("use matrix-route"));

        let buzzer = MatrixTable
            .encode(Request::Buzzer(crate::value::Buzzer::Muted), AsciiStyle::DEFAULT)
            .expect_err("the matrix buzzer command is a different string");
        assert!(std::format!("{buzzer}").contains("inverted values"), "{buzzer}");
    }

    #[test]
    fn runtime_dispatch_matches_the_static_profiles() {
        let request = Request::Switch(Port::new(2).unwrap());
        for (protocol, expected) in [
            (Protocol::Kvm, KvmTable.encode(request, AsciiStyle::DEFAULT)),
            (Protocol::KvmDual, KvmDualTable.encode(request, AsciiStyle::DEFAULT)),
            (Protocol::Sdi, SdiTable.encode(request, AsciiStyle::DEFAULT)),
        ] {
            assert_eq!(
                protocol.encode(request, AsciiStyle::DEFAULT).map(|t| t.bytes()),
                expected.map(|t| t.bytes())
            );
        }
        assert_eq!(Protocol::Kvm.protocol(), Protocol::Kvm);
        assert!(Protocol::Kvm.uses_frames());
        assert!(!Protocol::Matrix.uses_frames());
    }

    #[test]
    fn support_is_driven_by_the_command_table() {
        assert!(Protocol::Kvm.supports(RequestKind::ReadActivePort));
        assert!(!Protocol::KvmDual.supports(RequestKind::ReadActivePort));
        assert!(Protocol::KvmDual.supports(RequestKind::AutoSwitchInterval));
        assert!(!Protocol::Matrix.supports(RequestKind::Switch));
        assert!(Protocol::Sdi.supports(RequestKind::Switch));
        assert!(!Protocol::Sdi.supports(RequestKind::ReadActivePort));
        assert!(Protocol::Matrix.supports(RequestKind::MatrixBuzzer));
        assert!(!Protocol::Kvm.supports(RequestKind::MatrixBuzzer));

        // every table entry must be encodable, and nothing outside it may be
        for protocol in Protocol::ALL {
            for entry in protocol.commands() {
                assert!(protocol.supports(entry.kind), "{protocol} {}", entry.kind);
            }
        }
    }
}
