//! The ASCII layer: the network side-channel, and dispatch to the matrix language.
//!
//! Two unrelated text protocols share this transport. This module owns the network
//! side-channel (`IP?`, `PT:5000;`) and the dispatch that tells the two apart; the matrix
//! command language lives in [`crate::wire::matrix`].
//!
//! Single-monitor KVMs carry their network settings as bare ASCII on the same connection as the hex
//! frames, with no framing of any kind. None of this is in TESmart's protocol PDFs; it comes
//! from packet captures of the official Windows controller and from community tools.
//!
//! # Two spellings, both reported working
//!
//! | Dialect | Query | Set |
//! | --- | --- | --- |
//! | [`AsciiDialect::Captured`] | `IP?` | `IP:10.0.2.12;` |
//! | [`AsciiDialect::TesmartSh`] | `IP?;` | `IP: 10.0.2.12;` |
//!
//! Replies are zero padded (`IP:010.000.002.012`, `PT:05000`) and may arrive split across
//! segments, so parsing normalises each octet and tolerates a missing terminator.
//!
//! # Two scopes, for matrices
//!
//! Matrices are the one protocol where TESmart *documents* this channel, and they document it
//! twice: the bare strings above work over LAN, while RS232 needs the same commands wrapped in
//! an `MT80xx` prefix. See [`AsciiScope`].
//!
//! Sources: `bbeaudoin/bash` (captures), `pschmitt/tesmart.sh`, `luborpetr/tesmart`, and
//! TESmart's 4x4, 8x8 and 16x16 matrix protocol documents for the wrapped forms.

use core::fmt;
use core::net::Ipv4Addr;

use crate::value::{NetField, NetSetting};
use crate::wire::matrix::{self, MatrixCommand, RoutingReply};
use crate::wire::text::{render, strip_prefix_ignore_case};
use crate::wire::{Transmit, TransmitKind, WireBytes};

/// Which spelling of the ASCII commands to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum AsciiDialect {
    /// What TESmart's own Windows controller sends: `IP?` and `IP:10.0.2.12;`.
    #[default]
    Captured,
    /// What `tesmart.sh` sends: `IP?;` and `IP: 10.0.2.12;`.
    TesmartSh,
}

/// Which transport the ASCII commands are going out over.
///
/// Only matrices care. TESmart's matrix documents list the network commands twice, once per
/// transport, and the two lists are not the same strings:
///
/// | Scope | Query | Set |
/// | --- | --- | --- |
/// | [`AsciiScope::Lan`] | `IP?` | `IP:010.000.002.012;` |
/// | [`AsciiScope::Serial`] | `MT8003IP?` | `MT8019IP:010.000.002.012;` |
///
/// Each form is annotated "for LAN connection only" or "for RS232 connection only" in the
/// documents, so sending the wrong one is expected to be ignored. Replies are identical in both
/// scopes, which is why only the encoder is scope aware.
///
/// Single-monitor KVMs have no wrapped form documented or captured, so they ignore this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum AsciiScope {
    /// Reached over TCP: the bare commands.
    #[default]
    Lan,
    /// Reached over an RS232 line: the `MT80xx` wrapped commands, on a matrix.
    Serial,
}

/// How to spell the ASCII commands: which dialect, and which transport's form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AsciiStyle {
    /// Which spelling to use.
    pub dialect: AsciiDialect,
    /// Which transport's command set to use.
    pub scope: AsciiScope,
}

impl AsciiStyle {
    /// The captured LAN spelling: what the official controller sends.
    pub const DEFAULT: Self = Self { dialect: AsciiDialect::Captured, scope: AsciiScope::Lan };

    /// Builds a style from a dialect, leaving the scope at [`AsciiScope::Lan`].
    pub const fn new(dialect: AsciiDialect) -> Self {
        Self { dialect, scope: AsciiScope::Lan }
    }

    /// Sets the transport's command set.
    #[must_use]
    pub const fn with_scope(mut self, scope: AsciiScope) -> Self {
        self.scope = scope;
        self
    }
}

impl From<AsciiDialect> for AsciiStyle {
    fn from(dialect: AsciiDialect) -> Self {
        Self::new(dialect)
    }
}

/// Encodes a query for one network field.
pub fn encode_query(field: NetField, dialect: AsciiDialect) -> Transmit {
    let bytes = match dialect {
        AsciiDialect::Captured => render(format_args!("{}?", field.keyword())),
        AsciiDialect::TesmartSh => render(format_args!("{}?;", field.keyword())),
    };
    Transmit::new(bytes, TransmitKind::Ascii)
}

