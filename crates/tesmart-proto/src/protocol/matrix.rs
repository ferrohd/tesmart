//! The `matrix` protocol: HDMI matrices.
//!
//! Matrices answer on the same default address as KVMs (192.168.1.10:5000, 9600 8N1) but share
//! none of their commands, so a KVM tool pointed at a matrix connects and does nothing useful.
//!
//! | Command | Meaning |
//! | --- | --- |
//! | `MT00SWXXYYNT` | route input `XX` to output `YY` |
//! | `MT00SWXX00NT` | route input `XX` to every output |
//! | `MT00SW0000NT` | mirror 1 to 1, 2 to 2, ... |
//! | `MT00RD0000NT` | request routing; answered `LINK:OYIX;END` |
//! | `MT00RD01XXNT` | load routing preset `XX`, `01`-`08` |
//! | `MT00SV00XXNT` | save routing into preset `XX`, `01`-`08` |
//! | `MT00BZEN01NT` | mute the buzzer; `00NT` unmutes |
//! | `MT00CTXXXXNT` | display timeout, `0000`-`9999` seconds |
//!
//! # One protocol, four command sets
//!
//! The table above is the union. No matrix implements all of it, and the gaps do not nest:
//!
//! | Models | route, mirror, read | `MT00CT` | buzzer | presets | network |
//! | --- | --- | --- | --- | --- | --- |
//! | 4x4 over RS232 only | yes | no | no | no | no |
//! | 4x4 over RS232+IP | yes | **yes** | no | no | yes |
//! | 8x8 | yes | no | **yes** | **yes** | yes |
//! | 16x16 | yes | no | **yes** | **yes** | yes |
//!
//! So `MT00CT` and the preset/buzzer commands are mutually exclusive in practice, and which
//! ones a given unit has is a property of the model, not of this protocol: see
//! [`Capabilities`](crate::Capabilities). This table encodes the union and lets the registry
//! refuse what a model lacks.
//!
//! # Two transports, two spellings
//!
//! The network commands are listed twice in every matrix document, once "for LAN connection
//! only" and once "for RS232 connection only", and the two lists are different strings. See
//! [`crate::wire::AsciiScope`].
//!
//! Sources: TESmart's 4x4 (RS232), 4x4 (RS232+IP), 8x8 and 16x16 matrix protocol documents.

use crate::command::{CommandInfo, Request, RequestKind, Response};
use crate::protocol::{CommandTable, Protocol, Unsupported, base, sealed};
use crate::wire::text::render;
use crate::wire::{AsciiScope, AsciiStyle, Chunk, Transmit, TransmitKind, ascii};

/// The matrix command table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MatrixTable;

impl sealed::Sealed for MatrixTable {}

impl CommandTable for MatrixTable {
    fn protocol(self) -> Protocol {
        Protocol::Matrix
    }

    fn encode(self, request: Request, style: AsciiStyle) -> Result<Transmit, Unsupported> {
        match request {
            Request::MatrixRoute { input, output } => {
                Ok(command(format_args!("MT00SW{:02}{:02}NT", input.get(), output.get())))
            }
            Request::MatrixRouteAll(input) => {
                Ok(command(format_args!("MT00SW{:02}00NT", input.get())))
            }
            Request::MatrixMirror => Ok(command(format_args!("MT00SW0000NT"))),
            Request::MatrixReadRouting => Ok(command(format_args!("MT00RD0000NT"))),
            Request::MatrixDisplayTimeout(timeout) => {
                Ok(command(format_args!("MT00CT{:04}NT", timeout.as_secs())))
            }
            // Inverted against the KVM buzzer command: on a matrix, 01 mutes.
            Request::MatrixBuzzer(buzzer) => {
                Ok(command(format_args!("MT00BZEN{:02}NT", buzzer.to_matrix_field())))
            }
            Request::MatrixLoadPreset(preset) => {
                Ok(command(format_args!("MT00RD01{:02}NT", preset.get())))
            }
            Request::MatrixSavePreset(preset) => {
                Ok(command(format_args!("MT00SV00{:02}NT", preset.get())))
            }

            // The one protocol where the network side-channel is documented, and the one protocol
            // where the spelling depends on how you are connected.
            Request::NetQuery(field) => Ok(match style.scope {
                AsciiScope::Lan => ascii::encode_query(field, style.dialect),
                AsciiScope::Serial => ascii::encode_wrapped_query(field),
            }),
            Request::NetSet(setting) => Ok(match style.scope {
                AsciiScope::Lan => ascii::encode_set(setting, style.dialect),
                AsciiScope::Serial => ascii::encode_wrapped_set(setting),
            }),

            Request::Switch(_)
            | Request::ReadActivePort
            | Request::Buzzer(_)
            | Request::DisplayTimeout(_)
            | Request::AutoInputDetect(_)
            | Request::StepOutput { .. }
            | Request::MoveUsb
            | Request::KmPassthrough(_)
            | Request::HotkeyTrigger(_)
            | Request::AutoSwitch(_)
            | Request::AutoSwitchInterval(_) => {
                Err(Unsupported::new(Protocol::Matrix, request.kind()))
            }
        }
    }

    fn decode(self, chunk: &Chunk<'_>) -> Option<Response> {
        match chunk {
            Chunk::Ascii(text) => base::decode_ascii(text),
            // A matrix has no hex protocol at all, so a frame here means the wrong protocol was
            // selected, or the wrong device answered.
            Chunk::PortReport(_)
            | Chunk::BadPortReport { .. }
            | Chunk::Frame(_)
            | Chunk::Discarded(_) => None,
        }
    }

    fn commands(self) -> &'static [CommandInfo] {
        COMMANDS
    }
}

fn command(args: core::fmt::Arguments<'_>) -> Transmit {
    Transmit::new(render(args), TransmitKind::Ascii)
}

