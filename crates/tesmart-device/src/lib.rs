//! The switch side of the TESmart protocol.
//!
//! [`tesmart_proto`] is the controller: it turns requests into bytes and bytes into events.
//! This crate is the other end of the same wire — bytes in, bytes out, no IO and no clock —
//! so the two together describe the whole protocol without anything having to be plugged in.
//!
//! [`tesmart-sim`]: https://crates.io/crates/tesmart-sim
#![cfg_attr(not(test), no_std)]
// `must_use_candidate` fires on nearly every accessor in a crate made of value types.
#![allow(clippy::must_use_candidate)]

mod kvm;
mod matrix;

use core::net::Ipv4Addr;
use core::num::NonZeroU8;

use tesmart_proto::model::{Capabilities, ModelInfo, Support};
use tesmart_proto::port::{Port, PortCount};
use tesmart_proto::profile::Family;
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, MatrixTimeout, NetConfig, NetSetting, Output, Preset,
    SwitchInterval, Toggle,
};
use tesmart_proto::wire::{AsciiDialect, Chunk, DecodeMode, StreamDecoder, WireBytes, ascii};

pub use crate::matrix::Routing;

/// How many reply segments can be queued before the oldest is dropped.
const OUTBOX: usize = 8;

/// The queue of segments a device is waiting to put on the wire.
///
/// Fixed capacity, and it evicts the oldest rather than growing or panicking: there is no
/// allocator here, and a controller that has stopped reading is a controller that no longer
/// cares about the segment from eight replies ago.
#[derive(Debug)]
struct Outbox {
    items: [Option<WireBytes>; OUTBOX],
    head: usize,
    len: usize,
}

impl Outbox {
    const fn new() -> Self {
        Self { items: [const { None }; OUTBOX], head: 0, len: 0 }
    }

    /// Queues a segment, dropping the oldest if there is no room.
    fn push(&mut self, bytes: WireBytes) {
        if self.len == OUTBOX {
            let _evicted = self.pop();
        }
        self.items[(self.head + self.len) % OUTBOX] = Some(bytes);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<WireBytes> {
        if self.len == 0 {
            return None;
        }
        let item = self.items[self.head].take();
        self.head = (self.head + 1) % OUTBOX;
        self.len -= 1;
        item
    }
}

/// How many bytes the LAN bridge hands over at a time.
///
/// Observed at about 48, which is why a padded address reply arrives with its `;` in a segment
/// of its own, and why a 16x16 routing report arrives in two pieces.
const SEGMENT: usize = 48;

/// Deliberate misbehaviour, to reproduce what hardware does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Faults {
    /// Terminate frames with this byte instead of `0xEE`. `0x16` and `0x1C` have been observed.
    pub terminator: Option<u8>,

    /// Send the `;` of an address reply as its own segment, like the real LAN bridge does.
    pub split_ascii: bool,

    /// Drop every n-th reply. Read replies are sometimes simply missing.
    pub drop_every: Option<NonZeroU8>,

    /// Prepend this many junk bytes to the next reply.
    pub garbage_before: u8,

    /// Make every n-th port report carry an impossible port index.
    pub bogus_port_every: Option<NonZeroU8>,

    /// Accept switch commands and do nothing.
    ///
    /// A switch that acknowledges nothing and changes nothing is indistinguishable from a
    /// working one until the port is read back, which is exactly what makes this worth
    /// emulating: it is the only way a verified switch can legitimately fail.
    pub ignore_switch: bool,

    /// Acknowledge a network write with `OK` and store something else.
    ///
    /// The reason network writes are read back before the user is told to power cycle. Models a
    /// field the firmware silently adjusts or refuses.
    pub garble_net_writes: bool,
}

impl Faults {
    /// A perfectly behaved switch. Useful to isolate a failure, unlike real hardware.
    pub const NONE: Self = Self {
        terminator: None,
        split_ascii: false,
        drop_every: None,
        garbage_before: 0,
        bogus_port_every: None,
        ignore_switch: false,
        garble_net_writes: false,
    };

