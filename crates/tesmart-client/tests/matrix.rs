//! Matrix operations against emulated matrices.
//!
//! The reason this file exists separately from `operations.rs`: a matrix can read its routing
//! back, so these tests assert *confirmation*, which no KVM test can.

use std::time::Duration;

use tesmart_client::{
    ConnectionPolicy, ErrorClass, Matrix, OpError, RouteOptions, RouteOutcome, Unverified,
    VerifyRouting, Warning,
};
use tesmart_proto::engine::Policy;
use tesmart_proto::value::{Buzzer, MatrixTimeout, NetSetting, Preset};
use tesmart_proto::{Port, RequestKind, Support, model};
use tesmart_sim::Faults;
use tesmart_sim::{SimHandle, Simulator};

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

fn preset(n: u8) -> Preset {
    Preset::new(n).unwrap()
}

fn matrix(key: &str) -> (Matrix<tesmart_sim::InProcess>, SimHandle) {
    let info = model::lookup(key).unwrap_or_else(|| panic!("unknown model {key}"));
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::REALISTIC);

    let matrix = Matrix::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .route_defaults(RouteOptions::default().settle(Duration::ZERO))
        .build()
        .expect("a matrix model");

    (matrix, sim.handle())
}

#[tokio::test(start_paused = true)]
async fn a_matrix_reports_its_routing() {
    let (mut matrix, _handle) = matrix("HMA0404B30");

    let routing = matrix.routing().await.unwrap();
    assert_eq!(routing.links.len(), 4);
    assert!(routing.is_mirrored(), "mirrored at power on: {routing}");
    assert_eq!(routing.input_for(port(3)), Some(port(3)));
}

/// The one write in the whole protocol that can be confirmed.
#[tokio::test(start_paused = true)]
async fn routing_is_confirmed_by_reading_it_back() {
    let (mut matrix, _handle) = matrix("HMA0404B30");

    let outcome = matrix.route(port(1), port(2)).await.unwrap();

    assert!(outcome.is_confirmed(), "a matrix can confirm a route: {outcome}");
    let routing = outcome.routing().expect("verified outcomes carry the routing");
    assert_eq!(routing.input_for(port(2)), Some(port(1)));
    // and the other outputs are untouched
    assert_eq!(routing.input_for(port(1)), Some(port(1)));
    assert_eq!(routing.input_for(port(3)), Some(port(3)));
}

#[tokio::test(start_paused = true)]
async fn routing_one_input_everywhere_and_back_to_mirrored() {
    let (mut matrix, _handle) = matrix("HMA0808A10");

    let outcome = matrix.route_all(port(5)).await.unwrap();
    let routing = outcome.routing().expect("confirmed");
    assert_eq!(routing.links.len(), 8);
    assert!(routing.links.iter().all(|(_, input)| *input == port(5)), "{routing}");

    let outcome = matrix.mirror().await.unwrap();
    assert!(outcome.routing().expect("confirmed").is_mirrored());
}

/// A 16x16 reports ports above 9 as `A`-`G`, in a reply too long for one segment.
#[tokio::test(start_paused = true)]
async fn a_16x16_routes_and_reports_all_sixteen_outputs() {
    let (mut matrix, _handle) = matrix("HMA1616A10");

    let outcome = matrix.route(port(16), port(10)).await.unwrap();
    let routing = outcome.routing().expect("confirmed");

    assert_eq!(routing.links.len(), 16, "every output is reported: {routing}");
    assert_eq!(routing.input_for(port(10)), Some(port(16)));
    assert_eq!(routing.input_for(port(16)), Some(port(16)));
    assert_eq!(routing.input_for(port(1)), Some(port(1)));
}

#[tokio::test(start_paused = true)]
async fn verification_can_be_switched_off() {
    let (mut matrix, _handle) = matrix("HMA0404B30");

    let options = RouteOptions::default().verify(VerifyRouting::No);
    let outcome = matrix.route_with(port(1), port(2), options).await.unwrap();

    assert_eq!(
        outcome,
        RouteOutcome::Unverified { reason: Unverified::Disabled, warnings: Vec::new() }
    );
    assert!(!outcome.is_confirmed());

    // It still happened, though.
    assert_eq!(matrix.routing().await.unwrap().input_for(port(2)), Some(port(1)));
}

#[tokio::test(start_paused = true)]
async fn a_silent_matrix_leaves_the_route_unconfirmed() {
    let info = model::lookup("HMA0404B30").unwrap();
    let sim = Simulator::from_model(info);
    // Drop every reply: the route still goes out, nothing comes back.
    sim.handle().set_faults(Faults { drop_every: std::num::NonZeroU8::new(1), ..Faults::NONE });

    let mut matrix = Matrix::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .route_defaults(RouteOptions::default().settle(Duration::ZERO))
        .build()
        .unwrap();

    let outcome = matrix.route(port(1), port(2)).await.unwrap();
    assert!(!outcome.is_confirmed());
    assert!(
        matches!(outcome, RouteOutcome::Unverified { reason: Unverified::NoReply { .. }, .. }),
        "{outcome}"
    );
}

