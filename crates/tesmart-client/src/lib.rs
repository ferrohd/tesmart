//! Async driver and operations for TESmart KVM switches.
//!
//! This is the only crate in the workspace that awaits anything. It takes the sans-IO engine
//! from [`tesmart_proto`], gives it a connection and a clock, and exposes operations that know
//! what the attached model can and cannot confirm.
//!
//! Transports are not here. A transport is one implementation of [`Connector`], which is why
//! `tesmart-tcp` and `tesmart-serial` contain no protocol or operation code at all.
#![allow(clippy::must_use_candidate)]

mod error;
mod outcome;
mod spec;
mod transport;

pub use crate::error::{ClientError, ErrorClass, OpError};
pub use crate::outcome::{
    Confirmed, NetOutcome, NetReport, PortCountProbe, PresetOutcome, RouteOutcome, RoutingReport,
    Sent, SwitchOutcome, Unverified, Warning,
};
pub use crate::spec::ModelSpec;
pub use crate::transport::{ConnectionPolicy, Connector, TransportError, TransportKind};
