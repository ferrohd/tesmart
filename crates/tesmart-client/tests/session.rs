//! The driver loop, against an emulated switch, with a paused clock.
//!
//! Every test here runs in zero wall-clock time: `start_paused` makes `tokio::time` virtual, so
//! the 1 s command spacing and the 200 ms retry backoff are asserted rather than waited for.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tesmart_client::{
    ClientError, ConnectionPolicy, Connector, Session, TransportError, TransportKind,
};
use tesmart_proto::engine::{Event, Policy};
use tesmart_proto::protocol::KvmTable;
use tesmart_proto::value::{Buzzer, NetField, NetSetting};
use tesmart_proto::{Port, Request, Response, model};
use tesmart_sim::Faults;
use tesmart_sim::{SimHandle, Simulator};
use tokio::io::DuplexStream;
use tokio::time::Instant;

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

/// A session against an 8-port EB23, which can do everything the protocol documents.
fn eb23(faults: Faults) -> (Session<tesmart_sim::InProcess, KvmTable>, SimHandle) {
    let sim = Simulator::from_model(model::lookup("HKS801-EB23").unwrap());
    sim.handle().set_faults(faults);
    let session = Session::new(sim.connector(), KvmTable);
    let handle = sim.handle();
    (session, handle)
}

#[tokio::test(start_paused = true)]
async fn a_read_returns_the_active_port() {
    let (mut session, handle) = eb23(Faults::REALISTIC);
    handle.force_port(port(6));

    let response = session.exchange(Request::ReadActivePort).await.unwrap();

    assert_eq!(response, Response::ActivePort(port(6)));
    assert_eq!(session.stats().exchanges, 1);
    assert_eq!(session.stats().writes, 1);
    assert_eq!(session.stats().retries, 0);
}

#[tokio::test(start_paused = true)]
async fn a_write_returns_as_soon_as_the_bytes_are_out() {
    let (mut session, handle) = eb23(Faults::REALISTIC);

    let response = session.exchange(Request::Switch(port(3))).await.unwrap();

    // The protocol defines no reply to 0x01, so this is all the client can honestly report.
    assert_eq!(response, Response::Accepted);
    handle.settle(|state| state.active_port.get() == 3).await;
}

#[tokio::test(start_paused = true)]
async fn commands_are_spaced_a_second_apart() {
    let (mut session, _handle) = eb23(Faults::NONE);

    let start = Instant::now();
    session.exchange(Request::Switch(port(1))).await.unwrap();
    let after_first = Instant::now();
    session.exchange(Request::Switch(port(2))).await.unwrap();
    let after_second = Instant::now();

    assert!(
        after_first.duration_since(start) < Duration::from_millis(10),
        "the first command should not be delayed"
    );
    assert!(
        after_second.duration_since(after_first) >= Duration::from_secs(1),
        "the switch needs a second between commands, got {:?}",
        after_second.duration_since(after_first)
    );
}

#[tokio::test(start_paused = true)]
async fn the_command_gap_is_waited_out_before_connecting() {
    let (mut session, handle) = eb23(Faults::NONE);
    session.set_connection_policy(ConnectionPolicy::PerCommand);

    session.exchange(Request::Switch(port(1))).await.unwrap();
    assert_eq!(handle.connections(), 1);
    assert!(!session.is_connected(), "per-command policy closes after each command");

    session.exchange(Request::Switch(port(2))).await.unwrap();
    assert_eq!(
        handle.connections(),
        2,
        "the second command opens its own connection, after the gap rather than during it"
    );
}

