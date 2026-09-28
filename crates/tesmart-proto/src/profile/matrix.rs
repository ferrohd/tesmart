//! Family B: HDMI matrices.
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
//! # One family, four command sets
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
//! ones a given unit has is a property of the model, not of this family: see
//! [`Capabilities`](crate::Capabilities). This profile encodes the union and lets the registry
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
use crate::profile::{Family, Profile, Unsupported, base, sealed};
use crate::wire::text::render;
use crate::wire::{AsciiScope, AsciiStyle, Chunk, Transmit, TransmitKind, ascii};

/// The matrix command table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Matrix;

impl sealed::Sealed for Matrix {}

impl Profile for Matrix {
    fn family(self) -> Family {
        Family::Matrix
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

            // The one family where the network side-channel is documented, and the one family
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
                Err(Unsupported::new(Family::Matrix, request.kind()))
            }
        }
    }

    fn decode(self, chunk: &Chunk<'_>) -> Option<Response> {
        match chunk {
            Chunk::Ascii(text) => base::decode_ascii(text),
            // A matrix has no hex protocol at all, so a frame here means the wrong family was
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
