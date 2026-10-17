//! The matrix command language: `MT00…NT` strings and the `LINK:` routing report.
//!
//! The `matrix` protocol speaks a different language from everything else TESmart makes. It
//! shares the transport with the network side-channel in [`ascii`](crate::wire::ascii), and
//! nothing else: the grammar, the vocabulary and the reply format are all its own.
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
//! Encoding the commands is [`MatrixTable`](crate::protocol::MatrixTable)'s job; this module
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

/// A port as a `LINK:` reply writes it: `1`-`9` then `A`-`G`.
struct LinkPort(Port);

impl fmt::Display for LinkPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.get() {
            number @ 1..=9 => write!(f, "{number}"),
            number => f.write_char(char::from(b'A' + number - 10)),
        }
    }
}

/// Parses one port field of a `LINK:` reply.
///
/// 4x4 and 8x8 matrices print plain digits (`1`-`4` and `1`-`8`). A 16x16 prints a single
/// character from `1`-`9`, `A`, `B`, `C`-`G`, so 10 is `A` and 16 is `G`. A longer field is
/// read as decimal: no document shows one, but accepting it costs nothing and a firmware that
/// printed `O10I16` would otherwise be unreadable.
fn parse_link_port(field: &str) -> Option<Port> {
    let field = field.trim();
    let number = match *field.as_bytes() {
        [digit @ b'1'..=b'9'] => digit - b'0',
        [letter @ (b'A'..=b'G' | b'a'..=b'g')] => (letter | 0x20) - b'a' + 10,
        [_] => return None,
        _ => field.parse().ok()?,
    };
    Port::new(number).ok()
}