#[tokio::test(start_paused = true)]
async fn a_persistent_session_reuses_one_connection() {
    let (mut session, handle) = eb23(Faults::NONE);
    session.set_connection_policy(ConnectionPolicy::Persistent);

    for human in 1..=3 {
        session.exchange(Request::Switch(port(human))).await.unwrap();
    }

    assert_eq!(handle.connections(), 1);
    assert!(session.is_connected());

    session.disconnect().await;
    assert!(!session.is_connected());
    assert_eq!(session.stats().disconnections, 1);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_reply_is_retried_within_the_budget() {
    // Every second reply goes missing, which is a documented behaviour of real units.
    let (mut session, handle) =
        eb23(Faults { drop_every: std::num::NonZeroU8::new(2), ..Faults::REALISTIC });
    handle.force_port(port(2));

    let first = session.exchange(Request::ReadActivePort).await.unwrap();
    assert_eq!(first, Response::ActivePort(port(2)));
    assert_eq!(session.stats().retries, 0, "the first reply got through");

    let second = session.exchange(Request::ReadActivePort).await.unwrap();
    assert_eq!(second, Response::ActivePort(port(2)));
    assert!(session.stats().retries >= 1, "the second needed a retransmission");
}

#[tokio::test(start_paused = true)]
async fn a_silent_switch_fails_with_no_reply() {
    let (mut session, _handle) =
        eb23(Faults { drop_every: std::num::NonZeroU8::new(1), ..Faults::NONE });

    let error = session.exchange(Request::ReadActivePort).await.unwrap_err();

    match error {
        ClientError::NoReply { kind, attempts } => {
            assert_eq!(kind, Some(tesmart_proto::RequestKind::ReadActivePort));
            assert_eq!(attempts, 6, "one attempt plus the five retries of the default policy");
        }
        other => panic!("expected NoReply, got {other}"),
    }
    assert!(session.stats().retries >= 5);
}

#[tokio::test(start_paused = true)]
async fn the_session_recovers_after_a_failure() {
    let (mut session, handle) =
        eb23(Faults { drop_every: std::num::NonZeroU8::new(1), ..Faults::NONE });

    session.exchange(Request::ReadActivePort).await.unwrap_err();

    // The engine must not be left holding the failed exchange.
    handle.set_faults(Faults::NONE);
    handle.force_port(port(5));
    let response = session.exchange(Request::ReadActivePort).await.unwrap();
    assert_eq!(response, Response::ActivePort(port(5)));
}

#[tokio::test(start_paused = true)]
async fn a_split_ascii_reply_is_reassembled() {
    // The emulator sends `IP:192.168.001.010` and `;` as separate segments, like the real
    // LAN bridge does.
    let (mut session, _handle) = eb23(Faults::REALISTIC);

    let response = session.exchange(Request::NetQuery(NetField::Ip)).await.unwrap();

    assert_eq!(
        response,
        Response::Net(NetSetting::Ip("192.168.1.10".parse().unwrap())),
        "the padding must be stripped and the segments joined"
    );
}

#[tokio::test(start_paused = true)]
async fn garbage_and_bad_terminators_do_not_break_a_read() {
    let (mut session, handle) =
        eb23(Faults { terminator: Some(0x1C), garbage_before: 3, ..Faults::REALISTIC });
    handle.force_port(port(7));

    let response = session.exchange(Request::ReadActivePort).await.unwrap();

    assert_eq!(response, Response::ActivePort(port(7)));
    assert!(session.stats().desyncs >= 1, "the junk prefix should be reported, not hidden");
}

#[tokio::test(start_paused = true)]
async fn a_front_panel_press_is_not_mistaken_for_the_answer_to_another_command() {
    let (mut session, handle) = eb23(Faults::NONE);
    session.set_connection_policy(ConnectionPolicy::Persistent);

    // Open the connection, then press a button: the switch pushes a port report.
    session.exchange(Request::Buzzer(Buzzer::Muted)).await.unwrap();
    handle.press_button(port(8));

    // A query for something else must get its own answer, not the port report.
    let response = session.exchange(Request::NetQuery(NetField::Port)).await.unwrap();
    assert_eq!(response, Response::Net(NetSetting::Port(5000)));

    // ... and the report is kept for whoever is interested.
    let reported: Vec<Event> = session.take_unsolicited().collect();
    assert_eq!(reported, [Event::PortChanged(port(8))]);
    assert_eq!(session.stats().unsolicited, 1);
}

#[tokio::test(start_paused = true)]
async fn next_event_delivers_what_the_switch_volunteers() {
    let (mut session, handle) = eb23(Faults::NONE);
    session.set_connection_policy(ConnectionPolicy::Persistent);

    handle.press_button(port(8));
    assert_eq!(session.next_event().await.unwrap(), Event::PortChanged(port(8)));

    handle.press_button(port(3));
    assert_eq!(session.next_event().await.unwrap(), Event::PortChanged(port(3)));
}

#[tokio::test(start_paused = true)]
async fn an_unsupported_command_never_reaches_the_wire() {
    let sim = Simulator::from_model(model::lookup("dual-4port").unwrap());
    let handle = sim.handle();
    let mut session = Session::new(sim.connector(), tesmart_proto::protocol::KvmDualTable);

    // 0x03 steps output A on this protocol; asking for a display timeout is a different command.
    let error = session
        .exchange(Request::DisplayTimeout(tesmart_proto::value::DisplayTimeout::Never))
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("0x03 steps output A"), "{error}");
    assert_eq!(handle.connections(), 0, "nothing should have been connected");
    assert_eq!(session.stats().writes, 0);
}

#[tokio::test(start_paused = true)]
async fn raw_exchanges_pass_bytes_through() {
    let (mut session, handle) = eb23(Faults::NONE);
    handle.force_port(port(4));

    // The read command, by hand.
    let response = session.exchange_raw(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE], true).await.unwrap();
    assert_eq!(response, Some(Response::ActivePort(port(4))));

    // And a command with no expected reply.
    let none = session.exchange_raw(&[0xAA, 0xBB, 0x03, 0x01, 0x02, 0xEE], false).await.unwrap();
    assert_eq!(none, None);
    handle.settle(|state| state.active_port.get() == 2).await;
}

#[tokio::test(start_paused = true)]
async fn the_operation_budget_is_enforced() {
    let (mut session, _handle) =
        eb23(Faults { drop_every: std::num::NonZeroU8::new(1), ..Faults::NONE });
    // Shorter than the engine's retry budget, so the budget is what gives up first.
    session.set_operation_timeout(Duration::from_millis(300));

    let error = session.exchange(Request::ReadActivePort).await.unwrap_err();

    assert!(matches!(error, ClientError::Timeout { .. }), "{error}");
    assert!(!session.is_connected(), "a failed exchange closes the connection");
}