static COMMANDS: &[CommandInfo] = &[
    CommandInfo::ascii(RequestKind::MatrixRoute),
    CommandInfo::ascii(RequestKind::MatrixRouteAll),
    CommandInfo::ascii(RequestKind::MatrixMirror),
    CommandInfo::ascii(RequestKind::MatrixReadRouting)
        .with_note("the only matrix command that answers, which is what makes routing verifiable"),
    CommandInfo::ascii(RequestKind::MatrixDisplayTimeout)
        .with_note("write only; documented for 4x4 matrices only"),
    CommandInfo::ascii(RequestKind::MatrixBuzzer).with_note(
        "write only; 8x8 and 16x16 only, and the values are inverted against the KVM command",
    ),
    CommandInfo::ascii(RequestKind::MatrixLoadPreset)
        .with_note("8x8 and 16x16 only; loading a preset changes the routing without replying"),
    CommandInfo::ascii(RequestKind::MatrixSavePreset).with_note("write only; 8x8 and 16x16 only"),
    CommandInfo::ascii(RequestKind::NetQuery)
        .with_note("bare form over LAN, MT8003 wrapped form over RS232"),
    CommandInfo::ascii(RequestKind::NetSet)
        .with_note("bare form over LAN, MT8019/MT8009 wrapped form over RS232"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Port;
    use crate::value::{Buzzer, MatrixTimeout, NetField, NetSetting, Preset};

    fn encode_with(request: Request, style: AsciiStyle) -> std::string::String {
        MatrixTable
            .encode(request, style)
            .expect("supported by the matrix protocol")
            .bytes()
            .as_str()
            .expect("ASCII")
            .to_owned()
    }

    fn encode(request: Request) -> std::string::String {
        encode_with(request, AsciiStyle::DEFAULT)
    }

    fn port(number: u8) -> Port {
        Port::new(number).expect("a valid port")
    }

    #[test]
    fn encodes_the_documented_strings() {
        assert_eq!(
            encode(Request::MatrixRoute { input: Port::FIRST, output: port(2) }),
            "MT00SW0102NT"
        );
        assert_eq!(encode(Request::MatrixRouteAll(port(3))), "MT00SW0300NT");
        assert_eq!(encode(Request::MatrixMirror), "MT00SW0000NT");
        assert_eq!(encode(Request::MatrixReadRouting), "MT00RD0000NT");
        assert_eq!(encode(Request::MatrixDisplayTimeout(MatrixTimeout::NEVER)), "MT00CT0000NT");
        assert_eq!(
            encode(Request::MatrixDisplayTimeout(MatrixTimeout::from_secs(30).unwrap())),
            "MT00CT0030NT"
        );
        assert_eq!(
            encode(Request::MatrixDisplayTimeout(MatrixTimeout::from_secs(9999).unwrap())),
            "MT00CT9999NT"
        );
    }

    /// A 16x16 matrix routes two-digit ports, so the field is not just the 4x4 case padded.
    #[test]
    fn encodes_the_widest_routing_command() {
        assert_eq!(
            encode(Request::MatrixRoute { input: port(16), output: port(16) }),
            "MT00SW1616NT"
        );
        assert_eq!(encode(Request::MatrixRouteAll(port(16))), "MT00SW1600NT");
    }

    /// The buzzer is the one command where the matrix and KVM encodings disagree.
    #[test]
    fn the_matrix_buzzer_is_inverted_against_the_kvm_one() {
        assert_eq!(encode(Request::MatrixBuzzer(Buzzer::Muted)), "MT00BZEN01NT");
        assert_eq!(encode(Request::MatrixBuzzer(Buzzer::Audible)), "MT00BZEN00NT");

        // Same value, opposite byte, on the KVM side.
        assert_eq!(Buzzer::Muted.to_byte(), 0x00);
        assert_eq!(Buzzer::Muted.to_matrix_field(), 1);
    }

    #[test]
    fn encodes_presets_in_both_directions() {
        assert_eq!(encode(Request::MatrixSavePreset(Preset::FIRST)), "MT00SV0001NT");
        assert_eq!(encode(Request::MatrixLoadPreset(Preset::FIRST)), "MT00RD0101NT");
        assert_eq!(encode(Request::MatrixSavePreset(Preset::new(8).unwrap())), "MT00SV0008NT");
        assert_eq!(encode(Request::MatrixLoadPreset(Preset::new(8).unwrap())), "MT00RD0108NT");

        // Reading the routing and loading preset 1 differ by one character.
        assert_ne!(
            encode(Request::MatrixReadRouting),
            encode(Request::MatrixLoadPreset(Preset::FIRST))
        );
    }

    #[test]
    fn network_commands_follow_the_transport() {
        let lan = AsciiStyle::DEFAULT;
        let serial = AsciiStyle::DEFAULT.with_scope(AsciiScope::Serial);

        assert_eq!(encode_with(Request::NetQuery(NetField::Ip), lan), "IP?");
        assert_eq!(encode_with(Request::NetQuery(NetField::Ip), serial), "MT8003IP?");

        let setting = NetSetting::Port(5000);
        assert_eq!(encode_with(Request::NetSet(setting), lan), "PT:5000;");
        assert_eq!(encode_with(Request::NetSet(setting), serial), "MT8009PT:05000;");
    }

    #[test]
    fn decodes_routing_reports_only_from_ascii() {
        let Some(Response::Routing(routing)) = MatrixTable.decode(&Chunk::Ascii("LINK:O1I2;END"))
        else {
            panic!("expected a routing report")
        };
        assert_eq!(routing.entries().len(), 1);
        assert_eq!(MatrixTable.decode(&Chunk::PortReport(Port::FIRST)), None);
    }
}
