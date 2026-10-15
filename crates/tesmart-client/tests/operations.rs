//! Operations against emulated switches, one test per honest answer.
//!
//! The point of this file is the capability matrix: the same `switch` call has to produce a
//! confirmed result on an EB23, an unconfirmable one on a 4x1 DVI, and a loud failure on a
//! switch that silently ignores commands.

use std::num::NonZeroU8;
use std::time::Duration;

use tesmart_client::{
    BuildError, ConnectionPolicy, ErrorClass, Kvm, OpError, SwitchOptions, SwitchOutcome,
    Unverified, Verify, Warning,
};
use tesmart_proto::engine::Policy;
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, Output, Step, SwitchInterval, Toggle,
};
use tesmart_proto::{Family, Port, PortCount, RequestKind, Support, model};
use tesmart_sim::Faults;
use tesmart_sim::{SimHandle, Simulator};

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

/// A switch handle plus a handle on the emulated hardware behind it.
///
/// `Policy::IMMEDIATE` drops the 1 s command spacing; the spacing itself is tested in
/// `tests/session.rs`, and paying it here would only make these tests slower to read.
fn switch(key: &str, faults: Faults) -> (Kvm<tesmart_sim::InProcess>, SimHandle) {
    let info = model::lookup(key).unwrap_or_else(|| panic!("unknown model {key}"));
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(faults);

    let kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .switch_defaults(SwitchOptions::default().settle(Duration::ZERO))
        .build()
        .expect("a KVM model");

    (kvm, sim.handle())
}

#[tokio::test(start_paused = true)]
async fn switching_is_confirmed_on_a_switch_that_can_be_read() {
    let (mut kvm, handle) = switch("HKS1601-EB23", Faults::REALISTIC);

    let outcome = kvm.switch(port(12)).await.unwrap();

    assert_eq!(outcome, SwitchOutcome::Verified { port: port(12), attempts: 1 });
    assert!(outcome.is_confirmed());
    assert_eq!(handle.snapshot().active_port.get(), 12);
    assert_eq!(kvm.active_port().await.unwrap(), port(12));
}

#[tokio::test(start_paused = true)]
async fn switching_is_unconfirmable_on_a_switch_that_cannot_be_read() {
    // The 4x1 DVI protocol document lists four switch commands and nothing else.
    let (mut kvm, handle) = switch("DKS0401A30", Faults::NONE);

    let outcome = kvm.switch(port(3)).await.unwrap();

    match outcome {
        SwitchOutcome::Unverified { port: requested, reason, .. } => {
            assert_eq!(requested, port(3));
            assert_eq!(reason, Unverified::ModelCannotRead { support: Support::Unsupported });
        }
        other => panic!("expected an unverified outcome, got {other}"),
    }
    // The command still happened; only the confirmation is missing.
    handle.settle(|state| state.active_port.get() == 3).await;
}

#[tokio::test(start_paused = true)]
async fn reading_the_port_is_refused_when_the_model_cannot() {
    let (mut kvm, _handle) = switch("DKS0401A30", Faults::NONE);

    let error = kvm.active_port().await.unwrap_err();

    assert_eq!(error.class(), ErrorClass::Unsupported);
    assert!(error.to_string().contains("read-active-port is unsupported on DKS0401A30"), "{error}");
    assert_eq!(kvm.session().stats().writes, 0, "nothing should have been sent");
}

#[tokio::test(start_paused = true)]
async fn an_unverified_model_is_not_trusted_but_is_still_tried() {
    // Nobody has reported reading the active port on a P23.
    let (mut kvm, _handle) = switch("HKS801-P23", Faults::NONE);

    // By default the read-back is skipped, because the capability is unproven.
    let outcome = kvm.switch(port(5)).await.unwrap();
    assert!(matches!(
        outcome,
        SwitchOutcome::Unverified {
            reason: Unverified::ModelCannotRead { support: Support::Unverified },
            ..
        }
    ));

    // Asking explicitly tries anyway, and on this emulator it works.
    let outcome = kvm
        .switch_with(
            port(6),
            SwitchOptions::default().verify(Verify::Always).settle(Duration::ZERO),
        )
        .await
        .unwrap();
    assert_eq!(outcome, SwitchOutcome::Verified { port: port(6), attempts: 1 });

    // And the result carries the caveat, rather than hiding it in a log line.
    let sent = kvm.set_buzzer(Buzzer::Muted).await.unwrap();
    assert!(sent.warnings.is_empty(), "the buzzer is documented on this model");
}

