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

/// Deliberate misbehaviour, to reproduce what hardware does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Faults {
    /// Terminate frames with this byte instead of `0xEE`. `0x16` and `0x1C` have been observed.
    pub terminator: Option<u8>,

    /// Send the `;` of an address reply as its own segment, like the real LAN bridge does.
    pub split_ascii: bool,

    /// Drop every n-th reply. Read replies are sometimes simply missing.
    pub drop_every: Option<NonZeroU8>,

    /// Prepend this many junk bytes to the next reply.
    pub garbage_before: u8,

    /// Make every n-th port report carry an impossible port index.
    pub bogus_port_every: Option<NonZeroU8>,

    /// Accept switch commands and do nothing.
    ///
    /// A switch that acknowledges nothing and changes nothing is indistinguishable from a
    /// working one until the port is read back, which is exactly what makes this worth
    /// emulating: it is the only way a verified switch can legitimately fail.
    pub ignore_switch: bool,

    /// Acknowledge a network write with `OK` and store something else.
    ///
    /// The reason network writes are read back before the user is told to power cycle. Models a
    /// field the firmware silently adjusts or refuses.
    pub garble_net_writes: bool,
}

impl Faults {
    /// A perfectly behaved switch. Useful to isolate a failure, unlike real hardware.
    pub const NONE: Self = Self {
        terminator: None,
        split_ascii: false,
        drop_every: None,
        garbage_before: 0,
        bogus_port_every: None,
        ignore_switch: false,
        garble_net_writes: false,
    };

    /// What hardware actually does: address replies arrive in two segments.
    pub const REALISTIC: Self = Self { split_ascii: true, ..Self::NONE };
}

impl Default for Faults {
    fn default() -> Self {
        Self::REALISTIC
    }
}

/// What kind of switch to emulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceConfig {
    /// Which protocol family to speak.
    pub family: Family,
    /// How many inputs to pretend to have.
    pub ports: PortCount,
    /// How many outputs to pretend to have. One, for anything that is not a matrix.
    pub outputs: PortCount,
    /// Which commands to honour. Anything not available is silently ignored.
    pub caps: Capabilities,
    /// Which spelling of ASCII replies to produce.
    pub dialect: AsciiDialect,
    /// How to misbehave.
    pub faults: Faults,
}

impl DeviceConfig {
    /// Emulates a registry entry, including its capability gaps.
    pub fn from_model(model: &ModelInfo) -> Self {
        Self {
            family: model.family,
            ports: model.ports,
            outputs: model.outputs,
            caps: model.caps,
            dialect: AsciiDialect::Captured,
            faults: Faults::REALISTIC,
        }
    }

    /// Replaces the fault configuration.
    #[must_use]
    pub const fn with_faults(mut self, faults: Faults) -> Self {
        self.faults = faults;
        self
    }
}
