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

use core::fmt;
use core::str::FromStr;

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
