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
//! ```
//! # #[tokio::main(flavor = "current_thread")] async fn main() {
//! use tesmart_client::Session;
//! use tesmart_proto::engine::Policy;
//! use tesmart_proto::{Port, Request, Response, model, profile::Kvm};
//! use tesmart_sim::Simulator;
//!
//! let sim = Simulator::from_model(model::lookup("HKS1601-EB23").unwrap());
//! let handle = sim.handle();
//!
//! // `IMMEDIATE` drops the 1 s command spacing. Against hardware, use `Policy::DEFAULT`.
//! let mut session = Session::with_policy(sim.connector(), Kvm, Policy::IMMEDIATE);
//!
//! session.exchange(Request::Switch(Port::new(4).unwrap())).await.unwrap();
//! let active = session.exchange(Request::ReadActivePort).await.unwrap();
//! assert_eq!(active, Response::ActivePort(Port::new(4).unwrap()));
//!
//! // someone walks up and presses a button
//! handle.press_button(Port::new(2).unwrap());
//! assert_eq!(handle.snapshot().active_port.get(), 2);
//! # }
//! ```
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

/// An emulated switch.
///
/// Cheap to clone: every clone drives the same switch.
#[derive(Debug, Clone)]
pub struct Simulator {
    inner: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    device: Mutex<Device>,
    connections: AtomicU64,
    label: String,
    /// Wakes the connection task when something queues output behind its back, so a front
    /// panel press is pushed out instead of waiting for the client to speak first.
    wake: Notify,
}

impl Simulator {
    /// Emulates a registry entry, including its capability gaps.
    pub fn from_model(model: &ModelInfo) -> Self {
        Self::with_config(DeviceConfig::from_model(model), model.key)
    }

    /// Emulates an arbitrary switch.
    pub fn with_config(config: DeviceConfig, label: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Shared {
                device: Mutex::new(Device::new(config)),
                connections: AtomicU64::new(0),
                label: label.into(),
                wake: Notify::new(),
            }),
        }
    }

    /// Emulates a switch described only by family and port count, as an unknown model would be.
    ///
    /// A matrix is assumed square, since every published TESmart matrix is. Use
    /// [`with_config`](Self::with_config) for anything else.
    pub fn custom(family: Family, ports: PortCount, caps: tesmart_proto::Capabilities) -> Self {
        let outputs = match family {
            Family::Matrix => ports,
            Family::Kvm | Family::KvmDual | Family::Sdi => PortCount::MIN,
        };
        let config = DeviceConfig {
            family,
            ports,
            outputs,
            caps,
            dialect: tesmart_proto::wire::AsciiDialect::Captured,
            faults: Faults::REALISTIC,
        };
        Self::with_config(config, "custom")
    }

    /// A [`Connector`] that opens in-process pipes to this switch.
    pub fn connector(&self) -> InProcess {
        InProcess { simulator: self.clone() }
    }

    /// A handle for poking the switch from outside the protocol.
    pub fn handle(&self) -> SimHandle {
        SimHandle { inner: Arc::clone(&self.inner) }
    }

    /// Accepts TCP connections until the listener fails.
    ///
    /// Connections are served one at a time, like the real switch, which has a single serial
    /// bridge behind its LAN port.
    ///
    /// # Errors
    ///
    /// Propagates the first `accept` failure.
    pub async fn serve(&self, listener: TcpListener) -> io::Result<()> {
        loop {
            let (stream, peer) = listener.accept().await?;
            let simulator = self.clone();
            tokio::spawn(async move {
                tracing::debug!(target: "tesmart::sim", %peer, "connection accepted");
                if let Err(error) = simulator.serve_stream(stream).await {
                    tracing::debug!(target: "tesmart::sim", %peer, %error, "connection ended");
                }
            });
        }
    }

    /// Binds a TCP listener and reports the address it got.
    ///
    /// Pass port 0 to let the operating system choose, which is what tests should do.
    ///
    /// # Errors
    ///
    /// Propagates the bind failure.
    pub async fn bind(addr: SocketAddr) -> io::Result<(TcpListener, SocketAddr)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        Ok((listener, local))
    }

    /// Serves one TCP connection to completion.
    ///
    /// # Errors
    ///
    /// Propagates read and write failures; a clean close by the peer is not an error.
    pub async fn serve_stream(&self, stream: TcpStream) -> io::Result<()> {
        self.inner.connections.fetch_add(1, Ordering::Relaxed);
        self.serve_io(stream).await
    }

    /// Drives the emulated switch over any byte stream until it closes.
    ///
    /// The emulator does not care what it is plugged into: a TCP socket, an in-process pipe, or
    /// one end of a pseudo terminal, which is how the serial transport is tested without
    /// hardware.
    ///
    /// The device lock is a `std::sync::Mutex` and is never held across an `await`: the compiler
    /// enforces that, because a guard held across one would make this future non-`Send` and
    /// [`tokio::spawn`] would refuse it.
    ///
    /// # Errors
    ///
    /// Propagates read and write failures; a clean close by the peer is not an error.
    /// Does not touch [`SimHandle::connections`]: counting happens where a connection is
    /// established, which is in the connector or in [`serve_stream`](Self::serve_stream), and
    /// has to be observable before the serving task has had a chance to run.
    pub async fn serve_io<S>(&self, mut io: S) -> io::Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let mut rx = [0u8; PIPE_CAPACITY];

        loop {
            // Anything the switch has to say goes out first, so a reply is never held back
            // behind the next read.
            loop {
                let segment = self.device().poll_transmit();
                let Some(segment) = segment else { break };
                tracing::debug!(
                    target: "tesmart::sim",
                    dir = "tx", bytes = ?segment,
                    "switch writes"
                );
                io.write_all(&segment).await?;
                io.flush().await?;
            }

            // Either the client says something, or something queued output on our side. Without
            // the second branch, a front panel press would sit in the outbox until the client
            // happened to send a command.
            let count = tokio::select! {
                read = io.read(&mut rx) => read?,
                () = self.inner.wake.notified() => continue,
            };
            if count == 0 {
                return Ok(());
            }
            tracing::debug!(
                target: "tesmart::sim",
                dir = "rx", bytes = %Hex(&rx[..count]),
                "switch reads"
            );

            self.device().handle_input(&rx[..count]);
        }
    }

    fn device(&self) -> MutexGuard<'_, Device> {
        self.inner.device.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Opens in-process connections to a [`Simulator`].
