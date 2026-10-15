//! The TCP transport against an emulated switch on a real socket.
//!
//! This is the whole stack: `tesmart-tcp` opening sockets, `tesmart-client` driving the
//! conversation, `tesmart-proto` encoding it, and `tesmart-sim` answering as the hardware would.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tesmart_client::{
    ClientError, ConnectionPolicy, Kvm, OpError, SwitchOptions, SwitchOutcome, TransportError,
};
use tesmart_proto::engine::Policy;
use tesmart_sim::Faults;

use tesmart_proto::{Port, model};
use tesmart_sim::Simulator;
use tesmart_tcp::TcpConnector;
use tokio::task::JoinHandle;

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

/// Starts an emulated 16 port switch on an ephemeral port.
async fn switch_on_localhost(faults: Faults) -> (Simulator, SocketAddr, JoinHandle<()>) {
    let sim = Simulator::from_model(model::lookup("HKS1601-EB23").unwrap());
    sim.handle().set_faults(faults);

    let (listener, addr) = Simulator::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("an ephemeral port");

    let server = sim.clone();
    let serving = tokio::spawn(async move {
        // Ends when the test drops the handle.
        let _result = server.serve(listener).await;
    });

    (sim, addr, serving)
}

fn kvm(addr: SocketAddr) -> Kvm<TcpConnector> {
    Kvm::builder(TcpConnector::from_addr(addr))
        .model(model::lookup("HKS1601-EB23").unwrap())
        // The real defaults space commands a second apart and wait another second before
        // reading a port back. Both are tested where they belong, in `tesmart-client`; these
        // tests are about the socket.
        .protocol_policy(Policy::IMMEDIATE)
        .switch_defaults(SwitchOptions::default().settle(Duration::ZERO))
        .build()
        .expect("a KVM model")
}

#[tokio::test]
async fn the_whole_stack_switches_and_verifies_over_tcp() {
    let (sim, addr, serving) = switch_on_localhost(Faults::REALISTIC).await;
    let mut kvm = kvm(addr);

    let outcome = kvm.switch(port(11)).await.unwrap();

    assert_eq!(outcome, SwitchOutcome::Verified { port: port(11), attempts: 1 });
    assert_eq!(kvm.active_port().await.unwrap(), port(11));
    assert_eq!(sim.handle().snapshot().active_port.get(), 11);

    serving.abort();
}

#[tokio::test]
async fn a_connection_per_command_keeps_the_switch_state() {
    let (sim, addr, serving) = switch_on_localhost(Faults::NONE).await;
    let mut kvm = kvm(addr);

    assert_eq!(
        kvm.session().connection_policy(),
        ConnectionPolicy::PerCommand,
        "TCP connections are cheap, so the transport asks for one per command"
    );

    for human in [3, 7, 16] {
        kvm.switch(port(human)).await.unwrap();
    }

    // Three switches, each with a read-back, each on its own connection.
    let opened = kvm.session().stats().connections;
    assert!(opened >= 6, "expected a connection per command, got {opened}");
    assert_eq!(opened, sim.handle().connections(), "both ends agree");
    assert!(!kvm.session().is_connected(), "and nothing is left open");

    // The switch kept its port across all of that, as hardware does.
    assert_eq!(sim.handle().snapshot().active_port.get(), 16);

    serving.abort();
}

#[tokio::test]
async fn the_ascii_side_channel_works_over_tcp() {
    let (_sim, addr, serving) = switch_on_localhost(Faults::REALISTIC).await;
    let mut kvm = kvm(addr);

    let report = kvm.network().await.unwrap();

    // The emulator answers the way hardware does: zero padded, with the `;` in its own segment.
    assert_eq!(report.config.ip, Some(Ipv4Addr::new(192, 168, 1, 10)));
    assert_eq!(report.config.port, Some(5000));
    assert!(report.missing.is_empty(), "{report:?}");
    assert!(
        report.warnings.is_empty(),
        "over TCP the network commands are not disputed: {:?}",
        report.warnings
    );

    serving.abort();
}

#[tokio::test]
async fn a_front_panel_press_is_seen_over_a_persistent_connection() {
    let (sim, addr, serving) = switch_on_localhost(Faults::NONE).await;
    let mut kvm = Kvm::builder(TcpConnector::from_addr(addr))
        .model(model::lookup("HKS1601-EB23").unwrap())
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .build()
        .unwrap();

    let mut watch = kvm.watch(tesmart_client::WatchOptions::listen_only());
    sim.handle().press_button(port(9));

    let change = watch.next_change().await.unwrap();
    assert_eq!(change.port, port(9));
    watch.close().await;

    serving.abort();
}

#[tokio::test]
async fn nothing_listening_is_reported_with_the_target() {
    // Port 1 on localhost: nothing is listening, and the error has to say where it looked.
    let connector = TcpConnector::new(Ipv4Addr::LOCALHOST, 1);
    let mut kvm =
        Kvm::builder(connector).model(model::lookup("HKS801-EB23").unwrap()).build().unwrap();

    let error = kvm.active_port().await.unwrap_err();

    assert!(error.to_string().contains("tcp://127.0.0.1:1"), "{error}");
    assert!(
        matches!(error, OpError::Client(ClientError::Transport(TransportError::Connect { .. }))),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_switch_that_is_not_there_fails_within_the_connect_timeout() {
    // 192.0.2.1 is TEST-NET-1 (RFC 5737) and must never be assigned, so this either hangs
    // until the timeout (a network that drops the packets) or fails at once (a machine with no
    // route to it). Both are correct; what must hold either way is that it gives up quickly and
    // says where it was looking. The exact mapping to `ConnectTimeout` is asserted in the unit
    // tests, where no network is involved.
    let connector = TcpConnector::new(Ipv4Addr::new(192, 0, 2, 1), 5000)
        .with_connect_timeout(Duration::from_millis(250));
    let mut kvm = Kvm::builder(connector)
        .model(model::lookup("HKS801-EB23").unwrap())
        .operation_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let started = std::time::Instant::now();
    let error = kvm.active_port().await.unwrap_err();
    let elapsed = started.elapsed();

    assert!(error.to_string().contains("tcp://192.0.2.1:5000"), "{error}");
    assert!(matches!(error, OpError::Client(ClientError::Transport(_))), "{error:?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "the connect timeout should have ended this in 250ms, took {elapsed:?}"
    );
}

#[tokio::test]
async fn a_name_that_resolves_to_nothing_fails_with_the_name() {
    // `.invalid` is reserved by RFC 2606 and must never resolve.
    let connector: TcpConnector = "switch.invalid:5000".parse().unwrap();
    let mut kvm = Kvm::builder(connector)
        .model(model::lookup("HKS801-EB23").unwrap())
        .operation_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let error = kvm.active_port().await.unwrap_err();

    assert!(error.to_string().contains("switch.invalid"), "{error}");
}

#[tokio::test]
async fn localhost_is_resolved_at_connect_time() {
    let (sim, addr, serving) = switch_on_localhost(Faults::NONE).await;

    // A name, not an address: this goes through the resolver on every connection.
    let connector =
        TcpConnector::new("localhost".parse::<tesmart_tcp::Host>().unwrap(), addr.port());
    assert_eq!(connector.socket_addr(), None, "nothing is resolved before connecting");

    let mut kvm = Kvm::builder(connector)
        .model(model::lookup("HKS1601-EB23").unwrap())
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    kvm.switch(port(5)).await.unwrap();
    assert_eq!(sim.handle().snapshot().active_port.get(), 5);

    serving.abort();
}
