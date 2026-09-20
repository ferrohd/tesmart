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

/// How many ports a particular switch has (4, 8 or 16 for documented models).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "u8", into = "u8"))]
pub struct PortCount(NonZeroU8);

impl PortCount {
    /// One: the smallest a switch can have, and what a single-monitor KVM has outputs of.
    pub const MIN: Self = Self::known(1);

    /// Creates a port count.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::OutOfRange`] unless `1 <= count <= 16`.
    pub const fn new(count: u8) -> Result<Self, PortError> {
        match NonZeroU8::new(count) {
            Some(count_nz) if count <= Port::MAX => Ok(Self(count_nz)),
            _ => Err(PortError::OutOfRange { value: count, max: Port::MAX }),
        }
    }

    /// Creates a port count in a const context, panicking on an invalid value.
    ///
    /// Used by the model registry, where the values are literals checked at compile time.
    ///
    /// # Panics
    ///
    /// Panics if `count` is not in `1..=16`.
    pub(crate) const fn known(count: u8) -> Self {
        assert!(matches!(count, 1..=Port::MAX), "invalid port count in the model registry");
        match NonZeroU8::new(count) {
            Some(count_nz) => Self(count_nz),
            None => panic!("invalid port count in the model registry"),
        }
    }

    /// The number of ports.
    pub const fn get(self) -> u8 {
        self.0.get()
    }

    /// The highest port on this switch.
    pub const fn last(self) -> Port {
        match Port::new(self.0.get()) {
            Ok(port) => port,
            Err(_) => Port::FIRST,
        }
    }

    /// Whether `port` exists on this switch.
    pub const fn contains(self, port: Port) -> bool {
        port.get() <= self.0.get()
    }

    /// Validates `port` against this switch.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::OutOfRange`] if the switch has fewer ports than `port`.
    pub const fn check(self, port: Port) -> Result<Port, PortError> {
        if self.contains(port) {
            Ok(port)
        } else {
            Err(PortError::OutOfRange { value: port.get(), max: self.0.get() })
        }
    }
}

impl fmt::Display for PortCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

impl From<PortCount> for u8 {
    fn from(count: PortCount) -> Self {
        count.get()
    }
}

impl TryFrom<u8> for PortCount {
    type Error = PortError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_are_one_based_and_reads_are_zero_based() {
        // docs/protocol-reference.md: write 0x01 = PC1, read reply 0x00 = PC1
        let pc1 = Port::new(1).unwrap();
        assert_eq!(pc1.to_write_byte(), 0x01);
        assert_eq!(pc1.to_read_byte(), 0x00);

        let pc16 = Port::new(16).unwrap();
        assert_eq!(pc16.to_write_byte(), 0x10);
        assert_eq!(pc16.to_read_byte(), 0x0F);

        assert_eq!(Port::from_read_byte(0x00), Ok(pc1));
        assert_eq!(Port::from_read_byte(0x0F), Ok(pc16));
    }

    #[test]
    fn read_byte_round_trips_for_every_port() {
        for human in 1..=Port::MAX {
            let port = Port::new(human).unwrap();
            assert_eq!(Port::from_read_byte(port.to_read_byte()), Ok(port));
        }
    }

    #[test]
    fn rejects_out_of_range_values() {
        assert_eq!(Port::new(0), Err(PortError::OutOfRange { value: 0, max: 16 }));
        assert_eq!(Port::new(17), Err(PortError::OutOfRange { value: 17, max: 16 }));
        // 0x10 in a *reply* is one past the last port; it is not PC1, whatever tesmart.sh does
        assert_eq!(Port::from_read_byte(0x10), Err(PortError::ReplyOutOfRange { raw: 0x10 }));
        assert_eq!(Port::from_read_byte(0xFF), Err(PortError::ReplyOutOfRange { raw: 0xFF }));
    }

    #[test]
    fn port_count_gates_per_model_ranges() {
        let four = PortCount::new(4).unwrap();
        let pc3 = Port::new(3).unwrap();
        let pc8 = Port::new(8).unwrap();

        assert!(four.contains(pc3));
        assert!(!four.contains(pc8));
        assert_eq!(four.check(pc3), Ok(pc3));
        assert_eq!(four.check(pc8), Err(PortError::OutOfRange { value: 8, max: 4 }));
        assert_eq!(four.last(), Port::new(4).unwrap());
    }

    #[test]
    fn parses_and_displays() {
        assert_eq!("3".parse::<Port>(), Ok(Port::new(3).unwrap()));
        assert_eq!(" 16 ".parse::<Port>(), Ok(Port::new(16).unwrap()));
        assert_eq!("0".parse::<Port>(), Err(PortError::OutOfRange { value: 0, max: 16 }));
        assert_eq!("nas".parse::<Port>(), Err(PortError::Malformed));
        assert_eq!(std::format!("{}", Port::new(7).unwrap()), "7");
    }
}
