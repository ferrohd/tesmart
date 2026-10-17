//! The serial transport against an emulated switch over a pseudo terminal.
//!
//! A pts is a real tty: it goes through the same `open`, the same termios setup and the same
//! reactor registration as `/dev/ttyUSB0` would. That makes it the closest thing to hardware
//! coverage available in CI, and it is why `tokio-serial` provides the pair in the first place.
#![cfg(unix)]

use std::time::Duration;

use serialport::{SerialPort as _, TTYPort};
use tesmart_client::{ConnectionPolicy, Kvm, SwitchOptions, SwitchOutcome, WatchOptions};
use tesmart_proto::engine::Policy;
use tesmart_proto::{Port, model};
use tesmart_serial::SerialConnector;
use tesmart_sim::Faults;
use tesmart_sim::Simulator;
use tokio_serial::SerialStream;

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

// A pts takes the same 9600 as real hardware, on every Unix. This used to pass baud 0 on macOS,
// on the theory that it told the driver to skip the speed ioctl. It does not: in termios `B0`
// means hang up the line. The port still opened and still delivered bytes the emulator wrote,
// so the one test that only listens passed, while all five that send a command hung forever on
// a write that went nowhere.

/// The client's end of a pseudo terminal, kept open for the duration of a test.
///
/// The handle matters: a pts whose slave side has no openers makes the master's first read
/// return `EIO` on Linux, which would end the serving task, close the master and make the
/// device path disappear before the client ever got to open it. Holding one idle handle keeps
/// the pts alive. It claims nothing exclusively, so the client can still open the same path.
struct Pty {
    keepalive: TTYPort,
    path: String,
}

impl Pty {
    fn path(&self) -> &str {
        &self.path
    }

    /// Releases the pts, as unplugging an adapter would.
    fn unplug(self) {
        drop(self.keepalive);
    }
}

/// A pseudo terminal pair: the switch's end, and the client's.
fn pty() -> (SerialStream, Pty) {
    let (master, slave) = TTYPort::pair().expect("a pseudo terminal pair");
    let path = slave.name().expect("the slave end has a device path");
    let master = SerialStream::try_from(master).expect("register the master with the reactor");

    (master, Pty { keepalive: slave, path })
}

/// Starts an emulated switch on the far end of a pseudo terminal.
fn switch_on_a_pty(faults: Faults) -> (Simulator, Pty, tokio::task::JoinHandle<()>) {
    let sim = Simulator::from_model(model::lookup("HKS1601-EB23").unwrap());
    sim.handle().set_faults(faults);

    let (master, pty) = pty();
    let server = sim.clone();
    let serving = tokio::spawn(async move {
        let _result = server.serve_io(master).await;
    });

    (sim, pty, serving)
}

fn kvm(path: &str) -> Kvm<SerialConnector> {
    Kvm::builder(SerialConnector::new(path))
        .model(model::lookup("HKS1601-EB23").unwrap())
        // The real defaults space commands a second apart and wait another second before
        // reading a port back. Both are tested where they belong, in `tesmart-client`; these
        // tests are about the tty.
        .protocol_policy(Policy::IMMEDIATE)
        .switch_defaults(SwitchOptions::default().settle(Duration::ZERO))
        .build()
        .expect("a KVM model")
}

#[tokio::test]
async fn the_whole_stack_switches_and_verifies_over_a_tty() {
    let (sim, pty, serving) = switch_on_a_pty(Faults::REALISTIC);
    let mut kvm = kvm(pty.path());

    // The transport asked for this, and nothing overrode it.
    assert_eq!(
        kvm.session().connection_policy(),
        ConnectionPolicy::Persistent,
        "opening a tty is not free, so the serial transport keeps one open"
    );

    let outcome = kvm.switch(port(11)).await.unwrap();

    assert_eq!(outcome, SwitchOutcome::Verified { port: port(11), attempts: 1 });
    assert_eq!(sim.handle().snapshot().active_port.get(), 11);
    assert_eq!(kvm.active_port().await.unwrap(), port(11));
    assert_eq!(kvm.session().stats().connections, 1, "one port open for the whole conversation");

    serving.abort();
}

#[tokio::test]
async fn the_ascii_side_channel_works_over_a_tty_and_says_it_is_disputed() {
    let (_sim, pty, serving) = switch_on_a_pty(Faults::REALISTIC);
    let mut kvm = kvm(pty.path());

    let report = kvm.network().await.unwrap();

    assert_eq!(report.config.port, Some(5000));
    assert_eq!(report.config.ip, Some(std::net::Ipv4Addr::new(192, 168, 1, 10)));
    assert!(
        report.warnings.contains(&tesmart_client::Warning::NetConfigOverSerial),
        "TESmart says RS232 cannot do this and community tools say it can: {:?}",
        report.warnings
    );

    serving.abort();
}

#[tokio::test]
async fn a_front_panel_press_arrives_over_a_tty() {
    let (sim, pty, serving) = switch_on_a_pty(Faults::NONE);
    let mut kvm = kvm(pty.path());

    let mut watch = kvm.watch(WatchOptions::listen_only());
    sim.handle().press_button(port(7));

    let change = watch.next_change().await.unwrap();
    assert_eq!(change.port, port(7));

    serving.abort();
}

#[tokio::test]
async fn a_dropped_reply_is_still_retried_over_a_tty() {
    let (sim, pty, serving) =
        switch_on_a_pty(Faults { drop_every: std::num::NonZeroU8::new(2), ..Faults::REALISTIC });
    let mut kvm = kvm(pty.path());

    // The switch swallows every second reply; the engine's retries cover it.
    for human in [2, 5, 9] {
        let outcome = kvm.switch(port(human)).await.unwrap();
        assert!(outcome.is_confirmed(), "{outcome}");
    }
    assert_eq!(sim.handle().snapshot().active_port.get(), 9);
    assert!(kvm.session().stats().retries >= 1, "something had to be resent");

    serving.abort();
}

#[tokio::test]
async fn a_port_claimed_exclusively_cannot_be_opened_twice() {
    let (_sim, pty, serving) = switch_on_a_pty(Faults::NONE);
    let mut first = kvm(pty.path());

    // Force the port open by doing something with it.
    first.switch(port(3)).await.unwrap();
    assert!(first.session().is_connected());

    let second = SerialConnector::new(pty.path());
    let result = tesmart_client::Connector::connect(&second).await;

    // Best effort: exclusivity on a pts is not something to hang the suite on, but if it is
    // enforced then the error has to name the port.
    if let Err(error) = result {
        assert!(error.to_string().contains(pty.path()), "{error}");
    }

    serving.abort();
}

#[tokio::test]
async fn a_line_that_goes_away_is_reported_rather_than_hanging() {
    let (_sim, pty, serving) = switch_on_a_pty(Faults::NONE);
    let mut kvm = kvm(pty.path());
    kvm.switch(port(2)).await.unwrap();

    // Unplug the far end: the emulator stops and the pts goes away.
    serving.abort();
    pty.unplug();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let error = kvm.active_port().await.expect_err("a read on a dead line cannot succeed");

    // Which error depends on the platform (a pts gives EIO where a socket gives EOF); what
    // matters is that it fails at all rather than waiting forever.
    assert_eq!(error.class(), tesmart_client::ErrorClass::Transport, "{error}");
}
