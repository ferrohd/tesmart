//! Protocol families, as declarative command tables.
//!
//! TESmart uses four incompatible control protocols and **the same opcode byte means different
//! things across them**. `0x03` sets the front display timeout on a single-monitor KVM and steps
//! output A's input on a dual-monitor one. A profile is therefore not a formality: it is the
//! thing that stops a display-timeout command from silently changing which computer you are
//! looking at.
//!
//! | Profile | Family | Devices |
//! | --- | --- | --- |
//! | [`Kvm`] | A | single-monitor 4/8/16-port KVMs with RS232 or LAN |
//! | [`KvmDual`] | A' | 4-port dual-monitor KVMs (community documented) |
//! | [`Matrix`] | B | HDMI matrices |
//! | [`Sdi`] | C | the 4x1 SDI switch, which shares no framing with the rest |
//!
//! A request that is not in the active profile is refused by [`Profile::encode`] before
//! anything is sent, and the error says what the opcode would have done instead.
//!
//! The trait is sealed: the set of families is closed, which is what lets every table be an
//! exhaustive `match` and makes a new [`Request`] variant a compile error until all four
//! families have decided what to do with it. Unknown models are handled by choosing the family
//! that fits, not by writing a fifth profile.

pub mod base;
pub mod dual;
pub mod kvm;
pub mod matrix;
pub mod sdi;

use core::fmt;
use core::str::FromStr;

pub use crate::profile::dual::KvmDual;
pub use crate::profile::kvm::Kvm;
pub use crate::profile::matrix::Matrix;
pub use crate::profile::sdi::Sdi;

use crate::command::{CommandInfo, Request, RequestKind, Response};
use crate::wire::{AsciiStyle, Chunk, Transmit};

mod sealed {
    pub trait Sealed {}
}

/// The command table of one protocol family.
pub trait Profile: Copy + sealed::Sealed {
    /// Which family this is.
    fn family(self) -> Family;

    /// Encodes a request, or refuses it.
    ///
    /// # Errors
    ///
    /// Returns [`Unsupported`] if the request has no meaning in this family.
    fn encode(self, request: Request, style: AsciiStyle) -> Result<Transmit, Unsupported>;

    /// Interprets one decoded chunk, if it means anything in this family.
    fn decode(self, chunk: &Chunk<'_>) -> Option<Response>;

    /// Every command this family understands, for help output and capability reporting.
    fn commands(self) -> &'static [CommandInfo];

    /// Whether this family understands a kind of request.
    fn supports(self, kind: RequestKind) -> bool {
        self.command(kind).is_some()
    }

    /// The table entry for a kind of request.
    fn command(self, kind: RequestKind) -> Option<&'static CommandInfo> {
        self.commands().iter().find(|entry| entry.kind == kind)
    }
}

/// A protocol family, chosen at run time.
///
/// Deliberately has no `Default`: the family must come from the model registry or from an
/// explicit choice by the user. Guessing it from a model name is how you end up sending
/// `0x03` to the wrong kind of switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Family {
    /// Family A: single-monitor KVMs. See [`Kvm`].
    Kvm,
    /// Family A': dual-monitor KVMs. See [`KvmDual`].
    KvmDual,
    /// Family B: HDMI matrices. See [`Matrix`].
    Matrix,
    /// Family C: the 4x1 SDI switch. See [`Sdi`].
    Sdi,
}

impl Family {
    /// Every family, in documentation order.
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

    /// Whether this family uses the `AA BB` hex frame codec.
    ///
    /// False for [`Family::Sdi`], which sends binary but not *that* binary: its four-byte
    /// `AA CC 01 0n` commands are not frames this crate's codec can read or write, and it never
    /// replies, so there is nothing to decode either.
    pub const fn uses_frames(self) -> bool {
        matches!(self, Self::Kvm | Self::KvmDual)
    }

    /// Whether a switch in this family can report anything at all.
    ///
    /// False for [`Family::Sdi`]: no read command, no acknowledgement, nothing. A driver can
    /// use this to skip waiting for replies that are never coming.
    pub const fn can_reply(self) -> bool {
        !matches!(self, Self::Sdi)
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Family {
    type Err = UnknownFamily;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|family| s.eq_ignore_ascii_case(family.name()))
            .ok_or(UnknownFamily)
    }
}

