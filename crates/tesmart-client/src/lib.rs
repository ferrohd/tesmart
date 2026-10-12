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
mod ops;
mod outcome;
mod session;
mod spec;
mod transport;

pub use crate::error::{ClientError, ErrorClass, OpError};
pub use crate::outcome::{
    Confirmed, NetOutcome, NetReport, PortCountProbe, PresetOutcome, RouteOutcome, RoutingReport,
    Sent, SwitchOutcome, Unverified, Warning,
};
pub use crate::session::{Session, Stats};
pub use crate::spec::ModelSpec;
pub use crate::transport::{ConnectionPolicy, Connector, TransportError, TransportKind};
