//! Resident `PalettedContainer` of Java Edition 26.3: single-value, linear and
//! hash-map palettes with vanilla's bit-width thresholds, and the global
//! palette beyond them, over `SimpleBitStorage` words (values never span
//! longs). One container type serves blocks (16^3 entries) and biomes (4^3).
//!
//! Growth follows `PalettedContainer.onResize`: a full palette asks for one
//! more bit, the new configuration is built by re-adding the distinct values
//! in index order, and the value that did not fit is added last. Palettes
//! never shrink while resident; `pack` writes the canonical saved form.

use crate::nbt::Tag;
use crate::storage::{locate, mask};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `Strategy.createForBlockStates`: 4096 entries, linear palettes up to 4
    /// bits and hash-map palettes from 5 to 8.
    Blocks,
    /// `Strategy.createForBiomes`: 64 entries, linear palettes up to 3 bits.
    Biomes,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strategy {
    pub kind: Kind,
    /// `ceillog2` of the registry size: the width of the global palette.
    pub global_bits: u8,
}
/// `getConfigurationForBitCount`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Config {
    Single,
    /// Palette of up to `1 << bits` entries, stored in `bits` bits.
    List(u8),
    Global,
}
impl Strategy {
    pub const fn blocks(global_bits: u8) -> Self {
        Self {
            kind: Kind::Blocks,
            global_bits,
        }
    }
    pub const fn biomes(global_bits: u8) -> Self {
        Self {
            kind: Kind::Biomes,
            global_bits,
        }
    }
    pub const fn entries(self) -> usize {
        match self.kind {
            Kind::Blocks => 4096,
            Kind::Biomes => 64,
        }
    }
    fn config(self, bits: u8) -> Config {
        match (self.kind, bits) {
            (_, 0) => Config::Single,
            (Kind::Blocks, 1..=4) => Config::List(4),
            (Kind::Blocks, 5..=8) => Config::List(bits),
            (Kind::Biomes, 1..=3) => Config::List(bits),
            _ => Config::Global,
        }
    }
    /// Whether a container written with leading byte `bits` lists its palette
    /// (all but the global one).
    pub fn wire_has_palette(self, bits: u8) -> bool {
        self.config(bits) != Config::Global
    }
    /// Bits of the storage a configuration keeps in memory.
    fn storage_bits(self, config: Config) -> u8 {
        match config {
            Config::Single => 0,
            Config::List(bits) => bits,
            Config::Global => self.global_bits,
        }
    }
}
/// `Mth.ceillog2`.
pub fn ceil_log2(n: usize) -> u8 {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as u8
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteError {
    /// A palette bigger than its configuration allows, or empty.
    BadPalette { len: usize },
    /// Storage with the wrong number of longs.
    BadStorage { expected: usize, actual: usize },
    /// A stored index past the palette.
    BadIndex { index: u32, palette: usize },
    /// The declared bit count does not match the palette.
    BadBits { declared: u8, expected: u8 },
}
impl std::fmt::Display for PaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PaletteError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Palette {
    Single(u32),
    /// Linear and hash-map palettes alike: ids in insertion order.
    List(Vec<u32>),
    Global,
}
/// Words of `bits`-wide values: `entries.div_ceil(64 / bits)`.
pub fn storage_len(bits: u8, entries: usize) -> usize {
    if bits == 0 {
        0
    } else {
        entries.div_ceil(64 / usize::from(bits))
    }
}
fn read(words: &[u64], bits: u8, i: usize) -> u32 {
    let (word, shift) = locate(bits, i);
    ((words[word] >> shift) & mask(bits)) as u32
}
fn write(words: &mut [u64], bits: u8, i: usize, value: u32) {
    let (word, shift) = locate(bits, i);
    words[word] = (words[word] & !(mask(bits) << shift)) | (u64::from(value) << shift);
}
/// `word + add`, bit by bit: which bits carried out of their position. The
/// carry out of bit `i` is `a & b | (a | b) & !sum` for `sum = a + b`.
#[inline]
fn carry_out(word: u64, add: u64, sum: u64) -> u64 {
    (word & add) | ((word | add) & !sum)
}
/// The first of `count` `bits`-wide fields of `word` that is `len` or more.
fn first_bad(word: u64, count: usize, bits: usize, len: usize) -> PaletteError {
    for slot in 0..count {
        let index = ((word >> (slot * bits)) & ((1 << bits) - 1)) as u32;
        if index as usize >= len {
            return PaletteError::BadIndex {
                index,
                palette: len,
            };
        }
    }
    unreachable!("check_indices found no field past the palette")
}
/// Values below this are looked up in a table indexed by value once a
/// palette outgrows the scan: every block state and biome id of 26.3.
const DENSE_VALUES: u32 = 1 << 16;
thread_local! {
    /// `palette_ids`' table: 1 + the id of each value seen, 0 for the others.
    /// Only the entries of the palette being built are set, and they are
    /// cleared again before it returns.
    static DENSE_IDS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// The distinct `values` in first-occurrence order and each value's index in
/// them. A run of one value (most of a section) is matched once, a scan finds
/// up to 16 distinct values faster than any lookup, and past that a table
/// indexed by value finds them without hashing.
fn palette_ids(values: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut palette: Vec<u32> = Vec::new();
    let mut ids = Vec::with_capacity(values.len());
    let mut last: Option<(u32, u32)> = None;
    let mut rest = values;
    while let Some((&value, tail)) = rest.split_first() {
        let id = match last {
            Some((v, id)) if v == value => id,
            _ if palette.len() > 16 => break,
            _ => match palette.iter().position(|&v| v == value) {
                Some(id) => id as u32,
                None => {
                    palette.push(value);
                    palette.len() as u32 - 1
                }
            },
        };
        last = Some((value, id));
        ids.push(id);
        rest = tail;
    }
    if rest.is_empty() {
        return (palette, ids);
    }
    if palette.iter().chain(rest).all(|&v| v < DENSE_VALUES) {
        DENSE_IDS.with(|table| {
            let mut table = table.borrow_mut();
            for (id, &v) in palette.iter().enumerate() {
                let v = v as usize;
                if table.len() <= v {
                    table.resize(v + 1, 0);
                }
                table[v] = id as u32 + 1;
            }
            for &value in rest {
                let v = value as usize;
                if table.len() <= v {
                    table.resize(v + 1, 0);
                }
                let id = match table[v] {
                    0 => {
                        palette.push(value);
                        table[v] = palette.len() as u32;
                        palette.len() as u32 - 1
                    }
                    slot => slot - 1,
                };
                ids.push(id);
            }
            for &v in &palette {
                table[v as usize] = 0;
            }
        });
    } else {
        let mut lookup: std::collections::HashMap<u32, u32> = palette
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, i as u32))
            .collect();
        for &value in rest {
            let id = *lookup.entry(value).or_insert_with(|| {
                palette.push(value);
                palette.len() as u32 - 1
            });
            ids.push(id);
        }
    }
    (palette, ids)
}
/// Words holding `ids` in `bits`-wide slots.
fn pack_words(bits: u8, entries: usize, ids: impl Iterator<Item = u32>) -> Vec<u64> {
    let mut words = vec![0u64; storage_len(bits, entries)];
    for (i, id) in ids.enumerate() {
        write(&mut words, bits, i, id);
    }
    words
}
/// The saved form: distinct values in index order, and the packed indices
/// when there is more than one value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    pub palette: Vec<u32>,
    /// Bits per index in `data` (0 when `data` is absent).
    pub bits: u8,
    pub data: Option<Vec<u64>>,
}

