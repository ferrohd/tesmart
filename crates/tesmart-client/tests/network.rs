//! The network side-channel: the only operation that can lock you out of the switch.
//!
//! Nothing here is in TESmart's protocol documents; it comes from packet captures of the
//! official Windows controller and from two community tools. The tests therefore check the
//! warnings as carefully as the values.

use std::net::Ipv4Addr;
use std::num::NonZeroU8;

use tesmart_client::{
    Confirmed, ConnectionPolicy, Connector, ErrorClass, Kvm, TransportError, TransportKind, Warning,
};
use tesmart_proto::engine::Policy;
use tesmart_proto::value::{NetField, NetSetting};
use tesmart_proto::{
    model,
    wire::{AsciiDialect, AsciiScope},
};
use tesmart_sim::Faults;
use tesmart_sim::{SimHandle, Simulator};
use tokio::io::DuplexStream;

fn eb23(faults: Faults) -> (Kvm<tesmart_sim::InProcess>, SimHandle) {
    let info = model::lookup("HKS801-EB23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(faults);

    let kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .build()
        .unwrap();

    (kvm, sim.handle())
}

#[tokio::test(start_paused = true)]
async fn reading_the_network_settings_normalises_the_padding() {
    // The emulator answers the way hardware does: zero padded, and with the `;` in its own
    // segment for address fields.
    let (mut kvm, _handle) = eb23(Faults::REALISTIC);

    let report = kvm.network().await.unwrap();

    assert!(report.is_complete(), "{report:?}");
    assert_eq!(report.config.ip, Some(Ipv4Addr::new(192, 168, 1, 10)));
    assert_eq!(report.config.port, Some(5000));
    assert_eq!(report.config.gateway, Some(Ipv4Addr::new(192, 168, 1, 1)));
    assert_eq!(report.config.mask, Some(Ipv4Addr::new(255, 255, 255, 0)));
    assert!(report.warnings.is_empty(), "nothing to warn about over TCP");
}

#[tokio::test(start_paused = true)]
async fn a_field_that_stays_silent_is_reported_not_fatal() {
    // Every second reply is lost, so some fields answer and some do not.
    let (mut kvm, _handle) = eb23(Faults { drop_every: NonZeroU8::new(2), ..Faults::NONE });
    kvm.session_mut().set_protocol_policy(Policy::IMMEDIATE.with_max_retries(0));

    let report = kvm.network().await.unwrap();

    assert!(!report.missing.is_empty(), "some fields should have gone missing");
    assert!(!report.is_complete());
    // The ones that did answer are still usable.
    assert_eq!(report.missing.len() + count_known(&report.config), 4);
}

fn count_known(config: &tesmart_proto::value::NetConfig) -> usize {
    NetField::ALL.into_iter().filter(|field| config.get(*field).is_some()).count()
}

#[tokio::test(start_paused = true)]
async fn writing_settings_reads_them_back_and_warns_about_the_power_cycle() {
    let (mut kvm, handle) = eb23(Faults::REALISTIC);

    let changes = [
        NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)),
        NetSetting::Gateway(Ipv4Addr::new(10, 0, 2, 1)),
        NetSetting::Port(5001),
    ];

    let outcome = kvm.set_network(&changes, Confirmed::yes()).await.unwrap();

    assert!(outcome.is_consistent(), "{outcome:?}");
    assert_eq!(outcome.applied.ip, Some(Ipv4Addr::new(10, 0, 2, 12)));
    assert_eq!(outcome.applied.gateway, Some(Ipv4Addr::new(10, 0, 2, 1)));
    assert_eq!(outcome.applied.port, Some(5001));
    assert_eq!(outcome.applied.mask, Some(Ipv4Addr::new(255, 255, 255, 0)), "untouched");

    // Both facts the user needs are in the outcome, not buried in a log line.
    assert!(outcome.warnings.contains(&Warning::PowerCycleRequired));
    assert!(outcome.warnings.contains(&Warning::NoFactoryReset));

    // And the switch really stored them.
    assert_eq!(handle.snapshot().net.ip, Some(Ipv4Addr::new(10, 0, 2, 12)));
}

