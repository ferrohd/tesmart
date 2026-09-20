//! Port numbers, and the one place the protocol's asymmetric numbering is handled.
//!
//! TESmart's hex protocol is asymmetric: a *write* carries a 1-based port (`0x01` = PC1) while
//! the read reply carries a 0-based port (`0x00` = PC1). Every API in this crate speaks the
//! human port number, and the two conversions below are the only `±1` in the workspace.
//!
//! Source: `docs/protocol-reference.md`, "Port numbering is asymmetric"; the same encoding is
//! used by the PiKVM `kvmd` driver.

use core::fmt;
use core::num::NonZeroU8;
use core::str::FromStr;

/// A port number as humans and front panels count them: PC1 is `1`.
///
/// ```
/// use tesmart_proto::Port;
///
/// assert_eq!(Port::new(1).unwrap().get(), 1);
/// assert!(Port::new(0).is_err());
/// assert!(Port::new(17).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "u8", into = "u8"))]
pub struct Port(NonZeroU8);

impl Port {
    /// The highest port any documented TESmart switch exposes (16x1 models).
    pub const MAX: u8 = 16;

    /// PC1.
    pub const FIRST: Self = match NonZeroU8::new(1) {
        Some(one) => Self(one),
        None => panic!("one is not zero"),
    };

    /// Creates a port from a human port number.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::OutOfRange`] unless `1 <= human <= 16`. Whether the *attached*
    /// switch has that many ports is a separate question, answered by [`PortCount::contains`].
    pub const fn new(human: u8) -> Result<Self, PortError> {
        match NonZeroU8::new(human) {
            Some(port) if human <= Self::MAX => Ok(Self(port)),
            _ => Err(PortError::OutOfRange { value: human, max: Self::MAX }),
        }
    }

    /// The human port number.
    pub const fn get(self) -> u8 {
        self.0.get()
    }

    /// The byte used in a switch command: 1-based, `0x01` = PC1.
    ///
    /// Public because the protocol has two sides: `tesmart-device` encodes port reports and
    /// decodes switch commands, and the `±1` asymmetry must live in exactly one place rather
    /// than being re-derived per crate.
    pub const fn to_write_byte(self) -> u8 {
        self.0.get()
    }

    /// The byte used in a port report: 0-based, `0x00` = PC1.
    pub const fn to_read_byte(self) -> u8 {
        self.0.get() - 1
    }

    /// Reads the 0-based port index out of a `0x11` port report.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::ReplyOutOfRange`] for `raw > 0x0F`. Garbage read replies do happen
    /// on real hardware, so this is a retryable condition rather than a parse failure.
    pub const fn from_read_byte(raw: u8) -> Result<Self, PortError> {
        if raw >= Self::MAX {
            return Err(PortError::ReplyOutOfRange { raw });
        }
        Self::new(raw + 1)
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

impl From<Port> for u8 {
    fn from(port: Port) -> Self {
        port.get()
    }
}

impl TryFrom<u8> for Port {
    type Error = PortError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for Port {
    type Err = PortError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value: u8 = s.trim().parse().map_err(|_| PortError::Malformed)?;
        Self::new(value)
    }
}

/// Something was wrong with a port number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PortError {
    /// A port number outside the valid range.
    #[error("port {value} is out of range (1..={max})")]
    OutOfRange {
        /// The rejected value.
        value: u8,
        /// The highest accepted value.
        max: u8,
    },

    /// A port report whose 0-based index cannot be a real port.
    ///
    /// Switches do occasionally return garbage read replies; the engine retries instead of
    /// reporting an error to the caller.
    #[error("port report carried an out of range index 0x{raw:02X} (expected 0x00..=0x0F)")]
    ReplyOutOfRange {
        /// The byte that was in the report.
        raw: u8,
    },

    /// Text that is not a port number at all.
    #[error("expected a port number (1..=16)")]
    Malformed,
}