/// A container's identity for a reader that remembers what it read: `uid` is unique among every
/// container ever made (a clone, which is copy-on-write's way of editing a shared one, gets a new
/// one), `edits` counts the `set` calls of this container since it was made. A reader that kept
/// `(uid, edits)` and finds the same pair later knows the entries are the ones it read.
#[derive(Debug)]
struct Identity {
    uid: u64,
    edits: u32,
}
impl Default for Identity {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            uid: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            edits: 0,
        }
    }
}
impl Clone for Identity {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Byte budget for cached canonical section palettes held by resident
/// [`PalettedContainer`]s, shared by every save folder that holds the `Arc`.
#[derive(Debug)]
pub struct SavedPaletteBudget {
    limit: usize,
    used: AtomicUsize,
}

impl SavedPaletteBudget {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
        }
    }

    /// Bytes currently reserved by live cached section palettes.
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Maximum bytes this budget admits.
    pub fn limit(&self) -> usize {
        self.limit
    }

    fn reserve(&self, charge: usize) -> bool {
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = used.checked_add(charge).filter(|&next| next <= self.limit) else {
                return false;
            };
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return true,
                Err(actual) => used = actual,
            }
        }
    }
}

#[derive(Debug)]
struct SavedNbt {
    budget: Arc<SavedPaletteBudget>,
    charge: usize,
    ids: Box<[u32]>,
    words: Box<[i64]>,
}

