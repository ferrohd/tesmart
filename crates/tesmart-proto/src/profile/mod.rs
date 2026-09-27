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
