//! The ASCII layer: the network side-channel, and dispatch to the matrix language.
//!
//! Two unrelated text protocols share this transport. This module owns the network
//! side-channel (`IP?`, `PT:5000;`) and the dispatch that tells the two apart; the matrix
//! command language lives in [`crate::wire::matrix`].
//!
//! Family A KVMs carry their network settings as bare ASCII on the same connection as the hex
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
//! Matrices are the one family where TESmart *documents* this channel, and they document it
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
/// Family A KVMs have no wrapped form documented or captured, so they ignore this.
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
