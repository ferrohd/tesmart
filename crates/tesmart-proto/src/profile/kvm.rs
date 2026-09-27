//! Family A: single-monitor KVM switches.
//!
//! Covers `HKS0801A*`, `HKS1601A*`, `HSW0801A*`, `HSW1601A*`, `HKE0802A10`, `DKS0401A30`,
//! `HKS801/1601-EB23` and `HKS801/1601-P23`. Which of these commands a particular model
//! implements is a *capability* question, answered by [`crate::model`], not a protocol one: the
//! encoding is identical across the series.
//!
//! | Opcode | Command | Value |
//! | --- | --- | --- |
//! | `0x01` | switch input | port, 1-based |
//! | `0x02` | buzzer | `0x00` mute, `0x01` unmute |
//! | `0x03` | display timeout | `0x0A` 10 s, `0x1E` 30 s, `0x00` never |
//! | `0x81` | auto input detect | `0x00` off, `0x01` on |
//! | `0x10` | read active port | `0x00`, answered by `0x11` with a 0-based port |
//! | ASCII | network configuration | `IP?`, `IP:10.0.2.12;`, and the `PT` / `GW` / `MA` equivalents |
//!
//! Sources: the 8x1 v1, 8x1 v2, 16x1 and 4x1 DVI protocol PDFs; the EB23 and P23 manuals; and
//! packet captures for the ASCII side-channel.

use crate::command::{CommandInfo, Request, RequestKind, Response};
use crate::profile::base::{self, OPCODE_BUZZER, OPCODE_SWITCH};
use crate::profile::{Family, Profile, Unsupported, sealed};
use crate::wire::{AsciiStyle, Chunk, Frame, Transmit, ascii};

/// Set how long the front panel display stays lit.
pub const OPCODE_DISPLAY_TIMEOUT: u8 = 0x03;

/// Turn automatic input detection on or off.
pub const OPCODE_AUTO_INPUT_DETECT: u8 = 0x81;

/// Ask which input is active.
pub const OPCODE_READ_PORT: u8 = 0x10;

/// The reply to [`OPCODE_READ_PORT`], carrying a 0-based port.
pub const OPCODE_PORT_REPORT: u8 = crate::wire::frame::OPCODE_PORT_REPORT;

/// The Family A command table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Kvm;

impl sealed::Sealed for Kvm {}

impl Profile for Kvm {
    fn family(self) -> Family {
        Family::Kvm
    }

    fn encode(self, request: Request, style: AsciiStyle) -> Result<Transmit, Unsupported> {
        match request {
            Request::Switch(port) => Ok(base::switch(port)),
            Request::Buzzer(buzzer) => Ok(base::buzzer(buzzer)),
            Request::DisplayTimeout(timeout) => {
                Ok(Frame::one(OPCODE_DISPLAY_TIMEOUT, timeout.to_byte()).to_transmit())
            }
            Request::AutoInputDetect(state) => {
                Ok(Frame::one(OPCODE_AUTO_INPUT_DETECT, state.to_byte()).to_transmit())
            }
            Request::ReadActivePort => Ok(Frame::one(OPCODE_READ_PORT, 0x00).to_transmit()),
            // Only the bare forms: no wrapped KVM command has ever been documented or
            // captured, so the scope is deliberately ignored here.
            Request::NetQuery(field) => Ok(ascii::encode_query(field, style.dialect)),
            Request::NetSet(setting) => Ok(ascii::encode_set(setting, style.dialect)),

            // Listed one by one rather than caught by a wildcard: a new Request variant has to
            // be considered here before it compiles.
            Request::StepOutput { .. }
            | Request::MoveUsb
            | Request::KmPassthrough(_)
            | Request::HotkeyTrigger(_)
            | Request::AutoSwitch(_)
            | Request::AutoSwitchInterval(_)
            | Request::MatrixRoute { .. }
            | Request::MatrixRouteAll(_)
            | Request::MatrixMirror
            | Request::MatrixReadRouting
            | Request::MatrixDisplayTimeout(_)
            | Request::MatrixBuzzer(_)
            | Request::MatrixLoadPreset(_)
            | Request::MatrixSavePreset(_) => Err(Unsupported::new(Family::Kvm, request.kind())),
        }
    }

    fn decode(self, chunk: &Chunk<'_>) -> Option<Response> {
        base::decode_hex_family(chunk)
    }

    fn commands(self) -> &'static [CommandInfo] {
        COMMANDS
    }
}

static COMMANDS: &[CommandInfo] = &[
    CommandInfo::frame(RequestKind::Switch, OPCODE_SWITCH),
    CommandInfo::frame(RequestKind::ReadActivePort, OPCODE_READ_PORT)
        .with_note("answered by 0x11 with a 0-based port; replies are sometimes garbage, so retry"),
    CommandInfo::frame(RequestKind::Buzzer, OPCODE_BUZZER)
        .with_note("write only: the buzzer state cannot be read back on any series"),
    CommandInfo::frame(RequestKind::DisplayTimeout, OPCODE_DISPLAY_TIMEOUT)
        .with_note("write only; this opcode means something else on dual-monitor switches"),
    CommandInfo::frame(RequestKind::AutoInputDetect, OPCODE_AUTO_INPUT_DETECT)
        .with_note("write only; field-tested rather than documented on the 16x1 series"),
    CommandInfo::ascii(RequestKind::NetQuery)
        .with_note("ASCII side-channel, absent from TESmart's protocol PDFs"),
    CommandInfo::ascii(RequestKind::NetSet).with_note(
        "persists on OK but only takes effect after a power cycle; there is no factory reset",
    ),
];
