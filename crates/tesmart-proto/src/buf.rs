//! Fixed-capacity containers.
//!
//! The crate is `no_std` and allocation free, so the two containers it needs are implemented
//! here rather than pulled in from a dependency: a byte FIFO for the stream decoder and a
//! drop-oldest ring for the engine's event queue.

/// A fixed-capacity byte FIFO.
#[derive(Debug, Clone)]
pub(crate) struct ByteBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> ByteBuf<N> {
    pub(crate) const fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn as_slice(&self) -> &[u8] {
        self.buf.split_at(self.len).0
    }

    /// Appends as much of `data` as fits and returns the number of bytes accepted.
    pub(crate) fn feed(&mut self, data: &[u8]) -> usize {
        let accepted = (N - self.len).min(data.len());
        self.buf[self.len..self.len + accepted].copy_from_slice(&data[..accepted]);
        self.len += accepted;
        accepted
    }

    /// Drops the first `n` bytes, shifting the remainder down.
    pub(crate) fn consume(&mut self, n: usize) {
        let n = n.min(self.len);
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
    }

    pub(crate) const fn clear(&mut self) {
        self.len = 0;
    }
}
