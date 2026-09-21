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

/// The dwell time of the dual-monitor auto-switch mode (opcode `0x09`).
///
/// The frame carries two data bytes, `AA BB 04 09 MM SS EE`, each documented only as
/// "`0x00`-`0x38`, minimum 5 s". That `MM` is minutes and `SS` is seconds is an *inference* from
/// the field layout, not something TESmart documents, so [`SwitchInterval::from_raw`] is the
/// primary constructor and [`SwitchInterval::from_secs`] is the convenience built on top of the
/// guess.
///
/// Source: `spuder/Tesmart-KVM`, tested over RS232 on a 4-port dual-monitor switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SwitchInterval {
    minutes: u8,
    seconds: u8,
}

impl SwitchInterval {
    /// The largest value either field accepts (`0x38` = 56).
    pub const MAX_FIELD: u8 = 0x38;

    /// The shortest interval the switch accepts.
    pub const MIN_SECONDS: u16 = 5;

    /// Builds an interval from the two raw frame bytes.
    ///
    /// # Errors
    ///
    /// Returns [`IntervalError::FieldOutOfRange`] if either field exceeds `0x38`, or
    /// [`IntervalError::TooShort`] if the implied total is under 5 s.
    pub const fn from_raw(minutes: u8, seconds: u8) -> Result<Self, IntervalError> {
        if minutes > Self::MAX_FIELD {
            return Err(IntervalError::FieldOutOfRange { value: minutes });
        }
        if seconds > Self::MAX_FIELD {
            return Err(IntervalError::FieldOutOfRange { value: seconds });
        }
        let total = minutes as u16 * 60 + seconds as u16;
        if total < Self::MIN_SECONDS {
            return Err(IntervalError::TooShort { seconds: total });
        }
        Ok(Self { minutes, seconds })
    }

    /// Builds an interval from a number of seconds, assuming the `MM SS` reading of the frame.
    ///
    /// # Errors
    ///
    /// Returns [`IntervalError::TooShort`] below 5 s and [`IntervalError::FieldOutOfRange`]
    /// above `0x38` minutes (56 min 56 s is the largest representable interval).
    pub const fn from_secs(total: u16) -> Result<Self, IntervalError> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "both quotient and remainder of a u16 by 60 are checked against 0x38 below"
        )]
        let (minutes, seconds) = ((total / 60) as u8, (total % 60) as u8);
        Self::from_raw(minutes, seconds)
    }

    /// The two data bytes, in frame order.
    pub const fn to_raw(self) -> (u8, u8) {
        (self.minutes, self.seconds)
    }

    /// The interval in seconds, under the inferred `MM SS` reading.
    pub const fn as_secs(self) -> u16 {
        self.minutes as u16 * 60 + self.seconds as u16
    }
}

impl fmt::Display for SwitchInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}m{}s", self.minutes, self.seconds)
    }
}

/// An interval outside what the switch accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IntervalError {
    /// A field above the documented `0x38` ceiling.
    #[error("auto-switch interval field {value} is out of range (0..=0x38)")]
    FieldOutOfRange {
        /// The rejected field value.
        value: u8,
    },

    /// Below the documented 5 second minimum.
    #[error("auto-switch interval of {seconds}s is below the 5s minimum")]
    TooShort {
        /// The rejected total, in seconds.
        seconds: u16,
    },
}

/// A display timeout for HDMI matrices, in seconds (`MT00CTXXXXNT`, `0000`-`9999`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MatrixTimeout(u16);

impl MatrixTimeout {
    /// Keep the display on forever (`0000`).
    pub const NEVER: Self = Self(0);

    /// The largest value the four-digit field can carry.
    pub const MAX_SECONDS: u16 = 9999;

    /// Builds a matrix display timeout.
    ///
    /// # Errors
    ///
    /// Returns [`MatrixTimeoutError`] above 9999 seconds.
    pub const fn from_secs(seconds: u16) -> Result<Self, MatrixTimeoutError> {
        if seconds > Self::MAX_SECONDS {
            return Err(MatrixTimeoutError { seconds });
        }
        Ok(Self(seconds))
    }

    /// The timeout in seconds, `0` meaning never.
    pub const fn as_secs(self) -> u16 {
        self.0
    }
}

impl fmt::Display for MatrixTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => f.write_str("never"),
            seconds => write!(f, "{seconds}s"),
        }
    }
}

/// A matrix display timeout that does not fit the four-digit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("matrix display timeout of {seconds}s is out of range (0..=9999)")]
pub struct MatrixTimeoutError {
    /// The rejected value.
    pub seconds: u16,
}

/// One of a matrix's eight stored routing presets.
///
/// `MT00SV00XXNT` saves the current routing into slot `XX`, `MT00RD01XXNT` loads it back, with
/// `XX` in `01`-`08`. Documented for the 8x8 and 16x16 matrices only; the 4x4 documents have no
/// preset commands at all.
///
/// ```
/// use tesmart_proto::value::Preset;
///
/// assert_eq!(Preset::new(1).unwrap().get(), 1);
/// assert!(Preset::new(9).is_err(), "there are only eight slots");
/// assert!(Preset::new(0).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Preset(u8);

impl Preset {
    /// The first slot.
    pub const FIRST: Self = Self(1);

    /// How many slots a matrix has.
    pub const COUNT: u8 = 8;

    /// Builds a preset slot number.
    ///
    /// # Errors
    ///
    /// Returns [`PresetError`] outside `1..=8`.
    pub const fn new(number: u8) -> Result<Self, PresetError> {
        if number == 0 || number > Self::COUNT {
            return Err(PresetError { number });
        }
        Ok(Self(number))
    }

    /// The slot number, as the documents count them.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Preset {
    type Err = PresetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let number = s.parse().map_err(|_| PresetError { number: 0 })?;
        Self::new(number)
    }
}

/// A preset slot a matrix does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("preset {number} is out of range (1..=8)")]
pub struct PresetError {
    /// The rejected slot number.
    pub number: u8,
}