    /// What hardware actually does: address replies arrive in two segments.
    pub const REALISTIC: Self = Self { split_ascii: true, ..Self::NONE };
}

impl Default for Faults {
    fn default() -> Self {
        Self::REALISTIC
    }
}

/// What kind of switch to emulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceConfig {
    /// Which protocol family to speak.
    pub family: Family,
    /// How many inputs to pretend to have.
    pub ports: PortCount,
    /// How many outputs to pretend to have. One, for anything that is not a matrix.
    pub outputs: PortCount,
    /// Which commands to honour. Anything not available is silently ignored.
    pub caps: Capabilities,
    /// Which spelling of ASCII replies to produce.
    pub dialect: AsciiDialect,
    /// How to misbehave.
    pub faults: Faults,
}

impl DeviceConfig {
    /// Emulates a registry entry, including its capability gaps.
    pub fn from_model(model: &ModelInfo) -> Self {
        Self {
            family: model.family,
            ports: model.ports,
            outputs: model.outputs,
            caps: model.caps,
            dialect: AsciiDialect::Captured,
            faults: Faults::REALISTIC,
        }
    }

    /// Replaces the fault configuration.
    #[must_use]
    pub const fn with_faults(mut self, faults: Faults) -> Self {
        self.faults = faults;
        self
    }
}

/// The emulated switch's settings, for assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeviceState {
    /// The active input, as the front panel would show it.
    pub active_port: Port,
    /// The input feeding output B, on dual-monitor switches.
    pub output_b_port: Port,
    /// Buzzer state.
    pub buzzer: Buzzer,
    /// Front display timeout.
    pub display_timeout: DisplayTimeout,
    /// Automatic input detection.
    pub auto_detect: Toggle,
    /// Keyboard/mouse pass-through, dual-monitor only.
    pub km_passthrough: Toggle,
    /// Which key starts a hotkey sequence, dual-monitor only.
    pub hotkey: HotkeyTrigger,
    /// Auto-switching, dual-monitor only.
    pub auto_switch: Toggle,
    /// Auto-switch dwell time, dual-monitor only.
    pub auto_switch_interval: Option<SwitchInterval>,
    /// Which output the USB link is attached to, dual-monitor only.
    pub usb_output: Output,
    /// Network settings, as stored. Changes would need a power cycle to take effect.
    pub net: NetConfig,
    /// How many switch commands were accepted.
    pub switch_count: u32,
    /// Which input feeds each output, on a matrix.
    pub routing: Routing,
    /// The eight stored routing presets, on a matrix.
    ///
    /// All start out mirrored. What a real matrix returns from a slot that was never saved is
    /// not documented, so the emulator does not invent a distinction.
    pub presets: [Routing; Preset::COUNT as usize],
    /// The display timeout, on a matrix (`MT00CTXXXXNT`).
    pub matrix_timeout: MatrixTimeout,
    /// How many routing commands were accepted.
    pub route_count: u32,
}

impl DeviceState {
    /// The factory defaults from the documentation: 192.168.1.10:5000.
    fn new() -> Self {
        let mut net = NetConfig::default();
        net.set(NetSetting::Ip(Ipv4Addr::new(192, 168, 1, 10)));
        net.set(NetSetting::Port(5000));
        net.set(NetSetting::Gateway(Ipv4Addr::new(192, 168, 1, 1)));
        net.set(NetSetting::Mask(Ipv4Addr::new(255, 255, 255, 0)));

        Self {
            active_port: Port::FIRST,
            output_b_port: Port::FIRST,
            buzzer: Buzzer::Audible,
            display_timeout: DisplayTimeout::Seconds10,
            auto_detect: Toggle::On,
            km_passthrough: Toggle::On,
            hotkey: HotkeyTrigger::RightCtrl,
            auto_switch: Toggle::Off,
            auto_switch_interval: None,
            usb_output: Output::A,
            net,
            switch_count: 0,
            routing: Routing::MIRRORED,
            presets: [Routing::MIRRORED; Preset::COUNT as usize],
            matrix_timeout: MatrixTimeout::NEVER,
            route_count: 0,
        }
    }
}