#[tokio::test(start_paused = true)]
async fn the_command_gap_counts_against_the_operation_budget() {
    let (mut session, _handle) = eb23(Faults::NONE);
    session.exchange(Request::Switch(port(1))).await.unwrap();

    // The next command cannot start for a second, and the budget is shorter than that.
    session.set_operation_timeout(Duration::from_millis(100));
    let error = session.exchange(Request::Switch(port(2))).await.unwrap_err();

    assert!(matches!(error, ClientError::Timeout { .. }), "{error}");
}

#[tokio::test(start_paused = true)]
async fn a_closed_connection_is_reported_loudly() {
    let connector = ClosingConnector::default();
    let attempts = Arc::clone(&connector.connects);
    let mut session = Session::with_policy(connector, KvmTable, Policy::IMMEDIATE);

    let error = session.exchange(Request::ReadActivePort).await.unwrap_err();

    assert!(matches!(error, ClientError::Transport(TransportError::Closed)), "{error}");
    assert_eq!(attempts.load(Ordering::Relaxed), 1, "no hidden reconnect");
}

#[tokio::test(start_paused = true)]
async fn a_connector_that_cannot_connect_names_its_target() {
    let mut session = Session::new(RefusingConnector, KvmTable);

    let error = session.exchange(Request::ReadActivePort).await.unwrap_err();

    assert!(error.to_string().contains("tcp://192.0.2.1:5000"), "{error}");
    assert!(matches!(error, ClientError::Transport(TransportError::Connect { .. })));
}

#[tokio::test(start_paused = true)]
async fn a_transport_can_ask_for_a_persistent_session() {
    let sim = Simulator::from_model(model::lookup("HKS801-EB23").unwrap());
    sim.handle().set_faults(Faults::NONE);
    let handle = sim.handle();

    // A connector that knows its connections are expensive, as a serial line does.
    let mut session = Session::with_policy(
        ExpensiveConnector { inner: sim.connector() },
        KvmTable,
        Policy::IMMEDIATE,
    );
    assert_eq!(session.connection_policy(), ConnectionPolicy::Persistent);

    for human in 1..=3 {
        session.exchange(Request::Switch(port(human))).await.unwrap();
    }
    assert_eq!(handle.connections(), 1, "the transport's preference was honoured");

    // And the caller still has the last word: this command reuses the connection that is
    // already open, then closes it...
    session.set_connection_policy(ConnectionPolicy::PerCommand);
    session.exchange(Request::Switch(port(4))).await.unwrap();
    assert_eq!(handle.connections(), 1);
    assert!(!session.is_connected());

    // ... so the next one has to open its own.
    session.exchange(Request::Switch(port(5))).await.unwrap();
    assert_eq!(handle.connections(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_operations_builder_honours_and_overrides_the_preference() {
    use tesmart_client::Kvm as KvmOps;

    let info = model::lookup("HKS801-EB23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    let kvm = KvmOps::builder(ExpensiveConnector { inner: sim.connector() })
        .model(info)
        .policy(Policy::IMMEDIATE)
        .build()
        .unwrap();
    assert_eq!(
        kvm.session().connection_policy(),
        ConnectionPolicy::Persistent,
        "the builder should take the transport's preference"
    );

    let kvm = KvmOps::builder(ExpensiveConnector { inner: sim.connector() })
        .model(info)
        .policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::PerCommand)
        .build()
        .unwrap();
    assert_eq!(
        kvm.session().connection_policy(),
        ConnectionPolicy::PerCommand,
        "an explicit choice wins"
    );
}

/// A connector whose connections are costly, like a serial port's.
#[derive(Debug)]
struct ExpensiveConnector {
    inner: tesmart_sim::InProcess,
}

impl std::fmt::Display for ExpensiveConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("expensive://switch")
    }
}

impl Connector for ExpensiveConnector {
    type Io = DuplexStream;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        self.inner.connect()
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Serial
    }

    fn preferred_policy(&self) -> ConnectionPolicy {
        ConnectionPolicy::Persistent
    }
}

/// A switch that accepts a connection and immediately hangs up.
#[derive(Debug, Default)]
struct ClosingConnector {
    connects: Arc<AtomicU64>,
}

impl std::fmt::Display for ClosingConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tcp://closing")
    }
}

impl Connector for ClosingConnector {
    type Io = DuplexStream;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        self.connects.fetch_add(1, Ordering::Relaxed);
        let (client, switch) = tokio::io::duplex(64);
        drop(switch);
        std::future::ready(Ok(client))
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Tcp
    }
}

/// A switch that is not there.
#[derive(Debug)]
struct RefusingConnector;

impl std::fmt::Display for RefusingConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tcp://192.0.2.1:5000")
    }
}

impl Connector for RefusingConnector {
    type Io = DuplexStream;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        std::future::ready(Err(TransportError::connect(
            self,
            io::Error::from(io::ErrorKind::ConnectionRefused),
        )))
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Tcp
    }
}
