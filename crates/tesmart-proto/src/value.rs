//! Validated command arguments.
//!
//! Each setting gets its own type rather than a shared boolean, because the wire values are not
//! consistent: `0x00` means *mute* for the buzzer but *off* for auto input detect, and on
//! dual-monitor switches the keyboard/mouse pass-through values are inverted. Naming each
//! setting keeps those surprises in one place (the profile tables) instead of at every call site.

use core::fmt;
use core::net::Ipv4Addr;
use core::str::FromStr;

/// The buzzer that beeps on a port change.
///
/// Opcode `0x02`, values `0x00` mute / `0x01` unmute. Write-only: no TESmart series can read
/// the buzzer state back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Buzzer {
    /// Silent (`0x00`).
    Muted,
    /// Beeps on switch (`0x01`).
    Audible,
}

impl Buzzer {
    pub(crate) const fn to_byte(self) -> u8 {
        match self {
            Self::Muted => 0x00,
            Self::Audible => 0x01,
        }
    }

    /// The matrix field value, which is **the opposite way round** from [`Self::to_byte`].
    ///
    /// On a KVM, opcode `0x02` with `0x00` mutes. On a matrix, `MT00BZEN01NT` mutes and
    /// `MT00BZEN00NT` unmutes. Both are printed that way in TESmart's own documents (the 8x1
    /// v2 KVM protocol and the 8x8/16x16 matrix protocols respectively), so the inversion is
    /// real and this is the only place that knows about it.
    pub(crate) const fn to_matrix_field(self) -> u8 {
        match self {
            Self::Muted => 1,
            Self::Audible => 0,
        }
    }
}

impl fmt::Display for Buzzer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Muted => "muted",
            Self::Audible => "audible",
        })
    }
}

/// How long the front panel display stays lit after a change.
///
/// Opcode `0x03` on single-monitor KVMs, values `0x0A` / `0x1E` / `0x00`. The same opcode means
/// something entirely different on dual-monitor switches, which is why it lives behind a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum DisplayTimeout {
    /// 10 seconds (`0x0A`).
    Seconds10,
    /// 30 seconds (`0x1E`).
    Seconds30,
    /// Never turn the display off (`0x00`).
    Never,
}

impl DisplayTimeout {
    pub(crate) const fn to_byte(self) -> u8 {
        match self {
            Self::Seconds10 => 0x0A,
            Self::Seconds30 => 0x1E,
            Self::Never => 0x00,
        }
    }
}

impl fmt::Display for DisplayTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Seconds10 => "10s",
            Self::Seconds30 => "30s",
            Self::Never => "never",
        })
    }
}