/// An emulated switch.
#[derive(Debug)]
pub struct Device {
    config: DeviceConfig,
    state: DeviceState,
    decoder: StreamDecoder,
    outbox: Outbox,
    replies: u32,
    reports: u32,
}

impl Device {
    /// Creates an emulator.
    pub fn new(config: DeviceConfig) -> Self {
        Self {
            config,
            state: DeviceState::new(),
            decoder: StreamDecoder::with_mode(DecodeMode::Device),
            outbox: Outbox::new(),
            replies: 0,
            reports: 0,
        }
    }

    /// Creates an emulator of a registry entry.
    pub fn from_model(model: &ModelInfo) -> Self {
        Self::new(DeviceConfig::from_model(model))
    }

    /// The configuration in use.
    pub const fn config(&self) -> &DeviceConfig {
        &self.config
    }

    /// The current settings.
    pub const fn state(&self) -> &DeviceState {
        &self.state
    }

    /// Replaces the fault configuration mid-test.
    pub const fn set_faults(&mut self, faults: Faults) {
        self.config.faults = faults;
    }

    /// Feeds bytes written by a controller.
    pub fn handle_input(&mut self, data: &[u8]) {
        let mut rest = data;
        loop {
            let accepted = self.decoder.feed(rest);
            rest = &rest[accepted..];
            self.drain();

            if rest.is_empty() {
                break;
            }
            if accepted == 0 {
                self.decoder.reset();
            }
        }
    }

    /// The next segment the switch would put on the wire.
    pub fn poll_transmit(&mut self) -> Option<WireBytes> {
        self.outbox.pop()
    }

    /// Changes the active port without telling anyone.
    ///
    /// Models a unit that does not push unsolicited reports, which is firmware dependent and
    /// undocumented: the only way to notice is to read the port.
    pub fn force_port(&mut self, port: Port) {
        if !self.config.ports.contains(port) {
            return;
        }
        self.state.active_port = port;
        self.state.switch_count += 1;
    }

    /// Simulates someone pressing a front panel button, using a hotkey or the remote.
    ///
    /// The switch pushes an unsolicited port report when the active port changes by any means.
    pub fn press_button(&mut self, port: Port) {
        if !self.config.ports.contains(port) {
            return;
        }
        self.state.active_port = port;
        self.state.switch_count += 1;
        self.send_port_report(port);
    }

    fn drain(&mut self) {
        loop {
            let action = match self.decoder.poll_chunk() {
                None => break,
                Some(Chunk::Frame(frame)) => Action::Frame(frame.opcode, frame.payload),
                Some(Chunk::Ascii(text)) => Action::Ascii(ascii::parse_command(text)),
                // A controller never sends port reports, and junk needs no answer.
                Some(Chunk::PortReport(_) | Chunk::BadPortReport { .. } | Chunk::Discarded(_)) => {
                    continue;
                }
            };
            self.act(action);
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Frame(opcode, payload) => match self.config.family {
                Family::Kvm => self.single_monitor_frame(opcode, payload),
                Family::KvmDual => self.dual_monitor_frame(opcode, payload),
                // Matrices have no hex protocol at all, and the SDI switch's four-byte
                // commands are not frames this decoder produces.
                Family::Matrix | Family::Sdi => {}
            },
            Action::Ascii(command) => self.ascii_command(command),
        }
    }

    fn ascii_command(&mut self, command: ascii::AsciiCommand) {
        // The matrix commands are gated per capability, not by the network flag.
        if let ascii::AsciiCommand::Matrix(matrix) = command {
            if self.config.family == Family::Matrix {
                self.matrix_command(matrix);
            }
            return;
        }

        if !allows(self.config.caps.net_config) {
            return;
        }
        match command {
            ascii::AsciiCommand::Query(field) => {
                if let Some(setting) = self.state.net.get(field) {
                    self.send_net_reply(setting);
                }
            }
            ascii::AsciiCommand::Set(setting) => {
                let stored =
                    if self.config.faults.garble_net_writes { garble(setting) } else { setting };
                self.state.net.set(stored);
                // `OK` means "stored", not "stored what you asked for".
                self.send_text("OK");
            }
            // `Matrix` is handled above and `Unknown` is text no switch recognises. The
            // wildcard rather than a list is forced by `#[non_exhaustive]`, as in
            // `matrix_command`.
            _ => {}
        }
    }

