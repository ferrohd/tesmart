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
