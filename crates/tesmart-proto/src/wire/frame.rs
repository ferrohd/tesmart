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
