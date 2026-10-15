//! Following the active port, by both mechanisms.

use std::time::Duration;

use tesmart_client::{ChangeSource, ConnectionPolicy, Kvm, WatchOptions};
use tesmart_proto::engine::Policy;
use tesmart_proto::{Port, RequestKind, Support, model};
use tesmart_sim::Faults;
use tesmart_sim::{SimHandle, Simulator};

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

fn eb23() -> (Kvm<tesmart_sim::InProcess>, SimHandle) {
    let info = model::lookup("HKS1601-EB23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    let kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    (kvm, sim.handle())
}

#[tokio::test(start_paused = true)]
async fn a_front_panel_press_is_reported() {
    let (mut kvm, handle) = eb23();
    let mut watch = kvm.watch(WatchOptions::listen_only());

    handle.press_button(port(9));
    let change = watch.next_change().await.unwrap();

    assert_eq!(change.port, port(9));
    assert_eq!(change.previous, None);
    assert_eq!(change.source, ChangeSource::Reported);
    assert_eq!(change.to_string(), "port 9 (reported)");
    assert_eq!(watch.last_known(), Some(port(9)));

    handle.press_button(port(2));
    let change = watch.next_change().await.unwrap();
    assert_eq!(change.previous, Some(port(9)));
    assert_eq!(change.port, port(2));
    assert_eq!(change.to_string(), "port 9 -> 2 (reported)");
}

#[tokio::test(start_paused = true)]
async fn a_change_nobody_announces_is_found_by_polling() {
    // `force_port` changes the port without pushing a report, like firmware that volunteers
    // nothing.
    let (mut kvm, handle) = eb23();
    let mut watch = kvm.watch(WatchOptions::default().poll_interval(Duration::from_secs(10)));

    handle.force_port(port(4));
    let change = watch.next_change().await.unwrap();

    assert_eq!(change.port, port(4));
    assert_eq!(change.source, ChangeSource::Polled);
}

#[tokio::test(start_paused = true)]
async fn polling_is_quiet_when_nothing_changes() {
    let (mut kvm, handle) = eb23();
    let mut watch = kvm.watch(WatchOptions::default().poll_interval(Duration::from_millis(50)));
    watch.prime().await.unwrap();

    // Several poll intervals pass with the port untouched: no change may be reported.
    let quiet = tokio::time::timeout(Duration::from_millis(500), watch.next_change()).await;
    assert!(quiet.is_err(), "a poll that finds the same port is not a change");

    // And a real change is still noticed afterwards.
    handle.force_port(port(7));
    let change = watch.next_change().await.unwrap();
    assert_eq!(change.port, port(7));
    assert_eq!(change.previous, Some(port(1)));
}

#[tokio::test(start_paused = true)]
async fn a_press_and_a_poll_do_not_report_the_same_change_twice() {
    let (mut kvm, handle) = eb23();
    let mut watch = kvm.watch(WatchOptions::default().poll_interval(Duration::from_millis(50)));

    // The press is pushed *and* visible to the next poll.
    handle.press_button(port(5));
    let change = watch.next_change().await.unwrap();
    assert_eq!(change.source, ChangeSource::Reported);

    let repeat = tokio::time::timeout(Duration::from_millis(300), watch.next_change()).await;
    assert!(repeat.is_err(), "the poll must not re-report what was already reported");
}

#[tokio::test(start_paused = true)]
async fn watching_holds_one_connection_and_restores_the_policy() {
    let (mut kvm, handle) = eb23();
    kvm.session_mut().set_connection_policy(ConnectionPolicy::PerCommand);

    {
        let mut watch = kvm.watch(WatchOptions::listen_only());
        handle.press_button(port(3));
        watch.next_change().await.unwrap();
        handle.press_button(port(4));
        watch.next_change().await.unwrap();
        assert_eq!(handle.connections(), 1, "one connection for the whole watch");
        watch.close().await;
    }

    assert_eq!(
        kvm.session().connection_policy(),
        ConnectionPolicy::PerCommand,
        "the policy the caller set must come back"
    );
    assert!(!kvm.session().is_connected(), "closing the watch closes the connection");
}

#[tokio::test(start_paused = true)]
async fn polling_is_disabled_on_a_switch_that_cannot_be_read() {
    let info = model::lookup("DKS0401A30").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);
    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    let watch = kvm.watch(WatchOptions::default());

    assert_eq!(
        watch.options().poll_interval,
        None,
        "polling a switch with no read command would just time out forever"
    );
}

#[tokio::test(start_paused = true)]
async fn polling_is_attempted_on_an_unverified_model() {
    // Reading is unproven on a P23, but attempting it is the only way anyone will find out.
    let info = model::lookup("HKS801-P23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);
    let handle = sim.handle();
    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    assert_eq!(kvm.spec().support(RequestKind::ReadActivePort), Support::Unverified);

    let mut watch = kvm.watch(WatchOptions::default().poll_interval(Duration::from_millis(50)));
    handle.force_port(port(6));

    let change = watch.next_change().await.unwrap();
    assert_eq!(change.source, ChangeSource::Polled);
    assert_eq!(change.port, port(6));
}