/// Encodes a write of one network field.
pub fn encode_set(setting: NetSetting, dialect: AsciiDialect) -> Transmit {
    let keyword = setting.field().keyword();
    let bytes = match dialect {
        AsciiDialect::Captured => render(format_args!("{keyword}:{setting};")),
        AsciiDialect::TesmartSh => render(format_args!("{keyword}: {setting};")),
    };
    Transmit::new(bytes, TransmitKind::Ascii)
}

/// Encodes a reply the way a switch does: zero padded, with no space.
///
/// Used by the device emulator so tests see the padding real hardware sends.
pub fn encode_padded_reply(setting: NetSetting) -> WireBytes {
    render(format_args!("{}:{}", setting.field().keyword(), Padded(setting)))
}

/// Encodes a query for one network field in the matrix RS232 form (`MT8003IP?`).
///
/// Every query shares the one `MT8003` prefix. The dialect does not apply: the wrapped forms
/// are documented with exactly one spelling, so there is nothing to choose between.
pub fn encode_wrapped_query(field: NetField) -> Transmit {
    let bytes = render(format_args!("{QUERY_PREFIX}{}?", field.keyword()));
    Transmit::new(bytes, TransmitKind::Ascii)
}

/// Encodes a write of one network field in the matrix RS232 form (`MT8019IP:...;`).
///
/// The prefix is `MT8019` for the address fields and `MT8009` for the port. That inconsistency
/// is printed identically in TESmart's 4x4, 8x8 and 16x16 documents, so it is transcribed
/// rather than corrected.
///
/// Values are zero padded to the width the documents give (`AAA.BBB.CCC.DDD` and `XXXXX`),
/// unlike the bare LAN form, where captures of the official controller show no padding.
pub fn encode_wrapped_set(setting: NetSetting) -> Transmit {
    let field = setting.field();
    let prefix = write_prefix(field);
    let bytes = render(format_args!("{prefix}{}:{};", field.keyword(), Padded(setting)));
    Transmit::new(bytes, TransmitKind::Ascii)
}

/// The prefix every wrapped query shares.
const QUERY_PREFIX: &str = "MT8003";

/// The prefix a wrapped write uses, which is not the same for every field.
const fn write_prefix(field: NetField) -> &'static str {
    match field {
        // Printed as `MT8009PT:` in all three matrix documents, where the other three fields
        // use `MT8019`. Transcribed as documented.
        NetField::Port => "MT8009",
        NetField::Ip | NetField::Gateway | NetField::Mask => "MT8019",
    }
}

/// Every prefix that wraps a network command, for stripping one off on the device side.
const WRAPPERS: [&str; 3] = [QUERY_PREFIX, "MT8019", "MT8009"];

/// A setting's value, zero padded the way hardware pads it: `010.000.002.012`, `05000`.
struct Padded(NetSetting);

impl fmt::Display for Padded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            NetSetting::Ip(addr) | NetSetting::Gateway(addr) | NetSetting::Mask(addr) => {
                let [first, second, third, fourth] = addr.octets();
                write!(f, "{first:03}.{second:03}.{third:03}.{fourth:03}")
            }
            NetSetting::Port(port) => write!(f, "{port:05}"),
        }
    }
}

/// Something a switch said in ASCII.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AsciiReply {
    /// The value of one network field, normalised.
    Net(NetSetting),
    /// `OK`: a write was accepted. Settings only take effect after a power cycle.
    Ack,
    /// A matrix routing report.
    Routing(RoutingReply),
    /// Text this crate does not recognise.
    Unknown,
}

/// Parses a reply from a switch.
///
/// Leading zeros are stripped per octet, so `IP:010.000.002.012` becomes `10.0.2.12` and
/// `PT:05000` becomes `5000`.
pub fn parse_reply(text: &str) -> AsciiReply {
    let text = text.trim().trim_end_matches(';').trim();
    if text.is_empty() {
        return AsciiReply::Unknown;
    }
    if text.eq_ignore_ascii_case("ok") {
        return AsciiReply::Ack;
    }
    if let Some(body) = strip_prefix_ignore_case(text, "LINK:") {
        return RoutingReply::parse(body).map_or(AsciiReply::Unknown, AsciiReply::Routing);
    }
    match parse_field_value(text) {
        Some(setting) => AsciiReply::Net(setting),
        None => AsciiReply::Unknown,
    }
}

/// Something a controller said in ASCII, as seen from the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AsciiCommand {
    /// A query for one network field (`IP?`).
    Query(NetField),
    /// A write of one network field (`IP:10.0.2.12;`).
    Set(NetSetting),
    /// A matrix command (`MT00SW0102NT`).
    Matrix(MatrixCommand),
    /// Text the emulator does not implement.
    Unknown,
}

