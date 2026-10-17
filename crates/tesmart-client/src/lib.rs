//! Async driver and operations for TESmart KVM switches.
//!
//! This is the only crate in the workspace that awaits anything. It takes the sans-IO engine
//! from [`tesmart_proto`], gives it a connection and a clock, and exposes operations that know
//! what the attached model can and cannot confirm.
//!
//! Transports are not here. A transport is one implementation of [`Connector`], which is why
//! `tesmart-tcp` and `tesmart-serial` contain no protocol or operation code at all.
//!
//! | Type | Role |
//! | --- | --- |
//! | [`Connector`] | opens byte streams; implemented by the transport crates and by `tesmart-sim` |
//! | [`Session`] | the driver loop: one exchange at a time, command spacing, retries, reconnects |
//! | [`Kvm`] | KVM operations, gated on what the model is known to support |
//! | [`Matrix`] | HDMI matrix operations, which are a different protocol entirely |
//!
//! # What an operation can promise
//!
//! The protocol acknowledges almost nothing. Of the commands a single-monitor KVM understands,
//! only reading the active port and writing network settings produce a reply, and several models
//! cannot even do the former. Operations therefore return outcomes rather than `()`:
//!
//! | Operation | Best case | Honest worst case |
//! | --- | --- | --- |
//! | [`Kvm::switch`] | [`SwitchOutcome::Verified`] | [`SwitchOutcome::Unverified`]: sent, nothing can confirm it |
//! | [`Kvm::active_port`] | the port | [`OpError::Unsupported`]: this model cannot be read |
//! | [`Kvm::set_buzzer`] and the other settings | [`Sent`] | the same [`Sent`]: no series can read these back |
//! | [`Kvm::network`] | all four values | the values that answered, with the rest in [`NetReport::missing`] |
//! | [`Kvm::set_network`] | values that read back as written | [`NetOutcome::mismatched`], plus the power-cycle warning |
//! | [`Kvm::watch`] | changes as they happen | nothing, on firmware that neither pushes nor reads |
//! | [`Matrix::route`] | [`RouteOutcome::Verified`] | [`RouteOutcome::Unverified`] |
//!
//! A switch that *answers* with the wrong port is the one case that fails loudly:
//! [`OpError::VerificationFailed`].
//!
//! # Matrices are the exception
//!
//! A matrix can report its whole routing table, so [`Matrix::route`] is the only *write* in any
//! TESmart protocol whose effect can be confirmed. Everything a KVM can set is write-only.
//!
//! # Example
//!
//! ```
//! # #[tokio::main(flavor = "current_thread")] async fn main() {
//! use tesmart_client::{Kvm, SwitchOutcome};
//! use tesmart_proto::engine::Policy;
//! use tesmart_proto::{Port, model};
//! use tesmart_sim::Simulator;
//!
//! let model = model::lookup("HKS1601-EB23").unwrap();
//! let sim = Simulator::from_model(model);           // stands in for the real switch
//!
//! let mut kvm = Kvm::builder(sim.connector())
//!     .model(model)
//!     .policy(Policy::IMMEDIATE)           // no 1 s command spacing in an example
//!     .build()
//!     .unwrap();
//!
//! let outcome = kvm.switch(Port::new(4).unwrap()).await.unwrap();
//!
//! // This model can report its port, so the switch was confirmed rather than assumed.
//! assert!(outcome.is_confirmed());
//! assert_eq!(outcome, SwitchOutcome::Verified { port: Port::new(4).unwrap(), attempts: 1 });
//! # }
//! ```
//!
//! # Timings
//!
//! The protocol timings (1 s between commands, 500 ms reply deadline, retries 200 ms apart) live
//! in [`tesmart_proto::engine::Policy`] and are enforced by the engine, not here. This crate adds
//! one timeout of its own, [`Session::set_operation_timeout`], which bounds a whole operation
//! including connecting and waiting out the command gap.
//!
//! # Logging
//!
//! Three `tracing` targets, so byte-level noise can be enabled on its own:
//!
//! | Target | Level | Content |
//! | --- | --- | --- |
//! | `tesmart::wire` | debug | every byte in and out, hex formatted like the protocol documents |
//! | `tesmart::session` | debug | connects, closes, retries, command-gap waits |
//! | `tesmart::session` | warn | desyncs, with how many bytes were lost |
//! | `tesmart::op` | debug | operation spans, with the switch and port as fields |
#![allow(clippy::must_use_candidate)]

mod error;
mod kvm;
mod matrix;
mod ops;
mod outcome;
mod session;
mod spec;
mod transport;
mod watch;

pub use crate::error::{ClientError, ErrorClass, OpError};
pub use crate::kvm::{BuildError, Kvm, KvmBuilder, SwitchOptions, Verify};
pub use crate::matrix::{Matrix, MatrixBuilder, RouteOptions, VerifyRouting};
pub use crate::outcome::{
    Confirmed, NetOutcome, NetReport, PortCountProbe, PresetOutcome, RouteOutcome, RoutingReport,
    Sent, SwitchOutcome, Unverified, Warning,
};
pub use crate::session::{Session, Stats};
pub use crate::spec::ModelSpec;
pub use crate::transport::{ConnectionPolicy, Connector, TransportError, TransportKind};
pub use crate::watch::{ChangeSource, PortChange, Watch, WatchOptions};
