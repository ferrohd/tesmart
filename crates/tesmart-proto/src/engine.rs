//! The sans-IO request/response engine.
//!
//! One [`Engine`] drives one switch. It owns everything about the conversation except the bytes
//! moving: encoding, demultiplexing, command spacing, reply deadlines and retries. The caller
//! owns the socket and the clock.

use core::fmt;
use core::mem;
use core::num::NonZeroU32;
use core::time::Duration;

use crate::buf::Ring;
use crate::command::{Request, RequestKind, Response, ResponseKind};
use crate::port::Port;
use crate::profile::{Profile, Unsupported};
use crate::time::Timestamp;
use crate::wire::frame::OPCODE_PORT_REPORT;
use crate::wire::{
    AsciiDialect, AsciiScope, AsciiStyle, Chunk, Frame, StreamDecoder, Transmit, TransmitKind,
    WireBytes, WireError,
};

/// How many events are buffered before the oldest is dropped.
///
/// A conversation produces at most a handful of events per input chunk; a flood means the switch
/// is pushing unsolicited reports faster than the caller drains them, in which case the newest
/// state is the one worth keeping. Drops are counted by
/// [`dropped_events`](Engine::dropped_events).
const EVENT_CAPACITY: usize = 8;

/// Identifies one request/reply exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExchangeId(NonZeroU32);

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}
