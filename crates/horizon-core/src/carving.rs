//! Packed carving masks and vanilla's ordered inclusive column segments.
mod ellipsoid;
pub use ellipsoid::{
    CarveError, CarverOutput, can_reach, carve_ellipsoid, carve_ellipsoid_with_budget,
};

/// A contiguous carved interval in a local chunk column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarvingColumn {
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    pub max_y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarvingMaskError {
    InvalidBounds,
    NegativeIndex,
    Capacity,
}
impl std::fmt::Display for CarvingMaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidBounds => "carving mask height must be between 1 and 4064",
            Self::NegativeIndex => "carving mask index is negative",
            Self::Capacity => "carving mask exceeds its bounded bit capacity",
        })
    }
}
impl std::error::Error for CarvingMaskError {}

/// Bits are contiguous in Y, then Z, then X. Coordinates retain Java's wrapping
/// index arithmetic and positive out-of-range aliases. Growth is capped at
/// 2^24 bits; valid chunk coordinates use at most 256 * 4064 bits.
#[derive(Clone, Debug)]
pub struct CarvingMask {
    min_y: i32,
    max_y: i32,
    height: i32,
    words: Vec<u64>,
    used_words: usize,
}
impl CarvingMask {
    pub fn new(min_y: i32, max_y: i32) -> Result<Self, CarvingMaskError> {
        let height = i64::from(max_y) - i64::from(min_y) + 1;
        if !(1..=4064).contains(&height) {
            return Err(CarvingMaskError::InvalidBounds);
        }
        Ok(Self {
            min_y,
            max_y,
            height: height as i32,
            words: vec![0; (height as usize * 256).div_ceil(64)],
            used_words: 0,
        })
    }
    pub fn min_y(&self) -> i32 {
        self.min_y
    }
    pub fn max_y(&self) -> i32 {
        self.max_y
    }
    pub fn is_empty(&self) -> bool {
        self.used_words == 0
    }
    pub fn carve(&mut self, x: i32, y: i32, z: i32) -> Result<(), CarvingMaskError> {
        let index = y
            .wrapping_sub(self.min_y)
            .wrapping_add(z.wrapping_add(x.wrapping_shl(4)).wrapping_mul(self.height));
        if index < 0 {
            return Err(CarvingMaskError::NegativeIndex);
        }
        if index >= 1 << 24 {
            return Err(CarvingMaskError::Capacity);
        }
        let index = index as usize;
        let word = index / 64;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1 << (index & 63);
        self.used_words = self.used_words.max(word + 1);
        Ok(())
    }
    /// Traverse ascending packed runs, splitting each run at column boundaries.
    /// No per-cell or per-column allocation is required.
    pub fn columns(&self) -> impl Iterator<Item = CarvingColumn> + '_ {
        let mut cursor = Cursor::default();
        std::iter::from_fn(move || cursor.next(self))
    }
    /// Like vanilla's callback traversal, changes made by the visitor affect the
    /// next run search, but do not change the already selected run's boundaries.
    /// An error stops traversal immediately and retains visitor mutations.
    pub fn try_visit_mut<E>(
        &mut self,
        mut visitor: impl FnMut(&mut Self, CarvingColumn) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut cursor = Cursor::default();
        while let Some(column) = cursor.next(self) {
            visitor(self, column)?;
        }
        Ok(())
    }
    fn next_set(&self, from: usize) -> Option<usize> {
        let mut word = from / 64;
        if word >= self.used_words {
            return None;
        }
        let mut bits = self.words[word] & (u64::MAX << (from & 63));
        loop {
            if bits != 0 {
                return Some(word * 64 + bits.trailing_zeros() as usize);
            }
            word += 1;
            if word == self.used_words {
                return None;
            }
            bits = self.words[word];
        }
    }
    fn next_clear(&self, from: usize) -> usize {
        let mut word = from / 64;
        let mut bits = !self.words[word] & (u64::MAX << (from & 63));
        loop {
            if bits != 0 {
                return word * 64 + bits.trailing_zeros() as usize;
            }
            word += 1;
            if word == self.used_words {
                return word * 64;
            }
            bits = !self.words[word];
        }
    }
}

#[derive(Default)]
struct Cursor {
    search: usize,
    run: Option<(usize, usize, usize)>,
}
impl Cursor {
    fn next(&mut self, mask: &CarvingMask) -> Option<CarvingColumn> {
        let height = mask.height as usize;
        let (start, end, column) = match self.run {
            Some(run) => run,
            None => {
                let start = mask.next_set(self.search)?;
                let end = mask.next_clear(start) - 1;
                self.search = end + 1;
                (start, end, start / height)
            }
        };
        self.run = (column < end / height).then_some((start, end, column + 1));
        let base = column * height;
        Some(CarvingColumn {
            x: ((column >> 4) & 15) as i32,
            z: (column & 15) as i32,
            min_y: mask.min_y.wrapping_add(start.saturating_sub(base) as i32),
            max_y: mask.min_y.wrapping_add((end - base).min(height - 1) as i32),
        })
    }
}
