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

/// Every model this crate knows about.
///
/// Transcribed from `docs/protocol-reference.md`, which in turn cites the TESmart protocol PDFs
/// and the community tools that tested the undocumented parts. The model-to-document mapping
/// comes from TESmart's own index, "LAN RS232 Control Protocols", updated 2024-01-10.
pub static MODELS: &[ModelInfo] = &[
    // ---- Family A, single monitor ------------------------------------------------------------
    ModelInfo {
        key: "DKS0401A30",
        aliases: &["4x1-dvi"],
        family: Family::Kvm,
        ports: PortCount::known(4),
        outputs: PortCount::known(1),
        caps: archetype::KVM_SWITCH_ONLY,
        notes: "The 4x1 DVI protocol PDF lists only the four switch commands: no read, no settings.",
    },
    ModelInfo {
        key: "HKS0801A20",
        aliases: &["8x1-v1"],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_SWITCH_READ,
        notes: "First generation 8x1: switch and read active port only.",
    },
    ModelInfo {
        key: "HKS0801A30",
        aliases: &["8x1-v2"],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_FULL,
        notes: "",
    },
    ModelInfo {
        key: "HKS0801A40",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_FULL,
        notes: "",
    },
    ModelInfo {
        key: "HKS0801A1U",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_FULL,
        notes: "",
    },
    ModelInfo {
        key: "HSW0801A10",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_FULL,
        notes: "HDMI switch without KVM, same protocol as the 8x1 v2 KVMs.",
    },
    ModelInfo {
        key: "HSW0801A1U",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_FULL,
        notes: "HDMI switch without KVM, same protocol as the 8x1 v2 KVMs.",
    },
    ModelInfo {
        key: "HKE0802A10",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_FULL,
        notes: "",
    },
    ModelInfo {
        key: "HKS1601A10",
        aliases: &["16x1"],
        family: Family::Kvm,
        ports: PortCount::known(16),
        outputs: PortCount::known(1),
        caps: archetype::KVM_16X1,
        notes: "Auto input detect is absent from the 16x1 PDF but sent by working community tools.",
    },
    ModelInfo {
        key: "HKS1601A1U",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(16),
        outputs: PortCount::known(1),
        caps: archetype::KVM_16X1,
        notes: "Auto input detect is absent from the 16x1 PDF but sent by working community tools.",
    },
    ModelInfo {
        key: "HSW1601A10",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(16),
        outputs: PortCount::known(1),
        caps: archetype::KVM_16X1,
        notes: "HDMI switch without KVM, same protocol as the 16x1 KVMs.",
    },
    ModelInfo {
        key: "HSW1601A1U",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(16),
        outputs: PortCount::known(1),
        caps: archetype::KVM_16X1,
        notes: "HDMI switch without KVM, same protocol as the 16x1 KVMs.",
    },
    // ---- Family A, EB23 and P23 series --------------------------------------------------------
    ModelInfo {
        key: "HKS801-EB23",
        aliases: &["eb23-8"],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_EB23,
        notes: "4K30 with USB 2.0. The manual lists only 0x01, 0x02, 0x03 and 0x81, but reading \
                the active port and the ASCII network commands both work in practice \
                (luborpetr/tesmart, the Home Assistant integration, pschmitt/tesmart.sh).",
    },
    ModelInfo {
        key: "HKS1601-EB23",
        aliases: &["eb23-16"],
        family: Family::Kvm,
        ports: PortCount::known(16),
        outputs: PortCount::known(1),
        caps: archetype::KVM_EB23,
        notes: "As HKS801-EB23, with 16 inputs.",
    },
    ModelInfo {
        key: "HKS801-P23",
        aliases: &["p23-8"],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: archetype::KVM_P23,
        notes: "4K60 with USB 3.2 Gen1. The manual lists only 0x01, 0x02, 0x03 and 0x81; no \
                field report exists for reading the active port or for the network commands on \
                this series, so both are unverified.",
    },
    ModelInfo {
        key: "HKS1601-P23",
        aliases: &["p23-16"],
        family: Family::Kvm,
        ports: PortCount::known(16),
        outputs: PortCount::known(1),
        caps: archetype::KVM_P23,
        notes: "As HKS801-P23, with 16 inputs.",
    },
];
