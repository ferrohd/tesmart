//! What you can ask a switch, and what it can answer.
//!
//! [`Request`] is protocol independent on purpose: the same `Switch(Port)` works on a 16x1 KVM and
//! on a dual-monitor one, while `DisplayTimeout` only exists on single-monitor models. Which
//! requests a given switch understands is decided by the [`CommandTable`](crate::CommandTable),
//! and a request outside the active protocol is refused before a single byte is sent.

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

impl Request {
    /// The kind of request, without its argument.
    pub const fn kind(self) -> RequestKind {
        match self {
            Self::Switch(_) => RequestKind::Switch,
            Self::ReadActivePort => RequestKind::ReadActivePort,
            Self::Buzzer(_) => RequestKind::Buzzer,
            Self::DisplayTimeout(_) => RequestKind::DisplayTimeout,
            Self::AutoInputDetect(_) => RequestKind::AutoInputDetect,
            Self::StepOutput { .. } => RequestKind::StepOutput,
            Self::MoveUsb => RequestKind::MoveUsb,
            Self::KmPassthrough(_) => RequestKind::KmPassthrough,
            Self::HotkeyTrigger(_) => RequestKind::HotkeyTrigger,
            Self::AutoSwitch(_) => RequestKind::AutoSwitch,
            Self::AutoSwitchInterval(_) => RequestKind::AutoSwitchInterval,
            Self::NetQuery(_) => RequestKind::NetQuery,
            Self::NetSet(_) => RequestKind::NetSet,
            Self::MatrixRoute { .. } => RequestKind::MatrixRoute,
            Self::MatrixRouteAll(_) => RequestKind::MatrixRouteAll,
            Self::MatrixMirror => RequestKind::MatrixMirror,
            Self::MatrixReadRouting => RequestKind::MatrixReadRouting,
            Self::MatrixDisplayTimeout(_) => RequestKind::MatrixDisplayTimeout,
            Self::MatrixBuzzer(_) => RequestKind::MatrixBuzzer,
            Self::MatrixLoadPreset(_) => RequestKind::MatrixLoadPreset,
            Self::MatrixSavePreset(_) => RequestKind::MatrixSavePreset,
        }
    }

    /// What kind of reply, if any, this request is answered with.
    pub const fn expects(self) -> Option<ResponseKind> {
        self.kind().expects()
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Switch(port) => write!(f, "switch to port {port}"),
            Self::ReadActivePort => f.write_str("read active port"),
            Self::Buzzer(buzzer) | Self::MatrixBuzzer(buzzer) => write!(f, "set buzzer {buzzer}"),
            Self::DisplayTimeout(timeout) => write!(f, "set display timeout {timeout}"),
            Self::AutoInputDetect(state) => write!(f, "set auto input detect {state}"),
            Self::StepOutput { output, step } => write!(f, "step output {output} {step}"),
            Self::MoveUsb => f.write_str("move usb"),
            Self::KmPassthrough(state) => write!(f, "set keyboard/mouse pass-through {state}"),
            Self::HotkeyTrigger(key) => write!(f, "set hotkey trigger {key}"),
            Self::AutoSwitch(state) => write!(f, "set auto-switch {state}"),
            Self::AutoSwitchInterval(interval) => write!(f, "set auto-switch interval {interval}"),
            Self::NetQuery(field) => write!(f, "read network {field}"),
            Self::NetSet(setting) => write!(f, "set network {} to {setting}", setting.field()),
            Self::MatrixRoute { input, output } => {
                write!(f, "route input {input} to output {output}")
            }
            Self::MatrixRouteAll(input) => write!(f, "route input {input} to all outputs"),
            Self::MatrixMirror => f.write_str("mirror inputs to outputs"),
            Self::MatrixReadRouting => f.write_str("read routing"),
            // A different type from DisplayTimeout: four digits of seconds, not three choices.
            Self::MatrixDisplayTimeout(timeout) => write!(f, "set display timeout {timeout}"),
            Self::MatrixLoadPreset(preset) => write!(f, "load preset {preset}"),
            Self::MatrixSavePreset(preset) => write!(f, "save preset {preset}"),
        }
    }
}

