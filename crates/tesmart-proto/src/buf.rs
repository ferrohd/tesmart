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

/// A fixed-capacity queue that evicts the oldest item when full.
///
/// Used for events: losing the oldest diagnostic is always better than blocking the decoder or
/// panicking, and the engine counts evictions so the loss is never silent.
#[derive(Debug, Clone)]
pub(crate) struct Ring<T, const N: usize> {
    items: [Option<T>; N],
    head: usize,
    len: usize,
}

impl<T, const N: usize> Ring<T, N> {
    pub(crate) const fn new() -> Self {
        Self { items: [const { None }; N], head: 0, len: 0 }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Pushes `item`, returning `true` if an older item had to be evicted to make room.
    pub(crate) fn push(&mut self, item: T) -> bool {
        let evicted = self.len == N;
        if evicted {
            let _ = self.pop();
        }
        let tail = (self.head + self.len) % N;
        self.items[tail] = Some(item);
        self.len += 1;
        evicted
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.items[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_buf_feeds_and_consumes_fifo() {
        let mut buf = ByteBuf::<4>::new();
        assert_eq!(buf.feed(&[1, 2, 3]), 3);
        assert_eq!(buf.as_slice(), &[1, 2, 3]);

        buf.consume(2);
        assert_eq!(buf.as_slice(), &[3]);

        // only what fits is accepted, the caller keeps the rest
        assert_eq!(buf.feed(&[4, 5, 6, 7]), 3);
        assert_eq!(buf.as_slice(), &[3, 4, 5, 6]);
        assert_eq!(buf.feed(&[7]), 0);
    }

    #[test]
    fn byte_buf_consume_is_saturating() {
        let mut buf = ByteBuf::<4>::new();
        buf.feed(&[1, 2]);
        buf.consume(99);
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn ring_evicts_oldest_when_full() {
        let mut ring = Ring::<u8, 2>::new();
        assert!(!ring.push(1));
        assert!(!ring.push(2));
        assert!(ring.push(3), "pushing into a full ring evicts");
        assert_eq!(ring.pop(), Some(2));
        assert_eq!(ring.pop(), Some(3));
        assert_eq!(ring.pop(), None);
        assert!(ring.is_empty());
    }

    #[test]
    fn ring_wraps_without_losing_order() {
        let mut ring = Ring::<u8, 3>::new();
        for n in 0..9 {
            ring.push(n);
            assert_eq!(ring.pop(), Some(n));
        }
        assert_eq!(ring.len(), 0);
        assert!(ring.is_empty());
    }
}
