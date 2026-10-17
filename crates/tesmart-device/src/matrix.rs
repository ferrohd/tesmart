//! The `matrix` protocol in the emulator: routing, presets, the buzzer and the `LINK:` report.
//!
//! The only protocol whose state a controller can read back, which is why these tests can assert
//! what the emulated matrix *did* rather than only what it was sent.

use crate::{Device, SEGMENT, allows};
use tesmart_proto::port::{Port, PortCount};
use tesmart_proto::wire::{MatrixCommand, matrix};

/// Which input feeds each output of an emulated matrix.
///
/// Indexed by output, counting from one. Outputs past the model's count are tracked but never
/// reported, which keeps the type a fixed size regardless of the matrix being emulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Routing([Port; Port::MAX as usize]);

impl Routing {
    /// Mirrored: input 1 to output 1, 2 to 2, and so on. The documented power-on state.
    pub const MIRRORED: Self = {
        let mut inputs = [Port::FIRST; Port::MAX as usize];
        let mut number = 1u8;
        while number <= Port::MAX {
            inputs[number as usize - 1] = match Port::new(number) {
                Ok(port) => port,
                // Unreachable: number is in 1..=Port::MAX.
                Err(_) => Port::FIRST,
            };
            number += 1;
        }
        Self(inputs)
    };

    /// The input feeding one output.
    pub const fn input_for(&self, output: Port) -> Port {
        self.0[output.get() as usize - 1]
    }

    /// Points one output at one input.
    pub const fn connect(&mut self, output: Port, input: Port) {
        self.0[output.get() as usize - 1] = input;
    }

    /// Points every output at one input.
    pub fn connect_all(&mut self, input: Port) {
        self.0 = [input; Port::MAX as usize];
    }

    /// The `(output, input)` pairs for a matrix with this many outputs.
    fn pairs(&self, outputs: PortCount) -> impl Iterator<Item = (Port, Port)> + '_ {
        (1..=outputs.get())
            .filter_map(|number| Port::new(number).ok())
            .map(|output| (output, self.input_for(output)))
    }
}

impl Device {
    /// The `matrix` protocol. Routing is the only thing a matrix will tell you about.
    ///
    /// Called by the dispatcher in [`super`].
    pub(super) fn matrix_command(&mut self, command: MatrixCommand) {
        match command {
            MatrixCommand::Route { input, output } => {
                if self.has(input) && self.output_exists(output) {
                    self.state.routing.connect(output, input);
                    self.state.route_count += 1;
                }
            }
            MatrixCommand::RouteAll(input) => {
                if self.has(input) {
                    self.state.routing.connect_all(input);
                    self.state.route_count += 1;
                }
            }
            MatrixCommand::Mirror => {
                self.state.routing = Routing::MIRRORED;
                self.state.route_count += 1;
            }
            MatrixCommand::ReadRouting => self.send_routing_reply(),
            MatrixCommand::LoadPreset(slot) if allows(self.config.caps.presets) => {
                self.state.routing = self.state.presets[usize::from(slot.get()) - 1];
                self.state.route_count += 1;
            }
            MatrixCommand::SavePreset(slot) if allows(self.config.caps.presets) => {
                self.state.presets[usize::from(slot.get()) - 1] = self.state.routing;
            }
            MatrixCommand::DisplayTimeout(timeout) if allows(self.config.caps.display_timeout) => {
                self.state.matrix_timeout = timeout;
            }
            MatrixCommand::Buzzer(buzzer) if allows(self.config.caps.buzzer) => {
                self.state.buzzer = buzzer;
            }
            // Two things land here, both ignored exactly as the hardware ignores them:
            //
            // - a command this model does not have, when the guards above fail;
            // - a command added to `tesmart-proto` after this crate was written.
            //
            // The second is the price of the crate split: `MatrixCommand` is
            // `#[non_exhaustive]`, so a new variant cannot be a compile error here the way it
            // would be inside one crate. The equivalent guarantee on the controller side is
            // intact, because `Request` and the command tables live together in `tesmart-proto`.
            _ => {}
        }
    }

    fn send_routing_reply(&mut self) {
        if self.drop_this_reply() {
            return;
        }
        let outputs = self.config.outputs;
        let mut pairs = [(Port::FIRST, Port::FIRST); Port::MAX as usize];
        let mut len = 0;
        for pair in self.state.routing.pairs(outputs) {
            pairs[len] = pair;
            len += 1;
        }

        let reply = matrix::encode_link_reply(&pairs[..len]);
        if self.config.faults.split_ascii {
            // Long replies really do arrive in pieces: a 16x16 report is 88 bytes and the
            // bridge hands over about 48 at a time.
            self.push_split(reply, SEGMENT);
        } else {
            self.push(reply);
        }
    }

    fn has(&self, input: Port) -> bool {
        self.config.ports.contains(input)
    }

