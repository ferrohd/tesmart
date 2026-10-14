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
use tesmart_proto::profile::Kvm;
use tesmart_proto::value::{Buzzer, NetField, NetSetting};
use tesmart_proto::{Port, Request, Response, model};
use tesmart_sim::Faults;
use tesmart_sim::{SimHandle, Simulator};
use tokio::io::DuplexStream;
use tokio::time::Instant;

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

/// A session against an 8-port EB23, which can do everything the family documents.
fn eb23(faults: Faults) -> (Session<tesmart_sim::InProcess, Kvm>, SimHandle) {
    let sim = Simulator::from_model(model::lookup("HKS801-EB23").unwrap());
    sim.handle().set_faults(faults);
    let session = Session::new(sim.connector(), Kvm);
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