    fn send_net_reply(&mut self, setting: NetSetting) {
        if self.drop_this_reply() {
            return;
        }
        // Hardware pads every field: `IP:010.000.002.012`, `PT:05000`.
        let mut padded = ascii::encode_padded_reply(setting);
        if self.config.faults.split_ascii {
            // ... and sends the terminator in a segment of its own.
            self.push(padded);
            self.send_text(";");
        } else {
            use core::fmt::Write as _;
            padded.write_str(";").expect("a padded reply plus ';' fits in one wire unit");
            self.push(padded);
        }
    }

    fn send_text(&mut self, text: &str) {
        self.emit(text.as_bytes());
    }

    fn emit(&mut self, bytes: &[u8]) {
        let garbage = self.config.faults.garbage_before;
        if garbage > 0 {
            let junk = [0xFF_u8; 4];
            let len = usize::from(garbage).min(junk.len());
            self.push(WireBytes::from_bounded(&junk[..len]));
        }
        self.push(WireBytes::from_bounded(bytes));
    }

    fn push(&mut self, bytes: WireBytes) {
        self.outbox.push(bytes);
    }

    /// Hands over a long reply the way the bridge does: in segments.
    fn push_split(&mut self, bytes: WireBytes, segment: usize) {
        let data = bytes.as_slice();
        if data.len() <= segment {
            self.push(bytes);
            return;
        }
        for chunk in data.chunks(segment) {
            self.outbox.push(WireBytes::from_bounded(chunk));
        }
    }

    fn drop_this_reply(&mut self) -> bool {
        self.replies += 1;
        self.config
            .faults
            .drop_every
            .is_some_and(|every| self.replies.is_multiple_of(u32::from(every.get())))
    }
}

#[derive(Debug, Clone, Copy)]
enum Action {
    Frame(u8, tesmart_proto::wire::Payload),
    Ascii(ascii::AsciiCommand),
}

/// Whether to honour a command, given what the model is known to support.
///
/// [`Support::Unsupported`](tesmart_proto::Support) commands are ignored, exactly as the real model
/// ignores them. Unverified ones are honoured: emulating optimistic hardware is what lets a
/// caller's "try it anyway" path be tested.
fn allows(support: Support) -> bool {
    support.is_worth_trying()
}

fn switch_from(value: u8) -> Toggle {
    if value == 0 { Toggle::Off } else { Toggle::On }
}

/// Changes a setting just enough for a read-back to notice.
fn garble(setting: NetSetting) -> NetSetting {
    match setting {
        NetSetting::Ip(addr) => NetSetting::Ip(bump(addr)),
        NetSetting::Gateway(addr) => NetSetting::Gateway(bump(addr)),
        NetSetting::Mask(addr) => NetSetting::Mask(bump(addr)),
        NetSetting::Port(port) => NetSetting::Port(port.wrapping_add(1)),
    }
}

fn bump(addr: Ipv4Addr) -> Ipv4Addr {
    let [a, b, c, d] = addr.octets();
    Ipv4Addr::new(a, b, c, d.wrapping_add(1))
}

