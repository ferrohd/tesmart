//! The engine's timing and demultiplexing rules, with a fake clock.
//!
//! Every quirk here comes from `docs/protocol-reference.md`: 1 s command spacing, retries 200 ms
//! apart, unsolicited port reports, garbage read replies. None of it needs a socket or a real
//! clock to test.

use core::time::Duration;

use tesmart_proto::command::ResponseKind;
use tesmart_proto::engine::{Event, ExchangeError, Policy, SubmitError};
use tesmart_proto::protocol::{KvmDualTable, KvmTable};
use tesmart_proto::time::Tick;
use tesmart_proto::value::{Buzzer, DisplayTimeout, NetField, NetSetting};
use tesmart_proto::{Engine, Port, Request, Response};

const SECOND: Duration = Duration::from_secs(1);

fn kvm() -> Engine<KvmTable, Tick> {
    Engine::new(KvmTable)
}

fn events<P: tesmart_proto::CommandTable>(engine: &mut Engine<P, Tick>) -> Vec<Event> {
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

#[test]
fn unsupported_commands_never_reach_the_wire() {
    let mut engine: Engine<KvmDualTable, Tick> = Engine::new(KvmDualTable);
    let error = engine
        .submit(Request::DisplayTimeout(DisplayTimeout::Never), Tick::ZERO)
        .expect_err("0x03 means something else on dual-monitor switches");

    assert!(matches!(error, SubmitError::Unsupported(_)));
    assert!(engine.is_idle());
    assert!(engine.poll_transmit(Tick::ZERO).is_none());
}

#[test]
fn queries_are_retried_then_fail_loudly() {
    let policy = Policy::DEFAULT.with_max_retries(2);
    let mut engine = Engine::with_policy(KvmTable, policy);
    let mut now = Tick::ZERO;

    let id = engine.submit(Request::ReadActivePort, now).unwrap();
    assert_eq!(engine.poll_transmit(now).unwrap().attempt(), 1);

    for attempt in 2..=3 {
        // deadline passes with no reply
        now = engine.poll_timeout().unwrap();
        engine.handle_timeout(now);
        assert!(events(&mut engine).is_empty(), "a retry is not a failure");

        // the retry waits for the 200 ms backoff, not for the 1 s command gap
        let retry_at = engine.poll_timeout().unwrap();
        assert_eq!(retry_at, now + Duration::from_millis(200));
        assert!(engine.poll_transmit(now).is_none());

        now = retry_at;
        assert_eq!(engine.poll_transmit(now).unwrap().attempt(), attempt);
    }

    now = engine.poll_timeout().unwrap();
    engine.handle_timeout(now);
    assert_eq!(
        events(&mut engine),
        [Event::Failed { id, error: ExchangeError::Timeout { attempts: 3 } }]
    );
    assert!(engine.is_idle());
}

#[test]
fn writes_are_never_retried() {
    // Repeating `IP:10.0.2.12;` on a flaky link is not something to do behind the user's back.
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::NetSet(NetSetting::Port(5000)), now).unwrap();
    assert_eq!(&*engine.poll_transmit(now).unwrap(), b"PT:5000;");

    let deadline = engine.poll_timeout().unwrap();
    engine.handle_timeout(deadline);
    assert_eq!(
        events(&mut engine),
        [Event::Failed { id, error: ExchangeError::Timeout { attempts: 1 } }]
    );
}

#[test]
fn an_unsolicited_report_is_not_the_answer_to_another_command() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    // a switch command is in flight when someone presses a front panel button
    let id = engine.submit(Request::Switch(port(2)), now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();
    assert_eq!(events(&mut engine), [Event::Completed { id, response: Response::Accepted }]);

    engine.handle_input(&port_report(7), now);
    assert_eq!(events(&mut engine), [Event::PortChanged(port(7))]);
}

#[test]
fn a_report_arriving_mid_read_still_answers_the_read() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::ReadActivePort, now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();

    // two reports in one read: the first answers the query, the second is a change
    engine.handle_input(&port_report(1), now);
    engine.handle_input(&port_report(5), Tick::from_millis(10));

    assert_eq!(
        events(&mut engine),
        [
            Event::Completed { id, response: Response::ActivePort(port(1)) },
            Event::PortChanged(port(5)),
        ]
    );
}

