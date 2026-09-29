//! The model registry: how many ports, which family, and what is actually known to work.
//!
//! Only "switch input" is documented for every TESmart series. Everything else depends on which
//! PDF covers your model, and several commands are known to work only because community tools
//! use them successfully. The registry records that difference instead of flattening it: a
//! capability is [`Support::Documented`], [`Support::FieldTested`], [`Support::Unverified`] or
//! [`Support::Unsupported`], so a CLI can say *why* it is about to try something, or refuse.
//!
//! Capabilities are a property of the *model*; the wire encoding is a property of the
//! [`Family`]. The two are deliberately separate: an undocumented switch can be driven by
//! picking a family and a port count without inventing a registry entry.
//!
//! This module holds the vocabulary; [`registry`] holds the data. They are split because the
//! table is long and changes for a different reason: adding a model should not touch the code
//! that decides what a capability means.

use core::fmt;

use crate::command::RequestKind;
use crate::port::PortCount;
use crate::profile::{Family, Profile};

/// How well established a capability is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Support {
    /// In an official TESmart document for this model.
    Documented,
    /// Not in any TESmart document, but used successfully by community tools on this model.
    FieldTested,
    /// No evidence either way. Try it, but do not trust the result.
    Unverified,
    /// Known not to exist on this model.
    Unsupported,
}

impl Support {
    /// Whether it is reasonable to rely on this capability.
    ///
    /// [`Support::Unverified`] is *not* available: an operation that needs a reliable answer
    /// (such as verifying a port change) must refuse, or say it could not verify.
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Documented | Self::FieldTested)
    }

    /// Whether trying it is worthwhile even though the outcome is unknown.
    pub const fn is_worth_trying(self) -> bool {
        matches!(self, Self::Documented | Self::FieldTested | Self::Unverified)
    }

    /// A stable, hyphenated name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Documented => "documented",
            Self::FieldTested => "field-tested",
            Self::Unverified => "unverified",
            Self::Unsupported => "unsupported",
        }
    }

    /// A sentence fit for `--help` or a warning.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Documented => "documented by TESmart for this model",
            Self::FieldTested => "not in TESmart's documents, but working in community tools",
            Self::Unverified => "no evidence either way: test it on your unit",
            Self::Unsupported => "does not exist on this model",
        }
    }
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
