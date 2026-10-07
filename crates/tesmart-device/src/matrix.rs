//! Family B in the emulator: routing, presets, the buzzer and the `LINK:` report.
//!
//! The only family whose state a controller can read back, which is why these tests can assert
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
    /// Family B. Routing is the only thing a matrix will tell you about.
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
            // intact, because `Request` and the profile tables live together in `tesmart-proto`.
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