#[tokio::test(start_paused = true)]
async fn an_override_makes_an_unverified_model_verifiable() {
    let info = model::lookup("HKS801-P23").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);

    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        // "I tested this on my unit."
        .assume(RequestKind::ReadActivePort, Support::FieldTested)
        .protocol_policy(Policy::IMMEDIATE)
        .switch_defaults(SwitchOptions::default().settle(Duration::ZERO))
        .build()
        .unwrap();

    let outcome = kvm.switch(port(2)).await.unwrap();
    assert_eq!(outcome, SwitchOutcome::Verified { port: port(2), attempts: 1 });
}

#[tokio::test(start_paused = true)]
async fn a_switch_that_ignores_the_command_fails_loudly() {
    let (mut kvm, handle) = switch("HKS801-EB23", Faults { ignore_switch: true, ..Faults::NONE });

    let error = kvm.switch(port(4)).await.unwrap_err();

    assert_eq!(error.class(), ErrorClass::Verification);
    match error {
        OpError::VerificationFailed { expected, found } => {
            assert_eq!(expected, port(4));
            assert_eq!(found, port(1), "the switch never moved");
        }
        other => panic!("expected a verification failure, got {other}"),
    }
    assert_eq!(handle.snapshot().active_port, port(1));
}

#[tokio::test(start_paused = true)]
async fn a_silent_read_back_is_unverified_not_failed() {
    // The switch accepts the command but every reply is lost. The command probably worked;
    // saying it failed would be a lie.
    let (mut kvm, handle) =
        switch("HKS801-EB23", Faults { drop_every: NonZeroU8::new(1), ..Faults::NONE });

    let outcome = kvm.switch(port(7)).await.unwrap();

    assert!(
        matches!(outcome, SwitchOutcome::Unverified { reason: Unverified::NoReply { .. }, .. }),
        "{outcome}"
    );
    assert!(!outcome.is_confirmed());
    handle.settle(|state| state.active_port.get() == 7).await;
}

