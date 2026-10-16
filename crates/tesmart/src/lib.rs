//! Control TESmart KVM switches and HDMI matrices over TCP or RS232.
//!
//! This crate is the front door. It re-exports the pieces of the workspace behind one
//! dependency and one version, so that choosing a transport is a Cargo feature rather than a
//! second crate to find, name and keep in step.
//!
//! ```
//! use tesmart::{Kvm, Port, model, tcp::TcpConnector};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let info = model::lookup("HKS1601-EB23").expect("a model in the registry");
//! let mut kvm = Kvm::from_model("192.168.1.10".parse::<TcpConnector>()?, info)?;
//!
//! assert_eq!(kvm.ports().get(), 16);
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
//! With no transport features at all this crate is still useful: [`client`] and [`proto`] are
//! there, and you can supply your own [`Connector`].
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
//! Each transport parses its own address: `TcpConnector` and `SerialConnector` both implement
//! `FromStr`, for `192.168.1.10:5000` and `/dev/ttyUSB0@9600` respectively. Deciding which of
//! them a string like `/dev/ttyUSB0` was meant for is a job for whatever talks to the user, and
//! lives in the `tesmart` command line tool rather than here.
//!
//! Driving a switch whose model is not in the registry, or one you have tested yourself, means
//! going through [`client::Kvm::builder`] rather than [`client::Kvm::from_model`]; see
//! [`client`] for that.
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

pub use tesmart_client as client;
pub use tesmart_proto as proto;

#[cfg(feature = "serial")]
pub use tesmart_serial as serial;
#[cfg(feature = "sim")]
pub use tesmart_sim as sim;
#[cfg(feature = "tcp")]
pub use tesmart_tcp as tcp;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_re_exports_cover_the_common_path() {
        // If any of these stop resolving, the facade is not actually usable on its own.
        let port = Port::new(4).unwrap();
        assert_eq!(port.get(), 4);
        assert_eq!(Family::Kvm.name(), "kvm");
        assert!(model::lookup("HKS1601-EB23").is_some());
        assert_eq!(Buzzer::Muted.to_string(), "muted");
        assert_eq!(Support::Documented.name(), "documented");
    }
}