/// Helpers the per-family test modules share.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{Device, Faults};
    use tesmart_proto::model;

    /// A 16-port EB23, the most capable KVM in the registry.
    pub(crate) fn eb23() -> Device {
        device_for("HKS1601-EB23")
    }

    /// An emulated registry model, with the faults real hardware has.
    pub(crate) fn device_for(key: &str) -> Device {
        Device::from_model(model::lookup(key).unwrap_or_else(|| panic!("unknown model {key}")))
    }

    /// An emulated matrix that behaves perfectly, so routing assertions stay readable.
    pub(crate) fn matrix(key: &str) -> Device {
        let mut device = device_for(key);
        device.set_faults(Faults::NONE);
        device
    }

    /// Everything the switch has queued, one entry per segment it would write.
    pub(crate) fn segments(device: &mut Device) -> std::vec::Vec<std::vec::Vec<u8>> {
        let mut out = std::vec::Vec::new();
        while let Some(segment) = device.poll_transmit() {
            out.push(segment.as_slice().to_vec());
        }
        out
    }

    /// The queued segments joined back into the string a controller would see.
    pub(crate) fn text(device: &mut Device) -> std::string::String {
        segments(device).concat().into_iter().map(char::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{eb23, matrix as emulated_matrix, segments};
    use super::*;
    use tesmart_proto::model;

    #[test]
    fn honours_missing_capabilities() {
        // the 4x1 DVI protocol has no read command at all
        let mut device = Device::from_model(model::lookup("DKS0401A30").unwrap());
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        assert!(segments(&mut device).is_empty(), "this model never answers a read");
    }

    #[test]
    fn answers_network_queries_padded_and_split() {
        let mut device = eb23();
        device.handle_input(b"IP?");
        assert_eq!(segments(&mut device), [b"IP:192.168.001.010".to_vec(), b";".to_vec()]);

        device.handle_input(b"PT?");
        assert_eq!(segments(&mut device), [b"PT:05000".to_vec(), b";".to_vec()]);
    }

    #[test]
    fn stores_network_writes_and_acknowledges_them() {
        let mut device = eb23();
        device.handle_input(b"IP:10.0.2.12;");
        assert_eq!(segments(&mut device), [b"OK".to_vec()]);
        assert_eq!(device.state().net.ip, Some(Ipv4Addr::new(10, 0, 2, 12)));
    }

    #[test]
    fn a_kvm_ignores_matrix_commands_and_the_reverse() {
        let mut kvm = eb23();
        kvm.handle_input(b"MT00SW0102NT");
        assert_eq!(segments(&mut kvm), [] as [std::vec::Vec<u8>; 0]);
        assert_eq!(kvm.state().route_count, 0);

        let mut matrix = emulated_matrix("HMA0404B30");
        matrix.handle_input(&[0xAA, 0xBB, 0x03, 0x01, 0x02, 0xEE]);
        assert_eq!(matrix.state().active_port, Port::FIRST, "a matrix has no switch opcode");
        assert_eq!(segments(&mut matrix), [] as [std::vec::Vec<u8>; 0]);
    }

    #[test]
    fn applies_the_configured_faults() {
        let mut device = eb23();
        device.set_faults(Faults {
            terminator: Some(0x16),
            drop_every: NonZeroU8::new(2),
            garbage_before: 2,
            ..Faults::NONE
        });

        // first read: garbage prefix and a 0x16 terminator
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        assert_eq!(
            segments(&mut device),
            [std::vec![0xFF, 0xFF], std::vec![0xAA, 0xBB, 0x03, 0x11, 0x00, 0x16]]
        );

        // second read: dropped entirely
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        assert_eq!(segments(&mut device).len(), 0);
    }

    #[test]
    fn bogus_port_reports_are_deterministic() {
        let mut device = eb23();
        device.set_faults(Faults { bogus_port_every: NonZeroU8::new(1), ..Faults::NONE });
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        let [report] = segments(&mut device).try_into().unwrap();
        assert_eq!(report[4], 0x1F, "an index no switch could mean");
    }

    #[test]
    fn a_garbled_write_is_acknowledged_anyway() {
        let mut device = eb23();
        device.set_faults(Faults { garble_net_writes: true, ..Faults::NONE });

        device.handle_input(b"IP:10.0.2.12;");
        assert_eq!(segments(&mut device), [b"OK".to_vec()], "OK means stored, not correct");
        assert_eq!(
            device.state().net.ip,
            Some(Ipv4Addr::new(10, 0, 2, 13)),
            "which is why a write has to be read back"
        );
    }
}
