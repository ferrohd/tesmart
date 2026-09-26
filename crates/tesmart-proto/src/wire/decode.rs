//! The resynchronising stream decoder.
//!
//! The LAN port of a TESmart switch is a serial-to-Ethernet bridge, so everything that can go
//! wrong on a serial line can go wrong over TCP too. Observed in the field, and handled here:
//!
//! | Quirk | Handling |
//! | --- | --- |
//! | Replies ending in `0x16` or `0x1C` instead of `0xEE` | the terminator byte is accepted whatever its value |
//! | ASCII replies split across segments, the `;` arriving alone | bytes are buffered until a unit is complete |
//! | Unsolicited `AA BB 03 11 XX ..` port reports mixed into the stream | reported as their own chunk; the engine decides whether anything was waiting for them |
//! | Garbage bytes, partial frames, corrupt length bytes | skipped, with a [`Chunk::Discarded`] so the loss is never silent |
//! | Read replies carrying an impossible port index | reported as [`Chunk::BadPortReport`] so the engine can retry |
//!
//! Sources: `bbeaudoin/bash` packet captures, `pschmitt/tesmart.sh`, PiKVM `kvmd`.
//!

use core::ops::Range;

use crate::buf::ByteBuf;
use crate::port::Port;
use crate::wire::frame::{Frame, HEADER, MAX_LEN_BYTE, OPCODE_PORT_REPORT, Payload};

/// Default buffer size: far more than the 7 byte frames and ~20 byte ASCII replies the protocol
/// uses, so overflow only happens when the stream is garbage.
pub const DEFAULT_CAPACITY: usize = 256;

/// Which end of the link the decoder is on.
///
/// The two directions have different ASCII terminators: a switch answers `IP:10.0.2.12;` but is
/// asked `IP?`, with no `;` at all in the dialect captured from TESmart's own controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecodeMode {
    /// Decoding what a switch sends: replies terminated by `;`, a bare `OK`, or `END`.
    #[default]
    Host,
    /// Decoding what a controller sends: commands terminated by `;`, `?` or `NT`.
    Device,
}

impl DecodeMode {
    const fn is_host(self) -> bool {
        matches!(self, Self::Host)
    }

    const fn is_device(self) -> bool {
        matches!(self, Self::Device)
    }
}

/// One self-contained unit pulled out of the byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chunk<'a> {
    /// A `0x11` port report, converted to a human port number.
    PortReport(Port),

    /// A `0x11` port report whose index cannot be a port. Retry rather than report.
    BadPortReport {
        /// The byte that was in the report.
        raw: u8,
    },

    /// Any other well-formed frame: an echo, or an opcode this crate does not interpret.
    Frame(Frame),

    /// A complete ASCII unit, with its terminator and surrounding whitespace removed.
    Ascii(&'a str),

    /// Bytes that could not be part of anything valid and were dropped.
    Discarded(&'a [u8]),
}
