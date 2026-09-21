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

/// A plain on/off setting whose wire encoding is `0x00` off, `0x01` on.
///
/// Named `Toggle` rather than `Switch` because in this problem domain "switch" is the device,
/// and `Switch::On` sitting next to [`Request::Switch`](crate::Request::Switch) reads as if it
/// were about choosing an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Toggle {
    /// Disabled (`0x00`).
    Off,
    /// Enabled (`0x01`).
    On,
}

impl Toggle {
    pub(crate) const fn to_byte(self) -> u8 {
        match self {
            Self::Off => 0x00,
            Self::On => 0x01,
        }
    }

    /// The inverted encoding used by the dual-monitor keyboard/mouse pass-through command.
    pub(crate) const fn to_inverted_byte(self) -> u8 {
        match self {
            Self::Off => 0x01,
            Self::On => 0x00,
        }
    }
}

impl fmt::Display for Toggle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::On => "on",
        })
    }
}

/// One of the two video outputs of a dual-monitor switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Output {
    /// Output A, opcode `0x03`.
    A,
    /// Output B, opcode `0x04`.
    B,
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::A => "A",
            Self::B => "B",
        })
    }
}

/// A relative input change, the only way to steer a dual-monitor output.
///
/// TESmart support confirmed to the author of `spuder/Tesmart-KVM` that there is no absolute
/// per-output switch command; stepping is the closest equivalent to the front buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Step {
    /// One input down (`0x00`).
    Previous,
    /// One input up (`0x01`).
    Next,
}

impl Step {
    /// The wire value: `0x00` previous, `0x01` next.
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Previous => 0x00,
            Self::Next => 0x01,
        }
    }

    /// Reads a wire value, which a device does when it receives a step command.
    ///
    /// Anything that is not `0x00` is next, matching how the hardware behaves: the documented
    /// values are the only two, and a switch does not reject a third.
    pub const fn from_byte(raw: u8) -> Self {
        if raw == 0x00 { Self::Previous } else { Self::Next }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Previous => "previous",
            Self::Next => "next",
        })
    }
}

/// The key that starts a hotkey sequence on dual-monitor switches (opcode `0x07`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum HotkeyTrigger {
    /// Right Ctrl, pressed twice (`0x01`).
    RightCtrl,
    /// Scroll Lock, pressed twice (`0x00`).
    ScrollLock,
}

impl HotkeyTrigger {
    pub(crate) const fn to_byte(self) -> u8 {
        match self {
            Self::RightCtrl => 0x01,
            Self::ScrollLock => 0x00,
        }
    }
}

impl fmt::Display for HotkeyTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RightCtrl => "right-ctrl",
            Self::ScrollLock => "scroll-lock",
        })
    }
}
