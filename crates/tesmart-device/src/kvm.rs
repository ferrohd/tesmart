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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Faults;
    use crate::test_support::{eb23, segments};
    use tesmart_proto::model;

    #[test]
    fn switches_and_reports_the_port_zero_based() {
        let mut device = eb23();
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x01, 0x10, 0xEE]);
        assert_eq!(device.state().active_port.get(), 16);
        assert_eq!(device.state().switch_count, 1);

        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        assert_eq!(segments(&mut device), [[0xAA, 0xBB, 0x03, 0x11, 0x0F, 0xEE]]);
    }

    #[test]
    fn ignores_ports_it_does_not_have() {
        let mut device = Device::from_model(model::lookup("DKS0401A30").unwrap());
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x01, 0x08, 0xEE]);
        assert_eq!(device.state().active_port, Port::FIRST, "a 4-port switch has no PC8");
    }

    #[test]
    fn a_switch_that_ignores_commands_still_looks_normal() {
        let mut device = eb23();
        device.set_faults(Faults { ignore_switch: true, ..Faults::NONE });

        device.handle_input(&[0xAA, 0xBB, 0x03, 0x01, 0x05, 0xEE]);
        assert_eq!(segments(&mut device).len(), 0, "a switch command is never acknowledged");
        assert_eq!(device.state().active_port, Port::FIRST, "but nothing happened");

        // the only way to find out is to read the port back
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        assert_eq!(segments(&mut device), [[0xAA, 0xBB, 0x03, 0x11, 0x00, 0xEE]]);
    }

    #[test]
    fn force_port_changes_the_state_silently() {
        let mut device = eb23();
        device.set_faults(Faults::NONE);
        device.force_port(Port::new(7).unwrap());

        assert_eq!(device.state().active_port.get(), 7);
        assert_eq!(segments(&mut device).len(), 0, "no report: nothing to notice");
    }

    #[test]
    fn front_panel_changes_push_an_unsolicited_report() {
        let mut device = eb23();
        device.set_faults(Faults::NONE);
        device.press_button(Port::new(5).unwrap());
        assert_eq!(device.state().active_port.get(), 5);
        assert_eq!(segments(&mut device), [[0xAA, 0xBB, 0x03, 0x11, 0x04, 0xEE]]);
    }

    #[test]
    fn dual_monitor_steps_wrap_around() {
        let mut device = Device::from_model(model::lookup("dual-4port").unwrap());
        device.set_faults(Faults::NONE);

        // output A: step down from PC1 wraps to PC4
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x03, 0x00, 0xEE]);
        assert_eq!(device.state().active_port.get(), 4);
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x03, 0x01, 0xEE]);
        assert_eq!(device.state().active_port.get(), 1);

        // output B is independent
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x04, 0x01, 0xEE]);
        assert_eq!(device.state().output_b_port.get(), 2);
        assert_eq!(device.state().active_port.get(), 1);

        // inverted pass-through values
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x06, 0x01, 0xEE]);
        assert_eq!(device.state().km_passthrough, Toggle::Off);
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x06, 0x00, 0xEE]);
        assert_eq!(device.state().km_passthrough, Toggle::On);

        // the seven-byte interval frame
        device.handle_input(&[0xAA, 0xBB, 0x04, 0x09, 0x00, 0x1E, 0xEE]);
        assert_eq!(device.state().auto_switch_interval.map(SwitchInterval::as_secs), Some(30));

        // and no reply to anything, ever
        assert_eq!(segments(&mut device).len(), 0);
    }

    #[test]
    fn a_dual_monitor_switch_never_answers_a_read() {
        let mut device = Device::from_model(model::lookup("dual-4port").unwrap());
        device.handle_input(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
        assert_eq!(segments(&mut device).len(), 0);
    }
}
