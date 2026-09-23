//! The matrix command language: `MT00…NT` strings and the `LINK:` routing report.
//!
//! Family B speaks a different language from everything else TESmart makes. It shares the
//! transport with the network side-channel in [`ascii`](crate::wire::ascii), and nothing else:
//! the grammar, the vocabulary and the reply format are all its own.
//!
//! | Command | Meaning |
//! | --- | --- |
//! | `MT00SWXXYYNT` | route input `XX` to output `YY` |
//! | `MT00SWXX00NT` | route input `XX` to every output |
//! | `MT00SW0000NT` | mirror 1 to 1, 2 to 2, ... |
//! | `MT00RD0000NT` | request routing; answered `LINK:OYIX;END` |
//! | `MT00RD01XXNT` | load routing preset `XX` |
//! | `MT00SV00XXNT` | save routing into preset `XX` |
//! | `MT00BZEN01NT` | mute the buzzer; `00NT` unmutes |
//! | `MT00CTXXXXNT` | display timeout in seconds |
//!
//! Two traps live in that table. `MT00RD` is **two commands**: `MT00RD0000NT` reads the routing
//! while `MT00RD01XXNT` loads a preset, one character apart. And the buzzer values are
//! **inverted** against the KVM `0x02` command, where `0x00` mutes.
//!
//! Encoding the commands is the [`Matrix`](crate::profile::Matrix) profile's job; this module
//! parses them, which is what the device emulator needs, and owns the routing report in both
//! directions.
//!
//! Sources: TESmart's 4x4 (RS232), 4x4 (RS232+IP), 8x8 and 16x16 matrix protocol documents.

use core::fmt::{self, Write as _};

use crate::port::Port;
use crate::value::{Buzzer, MatrixTimeout, Preset};
use crate::wire::WireBytes;
use crate::wire::text::strip_prefix_ignore_case;

/// A matrix command, as seen from the matrix.
///
/// The `MT00`...`NT` envelope is stripped, and the fields are decoded into the values they
/// stand for. Used by the device emulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MatrixCommand {
    /// Route one input to one output (`MT00SWXXYYNT`).
    Route {
        /// The input to take.
        input: Port,
        /// The output to feed.
        output: Port,
    },
    /// Route one input to every output (`MT00SWXX00NT`).
    RouteAll(Port),
    /// Mirror input 1 to output 1, 2 to 2, and so on (`MT00SW0000NT`).
    Mirror,
    /// Report the current routing (`MT00RD0000NT`).
    ReadRouting,
    /// Load a stored routing preset (`MT00RD01XXNT`).
    LoadPreset(Preset),
    /// Save the current routing into a preset (`MT00SV00XXNT`).
    SavePreset(Preset),
    /// Set the display timeout in seconds (`MT00CTXXXXNT`).
    DisplayTimeout(MatrixTimeout),
    /// Mute or unmute the buzzer (`MT00BZEN01NT` mutes).
    Buzzer(Buzzer),
}

