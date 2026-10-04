//! Decoder torture tests.
//!
//! The switch's LAN port is a serial bridge, so the decoder is the component most likely to meet
//! something nobody designed for. The central property: **how the stream is cut up must not
//! change what comes out of it**.

use proptest::prelude::*;
use tesmart_proto::wire::{Chunk, StreamDecoder};

/// A chunk that does not borrow the decoder, so sequences can be compared.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unit {
    Port(u8),
    BadPort(u8),
    Frame(u8, Vec<u8>),
    Ascii(String),
    Discarded(usize),
}

/// Feeds `segments` in order and collects everything the decoder produces.
fn decode(segments: &[&[u8]]) -> Vec<Unit> {
    let mut decoder: StreamDecoder = StreamDecoder::new();
    let mut units = Vec::new();

    for segment in segments {
        let mut rest = *segment;
        while !rest.is_empty() {
            let accepted = decoder.feed(rest);
            rest = &rest[accepted..];
            drain(&mut decoder, &mut units);
            if accepted == 0 {
                decoder.reset();
            }
        }
        drain(&mut decoder, &mut units);
    }
    units
}

fn drain(decoder: &mut StreamDecoder, units: &mut Vec<Unit>) {
    while let Some(chunk) = decoder.poll_chunk() {
        units.push(match chunk {
            Chunk::PortReport(port) => Unit::Port(port.get()),
            Chunk::BadPortReport { raw } => Unit::BadPort(raw),
            Chunk::Frame(frame) => Unit::Frame(frame.opcode, frame.payload.as_slice().to_vec()),
            Chunk::Ascii(text) => Unit::Ascii(text.to_owned()),
            Chunk::Discarded(bytes) => Unit::Discarded(bytes.len()),
        });
    }
}

/// The meaningful units, plus the total number of bytes thrown away.
///
/// Discarded runs may be grouped differently depending on where the stream was cut, but nothing
/// meaningful may appear, disappear or change order, and the same bytes must be lost.
fn semantics(units: &[Unit]) -> (Vec<Unit>, usize) {
    let dropped = units
        .iter()
        .map(|unit| match unit {
            Unit::Discarded(len) => *len,
            _ => 0,
        })
        .sum();
    let kept = units.iter().filter(|unit| !matches!(unit, Unit::Discarded(_))).cloned().collect();
    (kept, dropped)
}

fn split_at_every_offset(trace: &[u8]) {
    let whole = semantics(&decode(&[trace]));

    for at in 0..=trace.len() {
        let (head, tail) = trace.split_at(at);
        assert_eq!(
            semantics(&decode(&[head, tail])),
            whole,
            "splitting after {at} bytes changed the result"
        );
    }

    let byte_by_byte: Vec<&[u8]> = trace.chunks(1).collect();
    assert_eq!(
        semantics(&decode(&byte_by_byte)),
        whole,
        "feeding one byte at a time changed the result"
    );
}

#[test]
fn documented_traces_survive_every_split() {
    let traces: &[&[u8]] = &[
        // a read reply
        &[0xAA, 0xBB, 0x03, 0x11, 0x02, 0xEE],
        // the seven-byte dual-monitor frame
        &[0xAA, 0xBB, 0x04, 0x09, 0x00, 0x05, 0xEE],
        // two replies back to back
        &[0xAA, 0xBB, 0x03, 0x11, 0x00, 0xEE, 0xAA, 0xBB, 0x03, 0x11, 0x01, 0xEE],
        // an address reply, then an unsolicited port report
        b"IP:010.000.002.012;\xAA\xBB\x03\x11\x03\xEE",
        // an acknowledgement with no terminator followed by a frame
        b"OK\xAA\xBB\x03\x11\x03\xEE",
        // garbage, a frame, more garbage, another frame
        &[
            0x00, 0xFF, 0x5A, 0xAA, 0xBB, 0x03, 0x11, 0x05, 0xEE, 0x13, 0xAA, 0xBB, 0x03, 0x11,
            0x06, 0xEE,
        ],
        // a bad terminator
        &[0xAA, 0xBB, 0x03, 0x11, 0x01, 0x16],
        // a matrix routing reply, whose terminator comes after a semicolon
        b"LINK:O1I2;O2I1;END",
    ];

    for trace in traces {
        split_at_every_offset(trace);
    }
}

