//! Sans-IO implementation of the TESmart KVM and matrix control protocols.
//!
//! This crate contains the protocol and nothing else: no sockets, no serial ports, no async
//! runtime, no allocator. It turns [`Request`]s into bytes, bytes into [`Event`]s, and owns the
//! timing quirks of real hardware (command spacing, reply retries, unsolicited port reports).
//! Driving the bytes is the job of `tesmart-client` and the transport crates.
//!
//! # Protocol references
//!
//! Every opcode, quirk and capability flag in this crate cites its source in a doc comment:
//! either an official TESmart document or a community tool that was observed to work. See
//! `docs/protocol-reference.md` in the repository for the transcribed protocol and the full
//! source list.
#![cfg_attr(not(test), no_std)]
// `must_use_candidate` fires on nearly every accessor in a crate made of value types.
#![allow(clippy::must_use_candidate)]

mod buf;

pub mod command;
pub mod port;
pub mod time;
pub mod value;
pub mod wire;

pub use crate::command::{Request, RequestKind, Response, ResponseKind};
pub use crate::port::{Port, PortCount};
