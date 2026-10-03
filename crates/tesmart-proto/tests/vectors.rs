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
