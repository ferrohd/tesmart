//! An emulated TESmart switch you can point a client at.
//!
//! The protocol behaviour lives in [`tesmart_device`]; this crate adds the two things it
//! deliberately lacks: a socket and a clock. Two ways to use it:
//!
//! - **in process**, over a [`tokio::io::duplex`] pipe, through [`Simulator::connector`]. No
//!   ports, no sockets, no ordering problems in tests.
//! - **over TCP**, through [`Simulator::serve`], for testing the real transport crates or for
//!   `tesmart sim serve` on the command line.
//!
//! One emulated switch can accept many connections in sequence and keeps its state across them,
//! which is what makes the default connection-per-command policy testable.
//!
//!
//! Note the read after the write: a switch command has no reply, so
//! [`Session::exchange`](tesmart_client::Session::exchange) returns as soon as the bytes are
//! written and the emulator may not have processed them yet. That is not an artefact of the
//! emulator, it is how the protocol behaves; [`SimHandle::settle`] exists for assertions that
//! cannot be phrased as a read.

#![allow(clippy::must_use_candidate)]

use core::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

use tesmart_client::{Connector, TransportError, TransportKind};
use tesmart_device::Device;
use tesmart_proto::model::ModelInfo;
use tesmart_proto::wire::Hex;
use tesmart_proto::{Family, Port, PortCount};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::net::{TcpListener, TcpStream};

/// Configuring an emulated switch needs one dependency, not two, so the types that appear in
/// this crate's own API are re-exported.
///
/// [`tesmart_device::Device`] itself is deliberately not among them: if you want the
/// state machine with no IO around it — on a microcontroller, say — depend on `tesmart-device`
/// directly rather than reaching through a crate built on tokio.
pub use tesmart_device::{DeviceConfig, DeviceState, Faults, Routing};

/// Buffer size of the in-process pipe. Larger than any reply, small enough to be realistic.
const PIPE_CAPACITY: usize = 256;
