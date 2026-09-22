//! The two codecs and the stream decoder that tells them apart.
//!
//! TESmart switches speak two things on the same connection: six- or seven-byte hex frames
//! (`AA BB 03 01 02 EE`) and bare ASCII strings (`IP?`, `IP:10.0.2.12;`). The LAN port is a
//! serial-to-Ethernet bridge, so both arrive as an unframed stream that may be split, padded
//! with garbage, or interrupted by an unsolicited port report. [`StreamDecoder`] is the part
//! that copes with that; the rest are pure codecs.

use core::fmt;
use core::ops::Deref;

/// The largest *logical* wire unit this crate stores.
///
/// Not the same thing as the largest segment. The LAN bridge delivers about 48 bytes at a time,
/// which is why a reply arrives split and why the `;` often turns up on its own, but the decoder
/// has to buffer a whole unit across however many segments it took to arrive. The limit is
/// therefore set by the longest single thing a switch can say, not by the MTU:
///
/// | Unit | Bytes |
/// | --- | --- |
/// | A hex frame | 7 |
/// | The longest command, `MT8019MA:255.255.255.255;` | 25 |
/// | A padded address reply, `IP:255.255.255.255;` | 19 |
/// | A 16x16 routing reply, `LINK:` + 16 × `OnIm;` + `END` | 88 |
///
/// The routing reply is the one that sets this. A 16x16 matrix reports sixteen pairs in one
/// `LINK:...;END` unit, so anything smaller than 88 bytes would make a 16x16 matrix's only
/// readable command permanently unreadable. 128 leaves room for a firmware that prints the
/// two-character port numbers (`O16I16;`) some documents hint at, which would come to 120.
pub const MAX_WIRE_LEN: usize = 128;

/// A short, owned, copyable run of protocol bytes.
///
/// Owned rather than borrowed so that an async driver can hold it across an `await` without
/// keeping the engine borrowed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WireBytes {
    buf: [u8; MAX_WIRE_LEN],
    len: u8,
}

impl WireBytes {
    /// An empty run.
    pub const fn new() -> Self {
        Self { buf: [0; MAX_WIRE_LEN], len: 0 }
    }

    /// Copies `data`.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::TooLong`] if `data` exceeds [`MAX_WIRE_LEN`].
    pub fn from_slice(data: &[u8]) -> Result<Self, WireError> {
        if data.len() > MAX_WIRE_LEN {
            return Err(WireError::TooLong { len: data.len(), max: MAX_WIRE_LEN });
        }
        let mut bytes = Self::new();
        bytes.buf[..data.len()].copy_from_slice(data);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "data.len() <= MAX_WIRE_LEN, which is far below u8::MAX"
        )]
        {
            bytes.len = data.len() as u8;
        }
        Ok(bytes)
    }

    /// Copies as much of `data` as fits, reporting whether anything was dropped.
    pub fn from_slice_truncating(data: &[u8]) -> (Self, bool) {
        let truncated = data.len() > MAX_WIRE_LEN;
        let keep = data.len().min(MAX_WIRE_LEN);
        let mut bytes = Self::new();
        bytes.buf[..keep].copy_from_slice(&data[..keep]);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "keep <= MAX_WIRE_LEN, which is far below u8::MAX"
        )]
        {
            bytes.len = keep as u8;
        }
        (bytes, truncated)
    }

    /// Copies `data`, which the caller knows statically fits.
    ///
    /// For encodings whose maximum length the protocol itself bounds: a frame is at most 7
    /// bytes, the longest ASCII command 25, the longest reply 88. Use [`Self::from_slice`] for
    /// anything whose length comes from outside.
    ///
    /// # Panics
    ///
    /// Panics if `data` is longer than [`MAX_WIRE_LEN`].
    pub fn from_bounded(data: &[u8]) -> Self {
        Self::from_slice(data).expect("protocol encoding longer than MAX_WIRE_LEN")
    }

    /// The bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }

    /// The bytes as text, if they are valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(self.as_slice()).ok()
    }

    /// Whether there are no bytes.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many bytes there are.
    pub const fn len(&self) -> usize {
        self.len as usize
    }
}

impl Default for WireBytes {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for WireBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for WireBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Hex, because that is how the protocol documents itself: `AA BB 03 01 02 EE`.
impl fmt::Debug for WireBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Hex(self.as_slice()).fmt(f)
    }
}

/// Formats borrowed bytes the way the protocol documents itself: `AA BB 03 01 02 EE`.
///
/// For logging bytes that are not in a [`WireBytes`], such as a read straight off a socket.
///
/// ```
/// use tesmart_proto::wire::Hex;
///
/// assert_eq!(format!("{}", Hex(&[0xAA, 0xBB, 0xEE])), "AA BB EE");
/// assert_eq!(format!("{:?}", Hex(&[])), "");
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Hex<'a>(pub &'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{byte:02X}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
