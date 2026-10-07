//! Families A and A' in the emulator: the hex frame handlers.
//!
//! The two share framing and the first two opcodes, and diverge from `0x03`: on a
//! single-monitor switch it sets the display timeout, on a dual-monitor one it steps output A's
//! input. Keeping them in one file keeps that clash visible.

use crate::{Device, allows, switch_from};
use tesmart_proto::port::Port;
use tesmart_proto::profile::{dual, kvm};
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, Output, Step, SwitchInterval, Toggle,
};
use tesmart_proto::wire::frame::{OPCODE_PORT_REPORT, TERMINATOR};

impl Device {
    /// Family A. Called by the dispatcher in [`super`].
    pub(super) fn single_monitor_frame(
        &mut self,
        opcode: u8,
        payload: tesmart_proto::wire::Payload,
    ) {
        let value = payload.as_slice()[0];
        match opcode {
            tesmart_proto::profile::base::OPCODE_SWITCH => self.switch_to(value),
            tesmart_proto::profile::base::OPCODE_BUZZER if allows(self.config.caps.buzzer) => {
                self.state.buzzer = if value == 0 { Buzzer::Muted } else { Buzzer::Audible };
            }
            kvm::OPCODE_DISPLAY_TIMEOUT if allows(self.config.caps.display_timeout) => {
                self.state.display_timeout = match value {
                    0x0A => DisplayTimeout::Seconds10,
                    0x1E => DisplayTimeout::Seconds30,
                    _ => DisplayTimeout::Never,
                };
            }
            kvm::OPCODE_AUTO_INPUT_DETECT if allows(self.config.caps.auto_detect) => {
                self.state.auto_detect = switch_from(value);
            }
            kvm::OPCODE_READ_PORT if allows(self.config.caps.read_port) => {
                self.send_port_report(self.state.active_port);
            }
            _ => {}
        }
    }

    /// Family A'. Called by the dispatcher in [`super`].
    pub(super) fn dual_monitor_frame(&mut self, opcode: u8, payload: tesmart_proto::wire::Payload) {
        let data = payload.as_slice();
        let value = data[0];
        match opcode {
            tesmart_proto::profile::base::OPCODE_SWITCH => self.switch_to(value),
            tesmart_proto::profile::base::OPCODE_BUZZER => {
                self.state.buzzer = if value == 0 { Buzzer::Muted } else { Buzzer::Audible };
            }
            dual::OPCODE_STEP_OUTPUT_A => {
                self.state.active_port = self.step(self.state.active_port, value);
                self.state.switch_count += 1;
            }
            dual::OPCODE_STEP_OUTPUT_B => {
                self.state.output_b_port = self.step(self.state.output_b_port, value);
            }
            dual::OPCODE_MOVE_USB => {
                self.state.usb_output = match self.state.usb_output {
                    Output::A => Output::B,
                    Output::B => Output::A,
                };
            }
            // Inverted, like the command it answers.
            dual::OPCODE_KM_PASSTHROUGH => {
                self.state.km_passthrough = if value == 0 { Toggle::On } else { Toggle::Off };
            }
            dual::OPCODE_HOTKEY_TRIGGER => {
                self.state.hotkey =
                    if value == 0 { HotkeyTrigger::ScrollLock } else { HotkeyTrigger::RightCtrl };
            }
            dual::OPCODE_AUTO_SWITCH => self.state.auto_switch = switch_from(value),
            dual::OPCODE_AUTO_SWITCH_INTERVAL => {
                if let [minutes, seconds] = *data {
                    self.state.auto_switch_interval =
                        SwitchInterval::from_raw(minutes, seconds).ok();
                }
            }
            _ => {}
        }
    }

    fn switch_to(&mut self, raw: u8) {
        // Writes are 1-based; a port the switch does not have is ignored.
        let Ok(port) = Port::new(raw) else { return };
        if !self.config.ports.contains(port) || self.config.faults.ignore_switch {
            return;
        }
        self.state.active_port = port;
        self.state.switch_count += 1;
    }

    fn step(&self, from: Port, direction: u8) -> Port {
        let last = self.config.ports.get();
        let current = from.get();
        let next = if Step::from_byte(direction) == Step::Next {
            if current >= last { 1 } else { current + 1 }
        } else if current <= 1 {
            last
        } else {
            current - 1
        };
        Port::new(next).unwrap_or(Port::FIRST)
    }

    /// Queues `AA BB 03 11 XX ..`. Also used by [`super`] for front panel presses.
    pub(super) fn send_port_report(&mut self, port: Port) {
        if self.drop_this_reply() {
            return;
        }
        self.reports += 1;
        let bogus = self
            .config
            .faults
            .bogus_port_every
            .is_some_and(|every| self.reports.is_multiple_of(u32::from(every.get())));

        let index = if bogus { 0x1F } else { port.to_read_byte() };
        self.emit(&[
            0xAA,
            0xBB,
            0x03,
            OPCODE_PORT_REPORT,
            index,
            self.config.faults.terminator.unwrap_or(TERMINATOR),
        ]);
    }
}
