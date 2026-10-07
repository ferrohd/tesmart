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