/// Parses a command sent to a switch. Used by the device emulator.
///
/// Accepts both scopes: an `MT80xx` wrapper is stripped before the command is read, so a
/// matrix emulator understands `MT8003IP?` and `IP?` alike.
pub fn parse_command(text: &str) -> AsciiCommand {
    let text = text.trim();
    // Matrix commands are checked first: `MT00...NT` cannot be confused with anything else,
    // and the network wrapper strips a different prefix.
    if let Some(command) = matrix::parse_command(text) {
        return AsciiCommand::Matrix(command);
    }

    let text = strip_wrapper(text);
    if let Some(keyword) = text.strip_suffix('?') {
        for field in NetField::ALL {
            if keyword.trim().eq_ignore_ascii_case(field.keyword()) {
                return AsciiCommand::Query(field);
            }
        }
        return AsciiCommand::Unknown;
    }
    match parse_field_value(text.trim_end_matches(';')) {
        Some(setting) => AsciiCommand::Set(setting),
        None => AsciiCommand::Unknown,
    }
}

fn parse_field_value(text: &str) -> Option<NetSetting> {
    for field in NetField::ALL {
        let Some(rest) = strip_prefix_ignore_case(text, field.keyword()) else {
            continue;
        };
        let value = rest.trim_start().strip_prefix(':')?.trim();
        return match field {
            NetField::Ip => parse_ipv4(value).map(NetSetting::Ip),
            NetField::Gateway => parse_ipv4(value).map(NetSetting::Gateway),
            NetField::Mask => parse_ipv4(value).map(NetSetting::Mask),
            NetField::Port => value.parse().ok().map(NetSetting::Port),
        };
    }
    None
}

/// Parses a dotted quad, accepting the zero padding switches reply with.
///
/// `Ipv4Addr::from_str` rejects `010.000.002.012` outright, so each octet is parsed on its own.
fn parse_ipv4(text: &str) -> Option<Ipv4Addr> {
    let mut octets = [0u8; 4];
    let mut seen = 0usize;
    for part in text.split('.') {
        if seen == octets.len() {
            return None;
        }
        octets[seen] = part.trim().parse().ok()?;
        seen += 1;
    }
    if seen != octets.len() {
        return None;
    }
    Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}

