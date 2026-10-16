//! Turning the global flags into a switch or matrix handle.

use tesmart::client::{ConnectionPolicy, Connector, Kvm, Matrix};
use tesmart::proto::engine::Policy;
use tesmart::proto::model::{self, ModelInfo};
use tesmart::proto::{Family, PortCount, RequestKind, Support};

use crate::cli::Global;
use crate::error::CliError;

/// Builds a switch handle from the global options.
///
/// # Errors
///
/// [`CliError::Usage`] if the model is unknown, the capability overrides do not parse, or
/// neither a model nor a family and port count were given.
pub fn build<C: Connector>(
    connector: C,
    global: &Global,
    family_default: Option<Family>,
) -> Result<Kvm<C>, CliError> {
    let model = resolve_model(global)?;

    let mut builder = Kvm::builder(connector)
        .protocol_policy(protocol_policy(global))
        .operation_timeout(global.timeout);

    if let Some(model) = model {
        builder = builder.model(model);
    }
    if let Some(family) = global.family {
        builder = builder.family(family.into());
    } else if model.is_none()
        && let Some(family) = family_default
    {
        // `raw` is the one command that does not need the tables: the bytes are the caller's,
        // and the family only decides how a reply is interpreted.
        builder = builder.family(family);
    }
    if let Some(ports) = global.ports {
        let ports = PortCount::new(ports)
            .map_err(|error| CliError::usage(format!("--ports {ports}: {error}")))?;
        builder = builder.ports(ports);
    } else if model.is_none() && family_default.is_some() {
        builder = builder.ports(
            PortCount::new(tesmart::proto::Port::MAX)
                .unwrap_or_else(|_| unreachable!("16 is a valid port count")),
        );
    }
    if global.persistent {
        builder = builder.connection_policy(ConnectionPolicy::Persistent);
    }

    for assumption in &global.assume {
        let Assumption { kind, support } = assumption.parse()?;
        builder = builder.assume(kind, support);
    }

    builder.build().map_err(build_error)
}

/// Builds a matrix handle from the global options.
///
/// # Errors
///
/// [`CliError::Usage`] if the model is unknown or is not a matrix, if the capability overrides
/// do not parse, or if no input count could be determined.
pub fn build_matrix<C: Connector>(connector: C, global: &Global) -> Result<Matrix<C>, CliError> {
    let model = resolve_model(global)?;

    let mut builder = Matrix::builder(connector)
        .protocol_policy(protocol_policy(global))
        .operation_timeout(global.timeout);

    if let Some(model) = model {
        builder = builder.model(model);
    }
    if let Some(ports) = global.ports {
        let ports = PortCount::new(ports)
            .map_err(|error| CliError::usage(format!("--ports {ports}: {error}")))?;
        builder = builder.inputs(ports);
    }
    if global.persistent {
        builder = builder.connection_policy(ConnectionPolicy::Persistent);
    }

    for assumption in &global.assume {
        let Assumption { kind, support } = assumption.parse()?;
        builder = builder.assume(kind, support);
    }

    builder.build().map_err(|error| match error {
        tesmart::client::BuildError::NoPortCount => CliError::usage(NO_MATRIX),
        other => build_error(other),
    })
}

fn build_error(error: tesmart::client::BuildError) -> CliError {
    match error {
        tesmart::client::BuildError::NoFamily | tesmart::client::BuildError::NoPortCount => {
            CliError::usage(NO_MODEL)
        }
        other => CliError::from(other),
    }
}

const NO_MODEL: &str = "\
no model given

  pass -m <MODEL> to use the built-in registry (see `tesmart models list`)
  or  -f kvm --ports 8 for a switch that is not in it

The same opcode means different things in different families: 0x03 sets the front display
timeout on a single-monitor switch and steps output A's input on a dual-monitor one. That is
why this is never guessed.";

const NO_MATRIX: &str = "\
no matrix given

  pass -m <MODEL> to use the built-in registry (see `tesmart models list`)
  or  --ports 8 for a matrix that is not in it

Matrices are assumed square, as every published TESmart model is.";

/// Whether the switch described by the global options is an HDMI matrix.
///
/// Used to send the commands both families share, such as `net`, to the right handle.
///
/// # Errors
///
/// [`CliError::Usage`] if `--model` names something not in the registry.
pub fn is_matrix(global: &Global) -> Result<bool, CliError> {
    if let Some(family) = global.family {
        return Ok(Family::from(family) == Family::Matrix);
    }
    Ok(resolve_model(global)?.is_some_and(|model| model.family == Family::Matrix))
}