/// Reads a fixed-width run of decimal digits, rejecting anything else.
fn digits(text: &str, width: usize) -> Option<u16> {
    if text.len() != width || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn preset(slot: u16) -> Option<Preset> {
    u8::try_from(slot).ok().and_then(|slot| Preset::new(slot).ok())
}

fn strip_suffix_ignore_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let at = text.len().checked_sub(suffix.len())?;
    let (rest, tail) = text.split_at_checked(at)?;
    tail.eq_ignore_ascii_case(suffix).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(number: u8) -> Port {
        Port::new(number).expect("a valid port")
    }

    fn routing(body: &str) -> RoutingReply {
        RoutingReply::parse(body).unwrap_or_else(|| panic!("{body:?} did not parse"))
    }

    #[test]
    fn parses_every_documented_command() {
        assert_eq!(
            parse_command("MT00SW0102NT"),
            Some(MatrixCommand::Route { input: port(1), output: port(2) })
        );
        assert_eq!(parse_command("MT00SW0300NT"), Some(MatrixCommand::RouteAll(port(3))));
        assert_eq!(parse_command("MT00SW0000NT"), Some(MatrixCommand::Mirror));
        assert_eq!(parse_command("MT00RD0000NT"), Some(MatrixCommand::ReadRouting));
        assert_eq!(
            parse_command("MT00RD0103NT"),
            Some(MatrixCommand::LoadPreset(Preset::new(3).unwrap()))
        );
        assert_eq!(
            parse_command("MT00SV0008NT"),
            Some(MatrixCommand::SavePreset(Preset::new(8).unwrap()))
        );
        assert_eq!(
            parse_command("MT00CT0030NT"),
            Some(MatrixCommand::DisplayTimeout(MatrixTimeout::from_secs(30).unwrap()))
        );
        // 16x16 addresses two digits per field
        assert_eq!(
            parse_command("MT00SW1616NT"),
            Some(MatrixCommand::Route { input: port(16), output: port(16) })
        );
        // and the case is not promised
        assert_eq!(parse_command("mt00sw0000nt"), Some(MatrixCommand::Mirror));
    }

    /// `MT00RD` is two commands one character apart.
    #[test]
    fn reading_the_routing_is_not_loading_preset_one() {
        assert_eq!(parse_command("MT00RD0000NT"), Some(MatrixCommand::ReadRouting));
        assert_eq!(parse_command("MT00RD0101NT"), Some(MatrixCommand::LoadPreset(Preset::FIRST)));
    }

    /// The buzzer field is the opposite way round from the KVM command.
    #[test]
    fn the_buzzer_is_inverted_against_the_kvm_command() {
        assert_eq!(parse_command("MT00BZEN01NT"), Some(MatrixCommand::Buzzer(Buzzer::Muted)));
        assert_eq!(parse_command("MT00BZEN00NT"), Some(MatrixCommand::Buzzer(Buzzer::Audible)));
    }

    #[test]
    fn rejects_anything_that_is_not_a_matrix_command() {
        for text in [
            "IP?",           // the other ASCII language
            "MT00SW0102",    // no terminator
            "SW0102NT",      // no envelope
            "MT00ZZ0102NT",  // unknown keyword
            "MT00SW01XXNT",  // not digits
            "MT00SW0010NT",  // output with no input
            "MT00RD0209NT",  // neither a read nor a preset
            "MT00SV0009NT",  // preset 9 does not exist
            "MT00CT99999NT", // five digits
            "",
        ] {
            assert_eq!(parse_command(text), None, "{text:?}");
        }
    }

    #[test]
    fn renders_the_report_a_matrix_answers_with() {
        let pairs = [(port(1), port(1)), (port(2), port(1))];
        assert_eq!(encode_link_reply(&pairs).as_str(), Some("LINK:O1I1;O2I1;END"));

        // ports above 9 are letters, so a full 16x16 report is 88 bytes
        let wide: std::vec::Vec<_> = (1..=16).map(|n| (port(n), port(n))).collect();
        let reply = encode_link_reply(&wide);
        assert_eq!(reply.len(), 88);
        assert_eq!(
            reply.as_str(),
            Some(
                "LINK:O1I1;O2I2;O3I3;O4I4;O5I5;O6I6;O7I7;O8I8;O9I9;OAIA;OBIB;OCIC;ODID;OEIE;OFIF;OGIG;END"
            )
        );
    }

    #[test]
    fn parses_reports_best_effort() {
        let report = routing("O1I2;O2I1;END");
        assert_eq!(report.entries().len(), 2);
        assert_eq!(report.input_for(port(1)), Some(port(2)));
        assert_eq!(report.input_for(port(2)), Some(port(1)));
        assert_eq!(report.input_for(port(3)), None);

        assert_eq!(RoutingReply::parse(";END"), None);
    }

    /// A 16x16 prints ports above 9 as `A`-`G`, which is not decimal.
    #[test]
    fn parses_the_16x16_hex_style_port_digits() {
        let report = routing("OAI1;OGIG;O9IF;END");
        let entries = report.entries();
        assert_eq!(entries.len(), 3);
        assert_eq!((entries[0].0.get(), entries[0].1.get()), (10, 1));
        assert_eq!((entries[1].0.get(), entries[1].1.get()), (16, 16));
        assert_eq!((entries[2].0.get(), entries[2].1.get()), (9, 15));

        // lower case too, since nothing promises the case
        let lower = routing("ogia;END");
        assert_eq!((lower.entries()[0].0.get(), lower.entries()[0].1.get()), (16, 10));
    }

    #[test]
    fn rejects_port_digits_no_matrix_can_mean() {
        // H would be 17, past the end of the alphabet the documents use
        assert_eq!(RoutingReply::parse("OHI1;END"), None);
        // 0 is not a port: the documents count outputs from 1
        assert_eq!(RoutingReply::parse("O0I1;END"), None);
        assert_eq!(RoutingReply::parse("O1I0;END"), None);
        assert_eq!(RoutingReply::parse("O17I1;END"), None);

        // and a decimal field is still read, if one ever shows up
        let decimal = routing("O10I16;END");
        assert_eq!((decimal.entries()[0].0.get(), decimal.entries()[0].1.get()), (10, 16));
    }

    /// Every rendered report must parse back to what produced it.
    #[test]
    fn reports_round_trip() {
        let pairs: std::vec::Vec<_> = (1..=16).map(|n| (port(n), port(17 - n))).collect();
        let rendered = encode_link_reply(&pairs);
        let text = rendered.as_str().expect("ASCII");
        let body = text.strip_prefix("LINK:").expect("the prefix");
        assert_eq!(routing(body).entries(), pairs.as_slice());
    }
}