    fn output_exists(&self, output: Port) -> bool {
        self.config.outputs.contains(output)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{matrix as emulated, segments, text};
    use crate::{Device, Faults};
    use core::net::Ipv4Addr;
    use tesmart_proto::model;
    use tesmart_proto::value::{Buzzer, MatrixTimeout};
    #[test]
    fn a_matrix_routes_and_reports_its_routing() {
        let mut device = emulated("HMA0404B30");

        // mirrored at power on
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I1;O2I2;O3I3;O4I4;END");

        // route input 1 to output 2
        device.handle_input(b"MT00SW0102NT");
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I1;O2I1;O3I3;O4I4;END");
        assert_eq!(device.state().route_count, 1);

        // one input to every output
        device.handle_input(b"MT00SW0300NT");
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I3;O2I3;O3I3;O4I3;END");

        // and back to mirrored
        device.handle_input(b"MT00SW0000NT");
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I1;O2I2;O3I3;O4I4;END");
    }

    #[test]
    fn a_matrix_ignores_ports_it_does_not_have() {
        let mut device = emulated("HMA0404B30");
        device.handle_input(b"MT00SW0808NT");
        assert_eq!(device.state().route_count, 0, "a 4x4 has no input 8");
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I1;O2I2;O3I3;O4I4;END");
    }

    /// The reply that sets `MAX_WIRE_LEN`: sixteen pairs, 88 bytes, two segments.
    #[test]
    fn a_16x16_reports_sixteen_pairs_as_hex_digits() {
        let mut device = emulated("HMA1616A10");
        device.handle_input(b"MT00SW1010NT");
        device.handle_input(b"MT00RD0000NT");

        let reply = text(&mut device);
        assert_eq!(
            reply,
            "LINK:O1I1;O2I2;O3I3;O4I4;O5I5;O6I6;O7I7;O8I8;O9I9;OAIA;OBIB;OCIC;ODID;OEIE;OFIF;OGIG;END"
        );
        assert_eq!(reply.len(), 88);

        // output 10 is `A`, and the input it was pointed at is also 10
        assert!(reply.contains("OAIA"), "{reply}");
    }

    #[test]
    fn a_long_routing_reply_arrives_in_segments() {
        let mut device = Device::from_model(model::lookup("HMA1616A10").unwrap());
        device.set_faults(Faults::REALISTIC);
        device.handle_input(b"MT00RD0000NT");

        let pieces = segments(&mut device);
        assert_eq!(pieces.len(), 2, "88 bytes does not fit one 48 byte segment");
        assert_eq!(pieces[0].len(), 48);
        assert_eq!(pieces.concat().len(), 88);
    }

    #[test]
    fn a_matrix_saves_and_loads_presets() {
        let mut device = emulated("HMA0808A10");

        device.handle_input(b"MT00SW0102NT");
        device.handle_input(b"MT00SV0003NT");

        // change the routing, then load the preset back
        device.handle_input(b"MT00SW0000NT");
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I1;O2I2;O3I3;O4I4;O5I5;O6I6;O7I7;O8I8;END");

        device.handle_input(b"MT00RD0103NT");
        device.handle_input(b"MT00RD0000NT");
        assert_eq!(text(&mut device), "LINK:O1I1;O2I1;O3I3;O4I4;O5I5;O6I6;O7I7;O8I8;END");
    }

    /// A 4x4 has the display timeout and no presets; an 8x8 is the other way round.
    #[test]
    fn a_matrix_honours_only_its_own_documented_commands() {
        let mut four = emulated("HMA0404B30");
        four.handle_input(b"MT00CT0030NT");
        assert_eq!(four.state().matrix_timeout, MatrixTimeout::from_secs(30).unwrap());
        four.handle_input(b"MT00SW0102NT");
        four.handle_input(b"MT00SV0001NT");
        four.handle_input(b"MT00SW0000NT");
        four.handle_input(b"MT00RD0101NT");
        four.handle_input(b"MT00RD0000NT");
        assert_eq!(
            text(&mut four),
            "LINK:O1I1;O2I2;O3I3;O4I4;END",
            "a 4x4 has no presets, so loading one must change nothing"
        );

        let mut eight = emulated("HMA0808A10");
        eight.handle_input(b"MT00CT0030NT");
        assert_eq!(
            eight.state().matrix_timeout,
            MatrixTimeout::NEVER,
            "an 8x8 has no MT00CT command"
        );

        // the buzzer, whose values are inverted against the KVM command
        eight.handle_input(b"MT00BZEN01NT");
        assert_eq!(eight.state().buzzer, Buzzer::Muted);
        eight.handle_input(b"MT00BZEN00NT");
        assert_eq!(eight.state().buzzer, Buzzer::Audible);
    }

    #[test]
    fn a_matrix_takes_network_commands_in_both_scopes() {
        let mut device = emulated("HMA0808A10");

        device.handle_input(b"MT8003IP?");
        assert_eq!(text(&mut device), "IP:192.168.001.010;");

        device.handle_input(b"MT8019IP:010.000.002.012;");
        assert_eq!(text(&mut device), "OK");
        assert_eq!(device.state().net.ip, Some(Ipv4Addr::new(10, 0, 2, 12)));

        // and the bare LAN form reaches the same state
        device.handle_input(b"IP:10.0.2.13;");
        assert_eq!(text(&mut device), "OK");
        assert_eq!(device.state().net.ip, Some(Ipv4Addr::new(10, 0, 2, 13)));
    }

    /// A matrix with no LAN port has no network commands at all.
    #[test]
    fn an_rs232_only_matrix_answers_no_network_query() {
        let mut device = emulated("HMA0404A40");
        device.handle_input(b"MT8003IP?");
        assert_eq!(segments(&mut device), [] as [std::vec::Vec<u8>; 0]);

        // but it still routes
        device.handle_input(b"MT00SW0102NT");
        assert_eq!(device.state().route_count, 1);
    }
}
