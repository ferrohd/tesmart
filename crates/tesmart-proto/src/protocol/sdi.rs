//! The `sdi` protocol: the 4x1 SDI switch.
//!
//! One model, `SSW04010A10`, and one command. It is a separate protocol because it shares neither
//! the header nor the framing of anything else TESmart makes:
//!
//! | Command | Bytes |
//! | --- | --- |
//! | Switch to input 1 | `AA CC 01 01` |
//! | Switch to input 2 | `AA CC 01 02` |
//! | Switch to input 3 | `AA CC 01 03` |
//! | Switch to input 4 | `AA CC 01 04` |
//!
//! Four bytes, header `AA CC` rather than `AA BB`, and **no terminator**: nothing corresponds
//! to the `0xEE` that ends a KVM frame. Whether the `01` is a length byte, as it is in
//! `AA BB 03 ...`, is not stated; since the document lists the four commands literally and
//! defines no other, it is treated here as part of a fixed prefix rather than as a length.
//!
//! The switch reports nothing. There is no read command, no settings, and no acknowledgement,
//! so every exchange in this protocol completes as [`Response::Accepted`] without the switch
//! having confirmed anything.
//!
//! Source: TESmart's `SSW0401A10-Communication-protocol` document (9600 8N1, parity none).

use crate::command::{CommandInfo, Request, RequestKind, Response};
use crate::port::Port;
use crate::protocol::{CommandTable, Protocol, Unsupported, sealed};
use crate::wire::{AsciiStyle, Chunk, Transmit, TransmitKind, WireBytes};

/// The fixed prefix every command carries: header `AA CC` and the constant `01`.
pub const PREFIX: [u8; 3] = [0xAA, 0xCC, 0x01];

/// How many inputs the one model in this protocol has.
pub const INPUTS: u8 = 4;

/// The SDI command table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SdiTable;

impl sealed::Sealed for SdiTable {}

impl CommandTable for SdiTable {
    fn protocol(self) -> Protocol {
        Protocol::Sdi
    }

    fn encode(self, request: Request, _style: AsciiStyle) -> Result<Transmit, Unsupported> {
        match request {
            Request::Switch(port) => Ok(switch(port)),

            Request::ReadActivePort
            | Request::Buzzer(_)
            | Request::DisplayTimeout(_)
            | Request::AutoInputDetect(_)
            | Request::StepOutput { .. }
            | Request::MoveUsb
            | Request::KmPassthrough(_)
            | Request::HotkeyTrigger(_)
            | Request::AutoSwitch(_)
            | Request::AutoSwitchInterval(_)
            | Request::NetQuery(_)
            | Request::NetSet(_)
            | Request::MatrixRoute { .. }
            | Request::MatrixRouteAll(_)
            | Request::MatrixMirror
            | Request::MatrixReadRouting
            | Request::MatrixDisplayTimeout(_)
            | Request::MatrixBuzzer(_)
            | Request::MatrixLoadPreset(_)
            | Request::MatrixSavePreset(_) => Err(Unsupported::new(Protocol::Sdi, request.kind())),
        }
    }

    /// Nothing. An SDI switch never sends a byte.
    fn decode(self, _chunk: &Chunk<'_>) -> Option<Response> {
        None
    }

    fn commands(self) -> &'static [CommandInfo] {
        COMMANDS
    }
}

/// `AA CC 01 <port>`, with the port 1-based and no terminator.
fn switch(port: Port) -> Transmit {
    let bytes = [PREFIX[0], PREFIX[1], PREFIX[2], port.to_write_byte()];
    Transmit::new(WireBytes::from_bounded(&bytes), TransmitKind::Frame)
}

static COMMANDS: &[CommandInfo] = &[CommandInfo::frame(RequestKind::Switch, PREFIX[2])
    .with_note("AA CC 01 <port>, four bytes with no terminator; the switch acknowledges nothing")];

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(request: Request) -> std::vec::Vec<u8> {
        SdiTable
            .encode(request, AsciiStyle::DEFAULT)
            .expect("supported by the sdi protocol")
            .bytes()
            .as_slice()
            .to_vec()
    }

    #[test]
    fn encodes_the_four_documented_commands() {
        for input in 1..=INPUTS {
            let port = Port::new(input).unwrap();
            assert_eq!(encode(Request::Switch(port)), std::vec![0xAA, 0xCC, 0x01, input]);
        }
    }

    #[test]
    fn the_frame_has_no_terminator() {
        let bytes = encode(Request::Switch(Port::FIRST));
        assert_eq!(bytes.len(), 4, "a KVM frame would be six bytes");
        assert_ne!(bytes.last(), Some(&0xEE));
    }

    #[test]
    fn nothing_else_is_in_the_protocol() {
        let error = SdiTable
            .encode(Request::ReadActivePort, AsciiStyle::DEFAULT)
            .expect_err("an SDI switch cannot be read");
        assert_eq!(error.protocol, Protocol::Sdi);
        assert!(std::format!("{error}").contains("does not report"), "{error}");
    }

    #[test]
    fn it_never_interprets_anything() {
        assert_eq!(SdiTable.decode(&Chunk::PortReport(Port::FIRST)), None);
        assert_eq!(SdiTable.decode(&Chunk::Ascii("OK")), None);
    }
}