#[tokio::test(start_paused = true)]
async fn writing_over_rs232_carries_the_disputed_warning() {
    let info = model::lookup("HKS801-EB23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    // Same emulated switch, reached over a connector that claims to be a serial line.
    let mut kvm = Kvm::builder(PretendSerial { inner: sim.connector() })
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .build()
        .unwrap();

    let outcome = kvm.set_network(&[NetSetting::Port(5002)], Confirmed::yes()).await.unwrap();

    assert!(
        outcome.warnings.contains(&Warning::NetConfigOverSerial),
        "TESmart says RS232 cannot do this and community tools say it can: say so"
    );

    let report = kvm.network().await.unwrap();
    assert!(report.warnings.contains(&Warning::NetConfigOverSerial));
}

/// A matrix over RS232 must send the wrapped `MT80xx` commands, and the caller must not have to
/// ask for that: the session takes the scope from the transport.
#[tokio::test(start_paused = true)]
async fn a_matrix_over_rs232_uses_the_wrapped_commands() {
    let info = model::lookup("HMA0808A10").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    let mut matrix = tesmart_client::Matrix::builder(PretendSerial { inner: sim.connector() })
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .build()
        .unwrap();

    // The policy was rewritten from the transport, without the builder being told.
    assert_eq!(matrix.session().protocol_policy().ascii_scope, AsciiScope::Serial);

    // And the emulated matrix answers the wrapped form, so the round trip really works.
    let report = matrix.network().await.unwrap();
    assert_eq!(report.config.ip, Some(Ipv4Addr::new(192, 168, 1, 10)));
    assert_eq!(report.config.port, Some(5000));

    let outcome = matrix.set_network(&[NetSetting::Port(5002)], Confirmed::yes()).await.unwrap();
    assert!(outcome.is_consistent(), "{:?}", outcome.mismatched);

    // TESmart documents the matrix network commands for RS232 explicitly, in this very
    // spelling, so the KVM dispute does not apply.
    assert!(!outcome.warnings.contains(&Warning::NetConfigOverSerial), "{:?}", outcome.warnings);
}

/// Over LAN the same matrix uses the bare strings.
#[tokio::test(start_paused = true)]
async fn a_matrix_over_lan_uses_the_bare_commands() {
    let info = model::lookup("HMA0808A10").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    let matrix = tesmart_client::Matrix::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    assert_eq!(matrix.session().protocol_policy().ascii_scope, AsciiScope::Lan);
}

#[tokio::test(start_paused = true)]
async fn the_tesmart_sh_dialect_is_understood_too() {
    let info = model::lookup("HKS801-EB23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    // `IP?;` and `IP: 10.0.2.12;` rather than `IP?` and `IP:10.0.2.12;`.
    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE.with_ascii_dialect(AsciiDialect::TesmartSh))
        .connection_policy(ConnectionPolicy::Persistent)
        .build()
        .unwrap();

    let outcome = kvm
        .set_network(&[NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12))], Confirmed::yes())
        .await
        .unwrap();

    assert!(outcome.is_consistent(), "{outcome:?}");
}

#[tokio::test(start_paused = true)]
async fn a_value_that_does_not_read_back_is_reported_with_the_rest() {
    // The switch answers `OK` and stores something else. The read-back is the only thing
    // standing between that and a power cycle into an unreachable address.
    let (mut kvm, _handle) = eb23(Faults { garble_net_writes: true, ..Faults::NONE });

    let changes = [NetSetting::Ip(Ipv4Addr::new(10, 0, 2, 12)), NetSetting::Port(5001)];
    let outcome = kvm.set_network(&changes, Confirmed::yes()).await.unwrap();

    assert!(!outcome.is_consistent(), "the mismatch must not be silent");
    assert_eq!(outcome.mismatched, [NetField::Ip, NetField::Port]);

    // Every value is still reported, so the user can see how far off it is rather than just
    // being told something failed.
    assert_eq!(outcome.applied.ip, Some(Ipv4Addr::new(10, 0, 2, 13)));
    assert_eq!(outcome.applied.port, Some(5002));
    assert_eq!(outcome.requested, changes);
    assert!(outcome.warnings.contains(&Warning::PowerCycleRequired));
}

#[tokio::test(start_paused = true)]
async fn a_model_without_the_side_channel_refuses_before_sending() {
    // Nothing is known about the M24 series, so the side-channel is unverified: attempted.
    let info = model::lookup("HKS801-M24").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);
    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    let report = kvm.network().await.unwrap();
    assert!(
        report.warnings.iter().any(|warning| matches!(
            warning,
            Warning::CapabilityUnverified { kind: tesmart_proto::RequestKind::NetQuery, .. }
        )),
        "{:?}",
        report.warnings
    );

    // The dual-monitor family has no side-channel at all, so there it is refused outright.
    let dual = model::lookup("dual-4port").unwrap();
    let sim = Simulator::from_model(dual);
    let mut kvm = Kvm::builder(sim.connector())
        .model(dual)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    let error = kvm.network().await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::Unsupported);
    assert_eq!(kvm.session().stats().writes, 0);
}

/// The emulator, behind a connector that reports itself as a serial line.
#[derive(Debug)]
struct PretendSerial {
    inner: tesmart_sim::InProcess,
}

impl std::fmt::Display for PretendSerial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("serial:///dev/ttyUSB0@9600")
    }
}

impl Connector for PretendSerial {
    type Io = DuplexStream;

    fn connect(&self) -> impl Future<Output = Result<Self::Io, TransportError>> + Send {
        self.inner.connect()
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Serial
    }
}

use std::future::Future;
