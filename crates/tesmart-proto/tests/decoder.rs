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