/// Text that does not name a protocol family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("expected one of kvm, kvm-dual, matrix, sdi")]
pub struct UnknownFamily;

impl sealed::Sealed for Family {}

/// Runtime dispatch over the four families.
impl Profile for Family {
    fn family(self) -> Family {
        self
    }

    fn encode(self, request: Request, style: AsciiStyle) -> Result<Transmit, Unsupported> {
        match self {
            Self::Kvm => Kvm.encode(request, style),
            Self::KvmDual => KvmDual.encode(request, style),
            Self::Matrix => Matrix.encode(request, style),
            Self::Sdi => Sdi.encode(request, style),
        }
    }

    fn decode(self, chunk: &Chunk<'_>) -> Option<Response> {
        match self {
            Self::Kvm => Kvm.decode(chunk),
            Self::KvmDual => KvmDual.decode(chunk),
            Self::Matrix => Matrix.decode(chunk),
            Self::Sdi => Sdi.decode(chunk),
        }
    }

    fn commands(self) -> &'static [CommandInfo] {
        match self {
            Self::Kvm => Kvm.commands(),
            Self::KvmDual => KvmDual.commands(),
            Self::Matrix => Matrix.commands(),
            Self::Sdi => Sdi.commands(),
        }
    }
}

/// A request that has no meaning in the active family.
///
/// The [`Display`](fmt::Display) implementation explains what the opcode would have done
/// instead, where the families clash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    /// The family that refused the request.
    pub family: Family,
    /// The request that was refused.
    pub request: RequestKind,
}

impl Unsupported {
    pub(crate) const fn new(family: Family, request: RequestKind) -> Self {
        Self { family, request }
    }

    /// Why this combination is refused, when there is something interesting to say.
    pub const fn note(self) -> Option<&'static str> {
        clash_note(self.family, self.request)
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not available on the {} profile", self.request, self.family)?;
        match self.note() {
            Some(note) => write!(f, ": {note}"),
            None => Ok(()),
        }
    }
}

impl core::error::Error for Unsupported {}

