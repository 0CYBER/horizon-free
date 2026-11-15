//! The allocation budget of one payload value, charged by the same rules when a value is read
//! and when it is written: a list by its entry count times its entry size, an owned string by
//! its length, a box by the size of what it holds. A writer charges what the peer's reader
//! will, so a body it sends is never refused for its allocation.
//!
//! Sizes are those of a 64-bit target, whatever the target running the codec: a size here is
//! scaled by `8 / size_of::<usize>()`, exact on a 64-bit target and at most twice the 64-bit size
//! on a 32-bit one (a pointer-wide field doubles, nothing grows more). So a wasm32 sender cannot
//! pass a body a native reader refuses; a wasm32 reader is at most that much stricter.

/// What is left of one value's allocation budget.
#[derive(Debug)]
pub(crate) struct Budget {
    left: u64,
}

impl Budget {
    pub(crate) fn new(bytes: u64) -> Self {
        Self { left: bytes }
    }
    /// A list of `count` entries of `T`, before it is reserved.
    pub(crate) fn list<T>(&mut self, count: usize) -> bool {
        self.take((count as u64).checked_mul(wide_size::<T>()))
    }
    /// An owned string of `len` bytes.
    pub(crate) fn string(&mut self, len: usize) -> bool {
        self.take(Some(len as u64))
    }
    /// A box holding a `T`.
    pub(crate) fn boxed<T>(&mut self) -> bool {
        self.take(Some(wide_size::<T>()))
    }
    fn take(&mut self, bytes: Option<u64>) -> bool {
        match bytes.and_then(|bytes| self.left.checked_sub(bytes)) {
            Some(left) => {
                self.left = left;
                true
            }
            None => false,
        }
    }
}

/// `T`'s size as a 64-bit target counts it.
fn wide_size<T>() -> u64 {
    widened(std::mem::size_of::<T>(), std::mem::size_of::<usize>())
}

/// A size measured on a target with `pointer`-byte pointers, as a 64-bit target counts it.
pub(crate) const fn widened(size: usize, pointer: usize) -> u64 {
    size as u64 * (8 / pointer as u64)
}

#[cfg(test)]
#[path = "payload_budget_tests.rs"]
mod tests;
