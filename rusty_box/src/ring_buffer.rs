//! Fixed-capacity ring buffer for no-alloc FIFO queues.
//!
//! Replaces `VecDeque<T>` in iodev modules where capacity is known at compile time.

/// Fixed-capacity ring buffer backed by an inline array.
///
/// Push/pop are O(1). When full, `push_back` drops the oldest element.
pub struct RingBuffer<T, const N: usize> {
    buf: [T; N],
    /// Index of the first (oldest) element, or arbitrary when empty.
    head: usize,
    /// Number of elements currently stored.
    len: usize,
}

impl<T: Default + Copy, const N: usize> RingBuffer<T, N> {
    /// Create an empty ring buffer with all slots default-initialized.
    pub fn new() -> Self {
        Self {
            buf: [T::default(); N],
            head: 0,
            len: 0,
        }
    }
}

impl<T: Default + Copy, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    /// Number of elements in the buffer.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True if the buffer contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Push an element to the back. If full, the oldest element is silently dropped.
    #[inline]
    pub fn push_back(&mut self, val: T) {
        let tail = (self.head + self.len) % N;
        self.buf[tail] = val;
        if self.len == N {
            // Overwrite oldest — advance head
            self.head = (self.head + 1) % N;
        } else {
            self.len += 1;
        }
    }

    /// Remove and return the oldest element, or `None` if empty.
    #[inline]
    pub fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let val = self.buf[self.head];
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(val)
    }

    /// Remove all elements.
    #[inline]
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Drain all elements, returning an iterator that yields them front-to-back.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T, N> {
        let d = Drain {
            buf: &self.buf,
            head: self.head,
            remaining: self.len,
        };
        self.head = 0;
        self.len = 0;
        d
    }

    /// Iterate over elements front-to-back without removing them.
    #[inline]
    pub fn iter(&self) -> Iter<'_, T, N> {
        Iter {
            buf: &self.buf,
            head: self.head,
            remaining: self.len,
        }
    }
}

/// Draining iterator — yields all elements and empties the buffer.
pub struct Drain<'a, T, const N: usize> {
    buf: &'a [T; N],
    head: usize,
    remaining: usize,
}

impl<T: Copy, const N: usize> Iterator for Drain<'_, T, N> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        if self.remaining == 0 {
            return None;
        }
        let val = self.buf[self.head];
        self.head = (self.head + 1) % N;
        self.remaining -= 1;
        Some(val)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T: Copy, const N: usize> ExactSizeIterator for Drain<'_, T, N> {}

/// Non-consuming iterator over elements front-to-back.
pub struct Iter<'a, T, const N: usize> {
    buf: &'a [T; N],
    head: usize,
    remaining: usize,
}

impl<'a, T: Copy, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        if self.remaining == 0 {
            return None;
        }
        let val = self.buf[self.head];
        self.head = (self.head + 1) % N;
        self.remaining -= 1;
        Some(val)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T: Copy, const N: usize> ExactSizeIterator for Iter<'_, T, N> {}

impl<T: Copy + core::fmt::Debug, const N: usize> core::fmt::Debug for RingBuffer<T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_basic() {
        let mut rb = RingBuffer::<u8, 4>::new();
        assert!(rb.is_empty());
        rb.push_back(1);
        rb.push_back(2);
        rb.push_back(3);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.pop_front(), Some(1));
        assert_eq!(rb.pop_front(), Some(2));
        assert_eq!(rb.pop_front(), Some(3));
        assert_eq!(rb.pop_front(), None);
    }

    #[test]
    fn overflow_drops_oldest() {
        let mut rb = RingBuffer::<u8, 3>::new();
        rb.push_back(1);
        rb.push_back(2);
        rb.push_back(3);
        rb.push_back(4); // drops 1
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.pop_front(), Some(2));
        assert_eq!(rb.pop_front(), Some(3));
        assert_eq!(rb.pop_front(), Some(4));
    }

    #[test]
    fn drain_yields_all() {
        let mut rb = RingBuffer::<u8, 4>::new();
        rb.push_back(10);
        rb.push_back(20);
        rb.push_back(30);
        let v: Vec<u8> = rb.drain().collect();
        assert_eq!(v, vec![10, 20, 30]);
        assert!(rb.is_empty());
    }

    #[test]
    fn clear_empties() {
        let mut rb = RingBuffer::<u8, 4>::new();
        rb.push_back(1);
        rb.push_back(2);
        rb.clear();
        assert!(rb.is_empty());
        assert_eq!(rb.pop_front(), None);
    }

    #[test]
    fn wraparound() {
        let mut rb = RingBuffer::<u8, 3>::new();
        rb.push_back(1);
        rb.push_back(2);
        rb.pop_front(); // head=1
        rb.push_back(3);
        rb.push_back(4); // wraps around
        assert_eq!(rb.pop_front(), Some(2));
        assert_eq!(rb.pop_front(), Some(3));
        assert_eq!(rb.pop_front(), Some(4));
        assert_eq!(rb.pop_front(), None);
    }
}