#[test]
fn the_late_reply_to_a_retried_read_is_not_a_port_change() {
    let mut engine = Engine::with_policy(KvmTable, Policy::DEFAULT.with_max_retries(1));
    let mut now = Tick::ZERO;

    let id = engine.submit(Request::ReadActivePort, now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();

    // first attempt times out, second attempt goes out
    now = engine.poll_timeout().unwrap();
    engine.handle_timeout(now);
    now = engine.poll_timeout().unwrap();
    let _sent = engine.poll_transmit(now).unwrap();

    // the switch answers twice: the slow first reply and the second one
    engine.handle_input(&port_report(3), now);
    now = now + Duration::from_millis(50);
    engine.handle_input(&port_report(3), now);

    assert_eq!(
        events(&mut engine),
        [Event::Completed { id, response: Response::ActivePort(port(3)) }],
        "the duplicate must not be reported as someone switching ports"
    );

    // but a genuine change after the window is reported
    now = now + Duration::from_secs(2);
    engine.handle_input(&port_report(3), now);
    assert_eq!(events(&mut engine), [Event::PortChanged(port(3))]);
}

#[test]
fn garbage_read_replies_are_reported_and_retried() {
    let mut engine = Engine::with_policy(KvmTable, Policy::DEFAULT.with_max_retries(1));
    let now = Tick::ZERO;

    engine.submit(Request::ReadActivePort, now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();

    // 0x10 is one past the last port: a known kind of bad reply
    engine.handle_input(&[0xAA, 0xBB, 0x03, 0x11, 0x10, 0xEE], now);
    assert_eq!(events(&mut engine), [Event::InvalidPortReport { raw: 0x10 }]);
    assert!(!engine.is_idle(), "the read is still outstanding");

    // the good reply to the retry is accepted
    let deadline = engine.poll_timeout().unwrap();
    engine.handle_timeout(deadline);
    let ready = engine.poll_timeout().unwrap();
    let _sent = engine.poll_transmit(ready).unwrap();
    engine.handle_input(&port_report(2), ready);

    assert!(matches!(
        events(&mut engine).as_slice(),
        [Event::Completed { response: Response::ActivePort(_), .. }]
    ));
}

#[test]
fn a_bare_ok_is_accepted_without_a_terminator() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id =
        engine.submit(Request::NetSet(NetSetting::Ip("10.0.2.12".parse().unwrap())), now).unwrap();
    assert_eq!(&*engine.poll_transmit(now).unwrap(), b"IP:10.0.2.12;");

    // acknowledgements have been observed with no terminator at all
    engine.handle_input(b"OK", now);
    assert_eq!(events(&mut engine), [Event::Completed { id, response: Response::Ack }]);
}

#[test]
fn a_truncated_reply_is_recovered_on_the_deadline() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::NetQuery(NetField::Ip), now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();

    // the value arrives but its `;` never does, because the switch closed the connection
    engine.handle_input(b"IP:010.000.002.012", now);
    assert!(events(&mut engine).is_empty(), "a reply without a terminator is not complete yet");

    let deadline = engine.poll_timeout().unwrap();
    engine.handle_timeout(deadline);
    assert_eq!(
        events(&mut engine),
        [Event::Completed {
            id,
            response: Response::Net(NetSetting::Ip("10.0.2.12".parse().unwrap())),
        }],
        "the deadline flushes what is buffered rather than throwing it away"
    );
}

#[test]
fn split_ascii_replies_are_reassembled() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::NetQuery(NetField::Ip), now).unwrap();
    assert_eq!(&*engine.poll_transmit(now).unwrap(), b"IP?");

    // the switch sends the value and the terminator in separate segments
    engine.handle_input(b"IP:010.000.002.012", now);
    assert_eq!(events(&mut engine).len(), 0);
    engine.handle_input(b";", now);

    assert_eq!(
        events(&mut engine),
        [Event::Completed {
            id,
            response: Response::Net(NetSetting::Ip("10.0.2.12".parse().unwrap())),
        }]
    );
}

