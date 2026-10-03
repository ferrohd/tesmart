//! Sans-IO implementation of the TESmart KVM and matrix control protocols.
//!
//! This crate contains the protocol and nothing else: no sockets, no serial ports, no async
//! runtime, no allocator. It turns [`Request`]s into bytes, bytes into [`Event`]s, and owns the
//! timing quirks of real hardware (command spacing, reply retries, unsolicited port reports).
//! Driving the bytes is the job of `tesmart-client` and the transport crates.
//!
//! # Layers
//!
//! | Module | Role |
//! | --- | --- |
//! | [`port`], [`value`] | validated protocol values; the only place the port `±1` conversion happens |
//! | [`wire`] | hex frame codec, ASCII codec, resynchronising stream decoder |
//! | [`command`] | [`Request`] / [`Response`], family-independent |
//! | [`profile`] | per-family command tables ([`profile::Kvm`], [`profile::KvmDual`], [`profile::Matrix`], [`profile::Sdi`]) |
//! | [`model`] | model registry: ports and capabilities, with provenance |
//! | [`engine`] | the sans-IO request/response engine |
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! | --- | --- | --- |
//! | `serde` | no | `Serialize`/`Deserialize` on ports, families, capabilities and settings |
//!
//! By default the crate is a bare protocol core: `no_std`, no allocator, and no dependencies
//! beyond `thiserror`. It is built for `thumbv7em-none-eabi` in CI to keep that honest.
//!
//! # Driving the engine
//!
//! ```
//! use core::time::Duration;
//! use tesmart_proto::{Engine, Event, Port, Request, Response, profile::Kvm, time::Tick};
//!
//! let mut now = Tick::ZERO;
//! let mut engine = Engine::new(Kvm);
//!
//! // 1. submit a request; it is validated against the profile immediately
//! let id = engine.submit(Request::ReadActivePort, now).unwrap();
//!
//! // 2. hand the bytes to a transport
//! let transmit = engine.poll_transmit(now).expect("first command is not rate limited");
//! assert_eq!(&*transmit, &[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
//!
//! // 3. feed whatever the transport read back, in arbitrarily sized chunks
//! engine.handle_input(&[0xAA, 0xBB, 0x03, 0x11], now);
//! engine.handle_input(&[0x02, 0xEE], now);
//!
//! // 4. collect events
//! let Some(Event::Completed { id: done, response }) = engine.poll_event() else {
//!     panic!("expected a completed exchange")
//! };
//! assert_eq!(done, id);
//! // the reply is 0-based on the wire, the API is always the human port number
//! assert_eq!(response, Response::ActivePort(Port::new(3).unwrap()));
//!
//! // timers are the caller's: `poll_timeout` says when to call `handle_timeout`
//! now = now + Duration::from_secs(1);
//! assert!(engine.poll_timeout().is_none());
//! ```
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
pub mod engine;
pub mod model;
pub mod port;
pub mod profile;
pub mod time;
pub mod value;
pub mod wire;

pub use crate::command::{Request, RequestKind, Response, ResponseKind};
pub use crate::engine::{Engine, Event, ExchangeId, Policy};
pub use crate::model::{Capabilities, ModelInfo, Support};
pub use crate::port::{Port, PortCount};
pub use crate::profile::{Family, Profile};