#[derive(Debug, Clone)]
pub struct InProcess {
    simulator: Simulator,
}

impl std::fmt::Display for InProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sim://{}", self.simulator.inner.label)
    }
}

impl Connector for InProcess {
    type Io = DuplexStream;

    /// Connecting to an in-process switch cannot block, so this future is ready immediately.
    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        let (client, switch) = tokio::io::duplex(PIPE_CAPACITY);
        let simulator = self.simulator.clone();
        // Counted here rather than in the serving task, so a test can assert on it immediately
        // after the connect future resolves.
        simulator.inner.connections.fetch_add(1, Ordering::Relaxed);

        tokio::spawn(async move {
            if let Err(error) = simulator.serve_io(switch).await {
                tracing::debug!(target: "tesmart::sim", %error, "pipe ended");
            }
        });

        core::future::ready(Ok(client))
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Simulated
    }
}

/// Pokes an emulated switch from outside the protocol: the equivalent of walking up to it.
#[derive(Debug, Clone)]
pub struct SimHandle {
    inner: Arc<Shared>,
}

impl SimHandle {
    /// Presses a front panel button: changes the port **and** pushes an unsolicited report.
    pub fn press_button(&self, port: Port) {
        self.with_device(|device| device.press_button(port));
    }

    /// Changes the port without telling anyone, like firmware that pushes no reports.
    pub fn force_port(&self, port: Port) {
        self.with_device(|device| device.force_port(port));
    }

    /// Replaces the fault configuration.
    pub fn set_faults(&self, faults: Faults) {
        self.with_device(|device| device.set_faults(faults));
    }

    /// The switch's current settings.
    pub fn snapshot(&self) -> DeviceState {
        self.with_device(|device| *device.state())
    }

    /// How many connections have been opened since the simulator was created.
    ///
    /// Counts connections made through a connector and TCP connections accepted by
    /// [`Simulator::serve`]; a stream handed straight to
    /// [`serve_io`](Simulator::serve_io) is not counted, because nothing opened it. Makes
    /// [`ConnectionPolicy`](tesmart_client::ConnectionPolicy) directly assertable.
    pub fn connections(&self) -> u64 {
        self.inner.connections.load(Ordering::Relaxed)
    }

    /// Waits until the emulated switch's state satisfies `predicate`.
    ///
    /// Needed because commands that have no reply return as soon as the bytes are written: the
    /// task driving the switch may not have run yet. Yields rather than sleeping, so it costs no
    /// time even under a paused clock.
    ///
    /// # Panics
    ///
    /// Panics if the predicate is still false after 10,000 yields, which means it is never going
    /// to be true.
    pub async fn settle(&self, predicate: impl Fn(&DeviceState) -> bool) {
        for _ in 0..10_000 {
            if predicate(&self.snapshot()) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the emulated switch never reached the expected state: {:?}", self.snapshot());
    }

    /// Runs `f` against the emulated switch.
    ///
    /// Synchronous on purpose: a test should be able to press a button without awaiting. The
    /// lock is only ever held for the length of one state update.
    fn with_device<T>(&self, f: impl FnOnce(&mut Device) -> T) -> T {
        let result = {
            let mut device = self.inner.device.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut device)
        };
        // `notify_one` stores a permit, so a press that happens before the connection task
        // starts waiting is still delivered.
        self.inner.wake.notify_one();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tesmart_proto::model;

    #[tokio::test]
    async fn serves_an_in_process_connection() {
        let sim = Simulator::from_model(model::lookup("HKS801-EB23").unwrap());
        let handle = sim.handle();
        let connector = sim.connector();

        let mut io = connector.connect().await.unwrap();
        io.write_all(&[0xAA, 0xBB, 0x03, 0x01, 0x03, 0xEE]).await.unwrap();

        // the read side of the reply: ask for the active port
        io.write_all(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]).await.unwrap();
        let mut reply = [0u8; 6];
        io.read_exact(&mut reply).await.unwrap();

        assert_eq!(reply, [0xAA, 0xBB, 0x03, 0x11, 0x02, 0xEE]);
        assert_eq!(handle.snapshot().active_port.get(), 3);
        assert_eq!(handle.connections(), 1);
    }
}