/// Removes an `MT80xx` wrapper, if there is one.
fn strip_wrapper(text: &str) -> &str {
    for wrapper in WRAPPERS {
        if let Some(rest) = strip_prefix_ignore_case(text, wrapper) {
            return rest;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: &WireBytes) -> &str {
        bytes.as_str().expect("ASCII")
    }

    #[test]
    fn encodes_both_documented_dialects() {
        let captured = encode_query(NetField::Ip, AsciiDialect::Captured);
        assert_eq!(text(&captured.bytes()), "IP?");
        let tesmart_sh = encode_query(NetField::Ip, AsciiDialect::TesmartSh);
        assert_eq!(text(&tesmart_sh.bytes()), "IP?;");

        let setting = NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12));
        assert_eq!(text(&encode_set(setting, AsciiDialect::Captured).bytes()), "IP:10.0.2.12;");
        assert_eq!(text(&encode_set(setting, AsciiDialect::TesmartSh).bytes()), "IP: 10.0.2.12;");
    }

    #[test]
    fn encodes_every_field() {
        for (setting, expected) in [
            (NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)), "IP:10.0.2.12;"),
            (NetSetting::Port(5000), "PT:5000;"),
            (NetSetting::Gateway(Ipv4Addr::new(10, 0, 2, 1)), "GW:10.0.2.1;"),
            (NetSetting::Mask(Ipv4Addr::new(255, 255, 255, 0)), "MA:255.255.255.0;"),
        ] {
            assert_eq!(text(&encode_set(setting, AsciiDialect::Captured).bytes()), expected);
        }
    }

    #[test]
    fn encodes_the_documented_matrix_rs232_forms() {
        // Queries all share one prefix.
        for (field, expected) in [
            (NetField::Ip, "MT8003IP?"),
            (NetField::Port, "MT8003PT?"),
            (NetField::Gateway, "MT8003GW?"),
            (NetField::Mask, "MT8003MA?"),
        ] {
            assert_eq!(text(&encode_wrapped_query(field).bytes()), expected);
        }

        // Writes do not: the port field uses MT8009 where the rest use MT8019.
        for (setting, expected) in [
            (NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)), "MT8019IP:010.000.002.012;"),
            (NetSetting::Port(5000), "MT8009PT:05000;"),
            (NetSetting::Gateway(Ipv4Addr::new(10, 0, 2, 1)), "MT8019GW:010.000.002.001;"),
            (NetSetting::Mask(Ipv4Addr::new(255, 255, 255, 0)), "MT8019MA:255.255.255.000;"),
        ] {
            assert_eq!(text(&encode_wrapped_set(setting).bytes()), expected);
        }
    }

    #[test]
    fn the_longest_command_still_fits_one_wire_unit() {
        let longest = encode_wrapped_set(NetSetting::Mask(Ipv4Addr::BROADCAST));
        assert_eq!(text(&longest.bytes()), "MT8019MA:255.255.255.255;");
        assert!(longest.bytes().len() <= crate::wire::MAX_WIRE_LEN);
    }

    #[test]
    fn a_device_understands_both_scopes() {
        assert_eq!(parse_command("MT8003IP?"), AsciiCommand::Query(NetField::Ip));
        assert_eq!(parse_command("mt8003pt?"), AsciiCommand::Query(NetField::Port));
        assert_eq!(
            parse_command("MT8019IP:010.000.002.012;"),
            AsciiCommand::Set(NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)))
        );
        assert_eq!(parse_command("MT8009PT:05000;"), AsciiCommand::Set(NetSetting::Port(5000)));
        // and the bare forms keep working
        assert_eq!(parse_command("IP?"), AsciiCommand::Query(NetField::Ip));
    }

    #[test]
    fn replies_are_padded_the_way_hardware_pads_them() {
        assert_eq!(
            text(&encode_padded_reply(NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)))),
            "IP:010.000.002.012"
        );
        assert_eq!(text(&encode_padded_reply(NetSetting::Port(5000))), "PT:05000");
    }

    #[test]
    fn strips_zero_padding_when_parsing() {
        assert_eq!(
            parse_reply("IP:010.000.002.012"),
            AsciiReply::Net(NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)))
        );
        assert_eq!(parse_reply("PT:05000;"), AsciiReply::Net(NetSetting::Port(5000)));
        assert_eq!(
            parse_reply("MA:255.255.255.000"),
            AsciiReply::Net(NetSetting::Mask(Ipv4Addr::new(255, 255, 255, 0)))
        );
        assert_eq!(
            parse_reply("GW: 010.000.002.001 "),
            AsciiReply::Net(NetSetting::Gateway(Ipv4Addr::new(10, 0, 2, 1)))
        );
    }

    #[test]
    fn recognises_acknowledgements_and_junk() {
        assert_eq!(parse_reply("OK"), AsciiReply::Ack);
        assert_eq!(parse_reply("ok;"), AsciiReply::Ack);
        assert_eq!(parse_reply(""), AsciiReply::Unknown);
        assert_eq!(parse_reply("IP:not.an.address"), AsciiReply::Unknown);
        assert_eq!(parse_reply("IP:1.2.3"), AsciiReply::Unknown);
        assert_eq!(parse_reply("IP:1.2.3.4.5"), AsciiReply::Unknown);
        assert_eq!(parse_reply("XX:1.2.3.4"), AsciiReply::Unknown);
    }

    #[test]
    fn parses_commands_from_the_device_side() {
        assert_eq!(parse_command("IP?"), AsciiCommand::Query(NetField::Ip));
        assert_eq!(parse_command("pt?"), AsciiCommand::Query(NetField::Port));
        assert_eq!(
            parse_command("IP: 10.0.2.12;"),
            AsciiCommand::Set(NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)))
        );
        assert_eq!(parse_command("ZZ?"), AsciiCommand::Unknown);
    }

    /// This module's job for the matrix language is to recognise it and hand it over. The
    /// grammar itself is tested in [`crate::wire::matrix`].
    #[test]
    fn hands_the_matrix_language_to_the_matrix_module() {
        // A reply: `LINK:` dispatches to the routing parser.
        let AsciiReply::Routing(routing) = parse_reply("LINK:O1I2;O2I1;END") else {
            panic!("expected a routing report")
        };
        assert_eq!(routing.entries().len(), 2);

        // A command: `MT00...NT` dispatches to the matrix command parser, and is checked
        // before the network wrapper, which strips a different prefix.
        assert_eq!(
            parse_command("MT00SW0102NT"),
            AsciiCommand::Matrix(MatrixCommand::Route {
                input: crate::port::Port::FIRST,
                output: crate::port::Port::new(2).unwrap(),
            })
        );

        // Something that looks like the matrix language but is not stays unrecognised rather
        // than falling through to the network parser.
        assert_eq!(parse_reply("LINK:;END"), AsciiReply::Unknown);
        assert_eq!(parse_command("MT00ZZ0000NT"), AsciiCommand::Unknown);
    }
}
