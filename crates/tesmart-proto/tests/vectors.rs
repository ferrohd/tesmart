//! Golden vectors: every byte sequence printed in `docs/protocol-reference.md`.
//!
//! If any of these change, the protocol reference has to change with them.

use tesmart_proto::profile::{Kvm, KvmDual, Matrix, Sdi};
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, MatrixTimeout, NetField, NetSetting, Output, Preset,
    Step, SwitchInterval, Toggle,
};
use tesmart_proto::wire::{
    AsciiDialect, AsciiScope, AsciiStyle, Chunk, Frame, Payload, StreamDecoder,
};
use tesmart_proto::{Port, Profile, Request, Response};

fn port(n: u8) -> Port {
    Port::new(n).unwrap()
}

fn bytes<P: Profile>(profile: P, request: Request) -> Vec<u8> {
    styled(profile, request, AsciiStyle::DEFAULT)
}

fn styled<P: Profile>(profile: P, request: Request, style: AsciiStyle) -> Vec<u8> {
    profile
        .encode(request, style)
        .unwrap_or_else(|error| panic!("{error}"))
        .bytes()
        .as_slice()
        .to_vec()
}

#[test]
fn family_a_frames() {
    let vectors: &[(Request, &[u8])] = &[
        // switch, the one command every series documents
        (Request::Switch(port(1)), &[0xAA, 0xBB, 0x03, 0x01, 0x01, 0xEE]),
        (Request::Switch(port(2)), &[0xAA, 0xBB, 0x03, 0x01, 0x02, 0xEE]),
        (Request::Switch(port(16)), &[0xAA, 0xBB, 0x03, 0x01, 0x10, 0xEE]),
        // buzzer
        (Request::Buzzer(Buzzer::Muted), &[0xAA, 0xBB, 0x03, 0x02, 0x00, 0xEE]),
        (Request::Buzzer(Buzzer::Audible), &[0xAA, 0xBB, 0x03, 0x02, 0x01, 0xEE]),
        // front display timeout
        (Request::DisplayTimeout(DisplayTimeout::Seconds10), &[0xAA, 0xBB, 0x03, 0x03, 0x0A, 0xEE]),
        (Request::DisplayTimeout(DisplayTimeout::Seconds30), &[0xAA, 0xBB, 0x03, 0x03, 0x1E, 0xEE]),
        (Request::DisplayTimeout(DisplayTimeout::Never), &[0xAA, 0xBB, 0x03, 0x03, 0x00, 0xEE]),
        // auto input detect
        (Request::AutoInputDetect(Toggle::Off), &[0xAA, 0xBB, 0x03, 0x81, 0x00, 0xEE]),
        (Request::AutoInputDetect(Toggle::On), &[0xAA, 0xBB, 0x03, 0x81, 0x01, 0xEE]),
        // read active port
        (Request::ReadActivePort, &[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]),
    ];

    for (request, expected) in vectors {
        assert_eq!(bytes(Kvm, *request), *expected, "{request}");
    }
}

#[test]
fn family_a_ascii() {
    let vectors: &[(Request, &str)] = &[
        (Request::NetQuery(NetField::Ip), "IP?"),
        (Request::NetQuery(NetField::Port), "PT?"),
        (Request::NetQuery(NetField::Gateway), "GW?"),
        (Request::NetQuery(NetField::Mask), "MA?"),
        (Request::NetSet(NetSetting::Ip("10.0.2.12".parse().unwrap())), "IP:10.0.2.12;"),
        (Request::NetSet(NetSetting::Port(5000)), "PT:5000;"),
        (Request::NetSet(NetSetting::Gateway("10.0.2.1".parse().unwrap())), "GW:10.0.2.1;"),
        (Request::NetSet(NetSetting::Mask("255.255.255.0".parse().unwrap())), "MA:255.255.255.0;"),
    ];

    for (request, expected) in vectors {
        assert_eq!(bytes(Kvm, *request), expected.as_bytes(), "{request}");
    }
}

#[test]
fn family_a_ascii_in_the_tesmart_sh_dialect() {
    let style = AsciiStyle::new(AsciiDialect::TesmartSh);
    let query = Kvm.encode(Request::NetQuery(NetField::Ip), style).unwrap();
    assert_eq!(&*query, b"IP?;");

    let set =
        Kvm.encode(Request::NetSet(NetSetting::Ip("10.0.2.12".parse().unwrap())), style).unwrap();
    assert_eq!(&*set, b"IP: 10.0.2.12;");
}

/// A KVM has no wrapped form, so the scope must not change a single byte it sends.
#[test]
fn family_a_ignores_the_ascii_scope() {
    let serial = AsciiStyle::DEFAULT.with_scope(AsciiScope::Serial);
    for request in [
        Request::NetQuery(NetField::Ip),
        Request::NetSet(NetSetting::Port(5000)),
        Request::Switch(port(3)),
    ] {
        assert_eq!(bytes(Kvm, request), styled(Kvm, request, serial), "{request}");
    }
}

