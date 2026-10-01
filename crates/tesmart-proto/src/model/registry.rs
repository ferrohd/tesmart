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
    // ---- Family A, no published protocol ------------------------------------------------------
    ModelInfo {
        key: "HKS801-M24",
        aliases: &["m24-8"],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: Capabilities::UNVERIFIED,
        notes: "No protocol document could be found for the M24 series. Family A is a guess based \
                on the product line; everything is unverified.",
    },
    ModelInfo {
        key: "HKS401-M24",
        aliases: &["m24-4"],
        family: Family::Kvm,
        ports: PortCount::known(4),
        outputs: PortCount::known(1),
        caps: Capabilities::UNVERIFIED,
        notes: "No protocol document could be found for the M24 series. Family A is a guess based \
                on the product line; everything is unverified.",
    },
    ModelInfo {
        key: "DKS203-M24",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(2),
        outputs: PortCount::known(1),
        caps: Capabilities::UNVERIFIED,
        notes: "Reported in use by community tooling (EatMoTacos/tesmart-kvm-pico-deck) but \
                absent from TESmart's protocol index. Family A and the port count are guesses \
                from the model number; everything is unverified.",
    },
    ModelInfo {
        key: "HKS801-L23",
        aliases: &["l23-8"],
        family: Family::Kvm,
        ports: PortCount::known(8),
        outputs: PortCount::known(1),
        caps: Capabilities::UNVERIFIED,
        notes: "Named in community reports; no protocol document is published for the L23 \
                series. Family A is a guess based on the product line.",
    },
    ModelInfo {
        key: "PKS0402A20",
        aliases: &[],
        family: Family::Kvm,
        ports: PortCount::known(4),
        outputs: PortCount::known(2),
        caps: Capabilities::UNVERIFIED,
        notes: "A 4-port dual-monitor model named in a community firmware backup \
                (francoisjacques). Whether it speaks family A or the dual-monitor variant is \
                untested, so the safer single-monitor family is assumed; everything is \
                unverified.",
    },
    // ---- Family A', dual monitor --------------------------------------------------------------
    ModelInfo {
        key: "dual-4port",
        aliases: &["kvm-dual-4", "4x2-dual"],
        family: Family::KvmDual,
        ports: PortCount::known(4),
        outputs: PortCount::known(2),
        caps: archetype::KVM_DUAL,
        notes: "4-port dual-monitor HDMI/USB-C switch, community documented over RS232 only \
                (spuder/Tesmart-KVM). No read command exists, so nothing can be verified.",
    },
    // ---- Family B, 4x4 matrices with RS232 only -----------------------------------------------
    // The "4x4 HDMI matrix communication protocol(RS232)" document has routing and nothing
    // else: no display timeout, no buzzer, no presets, and no network section whatsoever.
    ModelInfo {
        key: "HMA0404A30",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_RS232,
        notes: "4x4 HDMI matrix, RS232 only: routing commands and the LINK report, nothing else.",
    },
    ModelInfo {
        key: "HMA0404A40",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_RS232,
        notes: "4x4 HDMI matrix, RS232 only: routing commands and the LINK report, nothing else.",
    },
    ModelInfo {
        key: "HMA0404A50",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_RS232,
        notes: "4x4 HDMI matrix, RS232 only: routing commands and the LINK report, nothing else.",
    },
    ModelInfo {
        key: "HMA0404A70",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_RS232,
        notes: "4x4 HDMI matrix, RS232 only: routing commands and the LINK report, nothing else.",
    },
    ModelInfo {
        key: "HMA0404B40",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_RS232,
        notes: "4x4 HDMI matrix, RS232 only: routing commands and the LINK report, nothing else.",
    },
    ModelInfo {
        key: "HMA0404B50",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_RS232,
        notes: "4x4 HDMI matrix, RS232 only: routing commands and the LINK report, nothing else.",
    },
    // ---- Family B, 4x4 matrices with RS232 and LAN --------------------------------------------
    ModelInfo {
        key: "HMA0404B30",
        aliases: &["4x4-matrix"],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_4X4,
        notes: "4x4 HDMI matrix. The only matrix series with the MT00CT display timeout command.",
    },
    ModelInfo {
        key: "HMA0404A1U",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_4X4,
        notes: "4x4 HDMI matrix. The only matrix series with the MT00CT display timeout command.",
    },
    ModelInfo {
        key: "HME040410H",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_4X4,
        notes: "4x4 HDMI matrix with extenders, covered by the 4x4 (RS232+IP) document.",
    },
    ModelInfo {
        key: "HME040806R",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(4),
        outputs: PortCount::known(4),
        caps: archetype::MATRIX_4X4,
        notes: "HDMI matrix with extenders, covered by the 4x4 (RS232+IP) document, which \
                addresses inputs and outputs as 01~04 regardless of the model number.",
    },
    // ---- Family B, 8x8 matrices ---------------------------------------------------------------
    ModelInfo {
        key: "HMA0808A10",
        aliases: &["8x8-matrix"],
        family: Family::Matrix,
        ports: PortCount::known(8),
        outputs: PortCount::known(8),
        caps: archetype::MATRIX_LARGE,
        notes: "8x8 HDMI matrix: routing, eight presets and a buzzer, but no display timeout.",
    },
    ModelInfo {
        key: "HMA0808A20",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(8),
        outputs: PortCount::known(8),
        caps: archetype::MATRIX_LARGE,
        notes: "8x8 HDMI matrix: routing, eight presets and a buzzer, but no display timeout.",
    },
    ModelInfo {
        key: "HMA0808A30",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(8),
        outputs: PortCount::known(8),
        caps: archetype::MATRIX_LARGE,
        notes: "8x8 HDMI matrix: routing, eight presets and a buzzer, but no display timeout.",
    },
    ModelInfo {
        key: "HMA0808A1U",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(8),
        outputs: PortCount::known(8),
        caps: archetype::MATRIX_LARGE,
        notes: "8x8 HDMI matrix: routing, eight presets and a buzzer, but no display timeout.",
    },
    // ---- Family B, 16x16 matrices -------------------------------------------------------------
    ModelInfo {
        key: "HMA1616A10",
        aliases: &["16x16-matrix"],
        family: Family::Matrix,
        ports: PortCount::known(16),
        outputs: PortCount::known(16),
        caps: archetype::MATRIX_LARGE,
        notes: "16x16 HDMI matrix. Its LINK report writes ports above 9 as A-G, so output 16 \
                arrives as G, and a full report is 88 bytes.",
    },
    ModelInfo {
        key: "HMA1616A20",
        aliases: &[],
        family: Family::Matrix,
        ports: PortCount::known(16),
        outputs: PortCount::known(16),
        caps: archetype::MATRIX_LARGE,
        notes: "16x16 HDMI matrix. Its LINK report writes ports above 9 as A-G, so output 16 \
                arrives as G, and a full report is 88 bytes.",
    },
    // ---- Family C, the SDI switch -------------------------------------------------------------
    ModelInfo {
        key: "SSW04010A10",
        aliases: &["SSW0401A10", "4x1-sdi"],
        family: Family::Sdi,
        ports: PortCount::known(4),
        outputs: PortCount::known(1),
        caps: archetype::SDI,
        notes: "4x1 SDI switch. One command, AA CC 01 <port>, four bytes with no terminator; it \
                answers nothing. TESmart's index spells it SSW04010A10 and the document itself \
                SSW0401A10, so both resolve here.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::RequestKind;
    use crate::model::lookup;
    use crate::profile::Profile as _;

    #[test]
    fn looks_up_by_key_and_alias_ignoring_case() {
        assert_eq!(lookup("HKS801-EB23").map(|m| m.key), Some("HKS801-EB23"));
        assert_eq!(lookup("hks801-eb23").map(|m| m.key), Some("HKS801-EB23"));
        assert_eq!(lookup(" eb23-8 ").map(|m| m.key), Some("HKS801-EB23"));
        assert_eq!(lookup("dual-4port").map(|m| m.family), Some(Family::KvmDual));
        assert_eq!(lookup("nonexistent"), None);
    }

    #[test]
    fn keys_and_aliases_are_unique() {
        let mut names = std::collections::BTreeSet::new();
        for model in MODELS {
            assert!(names.insert(model.key.to_ascii_lowercase()), "duplicate {}", model.key);
            for alias in model.aliases {
                assert!(names.insert(alias.to_ascii_lowercase()), "duplicate alias {alias}");
            }
        }
    }

    #[test]
    fn capability_provenance_follows_the_protocol_reference() {
        // the 4x1 DVI PDF documents only switching
        let dvi = lookup("DKS0401A30").unwrap();
        assert_eq!(dvi.support(RequestKind::Switch), Support::Documented);
        assert_eq!(dvi.support(RequestKind::ReadActivePort), Support::Unsupported);
        assert!(!dvi.supports(RequestKind::ReadActivePort));

        // reading works on EB23 only because community tools do it
        let eb23 = lookup("HKS1601-EB23").unwrap();
        assert_eq!(eb23.support(RequestKind::ReadActivePort), Support::FieldTested);
        assert!(eb23.supports(RequestKind::ReadActivePort));

        // on P23 nobody has tried
        let p23 = lookup("HKS801-P23").unwrap();
        assert_eq!(p23.support(RequestKind::ReadActivePort), Support::Unverified);
        assert!(!p23.supports(RequestKind::ReadActivePort));
        assert!(p23.support(RequestKind::ReadActivePort).is_worth_trying());

        // the 16x1 PDF omits auto input detect
        let sixteen = lookup("HKS1601A10").unwrap();
        assert_eq!(sixteen.support(RequestKind::AutoInputDetect), Support::FieldTested);
    }

    #[test]
    fn family_support_overrides_capability_flags() {
        let dual = lookup("dual-4port").unwrap();
        // the dual-monitor family has no read command at all
        assert_eq!(dual.support(RequestKind::ReadActivePort), Support::Unsupported);
        // and its step command is community documented
        assert_eq!(dual.support(RequestKind::StepOutput), Support::FieldTested);
        assert!(dual.supports(RequestKind::StepOutput));

        // a KVM never gets matrix commands, whatever the baseline says
        let kvm = lookup("HKS0801A30").unwrap();
        assert_eq!(kvm.support(RequestKind::MatrixRoute), Support::Unsupported);

        // and a matrix never gets KVM commands
        let matrix = lookup("HMA0404B30").unwrap();
        assert_eq!(matrix.support(RequestKind::Switch), Support::Unsupported);
        assert_eq!(matrix.support(RequestKind::MatrixRoute), Support::Documented);
        assert_eq!(matrix.support(RequestKind::NetQuery), Support::Documented);
    }

    /// Each capability field and the requests that read it.
    ///
    /// One field can back more than one request, because the families spell the same idea
    /// differently: a KVM's `0x03` and a matrix's `MT00CT` are both `display_timeout`. A field
    /// is therefore legitimate for a model if *any* of its requests exists in that family.
    const FIELDS: &[(&str, &[RequestKind])] = &[
        ("read_port", &[RequestKind::ReadActivePort]),
        ("buzzer", &[RequestKind::Buzzer, RequestKind::MatrixBuzzer]),
        ("display_timeout", &[RequestKind::DisplayTimeout, RequestKind::MatrixDisplayTimeout]),
        ("auto_detect", &[RequestKind::AutoInputDetect]),
        ("presets", &[RequestKind::MatrixLoadPreset, RequestKind::MatrixSavePreset]),
        ("net_config", &[RequestKind::NetQuery, RequestKind::NetSet]),
    ];

    #[test]
    fn every_entry_is_internally_consistent() {
        for model in MODELS {
            // a capability can only be claimed for a command the family actually has
            for (field, kinds) in FIELDS {
                let claimed = kinds.iter().any(|kind| model.caps.support(*kind).is_available());
                let exists = kinds.iter().any(|kind| model.family.supports(*kind));
                assert!(
                    !claimed || exists,
                    "{} claims {field} but family {} has no such command",
                    model.key,
                    model.family
                );
            }

            assert!(model.ports.get() >= 2, "{} has too few inputs", model.key);
            assert_ne!(std::format!("{model}"), "");

            // a matrix is square in every published document; a KVM has one or two outputs
            match model.family {
                Family::Matrix => assert_eq!(
                    model.ports.get(),
                    model.outputs.get(),
                    "{} is not square; check the document",
                    model.key
                ),
                _ => {
                    assert!(model.outputs.get() <= 2, "{} claims more than two outputs", model.key);
                }
            }
        }
    }

    /// Whatever the capability flags say, the family has the last word.
    #[test]
    fn the_family_overrides_every_claim() {
        for model in MODELS {
            for (_, kinds) in FIELDS {
                for kind in *kinds {
                    if !model.family.supports(*kind) {
                        assert_eq!(
                            model.support(*kind),
                            Support::Unsupported,
                            "{} reports {kind} as available",
                            model.key
                        );
                    }
                }
            }
        }
    }

    /// The four matrix command sets differ, and the registry has to reflect that.
    #[test]
    fn matrix_capabilities_follow_their_documents() {
        // RS232-only 4x4: routing and nothing else
        let rs232 = lookup("HMA0404A40").unwrap();
        assert_eq!(rs232.support(RequestKind::MatrixRoute), Support::Documented);
        assert_eq!(rs232.support(RequestKind::MatrixDisplayTimeout), Support::Unsupported);
        assert_eq!(rs232.support(RequestKind::NetQuery), Support::Unsupported);
        assert_eq!(rs232.support(RequestKind::MatrixSavePreset), Support::Unsupported);

        // 4x4 with LAN: the only series with MT00CT, and still no presets
        let four = lookup("HMA0404B30").unwrap();
        assert_eq!(four.support(RequestKind::MatrixDisplayTimeout), Support::Documented);
        assert_eq!(four.support(RequestKind::NetQuery), Support::Documented);
        assert_eq!(four.support(RequestKind::MatrixSavePreset), Support::Unsupported);
        assert_eq!(four.support(RequestKind::MatrixBuzzer), Support::Unsupported);

        // 8x8 and 16x16: presets and a buzzer, but no MT00CT
        for key in ["HMA0808A10", "HMA1616A10"] {
            let large = lookup(key).unwrap();
            assert_eq!(large.support(RequestKind::MatrixSavePreset), Support::Documented, "{key}");
            assert_eq!(large.support(RequestKind::MatrixLoadPreset), Support::Documented, "{key}");
            assert_eq!(large.support(RequestKind::MatrixBuzzer), Support::Documented, "{key}");
            assert_eq!(
                large.support(RequestKind::MatrixDisplayTimeout),
                Support::Unsupported,
                "{key}"
            );
        }
    }

    /// The SDI switch has one command and cannot be asked anything.
    #[test]
    fn the_sdi_switch_has_exactly_one_command() {
        let sdi = lookup("SSW04010A10").unwrap();
        assert_eq!(sdi.family, Family::Sdi);
        assert_eq!(sdi.ports.get(), 4);
        assert_eq!(sdi.support(RequestKind::Switch), Support::Documented);

        for kind in [
            RequestKind::ReadActivePort,
            RequestKind::Buzzer,
            RequestKind::DisplayTimeout,
            RequestKind::AutoInputDetect,
            RequestKind::NetQuery,
            RequestKind::MatrixRoute,
        ] {
            assert_eq!(sdi.support(kind), Support::Unsupported, "{kind}");
        }

        // and the document's own spelling resolves to the same entry
        assert_eq!(lookup("SSW0401A10").map(|model| model.key), Some("SSW04010A10"));
    }
}
