//! What you can ask a switch, and what it can answer.
//!
//! [`Request`] is family independent on purpose: the same `Switch(Port)` works on a 16x1 KVM and
//! on a dual-monitor one, while `DisplayTimeout` only exists on single-monitor models. Which
//! requests a given switch understands is decided by the [`Profile`](crate::Profile), and a
//! request outside the active profile is refused before a single byte is sent.

use core::fmt;

use crate::port::Port;
use crate::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, MatrixTimeout, NetField, NetSetting, Output, Preset,
    Step, SwitchInterval, Toggle,
};
use crate::wire::{Frame, RoutingReply, WireBytes};

/// Something to ask the switch to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// Switch every output to a port. Opcode `0x01`, the one command every model documents.
    Switch(Port),

    /// Ask which port is active. Opcode `0x10`, answered with `0x11`.
    ReadActivePort,

    /// Mute or unmute the buzzer. Opcode `0x02`.
    Buzzer(Buzzer),

    /// Set the front display timeout. Opcode `0x03` on single-monitor KVMs **only**.
    DisplayTimeout(DisplayTimeout),

    /// Turn automatic input detection on or off. Opcode `0x81`.
    AutoInputDetect(Toggle),

    /// Step one output of a dual-monitor switch to the next or previous input.
    ///
    /// Opcodes `0x03` (output A) and `0x04` (output B). There is no absolute per-output switch
    /// command; TESmart support confirmed the step commands are the closest equivalent.
    StepOutput {
        /// Which output to steer.
        output: Output,
        /// Which direction to step.
        step: Step,
    },

    /// Move the USB (keyboard/mouse) link between the two outputs of a dual-monitor switch,
    /// in display mode 2. Opcode `0x05`.
    MoveUsb,

    /// Turn keyboard/mouse pass-through on or off. Opcode `0x06`, with inverted values.
    KmPassthrough(Toggle),

    /// Choose which key starts a hotkey sequence. Opcode `0x07`.
    HotkeyTrigger(HotkeyTrigger),

    /// Turn auto-switching on or off. Opcode `0x08`.
    AutoSwitch(Toggle),

    /// Set the auto-switch dwell time. Opcode `0x09`, the only two-byte payload in the protocol.
    AutoSwitchInterval(SwitchInterval),

    /// Read one network setting over the ASCII side-channel (`IP?`).
    NetQuery(NetField),

    /// Write one network setting (`IP:10.0.2.12;`).
    ///
    /// Settings persist when the switch answers `OK` but only take effect after a power cycle,
    /// and there is no factory reset: a wrong address leaves RS232 as the only way back.
    NetSet(NetSetting),

    /// Route one matrix input to one matrix output (`MT00SWXXYYNT`).
    MatrixRoute {
        /// The input to take.
        input: Port,
        /// The output to feed.
        output: Port,
    },

    /// Route one matrix input to every output (`MT00SWXX00NT`).
    MatrixRouteAll(Port),

    /// Mirror inputs to outputs one to one (`MT00SW0000NT`).
    MatrixMirror,

    /// Ask a matrix for its routing (`MT00RD0000NT`).
    MatrixReadRouting,

    /// Set a matrix display timeout (`MT00CTXXXXNT`). Documented for 4x4 matrices only.
    MatrixDisplayTimeout(MatrixTimeout),

    /// Mute or unmute a matrix buzzer (`MT00BZEN01NT` / `MT00BZEN00NT`).
    ///
    /// The values are **inverted** relative to the KVM [`Buzzer`] command: on a matrix `01`
    /// mutes. Documented for 8x8 and 16x16 matrices only.
    MatrixBuzzer(Buzzer),

    /// Load a stored matrix routing preset (`MT00RD01XXNT`).
    ///
    /// Note the prefix collision: `MT00RD00 00NT` reads the routing, `MT00RD01 XXNT` loads a
    /// preset. Documented for 8x8 and 16x16 matrices only.
    MatrixLoadPreset(Preset),

    /// Save the current matrix routing into a preset (`MT00SV00XXNT`).
    ///
    /// Documented for 8x8 and 16x16 matrices only.
    MatrixSavePreset(Preset),
}