#[tokio::test(start_paused = true)]
async fn ports_the_matrix_does_not_have_are_refused_before_sending() {
    let (mut matrix, handle) = matrix("HMA0404B30");

    let error = matrix.route(port(8), port(1)).await.unwrap_err();
    assert!(matches!(error, OpError::PortOutOfRange { .. }), "{error}");
    assert_eq!(error.class(), ErrorClass::Usage);

    let error = matrix.route(port(1), port(8)).await.unwrap_err();
    assert!(matches!(error, OpError::OutputOutOfRange { .. }), "{error}");

    // A read forces a round trip, so the matrix has demonstrably seen everything sent so far.
    matrix.routing().await.unwrap();
    assert_eq!(handle.snapshot().route_count, 0, "nothing reached the matrix");
}

#[tokio::test(start_paused = true)]
async fn presets_save_and_load_on_the_series_that_has_them() {
    let (mut matrix, _handle) = matrix("HMA0808A10");

    matrix.route(port(1), port(2)).await.unwrap();
    let saved = matrix.save_preset(preset(3)).await.unwrap();
    assert_eq!(saved.kind, RequestKind::MatrixSavePreset);

    matrix.mirror().await.unwrap();
    assert!(matrix.routing().await.unwrap().is_mirrored());

    let loaded = matrix.load_preset(preset(3)).await.unwrap();
    assert_eq!(loaded.preset, preset(3));
    let routing = loaded.routing.expect("the routing is read back after a load");
    assert_eq!(routing.input_for(port(2)), Some(port(1)));
}

/// The capability split between the matrix series, enforced before anything is sent.
#[tokio::test(start_paused = true)]
async fn each_series_refuses_what_its_document_does_not_list() {
    // A 4x4 has the display timeout but no presets and no buzzer.
    let (mut four, handle) = matrix("HMA0404B30");
    let thirty = MatrixTimeout::from_secs(30).unwrap();
    four.set_display_timeout(thirty).await.unwrap();
    handle.settle(|state| state.matrix_timeout == thirty).await;

    for error in [
        four.save_preset(preset(1)).await.unwrap_err(),
        four.load_preset(preset(1)).await.unwrap_err(),
        four.set_buzzer(Buzzer::Muted).await.unwrap_err(),
    ] {
        assert_eq!(error.class(), ErrorClass::Unsupported, "{error}");
    }

    // An 8x8 is the other way round.
    let (mut eight, handle) = matrix("HMA0808A10");
    eight.set_buzzer(Buzzer::Muted).await.unwrap();
    handle.settle(|state| state.buzzer == Buzzer::Muted).await;

    let error = eight.set_display_timeout(MatrixTimeout::NEVER).await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::Unsupported, "{error}");
}

#[tokio::test(start_paused = true)]
async fn an_rs232_only_matrix_has_no_network_commands() {
    let (mut matrix, _handle) = matrix("HMA0404A40");

    // It routes.
    assert!(matrix.route(port(2), port(1)).await.unwrap().is_confirmed());

    // It has no LAN port, so there is nothing to ask.
    let error = matrix.network().await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::Unsupported, "{error}");
    assert_eq!(
        matrix.spec().support(RequestKind::NetQuery),
        Support::Unsupported,
        "the RS232-only document has no network section at all"
    );
}

#[tokio::test(start_paused = true)]
async fn a_matrix_reads_and_writes_its_network_settings() {
    let (mut matrix, _handle) = matrix("HMA0808A10");

    let report = matrix.network().await.unwrap();
    assert_eq!(report.config.ip.map(|ip| ip.to_string()).as_deref(), Some("192.168.1.10"));
    assert_eq!(report.config.port, Some(5000));

    let outcome = matrix
        .set_network(
            &[NetSetting::Ip("10.0.2.12".parse().unwrap())],
            tesmart_client::Confirmed::yes(),
        )
        .await
        .unwrap();

    assert!(outcome.is_consistent(), "{:?}", outcome.mismatched);
    assert!(outcome.warnings.contains(&Warning::PowerCycleRequired));
    assert!(outcome.warnings.contains(&Warning::NoFactoryReset));

    // The RS232 dispute is a KVM problem: TESmart documents the matrix commands for both
    // transports, so no such warning belongs here.
    assert!(!outcome.warnings.contains(&Warning::NetConfigOverSerial), "{:?}", outcome.warnings);
}