/// Parses an `MT00`...`NT` matrix command, or returns `None` if it is not one.
///
/// Every command has a six-character body: a two-letter keyword and four digits, except the
/// buzzer, whose keyword is four letters and whose field is two digits.
pub fn parse_command(text: &str) -> Option<MatrixCommand> {
    let body = strip_prefix_ignore_case(text, "MT00")?;
    let body = strip_suffix_ignore_case(body, "NT")?;

    // `BZEN` is the one four-letter keyword, so it is matched before the split.
    if let Some(field) = strip_prefix_ignore_case(body, "BZEN") {
        return match digits(field, 2)? {
            // Inverted against the KVM buzzer command, as documented.
            1 => Some(MatrixCommand::Buzzer(Buzzer::Muted)),
            0 => Some(MatrixCommand::Buzzer(Buzzer::Audible)),
            _ => None,
        };
    }

    let (keyword, args) = body.split_at_checked(2)?;
    let whole = digits(args, 4)?;
    let high = digits(args.get(..2)?, 2)?;
    let low = digits(args.get(2..)?, 2)?;
    let port = |value: u16| u8::try_from(value).ok().and_then(|n| Port::new(n).ok());

    if keyword.eq_ignore_ascii_case("SW") {
        return match (high, low) {
            (0, 0) => Some(MatrixCommand::Mirror),
            (_, 0) => Some(MatrixCommand::RouteAll(port(high)?)),
            // An output with no input is not a command the documents define.
            (0, _) => None,
            _ => Some(MatrixCommand::Route { input: port(high)?, output: port(low)? }),
        };
    }
    if keyword.eq_ignore_ascii_case("RD") {
        return match (high, low) {
            (0, 0) => Some(MatrixCommand::ReadRouting),
            // `MT00RD01XXNT` loads preset XX: one digit away from reading the routing.
            (1, slot) => Some(MatrixCommand::LoadPreset(preset(slot)?)),
            _ => None,
        };
    }
    if keyword.eq_ignore_ascii_case("SV") {
        return match (high, low) {
            (0, slot) => Some(MatrixCommand::SavePreset(preset(slot)?)),
            _ => None,
        };
    }
    if keyword.eq_ignore_ascii_case("CT") {
        return MatrixTimeout::from_secs(whole).ok().map(MatrixCommand::DisplayTimeout);
    }
    None
}

/// Renders the routing report a matrix answers `MT00RD0000NT` with.
///
/// `LINK:O1I2;O2I1;...;END`, with ports above 9 written as `A`-`G` the way a 16x16 does. Used
/// by the device emulator.
///
/// # Panics
///
/// Panics if more pairs are given than fit [`MAX_WIRE_LEN`](crate::wire::MAX_WIRE_LEN), which
/// for sixteen outputs it does not.
pub fn encode_link_reply(entries: &[(Port, Port)]) -> WireBytes {
    let mut bytes = WireBytes::new();
    bytes.write_str("LINK:").expect("the prefix fits");
    for (output, input) in entries {
        bytes
            .write_fmt(format_args!("O{}I{};", LinkPort(*output), LinkPort(*input)))
            .expect("sixteen pairs fit in MAX_WIRE_LEN");
    }
    bytes.write_str("END").expect("the terminator fits");
    bytes
}

/// A matrix routing report, parsed out of a `LINK:...;END` reply.
///
/// The documents describe one pair, as `LINK:OYIX;END` with `Y` the output and `X` the input,
/// and do not say how a multi-output matrix lays out the rest. This parser therefore collects
/// every `O<port>I<port>` pair it can find, separated by semicolons, and ignores anything else.
/// The pair layout is documented; the repetition is not.
///
/// Port fields are not plain decimal. A 16x16 matrix prints them as single characters from
/// `1`-`9` then `A`-`G`, so output 16 arrives as `G`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingReply {
    entries: [(Port, Port); Port::MAX as usize],
    len: u8,
}

impl RoutingReply {
    /// Parses the body of a `LINK:` reply, with the `LINK:` prefix already removed.
    pub(crate) fn parse(body: &str) -> Option<Self> {
        let body = body.trim().trim_end_matches("END").trim_end_matches(';');
        let mut entries = [(Port::FIRST, Port::FIRST); Port::MAX as usize];
        let mut len = 0usize;

        for token in body.split(';') {
            let token = token.trim();
            if token.is_empty() || len == entries.len() {
                continue;
            }
            let Some(rest) = strip_prefix_ignore_case(token, "O") else { continue };
            let Some((output, input)) = rest.split_once(['I', 'i']) else { continue };
            let (Some(output), Some(input)) = (parse_link_port(output), parse_link_port(input))
            else {
                continue;
            };
            entries[len] = (output, input);
            len += 1;
        }

        if len == 0 {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "len is bounded by entries.len() == 16"
        )]
        Some(Self { entries, len: len as u8 })
    }

    /// The `(output, input)` pairs that were understood.
    pub fn entries(&self) -> &[(Port, Port)] {
        &self.entries[..self.len as usize]
    }

    /// The input feeding one output, if the report mentions it.
    pub fn input_for(&self, output: Port) -> Option<Port> {
        self.entries().iter().find(|(out, _)| *out == output).map(|(_, input)| *input)
    }
}