#[test]
fn junk_is_reported_without_losing_the_frame_behind_it() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::ReadActivePort, now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();

    let mut stream = vec![0x00, 0xFF, 0x5A];
    stream.extend_from_slice(&port_report(6));
    engine.handle_input(&stream, now);

    assert_eq!(
        events(&mut engine),
        [
            Event::Desync { dropped: 3 },
            Event::Completed { id, response: Response::ActivePort(port(6)) },
        ]
    );
}

#[test]
fn a_reply_nobody_asked_for_is_reported_as_unexpected() {
    let mut engine = kvm();
    engine.handle_input(b"OK;", Tick::ZERO);
    assert_eq!(events(&mut engine), [Event::UnexpectedReply { kind: ResponseKind::Ack }]);
}

#[test]
fn raw_exchanges_bypass_the_tables_and_accept_any_reply() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit_raw(&[0xAA, 0xBB, 0x03, 0x42, 0x01, 0xEE], true, now).unwrap();
    assert_eq!(&*engine.poll_transmit(now).unwrap(), &[0xAA, 0xBB, 0x03, 0x42, 0x01, 0xEE]);

    engine.handle_input(&[0xAA, 0xBB, 0x03, 0x43, 0x07, 0xEE], now);
    let [Event::Completed { id: done, response: Response::Frame(frame) }] =
        events(&mut engine).as_slice()[..]
    else {
        panic!("a raw exchange takes whatever comes back")
    };
    assert_eq!(done, id);
    assert_eq!(frame.opcode, 0x43);
}

#[test]
fn raw_bytes_longer_than_a_protocol_unit_are_refused() {
    let mut engine = kvm();
    let too_long = [0x00; tesmart_proto::wire::MAX_WIRE_LEN + 1];
    assert!(matches!(engine.submit_raw(&too_long, false, Tick::ZERO), Err(SubmitError::Wire(_))));

    // and the limit itself is accepted, so the boundary is where it claims to be
    let at_limit = [0x00; tesmart_proto::wire::MAX_WIRE_LEN];
    assert!(engine.submit_raw(&at_limit, false, Tick::ZERO).is_ok());
}

#[test]
fn reset_abandons_the_exchange_but_keeps_the_command_gap() {
    let mut engine = kvm();
    let now = Tick::ZERO;

    let id = engine.submit(Request::ReadActivePort, now).unwrap();
    let _sent = engine.poll_transmit(now).unwrap();
    engine.handle_input(&[0xAA, 0xBB], now); // partial frame, then the link drops

    engine.reset();
    assert_eq!(events(&mut engine), [Event::Failed { id, error: ExchangeError::Aborted }]);
    assert!(engine.is_idle());

    // a command on the new connection still waits for the switch's 1 s gap
    engine.submit(Request::Switch(port(1)), now).unwrap();
    assert!(engine.poll_transmit(now).is_none());
    assert!(engine.poll_transmit(now + SECOND).is_some());
}

#[test]
fn events_are_dropped_oldest_first_and_counted() {
    let mut engine = Engine::with_policy(KvmTable, Policy::IMMEDIATE);
    let mut now = Tick::ZERO;

    // more unsolicited reports than the event queue holds, alternating so none is a duplicate
    for round in 0..12u8 {
        let human = if round % 2 == 0 { 1 } else { 2 };
        engine.handle_input(&port_report(human), now);
        now = now + Duration::from_millis(200);
    }

    assert!(engine.dropped_events() > 0);
    assert_eq!(engine.queued_events(), 8);
    // the newest report survives
    let last = events(&mut engine).pop().unwrap();
    assert_eq!(last, Event::PortChanged(port(2)));
}

#[test]
fn settings_are_fire_and_forget_for_every_hex_protocol() {
    for request in
        [Request::Buzzer(Buzzer::Muted), Request::DisplayTimeout(DisplayTimeout::Seconds30)]
    {
        let mut engine = kvm();
        let id = engine.submit(request, Tick::ZERO).unwrap();
        let _sent = engine.poll_transmit(Tick::ZERO).unwrap();
        assert_eq!(
            events(&mut engine),
            [Event::Completed { id, response: Response::Accepted }],
            "{request} cannot be verified: the switch says nothing"
        );
    }
}