/// A [`Request`] without its argument: what command tables and capabilities are keyed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum RequestKind {
    /// [`Request::Switch`].
    Switch,
    /// [`Request::ReadActivePort`].
    ReadActivePort,
    /// [`Request::Buzzer`].
    Buzzer,
    /// [`Request::DisplayTimeout`].
    DisplayTimeout,
    /// [`Request::AutoInputDetect`].
    AutoInputDetect,
    /// [`Request::StepOutput`].
    StepOutput,
    /// [`Request::MoveUsb`].
    MoveUsb,
    /// [`Request::KmPassthrough`].
    KmPassthrough,
    /// [`Request::HotkeyTrigger`].
    HotkeyTrigger,
    /// [`Request::AutoSwitch`].
    AutoSwitch,
    /// [`Request::AutoSwitchInterval`].
    AutoSwitchInterval,
    /// [`Request::NetQuery`].
    NetQuery,
    /// [`Request::NetSet`].
    NetSet,
    /// [`Request::MatrixRoute`].
    MatrixRoute,
    /// [`Request::MatrixRouteAll`].
    MatrixRouteAll,
    /// [`Request::MatrixMirror`].
    MatrixMirror,
    /// [`Request::MatrixReadRouting`].
    MatrixReadRouting,
    /// [`Request::MatrixDisplayTimeout`].
    MatrixDisplayTimeout,
    /// [`Request::MatrixBuzzer`].
    MatrixBuzzer,
    /// [`Request::MatrixLoadPreset`].
    MatrixLoadPreset,
    /// [`Request::MatrixSavePreset`].
    MatrixSavePreset,
}

impl RequestKind {
    /// A stable, hyphenated name, suitable for CLI output and error messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::ReadActivePort => "read-active-port",
            Self::Buzzer => "buzzer",
            Self::DisplayTimeout => "display-timeout",
            Self::AutoInputDetect => "auto-input-detect",
            Self::StepOutput => "step-output",
            Self::MoveUsb => "move-usb",
            Self::KmPassthrough => "km-passthrough",
            Self::HotkeyTrigger => "hotkey-trigger",
            Self::AutoSwitch => "auto-switch",
            Self::AutoSwitchInterval => "auto-switch-interval",
            Self::NetQuery => "net-query",
            Self::NetSet => "net-set",
            Self::MatrixRoute => "matrix-route",
            Self::MatrixRouteAll => "matrix-route-all",
            Self::MatrixMirror => "matrix-mirror",
            Self::MatrixReadRouting => "matrix-read-routing",
            Self::MatrixDisplayTimeout => "matrix-display-timeout",
            Self::MatrixBuzzer => "matrix-buzzer",
            Self::MatrixLoadPreset => "matrix-load-preset",
            Self::MatrixSavePreset => "matrix-save-preset",
        }
    }

    /// What kind of reply this request is answered with, if any.
    ///
    /// `None` means fire and forget: the switch sends nothing back, so the new state cannot be
    /// confirmed. Every setting command is in that category.
    pub const fn expects(self) -> Option<ResponseKind> {
        match self {
            Self::ReadActivePort => Some(ResponseKind::ActivePort),
            Self::NetQuery => Some(ResponseKind::Net),
            Self::NetSet => Some(ResponseKind::Ack),
            Self::MatrixReadRouting => Some(ResponseKind::Routing),
            _ => None,
        }
    }

    /// Whether this request only reads state.
    ///
    /// Queries are the only requests the engine retransmits on a reply timeout: repeating a read
    /// is harmless, repeating a write is not.
    pub const fn is_query(self) -> bool {
        matches!(self, Self::ReadActivePort | Self::NetQuery | Self::MatrixReadRouting)
    }

    /// Whether the switch can confirm the effect of this request.
    ///
    /// Buzzer, display timeout and auto-detect cannot be read back on any series, so a CLI must
    /// report them as sent rather than applied.
    pub const fn is_verifiable(self) -> bool {
        self.expects().is_some()
    }
}

impl fmt::Display for RequestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Something the switch said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Response {
    /// The request was handed to the transport and the protocol defines no reply to it.
    ///
    /// Not a confirmation: nothing came back, because nothing ever does for this command.
    Accepted,

    /// The active port, converted from the 0-based wire value.
    ActivePort(Port),

    /// One network setting, with zero padding stripped.
    Net(NetSetting),

    /// `OK`: a network write was accepted. Takes effect after a power cycle.
    Ack,

    /// A matrix routing report.
    Routing(RoutingReply),

    /// A frame that no command table interprets. Only produced for raw exchanges.
    Frame(Frame),

    /// Text that no command table interprets. Only produced for raw exchanges.
    Text(WireBytes),
}

impl Response {
    /// The kind of response, without its payload.
    pub const fn kind(&self) -> ResponseKind {
        match self {
            Self::Accepted => ResponseKind::Accepted,
            Self::ActivePort(_) => ResponseKind::ActivePort,
            Self::Net(_) => ResponseKind::Net,
            Self::Ack => ResponseKind::Ack,
            Self::Routing(_) => ResponseKind::Routing,
            Self::Frame(_) => ResponseKind::Frame,
            Self::Text(_) => ResponseKind::Text,
        }
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted => f.write_str("accepted"),
            Self::ActivePort(port) => write!(f, "port {port}"),
            Self::Net(setting) => write!(f, "{} {setting}", setting.field()),
            Self::Ack => f.write_str("ok"),
            Self::Routing(_) => f.write_str("routing report"),
            Self::Frame(frame) => write!(f, "frame 0x{:02X}", frame.opcode),
            Self::Text(bytes) => match bytes.as_str() {
                Some(text) => f.write_str(text),
                None => write!(f, "{bytes:?}"),
            },
        }
    }
}

