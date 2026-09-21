//! Sans-IO implementation of the TESmart KVM and matrix control protocols.
//!
//! This crate contains the protocol and nothing else: no sockets, no serial ports, no async
//! runtime, no allocator. It turns [`Request`]s into bytes, bytes into [`Event`]s, and owns the
//! timing quirks of real hardware (command spacing, reply retries, unsolicited port reports).
//! Driving the bytes is the job of `tesmart-client` and the transport crates.
//!
#![cfg_attr(not(test), no_std)]
// `must_use_candidate` fires on nearly every accessor in a crate made of value types.
#![allow(clippy::must_use_candidate)]

mod buf;

pub mod port;
pub mod time;
pub mod value;

pub use crate::port::{Port, PortCount};
