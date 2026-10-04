//! The engine's timing and demultiplexing rules, with a fake clock.
//!
//! Every quirk here comes from `docs/protocol-reference.md`: 1 s command spacing, retries 200 ms
//! apart, unsolicited port reports, garbage read replies. None of it needs a socket or a real
//! clock to test.

use core::time::Duration;

use tesmart_proto::command::ResponseKind;
use tesmart_proto::engine::{Event, ExchangeError, Policy, SubmitError};
use tesmart_proto::profile::{Kvm, KvmDual};
use tesmart_proto::time::Tick;
use tesmart_proto::value::{Buzzer, DisplayTimeout, NetField, NetSetting};
use tesmart_proto::{Engine, Port, Request, Response};

const SECOND: Duration = Duration::from_secs(1);

fn kvm() -> Engine<Kvm, Tick> {
    Engine::new(Kvm)
}

fn events<P: tesmart_proto::Profile>(engine: &mut Engine<P, Tick>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(event) = engine.poll_event() {
        out.push(event);
    }
    out
}

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

/// `AA BB 03 11 <0-based port> EE`
fn port_report(human: u8) -> [u8; 6] {
    [0xAA, 0xBB, 0x03, 0x11, human - 1, 0xEE]
}

#[test]
fn a_write_completes_as_soon_as_it_is_handed_over() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::Switch(port(3)), now).unwrap();
    assert!(!engine.is_idle());

    let transmit = engine.poll_transmit(now).expect("first command goes out at once");
    assert_eq!(&*transmit, &[0xAA, 0xBB, 0x03, 0x01, 0x03, 0xEE]);
    assert_eq!(transmit.attempt(), 1);

    // The protocol defines no reply to 0x01, so there is nothing to wait for and nothing to
    // verify from the engine's point of view.
    assert_eq!(events(&mut engine), [Event::Completed { id, response: Response::Accepted }]);
    assert!(engine.is_idle());
    assert_eq!(engine.poll_timeout(), None);
}

#[test]
fn a_read_completes_on_its_reply() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::ReadActivePort, now).unwrap();
    assert_eq!(&*engine.poll_transmit(now).unwrap(), &[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]);
    assert_eq!(engine.poll_timeout(), Some(Tick::from_millis(500)));

    engine.handle_input(&port_report(4), Tick::from_millis(120));
    assert_eq!(
        events(&mut engine),
        [Event::Completed { id, response: Response::ActivePort(port(4)) }]
    );
    assert!(engine.is_idle());
}

#[test]
fn commands_are_spaced_a_second_apart() {
    let mut engine = kvm();
    let start = Tick::ZERO;

    engine.submit(Request::Switch(port(1)), start).unwrap();
    assert!(engine.poll_transmit(start).is_some());
    let _ = events(&mut engine);

    engine.submit(Request::Switch(port(2)), start).unwrap();
    assert!(engine.poll_transmit(start).is_none(), "a second command must not follow immediately");
    assert_eq!(engine.poll_timeout(), Some(start + SECOND));

    assert!(engine.poll_transmit(Tick::from_millis(999)).is_none());
    assert!(engine.poll_transmit(start + SECOND).is_some());
}

#[test]
fn the_engine_refuses_to_multiplex() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let first = engine.submit(Request::ReadActivePort, now).unwrap();
    assert_eq!(
        engine.submit(Request::Switch(port(1)), now),
        Err(SubmitError::Busy { pending: first })
    );
    assert_eq!(engine.pending(), Some(first));
}