#[tokio::test(start_paused = true)]
async fn a_dropped_read_back_is_retried_before_giving_up() {
    // Every second reply is lost. With the engine's own retries disabled, recovering is down to
    // the operation's read-back loop.
    let (mut kvm, _handle) =
        switch("HKS801-EB23", Faults { drop_every: NonZeroU8::new(2), ..Faults::NONE });
    kvm.session_mut().set_protocol_policy(Policy::IMMEDIATE.with_max_retries(0));

    // Burn the first reply so that the read-back inside `switch` is the one that goes missing.
    kvm.active_port().await.unwrap();

    let outcome = kvm
        .switch_with(port(5), SwitchOptions::default().settle(Duration::ZERO).attempts(3))
        .await
        .unwrap();

    match outcome {
        SwitchOutcome::Verified { port: confirmed, attempts } => {
            assert_eq!(confirmed, port(5));
            assert!(attempts >= 2, "the first read-back was swallowed");
        }
        other => panic!("expected a verified outcome, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn verification_can_be_turned_off() {
    let (mut kvm, _handle) = switch("HKS801-EB23", Faults::NONE);

    let outcome =
        kvm.switch_with(port(2), SwitchOptions::default().verify(Verify::Never)).await.unwrap();

    assert!(matches!(outcome, SwitchOutcome::Unverified { reason: Unverified::Disabled, .. }));
    assert_eq!(kvm.session().stats().writes, 1, "no read-back was sent");
}

#[tokio::test(start_paused = true)]
async fn verification_cannot_be_forced_on_a_switch_that_has_no_read_command() {
    let (mut kvm, _handle) = switch("DKS0401A30", Faults::NONE);

    let error = kvm
        .switch_with(port(2), SwitchOptions::default().verify(Verify::Always))
        .await
        .unwrap_err();

    assert_eq!(error.class(), ErrorClass::Unsupported);
    assert_eq!(kvm.session().stats().writes, 0, "refused before anything was sent");
}

#[tokio::test(start_paused = true)]
async fn a_redundant_switch_can_be_skipped() {
    let (mut kvm, handle) = switch("HKS801-EB23", Faults::NONE);
    handle.force_port(port(4));

    let outcome = kvm
        .switch_with(port(4), SwitchOptions::default().skip_if_active(true).settle(Duration::ZERO))
        .await
        .unwrap();

    assert_eq!(outcome, SwitchOutcome::AlreadyActive(port(4)));
    assert_eq!(kvm.session().stats().writes, 1, "only the pre-read was sent");
}

#[tokio::test(start_paused = true)]
async fn a_port_the_switch_does_not_have_is_refused_before_sending() {
    let (mut kvm, _handle) = switch("DKS0401A30", Faults::NONE);

    let error = kvm.switch(port(8)).await.unwrap_err();

    assert_eq!(error.class(), ErrorClass::Usage);
    assert!(error.to_string().contains("port 8 does not exist on a 4-port switch"), "{error}");
    assert_eq!(kvm.session().stats().writes, 0);
}

#[tokio::test(start_paused = true)]
async fn settings_report_that_they_cannot_be_confirmed() {
    let (mut kvm, handle) = switch("HKS801-EB23", Faults::NONE);

    let sent = kvm.set_buzzer(Buzzer::Muted).await.unwrap();
    assert_eq!(sent.kind, RequestKind::Buzzer);
    assert_eq!(sent.support, Support::Documented);
    assert_eq!(sent.to_string(), "buzzer sent, state cannot be read back");

    kvm.set_display_timeout(DisplayTimeout::Seconds30).await.unwrap();
    kvm.set_auto_input_detect(Toggle::Off).await.unwrap();

    handle
        .settle(|state| {
            state.buzzer == Buzzer::Muted
                && state.display_timeout == DisplayTimeout::Seconds30
                && state.auto_detect == Toggle::Off
        })
        .await;
}

/// Port counts are not reported by any command, so the probe switches inputs to find out.
#[tokio::test(start_paused = true)]
async fn the_port_count_can_be_probed_by_trying_every_input() {
    // An 8-port switch described, wrongly, as having 16.
    let sim = Simulator::from_model(model::lookup("HKS0801A30").unwrap());
    let mut kvm = Kvm::builder(sim.connector())
        .family(Family::Kvm)
        .ports(PortCount::new(16).unwrap())
        .assume(RequestKind::ReadActivePort, Support::Documented)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .switch_defaults(SwitchOptions::default().settle(Duration::ZERO))
        .build()
        .unwrap();

    let probe = kvm.probe_port_count().await.unwrap();

    assert_eq!(probe.found, Some(port(8)), "{probe}");
    assert!(!probe.agrees(), "16 was claimed, 8 was found");
    assert_eq!(probe.claimed, PortCount::new(16).unwrap());
    assert!(probe.restored, "the original input must be put back: {probe}");
    assert_eq!(probe.original, port(1));
    assert!(probe.to_string().contains("8 inputs, but 16 were expected"), "{probe}");

    // And the switch really is back where it started.
    assert_eq!(kvm.active_port().await.unwrap(), port(1));
}

#[tokio::test(start_paused = true)]
async fn probing_the_port_count_needs_a_readable_switch() {
    // The 4x1 DVI series cannot report its port, so there is nothing to probe against.
    let (mut kvm, _handle) = switch("DKS0401A30", Faults::NONE);

    let error = kvm.probe_port_count().await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::Unsupported, "{error}");
}

#[tokio::test(start_paused = true)]
async fn a_setting_the_model_lacks_is_refused() {
    // The first generation 8x1 documents switching and reading, and nothing else.
    let (mut kvm, _handle) = switch("HKS0801A20", Faults::NONE);

    let error = kvm.set_buzzer(Buzzer::Muted).await.unwrap_err();

    assert_eq!(error.class(), ErrorClass::Unsupported);
    assert_eq!(kvm.session().stats().writes, 0);

    // Reading, which this model does document, still works.
    assert_eq!(kvm.active_port().await.unwrap(), port(1));
}

#[tokio::test(start_paused = true)]
async fn an_unverified_setting_carries_its_caveat() {
    // Nothing is known about the M24 series beyond the product line.
    let (mut kvm, _handle) = switch("HKS801-M24", Faults::NONE);

    let sent = kvm.set_buzzer(Buzzer::Muted).await.unwrap();

    assert_eq!(
        sent.warnings,
        [Warning::CapabilityUnverified { kind: RequestKind::Buzzer, support: Support::Unverified }]
    );
}

#[tokio::test(start_paused = true)]
async fn dual_monitor_commands_work_on_a_dual_monitor_switch() {
    let info = model::lookup("dual-4port").unwrap();
    let sim = Simulator::from_model(info);
    sim.handle().set_faults(Faults::NONE);
    let handle = sim.handle();

    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .connection_policy(ConnectionPolicy::Persistent)
        .build()
        .unwrap();

    kvm.switch(port(2)).await.unwrap();
    kvm.step(Output::A, Step::Next).await.unwrap();
    kvm.step(Output::B, Step::Previous).await.unwrap();
    kvm.move_usb().await.unwrap();
    kvm.set_km_passthrough(Toggle::Off).await.unwrap();
    kvm.set_hotkey_trigger(HotkeyTrigger::ScrollLock).await.unwrap();
    kvm.set_auto_switch(Toggle::On).await.unwrap();
    kvm.set_auto_switch_interval(SwitchInterval::from_secs(30).unwrap()).await.unwrap();

    handle
        .settle(|state| {
            state.active_port.get() == 3
                && state.output_b_port.get() == 4
                && state.usb_output == Output::B
                && state.km_passthrough == Toggle::Off
                && state.hotkey == HotkeyTrigger::ScrollLock
                && state.auto_switch == Toggle::On
                && state.auto_switch_interval.map(SwitchInterval::as_secs) == Some(30)
        })
        .await;

    // None of it can be confirmed: the dual-monitor family has no read command at all.
    for kind in [
        RequestKind::KmPassthrough,
        RequestKind::HotkeyTrigger,
        RequestKind::AutoSwitch,
        RequestKind::AutoSwitchInterval,
    ] {
        assert_eq!(kvm.spec().support(kind), Support::FieldTested, "{kind}");
    }
}

#[tokio::test(start_paused = true)]
async fn dual_monitor_settings_are_refused_on_a_single_monitor_switch() {
    let (mut kvm, _handle) = switch("HKS801-EB23", Faults::NONE);

    for result in [
        kvm.set_km_passthrough(Toggle::On).await.err(),
        kvm.set_hotkey_trigger(HotkeyTrigger::RightCtrl).await.err(),
        kvm.set_auto_switch(Toggle::On).await.err(),
        kvm.set_auto_switch_interval(SwitchInterval::from_secs(10).unwrap()).await.err(),
    ] {
        let error = result.expect("a single-monitor switch has none of these commands");
        assert_eq!(error.class(), ErrorClass::Unsupported, "{error}");
    }
    assert_eq!(kvm.session().stats().writes, 0, "nothing reached the wire");
}

#[tokio::test(start_paused = true)]
async fn single_monitor_commands_are_refused_on_a_dual_monitor_switch() {
    let info = model::lookup("dual-4port").unwrap();
    let sim = Simulator::from_model(info);
    let mut kvm = Kvm::builder(sim.connector())
        .model(info)
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    // 0x03 steps output A here, so a display timeout is a different command entirely.
    let error = kvm.set_display_timeout(DisplayTimeout::Never).await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::Unsupported);

    // And switching cannot be verified, because the family has no read command.
    let outcome = kvm.switch(port(2)).await.unwrap();
    assert!(matches!(
        outcome,
        SwitchOutcome::Unverified {
            reason: Unverified::ModelCannotRead { support: Support::Unsupported },
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn dual_monitor_commands_are_refused_on_a_single_monitor_switch() {
    let (mut kvm, _handle) = switch("HKS801-EB23", Faults::NONE);

    let error = kvm.step(Output::A, Step::Next).await.unwrap_err();

    assert_eq!(error.class(), ErrorClass::Unsupported);
    assert_eq!(kvm.session().stats().writes, 0);
}

#[tokio::test(start_paused = true)]
async fn an_unknown_switch_can_be_driven_by_family_and_port_count() {
    let sim = Simulator::custom(
        Family::Kvm,
        PortCount::new(8).unwrap(),
        tesmart_proto::Capabilities::UNVERIFIED,
    );
    sim.handle().set_faults(Faults::NONE);

    let mut kvm = Kvm::builder(sim.connector())
        .family(Family::Kvm)
        .ports(PortCount::new(8).unwrap())
        .protocol_policy(Policy::IMMEDIATE)
        .build()
        .unwrap();

    assert_eq!(kvm.spec().name, None);
    assert_eq!(kvm.spec().to_string(), "unknown kvm switch (8 ports)");

    // Everything is unverified, so nothing is confirmed by default, but it all gets sent.
    let outcome = kvm.switch(port(8)).await.unwrap();
    assert!(!outcome.is_confirmed());
    assert_eq!(outcome.port(), port(8));
}

#[test]
fn the_family_is_never_guessed() {
    let sim = Simulator::from_model(model::lookup("HKS801-EB23").unwrap());

    // No model and no family: refused, rather than defaulting to the single-monitor table.
    assert_eq!(
        Kvm::builder(sim.connector()).ports(PortCount::new(8).unwrap()).build().err(),
        Some(BuildError::NoFamily)
    );

    // A family but no port count is equally unusable.
    assert_eq!(
        Kvm::builder(sim.connector()).family(Family::Kvm).build().err(),
        Some(BuildError::NoPortCount)
    );

    // And a matrix is not a KVM.
    assert_eq!(
        Kvm::from_model(sim.connector(), model::lookup("HMA0404B30").unwrap()).err(),
        Some(BuildError::WrongFamily { expected: "a KVM switch", actual: Family::Matrix })
    );

    // Nor the other way round: a KVM is not a matrix.
    assert_eq!(
        tesmart_client::Matrix::from_model(sim.connector(), model::lookup("HKS801-EB23").unwrap())
            .err(),
        Some(BuildError::WrongFamily { expected: "an HDMI matrix", actual: Family::Kvm })
    );
}
