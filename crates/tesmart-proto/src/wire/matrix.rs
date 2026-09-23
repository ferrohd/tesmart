//! The matrix command language: `MT00…NT` strings and the `LINK:` routing report.
//!
//! Family B speaks a different language from everything else TESmart makes. It shares the
//! transport with the network side-channel in [`ascii`](crate::wire::ascii), and nothing else:
//! the grammar, the vocabulary and the reply format are all its own.
//!
//! | Command | Meaning |
//! | --- | --- |
//! | `MT00SWXXYYNT` | route input `XX` to output `YY` |
//! | `MT00SWXX00NT` | route input `XX` to every output |
//! | `MT00SW0000NT` | mirror 1 to 1, 2 to 2, ... |
//! | `MT00RD0000NT` | request routing; answered `LINK:OYIX;END` |
//! | `MT00RD01XXNT` | load routing preset `XX` |
//! | `MT00SV00XXNT` | save routing into preset `XX` |
//! | `MT00BZEN01NT` | mute the buzzer; `00NT` unmutes |
//! | `MT00CTXXXXNT` | display timeout in seconds |
//!
//! Two traps live in that table. `MT00RD` is **two commands**: `MT00RD0000NT` reads the routing
//! while `MT00RD01XXNT` loads a preset, one character apart. And the buzzer values are
//! **inverted** against the KVM `0x02` command, where `0x00` mutes.
//!
//! Encoding the commands is the [`Matrix`](crate::profile::Matrix) profile's job; this module
//! parses them, which is what the device emulator needs, and owns the routing report in both
//! directions.
//!
//! Sources: TESmart's 4x4 (RS232), 4x4 (RS232+IP), 8x8 and 16x16 matrix protocol documents.

use core::fmt::{self, Write as _};

use crate::port::Port;
use crate::value::{Buzzer, MatrixTimeout, Preset};
use crate::wire::WireBytes;
use crate::wire::text::strip_prefix_ignore_case;

/// A matrix command, as seen from the matrix.
///
/// The `MT00`...`NT` envelope is stripped, and the fields are decoded into the values they
/// stand for. Used by the device emulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MatrixCommand {
    /// Route one input to one output (`MT00SWXXYYNT`).
    Route {
        /// The input to take.
        input: Port,
        /// The output to feed.
        output: Port,
    },
    /// Route one input to every output (`MT00SWXX00NT`).
    RouteAll(Port),
    /// Mirror input 1 to output 1, 2 to 2, and so on (`MT00SW0000NT`).
    Mirror,
    /// Report the current routing (`MT00RD0000NT`).
    ReadRouting,
    /// Load a stored routing preset (`MT00RD01XXNT`).
    LoadPreset(Preset),
    /// Save the current routing into a preset (`MT00SV00XXNT`).
    SavePreset(Preset),
    /// Set the display timeout in seconds (`MT00CTXXXXNT`).
    DisplayTimeout(MatrixTimeout),
    /// Mute or unmute the buzzer (`MT00BZEN01NT` mutes).
    Buzzer(Buzzer),
}
