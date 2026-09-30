//! The registry data: every model this crate knows about.
//!
//! Separated from [`super`] because this is a table, not logic. It changes when TESmart ships a
//! model or someone field-tests a command; the capability vocabulary next door changes when the
//! meaning of a capability changes. Keeping them apart means adding a row cannot disturb the
//! code that interprets rows.
//!
//! The model-to-document mapping comes from TESmart's own index, "LAN RS232 Control Protocols",
//! updated 2024-01-10. `docs/protocol-reference.md` transcribes each document and cites the
//! community tools that tested the undocumented parts.

use crate::model::{Capabilities, ModelInfo, Support};
use crate::port::PortCount;
use crate::profile::Family;

const DOC: Support = Support::Documented;
const FIELD: Support = Support::FieldTested;
const UNK: Support = Support::Unverified;
const NONE: Support = Support::Unsupported;

/// The capability sets the registry repeats, one per protocol document.
///
/// Named rather than spelled out per entry: thirty-odd models share eight distinct command
/// sets, and writing them out each time is how a row ends up disagreeing with its document.
mod archetype {
    use super::{Capabilities, DOC, FIELD, NONE, UNK};

    /// 8x1 v2 and 16x1 KVMs: everything except a documented network channel.
    pub(super) const KVM_FULL: Capabilities = Capabilities {
        baseline: DOC,
        read_port: DOC,
        buzzer: DOC,
        display_timeout: DOC,
        auto_detect: DOC,
        presets: NONE,
        net_config: UNK,
    };

    /// The 16x1 series, whose PDF omits auto input detect that community tools still use.
    pub(super) const KVM_16X1: Capabilities = Capabilities { auto_detect: FIELD, ..KVM_FULL };

    /// First generation 8x1: switch and read only.
    pub(super) const KVM_SWITCH_READ: Capabilities = Capabilities {
        baseline: DOC,
        read_port: DOC,
        buzzer: NONE,
        display_timeout: NONE,
        auto_detect: NONE,
        presets: NONE,
        net_config: UNK,
    };

    /// The 4x1 DVI KVM: four switch commands and nothing else.
    pub(super) const KVM_SWITCH_ONLY: Capabilities =
        Capabilities { read_port: NONE, ..KVM_SWITCH_READ };

    /// EB23 and P23: a four-command manual, with the rest resting on field reports.
    pub(super) const KVM_P23: Capabilities = Capabilities {
        baseline: DOC,
        read_port: UNK,
        buzzer: DOC,
        display_timeout: DOC,
        auto_detect: DOC,
        presets: NONE,
        net_config: UNK,
    };

    /// EB23, where reading the port and the network channel are both field tested.
    pub(super) const KVM_EB23: Capabilities =
        Capabilities { read_port: FIELD, net_config: FIELD, ..KVM_P23 };

    /// 4-port dual-monitor switches, documented only by the community.
    pub(super) const KVM_DUAL: Capabilities = Capabilities {
        baseline: FIELD,
        read_port: NONE,
        buzzer: FIELD,
        display_timeout: NONE,
        auto_detect: NONE,
        presets: NONE,
        net_config: NONE,
    };

    /// 4x4 matrices with an RS232 port only: routing, and nothing else at all.
    pub(super) const MATRIX_RS232: Capabilities = Capabilities {
        baseline: DOC,
        read_port: NONE,
        buzzer: NONE,
        display_timeout: NONE,
        auto_detect: NONE,
        presets: NONE,
        net_config: NONE,
    };

    /// 4x4 matrices with a LAN port: routing, a display timeout, and the network channel.
    pub(super) const MATRIX_4X4: Capabilities =
        Capabilities { display_timeout: DOC, net_config: DOC, ..MATRIX_RS232 };

    /// 8x8 and 16x16 matrices: routing, presets and a buzzer, but no display timeout.
    pub(super) const MATRIX_LARGE: Capabilities =
        Capabilities { buzzer: DOC, presets: DOC, net_config: DOC, ..MATRIX_RS232 };

    /// The SDI switch: one command.
    pub(super) const SDI: Capabilities = Capabilities { baseline: DOC, ..Capabilities::NONE };
}