#[test]
fn family_a_prime_frames() {
    let vectors: &[(Request, &[u8])] = &[
        (Request::Switch(port(4)), &[0xAA, 0xBB, 0x03, 0x01, 0x04, 0xEE]),
        (Request::Buzzer(Buzzer::Muted), &[0xAA, 0xBB, 0x03, 0x02, 0x00, 0xEE]),
        (
            Request::StepOutput { output: Output::A, step: Step::Previous },
            &[0xAA, 0xBB, 0x03, 0x03, 0x00, 0xEE],
        ),
        (
            Request::StepOutput { output: Output::A, step: Step::Next },
            &[0xAA, 0xBB, 0x03, 0x03, 0x01, 0xEE],
        ),
        (
            Request::StepOutput { output: Output::B, step: Step::Previous },
            &[0xAA, 0xBB, 0x03, 0x04, 0x00, 0xEE],
        ),
        (
            Request::StepOutput { output: Output::B, step: Step::Next },
            &[0xAA, 0xBB, 0x03, 0x04, 0x01, 0xEE],
        ),
        (Request::MoveUsb, &[0xAA, 0xBB, 0x03, 0x05, 0x01, 0xEE]),
        // inverted: 0x00 turns pass-through on
        (Request::KmPassthrough(Toggle::On), &[0xAA, 0xBB, 0x03, 0x06, 0x00, 0xEE]),
        (Request::KmPassthrough(Toggle::Off), &[0xAA, 0xBB, 0x03, 0x06, 0x01, 0xEE]),
        (Request::HotkeyTrigger(HotkeyTrigger::RightCtrl), &[0xAA, 0xBB, 0x03, 0x07, 0x01, 0xEE]),
        (Request::HotkeyTrigger(HotkeyTrigger::ScrollLock), &[0xAA, 0xBB, 0x03, 0x07, 0x00, 0xEE]),
        (Request::AutoSwitch(Toggle::Off), &[0xAA, 0xBB, 0x03, 0x08, 0x00, 0xEE]),
        (Request::AutoSwitch(Toggle::On), &[0xAA, 0xBB, 0x03, 0x08, 0x01, 0xEE]),
        // the only seven-byte frame in the protocol
        (
            Request::AutoSwitchInterval(SwitchInterval::from_raw(0x00, 0x05).unwrap()),
            &[0xAA, 0xBB, 0x04, 0x09, 0x00, 0x05, 0xEE],
        ),
        (
            Request::AutoSwitchInterval(SwitchInterval::from_raw(0x38, 0x38).unwrap()),
            &[0xAA, 0xBB, 0x04, 0x09, 0x38, 0x38, 0xEE],
        ),
    ];

    for (request, expected) in vectors {
        assert_eq!(bytes(KvmDual, *request), *expected, "{request}");
    }
}

#[test]
fn family_b_strings() {
    let vectors: &[(Request, &str)] = &[
        (Request::MatrixRoute { input: port(1), output: port(2) }, "MT00SW0102NT"),
        (Request::MatrixRouteAll(port(1)), "MT00SW0100NT"),
        (Request::MatrixMirror, "MT00SW0000NT"),
        (Request::MatrixReadRouting, "MT00RD0000NT"),
        (Request::MatrixDisplayTimeout(MatrixTimeout::NEVER), "MT00CT0000NT"),
        (Request::MatrixDisplayTimeout(MatrixTimeout::from_secs(9999).unwrap()), "MT00CT9999NT"),
        // 8x8 and 16x16: presets, and a buzzer whose values are the other way round
        (Request::MatrixSavePreset(Preset::new(1).unwrap()), "MT00SV0001NT"),
        (Request::MatrixSavePreset(Preset::new(8).unwrap()), "MT00SV0008NT"),
        (Request::MatrixLoadPreset(Preset::new(1).unwrap()), "MT00RD0101NT"),
        (Request::MatrixLoadPreset(Preset::new(8).unwrap()), "MT00RD0108NT"),
        (Request::MatrixBuzzer(Buzzer::Muted), "MT00BZEN01NT"),
        (Request::MatrixBuzzer(Buzzer::Audible), "MT00BZEN00NT"),
        // 16x16 addresses two digits per field
        (Request::MatrixRoute { input: port(16), output: port(16) }, "MT00SW1616NT"),
    ];

    for (request, expected) in vectors {
        assert_eq!(bytes(Matrix, *request), expected.as_bytes(), "{request}");
    }
}

/// The matrix network commands, which are spelled differently per transport.
#[test]
fn family_b_network_strings() {
    let lan: &[(Request, &str)] = &[
        (Request::NetQuery(NetField::Ip), "IP?"),
        (Request::NetQuery(NetField::Port), "PT?"),
        (Request::NetQuery(NetField::Gateway), "GW?"),
        (Request::NetQuery(NetField::Mask), "MA?"),
    ];
    for (request, expected) in lan {
        assert_eq!(bytes(Matrix, *request), expected.as_bytes(), "{request}");
    }

    let serial_style = AsciiStyle::DEFAULT.with_scope(AsciiScope::Serial);
    let serial: &[(Request, &str)] = &[
        (Request::NetQuery(NetField::Ip), "MT8003IP?"),
        (Request::NetQuery(NetField::Port), "MT8003PT?"),
        (Request::NetQuery(NetField::Gateway), "MT8003GW?"),
        (Request::NetQuery(NetField::Mask), "MT8003MA?"),
        (
            Request::NetSet(NetSetting::Ip("10.0.2.12".parse().unwrap())),
            "MT8019IP:010.000.002.012;",
        ),
        // the port write is the one that uses MT8009
        (Request::NetSet(NetSetting::Port(5000)), "MT8009PT:05000;"),
        (
            Request::NetSet(NetSetting::Gateway("10.0.2.1".parse().unwrap())),
            "MT8019GW:010.000.002.001;",
        ),
        (
            Request::NetSet(NetSetting::Mask("255.255.255.0".parse().unwrap())),
            "MT8019MA:255.255.255.000;",
        ),
    ];
    for (request, expected) in serial {
        assert_eq!(styled(Matrix, *request, serial_style), expected.as_bytes(), "{request}");
    }
}

/// Family C has four commands, and they are all four bytes long.
