//! Control TESmart KVM switches and HDMI matrices over TCP or RS232.
//!
//! This crate is the front door. It re-exports the pieces of the workspace behind one
//! dependency and one version, and adds the part that only makes sense once the transports are
//! in scope: turning a target string such as `192.168.1.10` or `/dev/ttyUSB0` into something
//! you can drive.
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use tesmart::Port;
//!
//! let mut kvm = tesmart::kvm("192.168.1.10", "HKS1601-EB23")?;
//! # assert_eq!(kvm.ports().get(), 16);
//! // kvm.switch(Port::new(4)?).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Nothing connects until you call an operation, so building a handle cannot fail for network
//! reasons.
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! | --- | --- | --- |
//! | `tcp` | yes | the switch's LAN port, via [`tesmart_tcp`] |
//! | `serial` | no | the switch's RS232 port, via [`tesmart_serial`] |
//! | `sim` | no | an emulated switch, via [`tesmart_sim`] |
//! | `serde` | no | `Serialize`/`Deserialize` on the protocol value types |
//!
//! A target naming a transport you did not enable is an error rather than a fallback. With no
//! transport features at all this crate is still useful: [`client`] and [`proto`] are there,
//! and you can supply your own [`Connector`].
//!
//! You may also see `_transport` in the feature list. It is internal, is set by each of the
//! three transport features, and exists only so that [`resolve`] and [`AnyConnector`] compile
//! away when there is no transport for them to choose between. Do not enable it directly.
//!
//! # Layers
//!
//! The re-exports keep their own names, so you can reach past this crate when you need to.
//!
//! | Module | Crate | Role |
//! | --- | --- | --- |
//! | [`proto`] | [`tesmart_proto`] | the protocol, controller side: sans-IO, `no_std`, no allocator |
//! | [`client`] | [`tesmart_client`] | the async driver and the operations |
//! | [`tcp`] | [`tesmart_tcp`] | one [`Connector`] over TCP |
//! | [`serial`] | [`tesmart_serial`] | one [`Connector`] over RS232 |
//! | [`sim`] | [`tesmart_sim`] | an emulated switch behind a [`Connector`] |
//! | — | `tesmart-device` | the switch side of the protocol, `no_std`; `sim` serves one |
//!
//! Driving a switch whose model is not in the registry, or one you have tested yourself, means
//! going through [`client::Kvm::builder`] rather than [`kvm`]; see [`client`] for that.
//!
//! # What an operation can promise
//!
//! Very little, and the types say so. This protocol acknowledges almost nothing: of the
//! commands a KVM understands, only reading the active port and writing network settings
//! produce a reply, and several models cannot even do the former. So
//! [`switch`](client::Kvm::switch) returns a [`SwitchOutcome`] rather than `()`, and
//! "the switch is on port 3" stays distinguishable from "port 3 was requested and nobody can
//! say what happened".
//!
//! Matrices are the exception: they report their whole routing table, so
//! [`Matrix::route`](client::Matrix::route) is the one write in any TESmart family whose effect
//! can actually be confirmed.
#![allow(clippy::must_use_candidate)]

// Target resolution needs at least one transport to choose between, so it follows the internal
// `_transport` feature that every transport enables.
#[cfg(feature = "_transport")]
mod target;

pub use tesmart_client as client;
pub use tesmart_proto as proto;

#[cfg(feature = "serial")]
pub use tesmart_serial as serial;
#[cfg(feature = "sim")]
pub use tesmart_sim as sim;
#[cfg(feature = "tcp")]
pub use tesmart_tcp as tcp;

#[cfg(feature = "sim")]
pub use crate::target::DEFAULT_SIM_MODEL;
#[cfg(feature = "_transport")]
pub use crate::target::{
    AnyConnector, AnyIo, DEFAULT_BAUD, DEFAULT_CONNECT_TIMEOUT, TargetError, TargetOptions,
    resolve, resolve_with,
};

// The handful of types a caller needs in order to do anything at all.
pub use tesmart_client::{
    ConnectionPolicy, Connector, Kvm, Matrix, NetOutcome, NetReport, OpError, RouteOutcome,
    RoutingReport, Sent, SwitchOutcome, TransportKind, Warning,
};
pub use tesmart_proto::value::{Buzzer, DisplayTimeout, MatrixTimeout, NetSetting, Preset, Toggle};
pub use tesmart_proto::{Family, Port, PortCount, Request, Response, Support, model};

/// Everything needed to drive a switch, for `use tesmart::prelude::*`.
pub mod prelude {
    pub use crate::client::{Connector as _, Kvm, Matrix, OpError};
    pub use crate::proto::{Family, Port, Support};
    pub use crate::{RouteOutcome, SwitchOutcome};

    #[cfg(feature = "_transport")]
    pub use crate::AnyConnector;
}

/// A handle could not be built from a target string and a model name.
#[cfg(feature = "_transport")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectError {
    /// The target string named no transport this build can reach.
    #[error(transparent)]
    Target(#[from] TargetError),

    /// The model is not in the built-in registry.
    ///
    /// Use [`client::Kvm::builder`] with an explicit family and port count for a switch the
    /// registry does not know.
    #[error("unknown model {0:?}; see tesmart::model::MODELS for the ones that are known")]
    UnknownModel(String),

    /// The model is in the registry, but speaks the wrong protocol family for this handle.
    #[error(transparent)]
    Build(#[from] client::BuildError),
}

/// Builds a KVM handle from a target string and a registry model name.
///
/// The one-liner for the common case. Nothing is opened: the first operation connects.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let kvm = tesmart::kvm("tcp://192.168.1.10:5000", "HKS801-EB23")?;
/// assert_eq!(kvm.ports().get(), 8);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`ConnectError`] if the target names no usable transport, the model is not in the registry,
/// or the model is a matrix — in which case use [`matrix`] instead.
#[cfg(feature = "_transport")]
pub fn kvm(target: &str, model: &str) -> Result<Kvm<AnyConnector>, ConnectError> {
    let info = lookup(model)?;
    Ok(Kvm::from_model(resolve(target)?, info)?)
}

/// Builds an HDMI matrix handle from a target string and a registry model name.
///
/// # Errors
///
/// As [`kvm`], except that the model must be a matrix.
#[cfg(feature = "_transport")]
pub fn matrix(target: &str, model: &str) -> Result<Matrix<AnyConnector>, ConnectError> {
    let info = lookup(model)?;
    Ok(Matrix::from_model(resolve(target)?, info)?)
}

#[cfg(feature = "_transport")]
fn lookup(model: &str) -> Result<&'static proto::ModelInfo, ConnectError> {
    model::lookup(model).ok_or_else(|| ConnectError::UnknownModel(model.to_owned()))
}