impl Drop for SavedNbt {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.charge, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
pub struct PalettedContainer {
    strategy: Strategy,
    bits: u8,
    palette: Palette,
    words: Vec<u64>,
    /// Which container this is, and how many writes it has had (see [`Identity`]).
    identity: Identity,
    saved_nbt: OnceLock<Arc<SavedNbt>>,
}
impl PalettedContainer {
    /// `(uid, edits)`: equal to an earlier answer only if this is the same container and none of
    /// its entries was written since (`edits` wraps after 2^32 writes; a reader that keeps a
    /// stamp for that long is not one this serves).
    #[inline]
    pub fn stamp(&self) -> (u64, u32) {
        (self.identity.uid, self.identity.edits)
    }
    /// `new PalettedContainer(initialValue, strategy)`.
    pub fn new(strategy: Strategy, value: u32) -> Self {
        Self {
            strategy,
            bits: 0,
            palette: Palette::Single(value),
            words: Vec::new(),
            identity: Identity::default(),
            saved_nbt: OnceLock::new(),
        }
    }
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }
    /// Bits per stored index (`BitStorage.getBits`).
    pub fn bits(&self) -> u8 {
        self.bits
    }
    /// What the stored ids stand for — the values of the list palette, `None`
    /// for a single value (`uniform()`) and for the global palette, whose
    /// stored ids are the values.
    pub fn palette(&self) -> Option<&[u32]> {
        match &self.palette {
            Palette::List(list) => Some(list),
            _ => None,
        }
    }
    /// Entries in the palette (the global palette reports its bit capacity).
    #[inline]
    pub fn palette_len(&self) -> usize {
        match &self.palette {
            Palette::Single(_) => 1,
            Palette::List(list) => list.len(),
            Palette::Global => 1 << self.bits.min(31),
        }
    }
    #[inline]
    pub fn is_global(&self) -> bool {
        matches!(self.palette, Palette::Global)
    }
    #[inline]
    fn raw(&self, i: usize) -> usize {
        let (word, shift) = locate(self.bits, i);
        ((self.words[word] >> shift) & mask(self.bits)) as usize
    }
    #[inline]
    pub fn get(&self, i: usize) -> u32 {
        match &self.palette {
            Palette::Single(value) => *value,
            Palette::List(list) => list[self.raw(i)],
            Palette::Global => self.raw(i) as u32,
        }
    }
    /// `get` of the entries `start..start + out.len()`, the palette matched once.
    #[inline]
    pub fn get_run(&self, start: usize, out: &mut [u32]) {
        match &self.palette {
            Palette::Single(value) => {
                out.fill(*value);
                return;
            }
            Palette::List(list) if self.bits == 4 && out.len() == 16 && (start & 15) == 0 => {
                let word = self.words[start >> 4];
                for (k, cell) in out.iter_mut().enumerate() {
                    *cell = list[((word >> (k * 4)) & 15) as usize];
                }
                return;
            }
            _ => {}
        }
        if out.len() == 2 && !matches!(self.palette, Palette::Single(_)) {
            let bits = usize::from(self.bits);
            let per = 64 / bits;
            let slot = start % per;
            if slot + 1 < per {
                let word = self.words[start / per] >> (slot * bits);
                let mask = mask(self.bits);
                let a = (word & mask) as usize;
                let b = ((word >> bits) & mask) as usize;
                match &self.palette {
                    Palette::List(list) => {
                        out[0] = list[a];
                        out[1] = list[b];
                    }
                    Palette::Global => {
                        out[0] = a as u32;
                        out[1] = b as u32;
                    }
                    Palette::Single(_) => unreachable!(),
                }
                return;
            }
        }
        match &self.palette {
            Palette::Single(value) => out.fill(*value),
            Palette::List(list) => {
                for (k, cell) in out.iter_mut().enumerate() {
                    *cell = list[self.raw(start + k)];
                }
            }
            Palette::Global => {
                for (k, cell) in out.iter_mut().enumerate() {
                    *cell = self.raw(start + k) as u32;
                }
            }
        }
    }
    /// The value when every entry is equal, without the palette having to be
    /// minimal.
    pub fn uniform(&self) -> Option<u32> {
        match &self.palette {
            Palette::Single(value) => Some(*value),
            Palette::List(list) if list.len() == 1 => Some(list[0]),
            Palette::Global => self.all_equal(self.get(0)).then(|| self.get(0)),
            Palette::List(list) => {
                let first = list[self.raw(0)];
                self.all_equal(first).then_some(first)
            }
        }
    }
    /// Whether `other` is stored the same way (bits, palette and words): then
    /// it holds the same values, found without unpacking either. (Containers
    /// stored differently can still hold the same values.)
    pub fn same_storage(&self, other: &Self) -> bool {
        self.bits == other.bits && self.palette == other.palette && self.words == other.words
    }
    /// Whether all entries equal `value`.
    pub fn is_uniform(&self, value: u32) -> bool {
        match &self.palette {
            Palette::Single(v) => *v == value,
            Palette::List(list) => match list.iter().position(|&v| v == value) {
                None => false,
                Some(_) if list.len() == 1 => true,
                Some(_) => self.all_equal(value),
            },
            Palette::Global => self.all_equal(value),
        }
    }
    fn all_equal(&self, value: u32) -> bool {
        let id = match &self.palette {
            Palette::Single(v) => return *v == value,
            Palette::List(list) => match list.iter().position(|&v| v == value) {
                Some(id) => id as u64,
                None => return false,
            },
            Palette::Global => u64::from(value),
        };
        let bits = usize::from(self.bits);
        let per = 64 / bits;
        let pattern = (0..per).fold(0u64, |word, slot| word | id << (slot * bits));
        let entries = self.strategy.entries();
        let full = entries / per;
        if self.words[..full].iter().any(|&w| w != pattern) {
            return false;
        }
        let tail = entries - full * per;
        tail == 0 || {
            let used = (1u64 << (tail * bits)) - 1;
            self.words[full] & used == pattern & used
        }
    }
    /// `getAndSet`: stores `value` and returns the previous one.
    pub fn set(&mut self, i: usize, value: u32) -> u32 {
        self.identity.edits = self.identity.edits.wrapping_add(1);
        if self.saved_nbt.get().is_some() && self.get(i) != value {
            self.saved_nbt.take();
        }
        let id = self.id_for(value);
        match &self.palette {
            Palette::Single(old) => *old,
            Palette::List(list) => {
                let old = read(&self.words, self.bits, i) as usize;
                write(&mut self.words, self.bits, i, id as u32);
                list[old]
            }
            Palette::Global => {
                let old = read(&self.words, self.bits, i);
                write(&mut self.words, self.bits, i, id as u32);
                old
            }
        }
    }
    /// `Palette.idFor`, growing the container when the palette is full.
    fn id_for(&mut self, value: u32) -> usize {
        match &mut self.palette {
            Palette::Single(v) if *v == value => 0,
            Palette::Single(_) => self.resize(1, value),
            Palette::List(list) => {
                if let Some(id) = list.iter().position(|&v| v == value) {
                    id
                } else if list.len() < 1 << self.bits {
                    list.push(value);
                    list.len() - 1
                } else {
                    self.resize(self.bits + 1, value)
                }
            }
            Palette::Global => value as usize,
        }
    }
    /// `PalettedContainer.onResize`: rebuild with the configuration for
    /// `bits`, then add `last` and return its id.
    fn resize(&mut self, bits: u8, last: u32) -> usize {
        let strategy = self.strategy;
        let config = strategy.config(bits);
        let storage_bits = strategy.storage_bits(config);
        let entries = strategy.entries();
        let mut palette = match config {
            Config::Single => Palette::Single(last),
            Config::List(_) => Palette::List(Vec::new()),
            Config::Global => Palette::Global,
        };
        let mut words = vec![0u64; storage_len(storage_bits, entries)];
        let id_in = |palette: &mut Palette, value: u32| -> u32 {
            match palette {
                Palette::Single(_) => 0,
                Palette::List(list) => match list.iter().position(|&v| v == value) {
                    Some(id) => id as u32,
                    None => {
                        list.push(value);
                        list.len() as u32 - 1
                    }
                },
                Palette::Global => value,
            }
        };
        if let Palette::Single(only) = self.palette {
            let id = id_in(&mut palette, only);
            if storage_bits > 0 && id != 0 {
                for i in 0..entries {
                    write(&mut words, storage_bits, i, id);
                }
            }
        } else {
            let (mut last_old, mut new_id) = (u32::MAX, 0);
            for i in 0..entries {
                let old = read(&self.words, self.bits, i);
                if old != last_old {
                    last_old = old;
                    let value = match &self.palette {
                        Palette::List(list) => list[old as usize],
                        _ => old,
                    };
                    new_id = id_in(&mut palette, value);
                }
                if storage_bits > 0 {
                    write(&mut words, storage_bits, i, new_id);
                }
            }
        }
        let id = id_in(&mut palette, last);
        self.palette = palette;
        self.bits = storage_bits;
        self.words = words;
        id as usize
    }
    /// All values in index order.
    pub fn unpack(&self, out: &mut [u32]) {
        assert_eq!(out.len(), self.strategy.entries());
        match &self.palette {
            Palette::Single(value) => out.fill(*value),
            Palette::List(list) if self.bits == 4 => {
                for (row, &word) in out.as_chunks_mut::<16>().0.iter_mut().zip(&self.words) {
                    for (x, cell) in row.iter_mut().enumerate() {
                        *cell = list[((word >> (x * 4)) & 15) as usize];
                    }
                }
            }
            Palette::List(list) => {
                let bits = usize::from(self.bits);
                let per = 64 / bits;
                let m = mask(self.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        let id = ((word >> (slot * bits)) & m) as usize;
                        *cell = list[id];
                    }
                }
            }
            Palette::Global => {
                let bits = usize::from(self.bits);
                let per = 64 / bits;
                let m = mask(self.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        *cell = ((word >> (slot * bits)) & m) as u32;
                    }
                }
            }
        }
    }
    /// The stored ids through `map`: `out[i] = map[id of entry i]`, in index
    /// order, the id being a palette id for a list palette and the value
    /// itself for the global one (see `palette`). A single-value palette
    /// stores no ids: `out` is left as it is.
    pub fn unpack_ids_mapped<T: Copy>(&self, out: &mut [T], map: &[T]) {
        assert_eq!(out.len(), self.strategy.entries());
        // Four bits a linear palette's usual width: a whole word of them is a
        // row of 16, and immediate shifts for the compiler to unroll.
        if self.bits == 4 {
            for (row, &word) in out.as_chunks_mut::<16>().0.iter_mut().zip(&self.words) {
                for (x, cell) in row.iter_mut().enumerate() {
                    *cell = map[((word >> (x * 4)) & 0xF) as usize];
                }
            }
            return;
        }
        if self.bits == 0 {
            return;
        }
        let bits = usize::from(self.bits);
        let per = 64 / bits;
        let m = mask(self.bits);
        for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
            let mut shift = 0;
            for cell in chunk {
                *cell = map[((word >> shift) & m) as usize];
                shift += bits;
            }
        }
    }
    /// The palette id of every entry in index order, and the list palette
    /// they index (`get(i)` is `list[ids[i]]`), without looking the values
    /// up; `None` (and `ids` untouched) for a single-value or global palette.
    pub fn unpack_ids(&self, ids: &mut [u16]) -> Option<&[u32]> {
        let Palette::List(list) = &self.palette else {
            return None;
        };
        assert_eq!(ids.len(), self.strategy.entries());
        let bits = usize::from(self.bits);
        let per = 64 / bits;
        let m = mask(self.bits);
        for (chunk, &word) in ids.chunks_mut(per).zip(&self.words) {
            for (slot, cell) in chunk.iter_mut().enumerate() {
                // List palettes stay at a few bits (eight at most for blocks).
                *cell = ((word >> (slot * bits)) & m) as u16;
            }
        }
        Some(list)
    }
    /// A container holding `values` in the smallest configuration for their
    /// distinct count, with palette ids in first-occurrence order, like a
    /// container unpacked from disk.
    pub fn from_values(strategy: Strategy, values: &[u32]) -> Self {
        assert_eq!(values.len(), strategy.entries());
        let (palette, ids) = palette_ids(values);
        Self::build(strategy, palette, &ids)
    }
    /// A container over a distinct-value `palette` and an id per entry.
    fn build(strategy: Strategy, palette: Vec<u32>, ids: &[u32]) -> Self {
        let config = strategy.config(ceil_log2(palette.len()));
        match config {
            Config::Single => Self::new(strategy, palette[0]),
            Config::List(bits) => Self {
                strategy,
                bits,
                words: pack_words(bits, ids.len(), ids.iter().copied()),
                identity: Identity::default(),
                palette: Palette::List(palette),
                saved_nbt: OnceLock::new(),
            },
            Config::Global => {
                let bits = strategy.global_bits;
                Self {
                    strategy,
                    bits,
                    words: pack_words(bits, ids.len(), ids.iter().map(|&id| palette[id as usize])),
                    identity: Identity::default(),
                    palette: Palette::Global,
                    saved_nbt: OnceLock::new(),
                }
            }
        }
    }
    /// `PalettedContainer.read`: the container as sent on the wire. `bits` is
    /// the leading byte, `palette` the VarInt entries (a single one for
    /// `bits == 0`, none for the global palette) and `words` the longs.
    pub fn from_wire(
        strategy: Strategy,
        bits: u8,
        palette: Vec<u32>,
        words: Vec<u64>,
    ) -> Result<Self, PaletteError> {
        let config = strategy.config(bits);
        let storage_bits = strategy.storage_bits(config);
        let expected = storage_len(storage_bits, strategy.entries());
        if words.len() != expected {
            return Err(PaletteError::BadStorage {
                expected,
                actual: words.len(),
            });
        }
        match config {
            Config::Single => {
                if palette.len() != 1 {
                    return Err(PaletteError::BadPalette { len: palette.len() });
                }
                Ok(Self::new(strategy, palette[0]))
            }
            Config::List(bits) => {
                if palette.is_empty() || palette.len() > 1 << bits {
                    return Err(PaletteError::BadPalette { len: palette.len() });
                }
                let container = Self {
                    strategy,
                    bits,
                    palette: Palette::List(palette),
                    words,
                    identity: Identity::default(),
                    saved_nbt: OnceLock::new(),
                };
                container.check_indices()?;
                Ok(container)
            }
            Config::Global => Ok(Self {
                strategy,
                bits: storage_bits,
                palette: Palette::Global,
                words,
                identity: Identity::default(),
                saved_nbt: OnceLock::new(),
            }),
        }
    }
    /// Rejects a stored index at or past the palette: the same inputs are
    /// accepted as an entry-by-entry scan accepts, and the same first offending
    /// entry is named. One whole word is checked at once: adding `2^bits - len`
    /// to a word lifts every field by that much, so a field carries out of its
    /// own bits exactly when its index is past the palette. A carry out of a
    /// field that a previous field's carry reached adds one more, so such a
    /// field can only look bad where a genuinely bad field precedes it in the
    /// same word; the word is then rescanned entry by entry for the error.
    fn check_indices(&self) -> Result<(), PaletteError> {
        let Palette::List(list) = &self.palette else {
            return Ok(());
        };
        let len = list.len();
        debug_assert!(len >= 1);
        debug_assert_eq!(
            self.words.len(),
            storage_len(self.bits, self.strategy.entries())
        );
        let bits = usize::from(self.bits);
        if len == 1 << bits {
            // Every field is a palette entry.
            return Ok(());
        }
        let per = 64 / bits;
        let entries = self.strategy.entries();
        // One in the low bit of each field of a word, the field's top bit, and
        // the bias that overflows a field whose index is `len` or more.
        let ones = (0..per).fold(0u64, |a, field| a | 1 << (field * bits));
        let bias = ones * ((1u64 << bits) - len as u64);
        let top = ones << (bits - 1);
        let full = entries / per;
        let (whole, tail) = self.words.split_at(full);
        for &word in whole {
            let sum = word.wrapping_add(bias);
            if carry_out(word, bias, sum) & top != 0 {
                return Err(first_bad(word, per, bits, len));
            }
        }
        // The last word's fields past `entries` are padding: whatever they
        // hold, the scan never reads them.
        let rest = entries - full * per;
        if rest > 0
            && let Some(&word) = tail.first()
        {
            let used = (1u64 << (rest * bits)) - 1;
            let sum = word.wrapping_add(bias & used);
            if carry_out(word, bias & used, sum) & top & used != 0 {
                return Err(first_bad(word, rest, bits, len));
            }
        }
        Ok(())
    }
    /// `PalettedContainer.write`: the leading bits byte, the palette entries
    /// (none for the global palette) and the raw longs, exactly as resident.
    pub fn to_wire(&self) -> (u8, Vec<u32>, &[u64]) {
        let palette = match &self.palette {
            Palette::Single(value) => vec![*value],
            Palette::List(list) => list.clone(),
            Palette::Global => Vec::new(),
        };
        (self.bits, palette, &self.words)
    }
    /// `PalettedContainer.pack`: distinct values in index order, indices
    /// re-encoded for the saved bit width.
    pub fn pack(&self) -> Packed {
        let entries = self.strategy.entries();
        // One value in every cell (air, stone, a filled section): the packed
        // form is that value and no indices, which `uniform` reads off the
        // stored words. Unpacking the 4,096 cells first to discover the same
        // thing is most of a section's save cost, and most sections of a
        // column hold one value.
        if let Some(value) = self.uniform() {
            return Packed {
                palette: vec![value],
                bits: 0,
                data: None,
            };
        }
        let mut values = vec![0u32; entries];
        self.unpack(&mut values);
        let (palette, ids) = palette_ids(&values);
        let bits = match self.strategy.config(ceil_log2(palette.len())) {
            Config::Single => 0,
            Config::List(bits) => bits,
            Config::Global => ceil_log2(palette.len()),
        };
        let data = (bits != 0).then(|| pack_words(bits, entries, ids.iter().copied()));
        Packed {
            palette,
            bits,
            data,
        }
    }

    fn pack_nbt_fields(
        palette: &[u32],
        data: Option<Vec<i64>>,
        name: impl FnMut(u32) -> Tag,
    ) -> Tag {
        let mut fields = vec![(
            "palette",
            Tag::List(palette.iter().copied().map(name).collect()),
        )];
        if let Some(words) = data {
            fields.push(("data", Tag::LongArray(words)));
        }
        Tag::compound(fields)
    }

    /// Canonical `palette` and `data` NBT compound for a saved section.
    ///
    /// When `budget` is `Some`, a non-uniform container caches its canonical
    /// palette ids and packed `data` words lazily on the first save within the
    /// budget's byte cap. Repeated saves of an unchanged container (or clones
    /// sharing the entry) reuse those words and re-resolve `name` against the
    /// caller's current registry. Mutating a cell via [`Self::set`] drops the
    /// entry and releases its budget reservation when the last owner drops.
    pub fn pack_nbt(
        &self,
        budget: Option<&Arc<SavedPaletteBudget>>,
        name: impl FnMut(u32) -> Tag,
    ) -> Tag {
        if let Some(saved) = self.saved_nbt.get()
            && budget.is_some_and(|budget| Arc::ptr_eq(budget, &saved.budget))
        {
            return Self::pack_nbt_fields(&saved.ids, Some(saved.words.to_vec()), name);
        }
        let Packed { palette, data, .. } = self.pack();
        let data = data.map(|words| {
            words
                .into_iter()
                .map(|word| word as i64)
                .collect::<Vec<_>>()
        });
        if let (Some(budget), Some(words), None) = (budget, data.as_deref(), self.saved_nbt.get()) {
            let charge = std::mem::size_of::<SavedNbt>()
                + 2 * std::mem::size_of::<usize>()
                + std::mem::size_of_val(palette.as_slice())
                + std::mem::size_of_val(words);
            if budget.reserve(charge) {
                let _ = self.saved_nbt.set(Arc::new(SavedNbt {
                    budget: Arc::clone(budget),
                    charge,
                    ids: palette.as_slice().into(),
                    words: words.into(),
                }));
            }
        }
        Self::pack_nbt_fields(&palette, data, name)
    }

    /// `PalettedContainer.unpack`: a container from the saved form.
    pub fn from_packed(
        strategy: Strategy,
        palette: Vec<u32>,
        data: Option<&[u64]>,
    ) -> Result<Self, PaletteError> {
        if palette.is_empty() {
            return Err(PaletteError::BadPalette { len: 0 });
        }
        let config = strategy.config(ceil_log2(palette.len()));
        let entries = strategy.entries();
        let disc_bits = match config {
            Config::Single => return Ok(Self::new(strategy, palette[0])),
            Config::List(bits) => bits,
            Config::Global => ceil_log2(palette.len()),
        };
        let data = data.ok_or(PaletteError::BadStorage {
            expected: storage_len(disc_bits, entries),
            actual: 0,
        })?;
        let expected = storage_len(disc_bits, entries);
        if data.len() != expected {
            return Err(PaletteError::BadStorage {
                expected,
                actual: data.len(),
            });
        }
        if let Config::List(bits) = config {
            // List palettes already use the saved indices as resident storage.
            // Validate them without allocating an unpacked copy that is unused.
            let container = Self {
                strategy,
                bits,
                words: data.to_vec(),
                identity: Identity::default(),
                palette: Palette::List(palette),
                saved_nbt: OnceLock::new(),
            };
            container.check_indices()?;
            return Ok(container);
        }
        let mut ids = Vec::with_capacity(entries);
        for i in 0..entries {
            let index = read(data, disc_bits, i);
            if index as usize >= palette.len() {
                return Err(PaletteError::BadIndex {
                    index,
                    palette: palette.len(),
                });
            }
            ids.push(index);
        }
        Ok(Self::build(strategy, palette, &ids))
    }
    /// Whether any palette entry satisfies `predicate` (`maybeHas`): entries
    /// no longer used by any position still count.
    pub fn maybe_has(&self, mut predicate: impl FnMut(u32) -> bool) -> bool {
        match &self.palette {
            Palette::Single(value) => predicate(*value),
            Palette::List(list) => list.iter().any(|&v| predicate(v)),
            Palette::Global => (0..1u32 << self.bits.min(31)).any(predicate),
        }
    }
    /// Whether any entry satisfies `predicate`. Unlike [`Self::maybe_has`],
    /// palette entries no entry uses any more don't count.
    pub fn has(&self, mut predicate: impl FnMut(u32) -> bool) -> bool {
        let list = match &self.palette {
            Palette::Single(value) => return predicate(*value),
            Palette::List(list) => list,
            Palette::Global => {
                return (0..self.strategy.entries()).any(|i| predicate(self.raw(i) as u32));
            }
        };
        let matching: Vec<bool> = list.iter().map(|&v| predicate(v)).collect();
        let count = matching.iter().filter(|&&m| m).count();
        if count == 0 || count == list.len() {
            return count != 0;
        }
        let (bits, entries) = (usize::from(self.bits), self.strategy.entries());
        let per = 64 / bits;
        // The words whose fields all hold entries; the last may be partial.
        let full = entries / per;
        if count <= 4 {
            // A field-wise zero test of each word XORed with the matching
            // index in every field: `(v - lo) & !v & hi` is non-zero exactly
            // when some field of `v` is zero.
            let lo = (0..per).fold(0u64, |a, f| a | 1 << (f * bits));
            let hi = lo << (bits - 1);
            let words = &self.words[..full];
            let found = matching
                .iter()
                .enumerate()
                .filter(|(_, m)| **m)
                .any(|(i, _)| {
                    let pattern = lo * i as u64;
                    words.iter().any(|&w| {
                        let v = w ^ pattern;
                        v.wrapping_sub(lo) & !v & hi != 0
                    })
                });
            return found || (full * per..entries).any(|i| matching[self.raw(i)]);
        }
        (0..entries).any(|i| matching[self.raw(i)])
    }
    /// Heap bytes held by the palette, the storage, and this container's share
    /// of any cached canonical saved palette: clones share one entry, and each
    /// counts its charge divided by the number of containers holding it, so
    /// summing them counts the entry once.
    pub fn heap_bytes(&self) -> usize {
        self.storage_heap_bytes()
            + self
                .saved_nbt
                .get()
                .map_or(0, |saved| saved.charge / Arc::strong_count(saved))
    }
    /// Heap bytes held by the palette and the storage.
    fn storage_heap_bytes(&self) -> usize {
        self.words.capacity() * 8
            + match &self.palette {
                Palette::List(list) => list.capacity() * 4,
                _ => 0,
            }
    }
}
impl PartialEq for PalettedContainer {
    /// Equal when every entry is. The same width, palette and words are the
    /// same entries, whatever they are: compared without reading each.
    fn eq(&self, other: &Self) -> bool {
        let entries = self.strategy.entries();
        if entries != other.strategy.entries() {
            return false;
        }
        let same_palette = match (&self.palette, &other.palette) {
            (Palette::Single(a), Palette::Single(b)) => a == b,
            (Palette::List(a), Palette::List(b)) => a == b,
            (Palette::Global, Palette::Global) => true,
            _ => false,
        };
        if same_palette && self.bits == other.bits && self.words == other.words {
            return true;
        }
        (0..entries).all(|i| self.get(i) == other.get(i))
    }
}

#[cfg(test)]
mod tests;
