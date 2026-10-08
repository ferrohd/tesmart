//! Engine against emulator: the whole protocol, both directions, no IO.
//!
//! These are the tests that would catch an inverted `±1` on the port number, a length byte
//! computed from the wrong end, or a retry budget that cannot survive a dropped reply.

use core::num::NonZeroU8;
use core::time::Duration;

use proptest::prelude::*;
use tesmart_device::{Device, DeviceConfig, Faults};
use tesmart_proto::command::ResponseKind;
use tesmart_proto::engine::{Event, ExchangeError, Policy, SubmitError};
use tesmart_proto::model;
use tesmart_proto::profile::{Kvm, KvmDual};
use tesmart_proto::time::Tick;
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, NetField, NetSetting, Output, Step, SwitchInterval,
    Toggle,
};
use tesmart_proto::{Engine, Port, Profile, Request, Response};

/// How many times the harness pumps the engine before declaring a livelock.
const MAX_STEPS: usize = 64;

/// A virtual cable between an engine and an emulated switch.
struct Link<P: Profile> {
    engine: Engine<P, Tick>,
    device: Device,
    now: Tick,
}

impl<P: Profile> Link<P> {
    fn new(profile: P, device: Device, policy: Policy) -> Self {
        Self { engine: Engine::with_policy(profile, policy), device, now: Tick::ZERO }
    }

    /// Runs one exchange to completion and returns the events it produced.
    fn run(&mut self, request: Request) -> Result<Vec<Event>, SubmitError> {
        let _id = self.engine.submit(request, self.now)?;
        Ok(self.pump())
    }

    /// Pumps both ends until the engine is idle.
    fn pump(&mut self) -> Vec<Event> {
        let mut events = Vec::new();

        for _ in 0..MAX_STEPS {
            while let Some(transmit) = self.engine.poll_transmit(self.now) {
                self.device.handle_input(&transmit);
            }
            while let Some(segment) = self.device.poll_transmit() {
                self.engine.handle_input(&segment, self.now);
            }
            while let Some(event) = self.engine.poll_event() {
                events.push(event);
            }
            if self.engine.is_idle() {
                return events;
            }
            // Nothing more to do until a deadline passes: jump the clock to it.
            if let Some(deadline) = self.engine.poll_timeout() {
                if deadline > self.now {
                    self.now = deadline;
                }
                self.engine.handle_timeout(self.now);
            }
        }

        panic!("the exchange never finished");
    }

    /// Advances the clock, so the next command is not held back by command spacing.
    fn settle(&mut self) {
        self.now = self.now + Duration::from_secs(2);
    }
}

fn eb23(faults: Faults) -> Link<Kvm> {
    let model = model::lookup("HKS1601-EB23").unwrap();
    let device = Device::new(DeviceConfig::from_model(model).with_faults(faults));
    Link::new(Kvm, device, Policy::DEFAULT)
}

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

/// A port count paired with a port that actually exists on it.
fn a_switch_and_one_of_its_ports() -> impl Strategy<Value = (u8, u8)> {
    prop::sample::select(vec![4u8, 8, 16]).prop_flat_map(|ports| (Just(ports), 1u8..=ports))
}

fn completed(events: &[Event]) -> Option<Response> {
    events.iter().find_map(|event| match event {
        Event::Completed { response, .. } => Some(*response),
        _ => None,
    })
}

#[test]
fn switching_then_reading_agrees_with_the_switch() {
    let mut link = eb23(Faults::REALISTIC);

    for human in 1..=16 {
        link.settle();
        link.run(Request::Switch(port(human))).unwrap();
        assert_eq!(link.device.state().active_port.get(), human);

        link.settle();
        let events = link.run(Request::ReadActivePort).unwrap();
        assert_eq!(
            completed(&events),
            Some(Response::ActivePort(port(human))),
            "port {human} did not survive the write/read asymmetry"
        );
    }
}

#[test]
fn the_network_round_trip_normalises_padding() {
    let mut link = eb23(Faults::REALISTIC);

    // the emulator answers with the documented zero padding, split across two segments
    let events = link.run(Request::NetQuery(NetField::Ip)).unwrap();
    assert_eq!(
        completed(&events),
        Some(Response::Net(NetSetting::Ip("192.168.1.10".parse().unwrap())))
    );

    link.settle();
    let events = link.run(Request::NetSet(NetSetting::Ip("10.0.2.12".parse().unwrap()))).unwrap();
    assert_eq!(completed(&events), Some(Response::Ack));

    link.settle();
    let events = link.run(Request::NetQuery(NetField::Ip)).unwrap();
    assert_eq!(
        completed(&events),
        Some(Response::Net(NetSetting::Ip("10.0.2.12".parse().unwrap())))
    );
}

