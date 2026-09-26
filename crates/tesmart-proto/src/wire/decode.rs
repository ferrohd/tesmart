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
//! # Lending iterator
//!
//! [`StreamDecoder::poll_chunk`] borrows the decoder for as long as the returned [`Chunk`]
//! lives, which is what makes ASCII chunks zero-copy. Consumption of the bytes is deferred to
//! the next call, so the usual loop just works:
//!
//! ```
//! use tesmart_proto::wire::{Chunk, StreamDecoder};
//!
//! let mut decoder: StreamDecoder = StreamDecoder::new();
//! decoder.feed(b"OK;\xAA\xBB\x03\x11\x02\xEE");
//!
//! let mut chunks = 0;
//! while let Some(chunk) = decoder.poll_chunk() {
//!     match chunk {
//!         Chunk::Ascii(text) => assert_eq!(text, "OK"),
//!         Chunk::PortReport(port) => assert_eq!(port.get(), 3),
//!         other => panic!("unexpected {other:?}"),
//!     }
//!     chunks += 1;
//! }
//! assert_eq!(chunks, 2);
//! ```

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

/// Buffers a byte stream and hands out one [`Chunk`] at a time.
#[derive(Debug, Clone)]
pub struct StreamDecoder<const N: usize = DEFAULT_CAPACITY> {
    buf: ByteBuf<N>,
    pending_consume: usize,
    mode: DecodeMode,
}

impl<const N: usize> Default for StreamDecoder<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> StreamDecoder<N> {
    /// A decoder for the controller side of the link.
    pub const fn new() -> Self {
        Self { buf: ByteBuf::new(), pending_consume: 0, mode: DecodeMode::Host }
    }

    /// A decoder for the given side of the link.
    pub const fn with_mode(mode: DecodeMode) -> Self {
        Self { buf: ByteBuf::new(), pending_consume: 0, mode }
    }

    /// Which side of the link this decoder is on.
    pub const fn mode(&self) -> DecodeMode {
        self.mode
    }

    /// Buffers as much of `data` as fits and returns how many bytes were accepted.
    ///
    /// A return value below `data.len()` means the buffer is full: drain it with
    /// [`poll_chunk`](Self::poll_chunk) and feed the rest. Zero with an empty drain means the
    /// buffer holds nothing but garbage and should be [`reset`](Self::reset).
    pub fn feed(&mut self, data: &[u8]) -> usize {
        self.apply_pending();
        self.buf.feed(data)
    }

    /// Pulls the next complete unit out of the buffer.
    pub fn poll_chunk(&mut self) -> Option<Chunk<'_>> {
        self.apply_pending();
        match scan(self.buf.as_slice(), self.mode) {
            Decision::NeedMore => None,
            Decision::Port { port, total } => {
                self.pending_consume = total;
                Some(Chunk::PortReport(port))
            }
            Decision::BadPort { raw, total } => {
                self.pending_consume = total;
                Some(Chunk::BadPortReport { raw })
            }
            Decision::Frame { frame, total } => {
                self.pending_consume = total;
                Some(Chunk::Frame(frame))
            }
            Decision::Ascii { text, total } => {
                self.pending_consume = total;
                let buffered = self.buf.as_slice();
                match core::str::from_utf8(&buffered[text]) {
                    Ok(text) => Some(Chunk::Ascii(text)),
                    // unreachable: `scan` only proposes printable ASCII
                    Err(_) => Some(Chunk::Discarded(&buffered[..total])),
                }
            }
            Decision::Discard { total } => {
                self.pending_consume = total;
                Some(Chunk::Discarded(&self.buf.as_slice()[..total]))
            }
        }
    }

    /// Gives up on finding a terminator and returns whatever printable text is buffered.
    ///
    /// The engine calls this when a reply deadline expires: an `OK` with no terminator, or a
    /// reply truncated by a closing connection, is still worth something. Returns `None` if the
    /// buffer is empty, holds frame data, or holds anything unprintable.
    pub fn flush_ascii(&mut self) -> Option<&str> {
        self.apply_pending();
        let buffered = self.buf.as_slice();
        if buffered.is_empty() || find_header(buffered).0.is_some() {
            return None;
        }
        let text = trim_text(buffered, buffered.len());
        if text.is_empty() || !is_printable(&buffered[text.clone()]) {
            return None;
        }
        self.pending_consume = buffered.len();
        core::str::from_utf8(&buffered[text]).ok()
    }

    /// How many bytes are waiting to be decoded.
    pub const fn buffered(&self) -> usize {
        self.buf.len() - self.pending_consume
    }

    /// The buffer size.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Throws away everything buffered. Use after a reconnect, or to recover from garbage.
    pub const fn reset(&mut self) {
        self.buf.clear();
        self.pending_consume = 0;
    }

    fn apply_pending(&mut self) {
        if self.pending_consume > 0 {
            self.buf.consume(self.pending_consume);
            self.pending_consume = 0;
        }
    }
}