/// Looks up `--model`, if one was given.
fn resolve_model(global: &Global) -> Result<Option<&'static ModelInfo>, CliError> {
    let Some(name) = global.model.as_deref() else {
        return Ok(None);
    };

    model::lookup(name).map(Some).ok_or_else(|| {
        CliError::usage(format!(
            "unknown model {name:?}; see `tesmart models list`, or use -f and --ports"
        ))
    })
}

/// The protocol timings, from the tuning flags.
fn protocol_policy(global: &Global) -> Policy {
    Policy::DEFAULT
        .with_min_command_interval(global.rate_limit)
        .with_max_retries(global.retries)
        .with_ascii_dialect(global.ascii_dialect.into())
}

/// Transport settings that come from the global flags.
pub fn target_options(global: &Global) -> crate::target::TargetOptions {
    crate::target::TargetOptions { connect_timeout: global.connect_timeout, baud: global.baud }
}

/// Something the user has tested on their own unit, overriding the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assumption {
    /// Which command.
    pub kind: RequestKind,
    /// What is now believed about it.
    pub support: Support,
}

impl core::str::FromStr for Assumption {
    type Err = CliError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (kind, support) = text.split_once('=').ok_or_else(|| {
            CliError::usage(format!(
                "--assume {text:?}: expected KIND=SUPPORT, such as read-port=yes"
            ))
        })?;

        Ok(Self { kind: parse_kind(kind.trim())?, support: parse_support(support.trim())? })
    }
}

/// The capabilities the registry tracks, under the names a person would type.
///
/// `buzzer` and `display-timeout` cover both spellings of the same idea: the KVM opcode and the
/// matrix string share one capability flag, because no model has both.
fn parse_kind(kind: &str) -> Result<RequestKind, CliError> {
    let normalised = kind.to_ascii_lowercase();
    Ok(match normalised.as_str() {
        "read-port" | "read-active-port" | "read" => RequestKind::ReadActivePort,
        "buzzer" => RequestKind::Buzzer,
        "display-timeout" | "timeout" => RequestKind::DisplayTimeout,
        "auto-detect" | "autodetect" | "auto-input-detect" => RequestKind::AutoInputDetect,
        "net" | "net-config" | "network" => RequestKind::NetQuery,
        "presets" | "preset" => RequestKind::MatrixSavePreset,
        "routing" | "read-routing" => RequestKind::MatrixReadRouting,
        _ => {
            return Err(CliError::usage(format!(
                "--assume {kind:?}: expected one of read-port, buzzer, display-timeout, \
                 auto-detect, net, presets, routing"
            )));
        }
    })
}

/// The four states a capability can be in, under the names a person would type.
fn parse_support(support: &str) -> Result<Support, CliError> {
    let normalised = support.to_ascii_lowercase();
    Ok(match normalised.as_str() {
        // "yes" means "I tested it", which is exactly what field-tested records.
        "yes" | "field" | "field-tested" | "works" => Support::FieldTested,
        "documented" | "doc" => Support::Documented,
        "unverified" | "unknown" | "maybe" => Support::Unverified,
        "no" | "unsupported" | "none" => Support::Unsupported,
        _ => {
            return Err(CliError::usage(format!(
                "--assume: {support:?} is not a support level; expected yes, no, documented, \
                 field-tested or unverified"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assume(text: &str) -> Assumption {
        text.parse().unwrap_or_else(|error| panic!("{text:?}: {error}"))
    }

    #[test]
    fn assumptions_use_the_words_people_type() {
        assert_eq!(
            assume("read-port=yes"),
            Assumption { kind: RequestKind::ReadActivePort, support: Support::FieldTested }
        );
        assert_eq!(
            assume("net=no"),
            Assumption { kind: RequestKind::NetQuery, support: Support::Unsupported }
        );
        assert_eq!(
            assume("display-timeout = documented"),
            Assumption { kind: RequestKind::DisplayTimeout, support: Support::Documented }
        );
        assert_eq!(assume("READ=Unverified").support, Support::Unverified);
    }

    #[test]
    fn a_bad_assumption_says_what_is_accepted() {
        let error = "read-port".parse::<Assumption>().unwrap_err();
        assert!(error.to_string().contains("KIND=SUPPORT"), "{error}");

        let error = "frobnicate=yes".parse::<Assumption>().unwrap_err();
        assert!(error.to_string().contains("read-port"), "{error}");

        let error = "read-port=probably".parse::<Assumption>().unwrap_err();
        assert!(error.to_string().contains("support level"), "{error}");
    }
}
