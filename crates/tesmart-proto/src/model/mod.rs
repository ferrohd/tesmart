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

/// What a model can do, with the provenance of each claim.
///
/// The fields are the commands that vary *within* a family. Everything else is covered by
/// [`Self::baseline`], because the family already decides whether it exists at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Capabilities {
    /// Provenance of every command this struct does not track individually, switching included.
    pub baseline: Support,
    /// Reading the active port (`0x10` / `0x11`).
    pub read_port: Support,
    /// Muting the buzzer (`0x02` on a KVM, `MT00BZEN` on a matrix).
    pub buzzer: Support,
    /// Setting the display timeout (`0x03` on a KVM, `MT00CT` on a matrix).
    pub display_timeout: Support,
    /// Automatic input detection (`0x81`).
    pub auto_detect: Support,
    /// Saving and loading matrix routing presets (`MT00SV` / `MT00RD01`).
    pub presets: Support,
    /// The ASCII network side-channel.
    pub net_config: Support,
}

impl Capabilities {
    /// Everything unknown: the right starting point for a model nobody has tested.
    pub const UNVERIFIED: Self = Self {
        baseline: Support::Unverified,
        read_port: Support::Unverified,
        buzzer: Support::Unverified,
        display_timeout: Support::Unverified,
        auto_detect: Support::Unverified,
        presets: Support::Unverified,
        net_config: Support::Unverified,
    };

    /// Nothing at all: the starting point for building up a model that has few commands.
    pub const NONE: Self = Self {
        baseline: Support::Unsupported,
        read_port: Support::Unsupported,
        buzzer: Support::Unsupported,
        display_timeout: Support::Unsupported,
        auto_detect: Support::Unsupported,
        presets: Support::Unsupported,
        net_config: Support::Unsupported,
    };

    /// What is known about one kind of request.
    ///
    /// One field can back more than one request, because the families spell the same idea
    /// differently: a KVM's `0x03` and a matrix's `MT00CT` are both `display_timeout`. No model
    /// has both, so sharing the field cannot lose information.
    pub const fn support(self, kind: RequestKind) -> Support {
        match kind {
            RequestKind::ReadActivePort => self.read_port,
            RequestKind::Buzzer | RequestKind::MatrixBuzzer => self.buzzer,
            RequestKind::DisplayTimeout | RequestKind::MatrixDisplayTimeout => self.display_timeout,
            RequestKind::AutoInputDetect => self.auto_detect,
            RequestKind::MatrixLoadPreset | RequestKind::MatrixSavePreset => self.presets,
            RequestKind::NetQuery | RequestKind::NetSet => self.net_config,
            _ => self.baseline,
        }
    }
}

/// One entry of the registry.
///
/// Fields are public so a caller can take a registry entry and override what it has tested on
/// its own hardware, without the registry having to model configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    /// The canonical model name, as TESmart prints it.
    pub key: &'static str,
    /// Other names that resolve to this entry.
    pub aliases: &'static [&'static str],
    /// Which protocol family the model speaks.
    pub family: Family,
    /// How many inputs it has.
    pub ports: PortCount,
    /// How many outputs it has.
    ///
    /// One for a single-monitor KVM, two for a dual-monitor one, and the output count for a
    /// matrix. A matrix needs this to range-check a routing command, which is the only reason
    /// it is tracked at all.
    pub outputs: PortCount,
    /// What is known to work.
    pub caps: Capabilities,
    /// Anything a user should know before trusting the entry.
    pub notes: &'static str,
}

impl ModelInfo {
    /// Whether the model both speaks and is believed to implement a kind of request.
    pub fn supports(&self, kind: RequestKind) -> bool {
        self.family.supports(kind) && self.caps.support(kind).is_available()
    }

    /// What is known about a kind of request, taking the family into account.
    ///
    /// A command the family does not have at all is [`Support::Unsupported`], whatever the
    /// capability flags say.
    pub fn support(&self, kind: RequestKind) -> Support {
        if self.family.supports(kind) { self.caps.support(kind) } else { Support::Unsupported }
    }

    /// Whether `name` refers to this model, ignoring case.
    pub fn matches(&self, name: &str) -> bool {
        let name = name.trim();
        name.eq_ignore_ascii_case(self.key)
            || self.aliases.iter().any(|alias| name.eq_ignore_ascii_case(alias))
    }
}

impl fmt::Display for ModelInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.family == Family::Matrix {
            return write!(f, "{} ({}, {}x{})", self.key, self.family, self.ports, self.outputs);
        }
        write!(f, "{} ({}, {} ports)", self.key, self.family, self.ports)
    }
}

/// Finds a model by name or alias, ignoring case.
pub fn lookup(name: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|model| model.matches(name))
}