/// A [`Response`] without its payload: what an in-flight exchange is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ResponseKind {
    /// [`Response::Accepted`].
    Accepted,
    /// [`Response::ActivePort`].
    ActivePort,
    /// [`Response::Net`].
    Net,
    /// [`Response::Ack`].
    Ack,
    /// [`Response::Routing`].
    Routing,
    /// [`Response::Frame`].
    Frame,
    /// [`Response::Text`].
    Text,
}

impl ResponseKind {
    /// A stable, hyphenated name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::ActivePort => "active-port",
            Self::Net => "net",
            Self::Ack => "ack",
            Self::Routing => "routing",
            Self::Frame => "frame",
            Self::Text => "text",
        }
    }
}

impl fmt::Display for ResponseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One entry of one command table, for introspection and help output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandInfo {
    /// The request this entry describes.
    pub kind: RequestKind,
    /// The hex opcode, or `None` for ASCII commands.
    pub opcode: Option<u8>,
    /// A caveat worth printing next to the command, if any.
    pub note: Option<&'static str>,
}

impl CommandInfo {
    pub(crate) const fn frame(kind: RequestKind, opcode: u8) -> Self {
        Self { kind, opcode: Some(opcode), note: None }
    }

    pub(crate) const fn ascii(kind: RequestKind) -> Self {
        Self { kind, opcode: None, note: None }
    }

    pub(crate) const fn with_note(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reads_expect_replies() {
        assert_eq!(Request::ReadActivePort.expects(), Some(ResponseKind::ActivePort));
        assert_eq!(Request::NetQuery(NetField::Ip).expects(), Some(ResponseKind::Net));
        assert_eq!(Request::NetSet(NetSetting::Port(5000)).expects(), Some(ResponseKind::Ack));

        // write-only settings: the switch says nothing, so nothing can be verified
        assert_eq!(Request::Buzzer(Buzzer::Muted).expects(), None);
        assert_eq!(Request::Switch(Port::FIRST).expects(), None);
        assert!(!RequestKind::DisplayTimeout.is_verifiable());
        assert!(!RequestKind::Switch.is_verifiable());
    }

    #[test]
    fn only_queries_are_safe_to_repeat() {
        assert!(RequestKind::ReadActivePort.is_query());
        assert!(RequestKind::NetQuery.is_query());
        assert!(RequestKind::MatrixReadRouting.is_query());
        assert!(!RequestKind::NetSet.is_query());
        assert!(!RequestKind::Switch.is_query());
    }

    #[test]
    fn kinds_round_trip_from_requests() {
        let requests = [
            Request::Switch(Port::FIRST),
            Request::ReadActivePort,
            Request::Buzzer(Buzzer::Audible),
            Request::DisplayTimeout(DisplayTimeout::Never),
            Request::AutoInputDetect(Toggle::On),
            Request::StepOutput { output: Output::A, step: Step::Next },
            Request::MoveUsb,
            Request::KmPassthrough(Toggle::On),
            Request::HotkeyTrigger(HotkeyTrigger::RightCtrl),
            Request::AutoSwitch(Toggle::Off),
            Request::AutoSwitchInterval(SwitchInterval::from_secs(10).unwrap()),
            Request::NetQuery(NetField::Ip),
            Request::NetSet(NetSetting::Port(5000)),
            Request::MatrixRoute { input: Port::FIRST, output: Port::FIRST },
            Request::MatrixRouteAll(Port::FIRST),
            Request::MatrixMirror,
            Request::MatrixReadRouting,
            Request::MatrixDisplayTimeout(MatrixTimeout::NEVER),
            Request::MatrixBuzzer(Buzzer::Muted),
            Request::MatrixLoadPreset(Preset::FIRST),
            Request::MatrixSavePreset(Preset::FIRST),
        ];

        // every variant has a distinct kind and a non-empty name
        let mut kinds = std::collections::BTreeSet::new();
        for request in requests {
            let kind = request.kind();
            assert!(kinds.insert(kind), "duplicate kind for {request}");
            assert_ne!(kind.name(), "");
            assert_ne!(std::format!("{request}"), "");
        }
        assert_eq!(kinds.len(), requests.len());
    }
}
