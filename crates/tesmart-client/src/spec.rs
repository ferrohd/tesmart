//! What the attached switch is: family, port count, and what it is believed to support.
//!
//! Separate from [`ModelInfo`] because a switch nobody has catalogued still has to be drivable:
//! give it a family and a port count and every capability starts out
//! [`Unverified`](Support::Unverified).

use core::fmt;

use tesmart_proto::model::ModelInfo;
use tesmart_proto::{Capabilities, Family, PortCount, Profile as _, RequestKind, Support};

/// The switch an operation is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    /// The registry key, when the switch came from the registry.
    pub name: Option<&'static str>,
    /// Which protocol family it speaks.
    pub family: Family,
    /// How many inputs it has.
    pub ports: PortCount,
    /// How many outputs it has. Only a matrix has more than two.
    pub outputs: PortCount,
    /// What is believed to work, and on what evidence.
    pub caps: Capabilities,
}

impl ModelSpec {
    /// Takes family, ports and capabilities from a registry entry.
    pub fn from_model(model: &'static ModelInfo) -> Self {
        Self {
            name: Some(model.key),
            family: model.family,
            ports: model.ports,
            outputs: model.outputs,
            caps: model.caps,
        }
    }

    /// Describes a switch that is not in the registry.
    ///
    /// Everything except switching starts out [`Support::Unverified`]: the family says which
    /// commands *exist*, and nothing says they work on this unit until someone tries.
    ///
    /// A matrix is assumed square and anything else single-output, which holds for every
    /// published model; set [`outputs`](Self::outputs) if yours differs.
    pub fn custom(family: Family, ports: PortCount) -> Self {
        let outputs = if family == Family::Matrix { ports } else { PortCount::MIN };
        Self { name: None, family, ports, outputs, caps: Capabilities::UNVERIFIED }
    }

    /// Records something you have tested on your own unit.
    #[must_use]
    pub fn with_capability(mut self, kind: RequestKind, support: Support) -> Self {
        let caps = &mut self.caps;
        match kind {
            RequestKind::ReadActivePort => caps.read_port = support,
            RequestKind::Buzzer | RequestKind::MatrixBuzzer => caps.buzzer = support,
            RequestKind::DisplayTimeout | RequestKind::MatrixDisplayTimeout => {
                caps.display_timeout = support;
            }
            RequestKind::AutoInputDetect => caps.auto_detect = support,
            RequestKind::MatrixLoadPreset | RequestKind::MatrixSavePreset => {
                caps.presets = support;
            }
            RequestKind::NetQuery | RequestKind::NetSet => caps.net_config = support,
            // Everything else is covered by the baseline, which is not individually overridable:
            // the family already decides whether the command exists at all.
            _ => caps.baseline = support,
        }
        self
    }

    /// What is known about a kind of request on this switch.
    ///
    /// A command the family does not have is [`Support::Unsupported`], whatever the capability
    /// flags claim.
    pub fn support(&self, kind: RequestKind) -> Support {
        if self.family.supports(kind) { self.caps.support(kind) } else { Support::Unsupported }
    }

    /// Whether a kind of request is worth relying on.
    pub fn supports(&self, kind: RequestKind) -> bool {
        self.support(kind).is_available()
    }

    /// Whether a kind of request is worth attempting, even unverified.
    pub fn may_attempt(&self, kind: RequestKind) -> bool {
        self.support(kind).is_worth_trying()
    }
}

impl From<&'static ModelInfo> for ModelSpec {
    fn from(model: &'static ModelInfo) -> Self {
        Self::from_model(model)
    }
}

impl fmt::Display for ModelSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name {
            Some(name) => write!(f, "{name} ({}, {} ports)", self.family, self.ports),
            None => write!(f, "unknown {} switch ({} ports)", self.family, self.ports),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tesmart_proto::model;

    #[test]
    fn registry_entries_keep_their_provenance() {
        let spec = ModelSpec::from_model(model::lookup("HKS801-P23").unwrap());
        assert_eq!(spec.support(RequestKind::ReadActivePort), Support::Unverified);
        assert!(!spec.supports(RequestKind::ReadActivePort));
        assert!(spec.may_attempt(RequestKind::ReadActivePort));
        assert_eq!(spec.to_string(), "HKS801-P23 (kvm, 8 ports)");
    }

    #[test]
    fn an_override_records_what_you_have_tested() {
        let spec = ModelSpec::from_model(model::lookup("HKS801-P23").unwrap())
            .with_capability(RequestKind::ReadActivePort, Support::FieldTested);
        assert!(spec.supports(RequestKind::ReadActivePort));
    }

    #[test]
    fn an_override_cannot_invent_a_command_the_family_lacks() {
        let spec = ModelSpec::from_model(model::lookup("dual-4port").unwrap())
            .with_capability(RequestKind::ReadActivePort, Support::Documented);
        assert_eq!(
            spec.support(RequestKind::ReadActivePort),
            Support::Unsupported,
            "the dual-monitor family has no read command to enable"
        );
    }

    #[test]
    fn unknown_switches_start_unverified() {
        let spec = ModelSpec::custom(Family::Kvm, PortCount::new(8).unwrap());
        assert_eq!(spec.support(RequestKind::Switch), Support::Unverified);
        assert_eq!(spec.to_string(), "unknown kvm switch (8 ports)");
    }
}
