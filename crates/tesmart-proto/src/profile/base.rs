//! The commands that mean the same thing in every hex family.
//!
//! Only two opcodes are safe to send without knowing which kind of switch is attached. Both hex
//! profiles build them from here, so the 1-based write encoding of a port number exists in
//! exactly one place.

use crate::command::Response;
use crate::port::Port;
use crate::value::Buzzer;
use crate::wire::{Chunk, Frame, Transmit, ascii};

/// Switch every output to a port. Documented for every TESmart series.
pub const OPCODE_SWITCH: u8 = 0x01;

/// Mute or unmute the buzzer.
pub const OPCODE_BUZZER: u8 = 0x02;

/// `AA BB 03 01 <port> EE`, with the port 1-based.
pub(crate) fn switch(port: Port) -> Transmit {
    Frame::one(OPCODE_SWITCH, port.to_write_byte()).to_transmit()
}

/// `AA BB 03 02 <00|01> EE`.
pub(crate) fn buzzer(buzzer: Buzzer) -> Transmit {
    Frame::one(OPCODE_BUZZER, buzzer.to_byte()).to_transmit()
}

/// Interprets anything a hex-family switch can say.
///
/// Both single- and dual-monitor profiles share this: a `0x11` port report is a port report
/// either way, and the ASCII side-channel is identical.
pub(crate) fn decode_hex_family(chunk: &Chunk<'_>) -> Option<Response> {
    match chunk {
        Chunk::PortReport(port) => Some(Response::ActivePort(*port)),
        Chunk::Ascii(text) => decode_ascii(text),
        Chunk::Frame(_) | Chunk::BadPortReport { .. } | Chunk::Discarded(_) => None,
    }
}

/// Interprets an ASCII reply.
pub(crate) fn decode_ascii(text: &str) -> Option<Response> {
    match ascii::parse_reply(text) {
        ascii::AsciiReply::Net(setting) => Some(Response::Net(setting)),
        ascii::AsciiReply::Ack => Some(Response::Ack),
        ascii::AsciiReply::Routing(routing) => Some(Response::Routing(routing)),
        ascii::AsciiReply::Unknown => None,
    }
}
