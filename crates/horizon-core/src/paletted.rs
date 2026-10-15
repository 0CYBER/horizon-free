//! Resident `PalettedContainer` of Java Edition 26.3: single-value, linear and
//! hash-map palettes with vanilla's bit-width thresholds, and the global
//! palette beyond them, over `SimpleBitStorage` words (values never span
//! longs). One container type serves blocks (16^3 entries) and biomes (4^3).
//!
//! Growth follows `PalettedContainer.onResize`: a full palette asks for one
//! more bit, the new configuration is built by re-adding the distinct values
//! in index order, and the value that did not fit is added last. Palettes
//! never shrink while resident; `pack` writes the canonical saved form.

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
/// `n / per_word` for `n < 4096` by multiplication: `MAGIC[bits] = ceil(2^20 /
/// (64 / bits))` is exact while `n * (64 / bits) < 2^20`.
const MAGIC: [u32; 33] = {
    let mut table = [0; 33];
    let mut bits = 1;
    while bits <= 32 {
        let per = 64 / bits as u32;
        table[bits] = (1u32 << 20).div_ceil(per);
        bits += 1;
    }
    table
};
/// Words of `bits`-wide values: `entries.div_ceil(64 / bits)`.
pub fn storage_len(bits: u8, entries: usize) -> usize {
    if bits == 0 {
        0
    } else {
        entries.div_ceil(64 / usize::from(bits))
    }
}
#[inline]
fn locate(bits: u8, i: usize) -> (usize, u32) {
    if bits.is_power_of_two() {
        // 64 / bits is a power of two: shifts instead of the multiplication.
        let log = bits.trailing_zeros();
        let word = i >> (6 - log);
        (word, ((i & ((64 >> log) - 1)) as u32) << log)
    } else {
        let per = 64 / u32::from(bits);
        let word = (i as u32 * MAGIC[usize::from(bits)]) >> 20;
        (word as usize, (i as u32 - word * per) * u32::from(bits))
    }
}
#[inline]
fn mask(bits: u8) -> u64 {
    (1u64 << bits) - 1
}
fn read(words: &[u64], bits: u8, i: usize) -> u32 {
    let (word, shift) = locate(bits, i);
    ((words[word] >> shift) & mask(bits)) as u32
}
fn write(words: &mut [u64], bits: u8, i: usize, value: u32) {
    let (word, shift) = locate(bits, i);
    words[word] = (words[word] & !(mask(bits) << shift)) | (u64::from(value) << shift);
}
/// The distinct `values` in first-occurrence order and each value's index in
/// them. Up to 16 distinct values a scan finds them faster than hashing.
fn palette_ids(values: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut palette: Vec<u32> = Vec::new();
    let mut small = true;
    let mut lookup: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut ids = Vec::with_capacity(values.len());
    for &value in values {
        let id = if small {
            match palette.iter().position(|&v| v == value) {
                Some(id) => id as u32,
                None => {
                    palette.push(value);
                    if palette.len() > 16 {
                        small = false;
                        lookup.extend(palette.iter().enumerate().map(|(i, &v)| (v, i as u32)));
                    }
                    palette.len() as u32 - 1
                }
            }
        } else {
            *lookup.entry(value).or_insert_with(|| {
                palette.push(value);
                palette.len() as u32 - 1
            })
        };
        ids.push(id);
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

#[derive(Clone, Debug)]
pub struct PalettedContainer {
    strategy: Strategy,
    bits: u8,
    palette: Palette,
    words: Vec<u64>,
}
impl PalettedContainer {
    /// `new PalettedContainer(initialValue, strategy)`.
    pub fn new(strategy: Strategy, value: u32) -> Self {
        Self {
            strategy,
            bits: 0,
            palette: Palette::Single(value),
            words: Vec::new(),
        }
    }
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }
    /// Bits per stored index (`BitStorage.getBits`).
    pub fn bits(&self) -> u8 {
        self.bits
    }
    /// Entries in the palette (the global palette reports its bit capacity).
    pub fn palette_len(&self) -> usize {
        match &self.palette {
            Palette::Single(_) => 1,
            Palette::List(list) => list.len(),
            Palette::Global => 1 << self.bits.min(31),
        }
    }
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
            palette => {
                let bits = usize::from(self.bits);
                let per = 64 / bits;
                let m = mask(self.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        let id = ((word >> (slot * bits)) & m) as u32;
                        *cell = match palette {
                            Palette::List(list) => list[id as usize],
                            _ => id,
                        };
                    }
                }
            }
        }
    }
    /// Block ids as 16-bit values (callers whose registry fits in 16 bits).
    pub fn unpack_u16(&self, out: &mut [u16; 4096]) {
        debug_assert_eq!(self.strategy.entries(), 4096);
        match &self.palette {
            Palette::Single(value) => out.fill(*value as u16),
            palette => {
                let bits = usize::from(self.bits);
                let per = 64 / bits;
                let m = mask(self.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        let id = ((word >> (slot * bits)) & m) as usize;
                        *cell = match palette {
                            Palette::List(list) => list[id] as u16,
                            _ => id as u16,
                        };
                    }
                }
            }
        }
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
                palette: Palette::List(palette),
            },
            Config::Global => {
                let bits = strategy.global_bits;
                Self {
                    strategy,
                    bits,
                    words: pack_words(bits, ids.len(), ids.iter().map(|&id| palette[id as usize])),
                    palette: Palette::Global,
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
                };
                container.check_indices()?;
                Ok(container)
            }
            Config::Global => Ok(Self {
                strategy,
                bits: storage_bits,
                palette: Palette::Global,
                words,
            }),
        }
    }
    fn check_indices(&self) -> Result<(), PaletteError> {
        let Palette::List(list) = &self.palette else {
            return Ok(());
        };
        if list.len() == 1 << self.bits {
            return Ok(());
        }
        for i in 0..self.strategy.entries() {
            let index = read(&self.words, self.bits, i);
            if index as usize >= list.len() {
                return Err(PaletteError::BadIndex {
                    index,
                    palette: list.len(),
                });
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
        Ok(match config {
            Config::List(bits) => Self {
                strategy,
                bits,
                words: data.to_vec(),
                palette: Palette::List(palette),
            },
            _ => Self::build(strategy, palette, &ids),
        })
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
    /// Heap bytes held by the palette and the storage.
    pub fn heap_bytes(&self) -> usize {
        self.words.capacity() * 8
            + match &self.palette {
                Palette::List(list) => list.capacity() * 4,
                _ => 0,
            }
    }
}
impl PartialEq for PalettedContainer {
    /// Equal when every entry is.
    fn eq(&self, other: &Self) -> bool {
        let entries = self.strategy.entries();
        entries == other.strategy.entries() && (0..entries).all(|i| self.get(i) == other.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BLOCKS: Strategy = Strategy::blocks(16);
    const BIOMES: Strategy = Strategy::biomes(7);
    fn rng(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
    #[test]
    fn grows_through_vanilla_widths() {
        let mut c = PalettedContainer::new(BLOCKS, 0);
        assert_eq!((c.bits(), c.palette_len()), (0, 1));
        let mut expected = Vec::new();
        for n in 1..=300u32 {
            c.set(n as usize, n);
            expected.push(match n + 1 {
                2..=16 => 4,
                17..=32 => 5,
                33..=64 => 6,
                65..=128 => 7,
                129..=256 => 8,
                _ => 16,
            });
            assert_eq!(c.bits(), *expected.last().unwrap(), "after {n} values");
        }
        assert!(c.is_global());
        for n in 1..=300u32 {
            assert_eq!(c.get(n as usize), n);
        }
        let mut b = PalettedContainer::new(BIOMES, 3);
        for n in 1..=9u32 {
            b.set(n as usize, n + 3);
            let want = match n + 1 {
                2 => 1,
                3..=4 => 2,
                5..=8 => 3,
                _ => 7,
            };
            assert_eq!(b.bits(), want, "after {n} biomes");
        }
    }
    #[test]
    fn matches_a_plain_array_under_random_edits() {
        for (strategy, distinct) in [(BLOCKS, 700u64), (BLOCKS, 12), (BIOMES, 9), (BIOMES, 3)] {
            let entries = strategy.entries();
            let mut state = 0x1234_5678_9ABC_DEF1u64;
            let mut c = PalettedContainer::new(strategy, 5);
            let mut plain = vec![5u32; entries];
            for _ in 0..20_000 {
                let r = rng(&mut state);
                let i = (r % entries as u64) as usize;
                let v = ((r >> 20) % distinct) as u32 * 7;
                assert_eq!(c.set(i, v), plain[i]);
                plain[i] = v;
            }
            let mut out = vec![0; entries];
            c.unpack(&mut out);
            assert_eq!(out, plain);
            for (i, &v) in plain.iter().enumerate() {
                assert_eq!(c.get(i), v);
            }
            let packed = c.pack();
            let loaded = PalettedContainer::from_packed(
                strategy,
                packed.palette.clone(),
                packed.data.as_deref(),
            )
            .unwrap();
            assert_eq!(loaded, c);
            assert_eq!(loaded.pack(), packed);
            let again = PalettedContainer::from_values(strategy, &plain);
            assert_eq!(again, c);
            assert_eq!(again.pack(), packed);
            let (bits, palette, words) = c.to_wire();
            let wire =
                PalettedContainer::from_wire(strategy, bits, palette, words.to_vec()).unwrap();
            assert_eq!(wire, c);
        }
    }
    #[test]
    fn uniform_detection_ignores_stale_palette_entries() {
        let mut c = PalettedContainer::new(BLOCKS, 0);
        c.set(10, 3);
        assert!(!c.is_uniform(0));
        c.set(10, 0);
        assert!(c.palette_len() == 2 && c.is_uniform(0) && c.uniform() == Some(0));
        assert!(!c.is_uniform(3));
        for i in 0..4096 {
            c.set(i, 3);
        }
        assert_eq!(c.uniform(), Some(3));
    }
    #[test]
    fn rejects_malformed_wire_data() {
        assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![1, 2], vec![0; 255]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![1, 2], vec![0x3333; 256]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![], vec![0; 256]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 0, vec![1, 2], vec![]).is_err());
        assert!(PalettedContainer::from_wire(BIOMES, 2, vec![1, 2, 3, 4, 5], vec![0; 4]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 0, vec![9], vec![]).is_ok());
    }
    #[test]
    fn division_magic_is_exact() {
        for bits in 1..=32u8 {
            let per = 64 / usize::from(bits);
            for i in 0..4096usize {
                assert_eq!(locate(bits, i).0, i / per, "bits {bits} index {i}");
            }
        }
    }
}