#[test]
fn an_undecided_header_byte_does_not_trigger_an_early_discard() {
    // Regression: `00 AA 00` dropped the leading byte when the stream was cut after the 0xAA,
    // because a lone 0xAA was treated as proof that the text could never be completed. Found by
    // `chunking_does_not_change_the_result`.
    assert_eq!(decode(&[&[0x00, 0xAA], &[0x00]]), decode(&[&[0x00, 0xAA, 0x00]]));
    assert_eq!(decode(&[&[0x00, 0xAA, 0x00]]), []);
}

#[test]
fn bad_terminators_are_accepted() {
    // 0x16 and 0x1C have both been seen in place of 0xEE
    for terminator in [0xEE, 0x16, 0x1C] {
        assert_eq!(
            decode(&[&[0xAA, 0xBB, 0x03, 0x11, 0x04, terminator]]),
            [Unit::Port(5)],
            "terminator 0x{terminator:02X}"
        );
    }
}

#[test]
fn an_impossible_port_index_is_reported_not_guessed() {
    // tesmart.sh maps a read value of 0x11 to PC1. Unexplained, and not copied here.
    assert_eq!(decode(&[&[0xAA, 0xBB, 0x03, 0x11, 0x11, 0xEE]]), [Unit::BadPort(0x11)]);
    assert_eq!(decode(&[&[0xAA, 0xBB, 0x03, 0x11, 0xFF, 0xEE]]), [Unit::BadPort(0xFF)]);
}

#[test]
fn a_corrupt_length_byte_does_not_stall_the_decoder() {
    // 0xAA BB FF would otherwise mean "wait for 258 bytes"
    let mut units =
        decode(&[&[0xAA, 0xBB, 0xFF, 0x11, 0x00, 0xEE, 0xAA, 0xBB, 0x03, 0x11, 0x07, 0xEE]]);
    units.retain(|unit| !matches!(unit, Unit::Discarded(_)));
    assert_eq!(units, [Unit::Port(8)], "the following frame must still be found");
}

#[test]
fn a_frame_longer_than_any_known_command_is_skipped_whole() {
    // len = 0x05 is well formed but carries three data bytes, which no command does
    let units = decode(&[&[
        0xAA, 0xBB, 0x05, 0x42, 0x01, 0x02, 0x03, 0xEE, 0xAA, 0xBB, 0x03, 0x11, 0x00, 0xEE,
    ]]);
    assert_eq!(units, [Unit::Discarded(8), Unit::Port(1)]);
}

#[test]
fn text_is_never_mistaken_for_a_frame_or_the_other_way_round() {
    // 0xAA not followed by 0xBB is not a header
    assert_eq!(
        decode(&[&[0xAA, 0x42, 0xAA, 0xBB, 0x03, 0x11, 0x00, 0xEE]]),
        [Unit::Discarded(2), Unit::Port(1)]
    );
    // and printable bytes before a header are not left in the buffer forever
    assert_eq!(
        decode(&[b"IP:10.0", &[0xAA, 0xBB, 0x03, 0x11, 0x00, 0xEE]]),
        [Unit::Discarded(7), Unit::Port(1)]
    );
}

#[test]
fn the_buffer_cannot_be_overrun() {
    let mut decoder: StreamDecoder = StreamDecoder::new();
    let flood = vec![0x5A; decoder.capacity() * 4];
    let mut accepted_total = 0;
    let mut rest = flood.as_slice();

    while !rest.is_empty() {
        let accepted = decoder.feed(rest);
        accepted_total += accepted;
        rest = &rest[accepted..];
        if accepted == 0 {
            // nothing decodable: the caller is expected to reset, which is what the engine does
            assert!(decoder.poll_chunk().is_none());
            decoder.reset();
        }
    }
    assert_eq!(accepted_total, flood.len());
    assert!(decoder.buffered() <= decoder.capacity());
}
