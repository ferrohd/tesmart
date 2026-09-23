//! The hex frame codec.
//!
//! ```text
//! AA BB <len> <opcode> <data...> <terminator>
//! ```
//!
//! Every official TESmart document shows `len = 0x03`, which is why it is often described as a
//! constant. It is not: the community-documented dual-monitor auto-switch interval command sends
//! `AA BB 04 09 MM SS EE`, a seven byte frame carrying two data bytes. Reading byte 2 as
//! `data.len() + 2` explains every frame ever observed, so that is what this codec does.
//!
//! Sources: the 8x1 / 16x1 / 4x1 DVI protocol PDFs, and `spuder/Tesmart-KVM` for the 7 byte
//! frame.

use crate::wire::{Transmit, TransmitKind, WireBytes};

/// Byte 0 and 1 of every frame.
pub const HEADER: [u8; 2] = [0xAA, 0xBB];

/// The documented terminator.
///
/// Replies ending in `0x16` or `0x1C` have been observed in the field, so the decoder accepts
/// any terminator byte; only the encoder uses this constant.
pub const TERMINATOR: u8 = 0xEE;

/// The largest value of the length byte the decoder will believe.
///
/// Known frames use `0x03` and `0x04`. A corrupted length byte must not make the decoder wait
/// for hundreds of bytes that will never arrive, so anything above this is treated as noise and
/// the decoder resynchronises past the header.
pub const MAX_LEN_BYTE: u8 = 0x08;

/// The only opcode a switch is documented to *send*: the reply to a read-port request.
///
/// It lives here rather than in a profile because the decoder needs it to turn a frame into a
/// port number; profiles decide what, if anything, it means for their family.
pub const OPCODE_PORT_REPORT: u8 = 0x11;

/// The data bytes of a frame.
///
/// One byte for every documented command, two for the dual-monitor auto-switch interval. Making
/// this an enum rather than a slice means the length byte is always derived and never typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Payload {
    /// A single argument byte, `len = 0x03`.
    One(u8),
    /// Two argument bytes, `len = 0x04`.
    Two([u8; 2]),
}

impl Payload {
    /// How many data bytes.
    pub const fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Two(_) => 2,
        }
    }

    /// Always false: every frame carries at least one data byte.
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// The data bytes.
    pub const fn as_slice(&self) -> &[u8] {
        match self {
            Self::One(byte) => core::slice::from_ref(byte),
            Self::Two(bytes) => bytes.as_slice(),
        }
    }
}

/// A decoded or ready-to-encode hex frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Frame {
    /// Byte 3: the command.
    pub opcode: u8,
    /// Bytes 4..: the argument.
    pub payload: Payload,
}

impl Frame {
    /// A frame with one argument byte.
    pub const fn one(opcode: u8, value: u8) -> Self {
        Self { opcode, payload: Payload::One(value) }
    }

    /// A frame with two argument bytes.
    pub const fn two(opcode: u8, values: [u8; 2]) -> Self {
        Self { opcode, payload: Payload::Two(values) }
    }

    /// Byte 2: `data.len() + 2`.
    pub const fn len_byte(&self) -> u8 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Payload::len() is 1 or 2 by construction"
        )]
        let data_len = self.payload.len() as u8;
        data_len + 2
    }

    /// The whole frame on the wire: 6 bytes for a one-byte payload, 7 for two.
    pub fn encode(&self) -> WireBytes {
        let data = self.payload.as_slice();
        let mut raw = [0u8; 7];
        raw[0] = HEADER[0];
        raw[1] = HEADER[1];
        raw[2] = self.len_byte();
        raw[3] = self.opcode;
        raw[4..4 + data.len()].copy_from_slice(data);
        raw[4 + data.len()] = TERMINATOR;
        WireBytes::from_bounded(&raw[..5 + data.len()])
    }

    /// The frame as a [`Transmit`].
    pub(crate) fn to_transmit(self) -> Transmit {
        Transmit::new(self.encode(), TransmitKind::Frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_documented_six_byte_frame() {
        // docs/protocol-reference.md: "switch to PC2" is AA BB 03 01 02 EE
        assert_eq!(
            Frame::one(0x01, 0x02).encode().as_slice(),
            &[0xAA, 0xBB, 0x03, 0x01, 0x02, 0xEE]
        );
    }

    #[test]
    fn encodes_the_seven_byte_dual_monitor_frame() {
        // AA BB 04 09 MM SS EE: the length byte follows the payload
        let frame = Frame::two(0x09, [0x00, 0x05]);
        assert_eq!(frame.len_byte(), 0x04);
        assert_eq!(frame.encode().as_slice(), &[0xAA, 0xBB, 0x04, 0x09, 0x00, 0x05, 0xEE]);
    }

    #[test]
    fn length_byte_is_derived_from_the_payload() {
        assert_eq!(Frame::one(0x01, 0x01).len_byte(), 0x03);
        assert_eq!(Payload::One(0).len(), 1);
        assert_eq!(Payload::Two([0, 0]).len(), 2);
        assert_eq!(Payload::One(0x7F).as_slice(), &[0x7F]);
        assert_eq!(Payload::Two([1, 2]).as_slice(), &[1, 2]);
    }
}