#[test]
fn every_family_a_command_reaches_the_switch() {
    let mut link = eb23(Faults::NONE);

    let requests = [
        Request::Switch(port(5)),
        Request::Buzzer(Buzzer::Muted),
        Request::DisplayTimeout(DisplayTimeout::Seconds30),
        Request::AutoInputDetect(Toggle::Off),
        Request::ReadActivePort,
        Request::NetQuery(NetField::Gateway),
        Request::NetSet(NetSetting::Port(5001)),
    ];

    for request in requests {
        link.settle();
        let events = link.run(request).unwrap();
        let response =
            completed(&events).unwrap_or_else(|| panic!("{request} produced {events:?}"));

        match request.expects() {
            Some(expected) => assert_eq!(response.kind(), expected, "{request}"),
            None => assert_eq!(response, Response::Accepted, "{request}"),
        }
    }

    let state = link.device.state();
    assert_eq!(state.active_port.get(), 5);
    assert_eq!(state.buzzer, Buzzer::Muted);
    assert_eq!(state.display_timeout, DisplayTimeout::Seconds30);
    assert_eq!(state.auto_detect, Toggle::Off);
    assert_eq!(state.net.port, Some(5001));
}

#[test]
fn every_dual_monitor_command_reaches_the_switch() {
    let model = model::lookup("dual-4port").unwrap();
    let device = Device::new(DeviceConfig::from_model(model).with_faults(Faults::NONE));
    let mut link = Link::new(KvmDual, device, Policy::DEFAULT);

    let requests = [
        Request::Switch(port(2)),
        Request::StepOutput { output: Output::A, step: Step::Next },
        Request::StepOutput { output: Output::B, step: Step::Previous },
        Request::MoveUsb,
        Request::KmPassthrough(Toggle::Off),
        Request::HotkeyTrigger(HotkeyTrigger::ScrollLock),
        Request::AutoSwitch(Toggle::On),
        Request::AutoSwitchInterval(SwitchInterval::from_secs(30).unwrap()),
        Request::Buzzer(Buzzer::Muted),
    ];

    for request in requests {
        link.settle();
        let events = link.run(request).unwrap();
        assert_eq!(completed(&events), Some(Response::Accepted), "{request}");
    }

    let state = link.device.state();
    assert_eq!(state.active_port.get(), 3, "PC2 then one step up");
    assert_eq!(state.output_b_port.get(), 4, "one step down from PC1 wraps");
    assert_eq!(state.usb_output, Output::B);
    assert_eq!(state.km_passthrough, Toggle::Off);
    assert_eq!(state.hotkey, HotkeyTrigger::ScrollLock);
    assert_eq!(state.auto_switch, Toggle::On);
    assert_eq!(state.auto_switch_interval.map(SwitchInterval::as_secs), Some(30));
}

#[test]
fn a_read_survives_a_dropped_reply() {
    let mut link = eb23(Faults { drop_every: NonZeroU8::new(2), ..Faults::REALISTIC });

    link.run(Request::Switch(port(9))).unwrap();
    link.settle();

    // the emulator swallows every second reply; the retry budget covers it
    for _ in 0..4 {
        let events = link.run(Request::ReadActivePort).unwrap();
        assert_eq!(completed(&events), Some(Response::ActivePort(port(9))));
        link.settle();
    }
}

#[test]
fn a_read_survives_garbage_and_bad_terminators() {
    let mut link = eb23(Faults {
        terminator: Some(0x1C),
        garbage_before: 3,
        bogus_port_every: NonZeroU8::new(3),
        ..Faults::REALISTIC
    });

    link.run(Request::Switch(port(4))).unwrap();

    for _ in 0..6 {
        link.settle();
        let events = link.run(Request::ReadActivePort).unwrap();
        assert_eq!(completed(&events), Some(Response::ActivePort(port(4))), "{events:?}");
    }
}

#[test]
fn a_read_fails_cleanly_when_every_reply_is_lost() {
    let mut link = eb23(Faults { drop_every: NonZeroU8::new(1), ..Faults::NONE });

    let events = link.run(Request::ReadActivePort).unwrap();
    assert!(
        matches!(
            events.as_slice(),
            [Event::Failed { error: ExchangeError::Timeout { attempts: 6 }, .. }]
        ),
        "{events:?}"
    );
}

#[test]
fn a_model_without_a_read_command_times_out_rather_than_lying() {
    // DKS0401A30 documents four switch commands and nothing else
    let model = model::lookup("DKS0401A30").unwrap();
    let device = Device::new(DeviceConfig::from_model(model).with_faults(Faults::NONE));
    let mut link = Link::new(Kvm, device, Policy::DEFAULT);

    link.run(Request::Switch(port(3))).unwrap();
    assert_eq!(link.device.state().active_port.get(), 3);
    link.settle();

    let events = link.run(Request::ReadActivePort).unwrap();
    assert!(
        matches!(events.as_slice(), [Event::Failed { .. }]),
        "a switch that cannot read must not appear to answer: {events:?}"
    );
}

#[test]
fn a_front_panel_change_during_an_exchange_is_reported_separately() {
    let mut link = eb23(Faults::NONE);

    link.engine.submit(Request::ReadActivePort, link.now).unwrap();
    let transmit = link.engine.poll_transmit(link.now).unwrap();
    link.device.handle_input(&transmit);

    // someone presses a button before the reply is read
    link.device.press_button(port(12));

    let events = link.pump();
    assert!(
        events.iter().any(|event| matches!(
            event,
            Event::Completed { response: Response::ActivePort(_), .. }
        )),
        "{events:?}"
    );
}
