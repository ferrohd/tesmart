//! Family A': 4-port dual-monitor KVM switches.
//!
//! Same framing as [`Kvm`](crate::profile::Kvm), **different meanings from `0x03` up**. No
//! official TESmart document covers this variant; everything here comes from
//! `spuder/Tesmart-KVM`, tested over RS232 at 9600 baud on TESmart's 4-port dual-monitor
//! HDMI/USB-C 4K60 switch.
//!
//! | Opcode | Command | Value |
//! | --- | --- | --- |
//! | `0x01` | switch input | port, 1-based |
//! | `0x02` | buzzer | `0x00` off, `0x01` on |
//! | `0x03` | output A input step | `0x00` down, `0x01` up |
//! | `0x04` | output B input step | `0x00` down, `0x01` up |
//! | `0x05` | move USB between outputs | `0x01` |
//! | `0x06` | keyboard/mouse pass-through | `0x00` **on**, `0x01` off |
//! | `0x07` | hotkey trigger | `0x01` Right Ctrl, `0x00` Scroll Lock |
//! | `0x08` | auto-switch | `0x00` off, `0x01` on |
//! | `0x09` | auto-switch interval | two bytes, `len = 0x04` |
//!
//! There is no read command, so nothing on a dual-monitor switch can be verified.

use crate::command::{CommandInfo, Request, RequestKind, Response};
use crate::profile::base::{self, OPCODE_BUZZER, OPCODE_SWITCH};
use crate::profile::{Family, Profile, Unsupported, sealed};
use crate::value::Output;
use crate::wire::{AsciiStyle, Chunk, Frame, Transmit};

/// Step output A's input up or down.
pub const OPCODE_STEP_OUTPUT_A: u8 = 0x03;

/// Step output B's input up or down.
pub const OPCODE_STEP_OUTPUT_B: u8 = 0x04;

/// Move the USB link between the two outputs (display mode 2).
pub const OPCODE_MOVE_USB: u8 = 0x05;

/// Keyboard/mouse pass-through. The values are inverted compared to every other setting.
pub const OPCODE_KM_PASSTHROUGH: u8 = 0x06;

/// Which key starts a hotkey sequence.
pub const OPCODE_HOTKEY_TRIGGER: u8 = 0x07;

/// Auto-switch on or off.
pub const OPCODE_AUTO_SWITCH: u8 = 0x08;

/// Auto-switch dwell time, the only command with a two-byte payload.
pub const OPCODE_AUTO_SWITCH_INTERVAL: u8 = 0x09;

/// The Family A' command table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KvmDual;

impl sealed::Sealed for KvmDual {}

impl Profile for KvmDual {
    fn family(self) -> Family {
        Family::KvmDual
    }

    fn encode(self, request: Request, _style: AsciiStyle) -> Result<Transmit, Unsupported> {
        match request {
            Request::Switch(port) => Ok(base::switch(port)),
            Request::Buzzer(buzzer) => Ok(base::buzzer(buzzer)),
            Request::StepOutput { output, step } => {
                let opcode = match output {
                    Output::A => OPCODE_STEP_OUTPUT_A,
                    Output::B => OPCODE_STEP_OUTPUT_B,
                };
                Ok(Frame::one(opcode, step.to_byte()).to_transmit())
            }
            Request::MoveUsb => Ok(Frame::one(OPCODE_MOVE_USB, 0x01).to_transmit()),
            // The one inverted setting in the protocol: 0x00 enables pass-through.
            Request::KmPassthrough(state) => {
                Ok(Frame::one(OPCODE_KM_PASSTHROUGH, state.to_inverted_byte()).to_transmit())
            }
            Request::HotkeyTrigger(key) => {
                Ok(Frame::one(OPCODE_HOTKEY_TRIGGER, key.to_byte()).to_transmit())
            }
            Request::AutoSwitch(state) => {
                Ok(Frame::one(OPCODE_AUTO_SWITCH, state.to_byte()).to_transmit())
            }
            Request::AutoSwitchInterval(interval) => {
                let (minutes, seconds) = interval.to_raw();
                Ok(Frame::two(OPCODE_AUTO_SWITCH_INTERVAL, [minutes, seconds]).to_transmit())
            }

            Request::ReadActivePort
            | Request::DisplayTimeout(_)
            | Request::AutoInputDetect(_)
            | Request::NetQuery(_)
            | Request::NetSet(_)
            | Request::MatrixRoute { .. }
            | Request::MatrixRouteAll(_)
            | Request::MatrixMirror
            | Request::MatrixReadRouting
            | Request::MatrixDisplayTimeout(_)
            | Request::MatrixBuzzer(_)
            | Request::MatrixLoadPreset(_)
            | Request::MatrixSavePreset(_) => {
                Err(Unsupported::new(Family::KvmDual, request.kind()))
            }
        }
    }

    /// Dual-monitor switches have no documented read command, but they share the framing, so an
    /// unsolicited `0x11` report is still interpreted if one ever shows up.
    fn decode(self, chunk: &Chunk<'_>) -> Option<Response> {
        base::decode_hex_family(chunk)
    }

    fn commands(self) -> &'static [CommandInfo] {
        COMMANDS
    }
}

static COMMANDS: &[CommandInfo] = &[
    CommandInfo::frame(RequestKind::Switch, OPCODE_SWITCH),
    CommandInfo::frame(RequestKind::Buzzer, OPCODE_BUZZER).with_note("write only"),
    CommandInfo::frame(RequestKind::StepOutput, OPCODE_STEP_OUTPUT_A)
        .with_note("0x03 for output A, 0x04 for output B; relative only, there is no absolute per-output switch"),
    CommandInfo::frame(RequestKind::MoveUsb, OPCODE_MOVE_USB)
        .with_note("display mode 2 only"),
    CommandInfo::frame(RequestKind::KmPassthrough, OPCODE_KM_PASSTHROUGH)
        .with_note("inverted values: 0x00 enables pass-through"),
    CommandInfo::frame(RequestKind::HotkeyTrigger, OPCODE_HOTKEY_TRIGGER)
        .with_note("hotkeys work only on the dedicated keyboard/mouse port"),
    CommandInfo::frame(RequestKind::AutoSwitch, OPCODE_AUTO_SWITCH).with_note("write only"),
    CommandInfo::frame(RequestKind::AutoSwitchInterval, OPCODE_AUTO_SWITCH_INTERVAL)
        .with_note("seven-byte frame with len = 0x04; the MM SS reading of the two data bytes is inferred, not documented"),
];
