//! The switch side of the TESmart protocol.
//!
//! [`tesmart_proto`] is the controller: it turns requests into bytes and bytes into events.
//! This crate is the other end of the same wire — bytes in, bytes out, no IO and no clock —
//! so the two together describe the whole protocol without anything having to be plugged in.
//!
//! [`tesmart-sim`]: https://crates.io/crates/tesmart-sim
#![cfg_attr(not(test), no_std)]
// `must_use_candidate` fires on nearly every accessor in a crate made of value types.
#![allow(clippy::must_use_candidate)]

use core::net::Ipv4Addr;
use core::num::NonZeroU8;

use tesmart_proto::model::{Capabilities, ModelInfo, Support};
use tesmart_proto::port::{Port, PortCount};
use tesmart_proto::profile::Family;
use tesmart_proto::value::{
    Buzzer, DisplayTimeout, HotkeyTrigger, MatrixTimeout, NetConfig, NetSetting, Output, Preset,
    SwitchInterval, Toggle,
};
use tesmart_proto::wire::{AsciiDialect, Chunk, DecodeMode, StreamDecoder, WireBytes, ascii};

/// How many reply segments can be queued before the oldest is dropped.
const OUTBOX: usize = 8;

/// The queue of segments a device is waiting to put on the wire.
///
/// Fixed capacity, and it evicts the oldest rather than growing or panicking: there is no
/// allocator here, and a controller that has stopped reading is a controller that no longer
/// cares about the segment from eight replies ago.
#[derive(Debug)]
struct Outbox {
    items: [Option<WireBytes>; OUTBOX],
    head: usize,
    len: usize,
}

impl Outbox {
    const fn new() -> Self {
        Self { items: [const { None }; OUTBOX], head: 0, len: 0 }
    }

    /// Queues a segment, dropping the oldest if there is no room.
    fn push(&mut self, bytes: WireBytes) {
        if self.len == OUTBOX {
            let _evicted = self.pop();
        }
        self.items[(self.head + self.len) % OUTBOX] = Some(bytes);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<WireBytes> {
        if self.len == 0 {
            return None;
        }
        let item = self.items[self.head].take();
        self.head = (self.head + 1) % OUTBOX;
        self.len -= 1;
        item
    }
}

/// How many bytes the LAN bridge hands over at a time.
///
/// Observed at about 48, which is why a padded address reply arrives with its `;` in a segment
/// of its own, and why a 16x16 routing report arrives in two pieces.
const SEGMENT: usize = 48;