/// The interesting half of an [`Unsupported`] message.
const fn clash_note(family: Family, request: RequestKind) -> Option<&'static str> {
    match (family, request) {
        (Family::KvmDual, RequestKind::DisplayTimeout) => Some(
            "on dual-monitor switches opcode 0x03 steps output A's input instead of setting the \
             display timeout",
        ),
        (Family::Kvm, RequestKind::StepOutput) => Some(
            "on single-monitor switches opcode 0x03 sets the front display timeout; use switch \
             to choose a port",
        ),
        (Family::Kvm, RequestKind::MoveUsb | RequestKind::KmPassthrough) => {
            Some("this command only exists on dual-monitor switches")
        }
        (
            Family::Kvm,
            RequestKind::HotkeyTrigger | RequestKind::AutoSwitch | RequestKind::AutoSwitchInterval,
        ) => Some("this command is only documented for dual-monitor switches"),
        (Family::KvmDual, RequestKind::ReadActivePort) => {
            Some("no read command is documented for dual-monitor switches")
        }
        (Family::KvmDual, RequestKind::NetQuery | RequestKind::NetSet) => {
            Some("network configuration is not documented for dual-monitor switches")
        }
        (Family::KvmDual, RequestKind::AutoInputDetect) => {
            Some("auto input detect is not documented for dual-monitor switches")
        }
        (Family::Matrix, RequestKind::Switch) => {
            Some("matrices route an input to an output; use matrix-route")
        }
        (Family::Matrix, RequestKind::DisplayTimeout) => {
            Some("matrices use their own display timeout command; use matrix-display-timeout")
        }
        (Family::Matrix, RequestKind::Buzzer) => {
            Some("matrices use their own buzzer command, with inverted values; use matrix-buzzer")
        }
        (Family::Matrix, RequestKind::ReadActivePort) => {
            Some("a matrix has no single active port; read its routing instead")
        }
        (Family::Matrix, _) => Some("matrices understand only the ASCII matrix commands"),
        (Family::Sdi, RequestKind::ReadActivePort) => {
            Some("the SDI switch does not report anything: it has no read command and never acks")
        }
        // Catches the matrix commands too: the SDI switch has one command, full stop.
        (Family::Sdi, _) => {
            Some("the SDI switch has exactly one command, AA CC 01 <port>; use switch")
        }
        (
            Family::Kvm | Family::KvmDual,
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
    fn families_parse_and_print() {
        assert_eq!("kvm".parse(), Ok(Family::Kvm));
        assert_eq!("KVM-DUAL".parse(), Ok(Family::KvmDual));
        assert_eq!(" matrix ".parse(), Ok(Family::Matrix));
        assert_eq!("sdi".parse(), Ok(Family::Sdi));
        assert_eq!("hdmi".parse::<Family>(), Err(UnknownFamily));
        assert_eq!(std::format!("{}", Family::KvmDual), "kvm-dual");

        // every family round-trips through its own name
        for family in Family::ALL {
            assert_eq!(family.name().parse(), Ok(family));
        }
    }

    #[test]
    fn only_the_sdi_family_is_write_only() {
        assert!(Family::Kvm.can_reply());
        assert!(Family::Matrix.can_reply());
        assert!(!Family::Sdi.can_reply());
        assert!(!Family::Sdi.uses_frames(), "AA CC is not the AA BB codec");
    }

    #[test]
    fn the_0x03_clash_is_explained_in_both_directions() {
        let dual = Kvm
            .encode(
                Request::StepOutput { output: Output::A, step: Step::Next },
                AsciiStyle::DEFAULT,
            )
            .expect_err("stepping an output is a dual-monitor command");
        let message = std::format!("{dual}");
        assert!(message.contains("step-output is not available on the kvm profile"), "{message}");
        assert!(message.contains("0x03 sets the front display timeout"), "{message}");

        let single = KvmDual
            .encode(Request::DisplayTimeout(DisplayTimeout::Never), AsciiStyle::DEFAULT)
            .expect_err("display timeout is a single-monitor command");
        let message = std::format!("{single}");
        assert!(message.contains("0x03 steps output A's input"), "{message}");
    }

    #[test]
    fn matrix_refuses_kvm_commands_with_a_hint() {
        let error = Matrix
            .encode(Request::Switch(Port::FIRST), AsciiStyle::DEFAULT)
            .expect_err("matrices have no switch opcode");
        assert!(std::format!("{error}").contains("use matrix-route"));

        let buzzer = Matrix
            .encode(Request::Buzzer(crate::value::Buzzer::Muted), AsciiStyle::DEFAULT)
            .expect_err("the matrix buzzer command is a different string");
        assert!(std::format!("{buzzer}").contains("inverted values"), "{buzzer}");
    }

    #[test]
    fn runtime_dispatch_matches_the_static_profiles() {
        let request = Request::Switch(Port::new(2).unwrap());
        for (family, expected) in [
            (Family::Kvm, Kvm.encode(request, AsciiStyle::DEFAULT)),
            (Family::KvmDual, KvmDual.encode(request, AsciiStyle::DEFAULT)),
            (Family::Sdi, Sdi.encode(request, AsciiStyle::DEFAULT)),
        ] {
            assert_eq!(
                family.encode(request, AsciiStyle::DEFAULT).map(|t| t.bytes()),
                expected.map(|t| t.bytes())
            );
        }
        assert_eq!(Family::Kvm.family(), Family::Kvm);
        assert!(Family::Kvm.uses_frames());
        assert!(!Family::Matrix.uses_frames());
    }

    #[test]
    fn support_is_driven_by_the_command_table() {
        assert!(Family::Kvm.supports(RequestKind::ReadActivePort));
        assert!(!Family::KvmDual.supports(RequestKind::ReadActivePort));
        assert!(Family::KvmDual.supports(RequestKind::AutoSwitchInterval));
        assert!(!Family::Matrix.supports(RequestKind::Switch));
        assert!(Family::Sdi.supports(RequestKind::Switch));
        assert!(!Family::Sdi.supports(RequestKind::ReadActivePort));
        assert!(Family::Matrix.supports(RequestKind::MatrixBuzzer));
        assert!(!Family::Kvm.supports(RequestKind::MatrixBuzzer));

        // every table entry must be encodable, and nothing outside it may be
        for family in Family::ALL {
            for entry in family.commands() {
                assert!(family.supports(entry.kind), "{family} {}", entry.kind);
            }
        }
    }
}
