//! A server's chunk columns (`ClientboundLevelChunkWithLightPacket`): the
//! sections and light of each packet become a client column on a decoder
//! thread as soon as the packet arrives, so the frame that handles it in
//! order only inserts the result (`ClientPacketListener.handleLevelChunkWithLight`).
use super::Game;
use horizon_client::{
    chunk::{
        BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy, known_block_state,
        known_blocks_only, shared_generation,
    },
    streaming::Streaming,
};
use horizon_core::paletted::{PalettedContainer, Strategy, storage_len};
use horizon_mod_runtime::attachments::Holder;
use horizon_protocol::{
    buf::{Reader, bit},
    chunk::{LightData, Paletted, SectionLight},
    packets::play::{self, Clientbound as C},
    registry::DimensionType,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

/// What turning a chunk packet into a client column depends on: the
/// dimension's height and sky, and the server's biome registry mapped to
/// ours. The connection replaces it when either changes, so a column decoded
/// with an older one is decoded again.
#[derive(Default)]
pub(super) struct ChunkFormat {
    /// The column decoded chunks hold: the client's for the level.
    column: ColumnLayout,
    min_y: i32,
    sections: usize,
    pub(super) has_skylight: bool,
    biome_map: Vec<u16>,
    biome_bits: u8,
}

/// Per-worker reusable scratch buffers for [`ChunkFormat::column_with_scratch`].
#[derive(Default)]
struct DecodeScratch {
    sections: Vec<Section>,
}

#[derive(Clone, Copy)]
enum RawSky<'a> {
    Absent,
    Empty,
    Layer(&'a [u8]),
}

static ZERO_LIGHT_LAYER: [u8; 2048] = [0u8; 2048];

#[inline]
fn all_distinct(s: &[u32]) -> bool {
    for i in 1..s.len() {
        if s[..i].contains(&s[i]) {
            return false;
        }
    }
    true
}

#[inline]
fn biome_container_from_cells(cells: &[u32; 64]) -> PalettedContainer {
    let first = cells[0];
    if cells.iter().all(|&c| c == first) {
        PalettedContainer::new(BIOME_STRATEGY, first)
    } else {
        PalettedContainer::from_values(BIOME_STRATEGY, cells)
    }
}

fn repeat_biome_layer(source: &PalettedContainer, layer: usize) -> PalettedContainer {
    if source.bits() == 0 {
        return PalettedContainer::new(BIOME_STRATEGY, source.get(0));
    }
    let base = layer * 16;
    let cells: [u32; 64] = std::array::from_fn(|j| source.get(base + (j & 15)));
    biome_container_from_cells(&cells)
}

fn unpack_wire_biomes(words: &[u64], bits: u8, out: &mut [u32; 64]) {
    let per = 64 / usize::from(bits);
    let mask = (1u64 << bits) - 1;
    let mut idx = 0;
    for &w in words {
        let mut word = w;
        let end = (idx + per).min(64);
        while idx < end {
            out[idx] = (word & mask) as u32;
            word >>= bits;
            idx += 1;
        }
    }
}

fn unpack_wire_biomes_be(raw_words: &[u8], bits: u8, out: &mut [u32; 64]) {
    let per = 64 / usize::from(bits);
    let mask = if bits == 32 {
        u64::from(u32::MAX)
    } else {
        (1u64 << bits) - 1
    };
    let mut idx = 0;
    let (raw_chunks, _) = raw_words.as_chunks::<8>();
    for chunk in raw_chunks {
        let mut word = u64::from_be_bytes(*chunk);
        let end = (idx + per).min(64);
        while idx < end {
            out[idx] = (word & mask) as u32;
            word >>= bits;
            idx += 1;
        }
    }
}

fn decode_block_container(
    r: &mut Reader<'_>,
    ours: Strategy,
    keep: bool,
) -> Result<Option<PalettedContainer>, String> {
    let wire = r.u8().map_err(|e| e.to_string())?;
    if wire == 0 {
        let raw = r.varint().map_err(|e| e.to_string())? as u32;
        if !keep {
            return Ok(None);
        }
        let v = if known_block_state(raw) && u64::from(raw) >> ours.global_bits == 0 {
            raw
        } else {
            0
        };
        return Ok(Some(PalettedContainer::new(ours, v)));
    }
    let (has_palette, bits) = if wire <= 4 {
        (true, 4)
    } else if wire <= 8 {
        (true, wire)
    } else {
        (false, 16u8.max(wire))
    };
    if bits > 32 {
        return Err(format!("{bits} bits per entry, more than 32"));
    }
    let word_count = storage_len(bits, 4096);
    if !keep {
        if has_palette {
            let n = r.count(1 << bits).map_err(|e| e.to_string())?;
            for _ in 0..n {
                let _ = r.varint().map_err(|e| e.to_string())?;
            }
        }
        let _ = r.bytes(word_count * 8).map_err(|e| e.to_string())?;
        return Ok(None);
    }
    let palette = if has_palette {
        let n = r.count(1 << bits).map_err(|e| e.to_string())?;
        let mut palette = Vec::with_capacity(n);
        for _ in 0..n {
            palette.push(r.varint().map_err(|e| e.to_string())? as u32);
        }
        palette
    } else {
        Vec::new()
    };
    let raw_words = r.bytes(word_count * 8).map_err(|e| e.to_string())?;
    let (raw_chunks, _) = raw_words.as_chunks::<8>();
    let words: Vec<u64> = raw_chunks.iter().map(|b| u64::from_be_bytes(*b)).collect();
    // A global section is as wide as the server's registry: 16 bits from a
    // vanilla server, wider from a Horizon host with the same content mods.
    // Read it at that width, then widen or narrow it to ours.
    let strategy = if ours.wire_has_palette(bits) {
        ours
    } else {
        Strategy::blocks(bits)
    };
    let mut blocks = known_blocks_only(
        PalettedContainer::from_wire(strategy, bits, palette, words).map_err(|e| e.to_string())?,
    );
    if blocks.bits() != 0
        && let Some(v) = blocks.uniform()
    {
        blocks = PalettedContainer::new(ours, v);
    }
    Ok(Some(blocks))
}

#[inline]
fn uniform_nibble_layer(slice: &[u8]) -> Option<u8> {
    let first = *slice.first()?;
    if (first & 15) != (first >> 4) {
        return None;
    }
    let word = u64::from_ne_bytes([first; 8]);
    let (chunks, rem) = slice.as_chunks::<8>();
    if chunks.iter().all(|c| u64::from_ne_bytes(*c) == word) && rem.iter().all(|&b| b == first) {
        Some(first & 15)
    } else {
        None
    }
}

#[inline]
fn copy_nibble_layer(slice: &[u8]) -> Box<[u8; 2048]> {
    let mut out = Box::new([0u8; 2048]);
    out.copy_from_slice(slice);
    out
}

#[inline]
fn unpack_bottom_row(slice: &[u8], above: &mut [u8; 256]) -> Option<u8> {
    let row = &slice[..128];
    if let Some(u) = uniform_nibble_layer(row) {
        return Some(u);
    }
    let (pairs, _) = above.as_chunks_mut::<2>();
    for (pair, &byte) in pairs.iter_mut().zip(row) {
        pair[0] = byte & 15;
        pair[1] = byte >> 4;
    }
    None
}

#[inline]
fn nibble_slice(slice: &[u8], i: usize) -> u8 {
    (slice[i >> 1] >> ((i & 1) << 2)) & 15
}

fn combine_sky_and_block(sky: &[u8], block: &[u8]) -> Box<[u8; 4096]> {
    let mut out = Box::new([0u8; 4096]);
    let (pairs, _) = out.as_chunks_mut::<2>();
    for (pair, (&s, &b)) in pairs.iter_mut().zip(sky.iter().zip(block)) {
        pair[0] = ((s & 0x0F) << 4) | (b & 0x0F);
        pair[1] = (s & 0xF0) | (b >> 4);
    }
    out
}

impl ChunkFormat {
    pub(super) fn new(dimension: &DimensionType, biome_map: Vec<u16>, biome_bits: u8) -> Self {
        Self {
            column: ColumnLayout::VANILLA,
            min_y: dimension.min_y,
            sections: dimension.sections(),
            has_skylight: dimension.has_skylight,
            biome_map,
            biome_bits,
        }
    }
    /// Decodes into chunks of `column`.
    pub(super) fn in_column(self, column: ColumnLayout) -> Self {
        Self { column, ..self }
    }
    /// The column decoded chunks hold.
    pub(super) fn layout(&self) -> ColumnLayout {
        self.column
    }
    pub(super) fn with_dimension(&self, dimension: &DimensionType) -> Self {
        Self {
            column: self.column,
            ..Self::new(dimension, self.biome_map.clone(), self.biome_bits)
        }
    }

    #[inline]
    fn map_biome(&self, raw: i32) -> u32 {
        u32::from(self.biome_map.get(raw as usize).copied().unwrap_or(0))
    }

    fn decode_biome_container(
        &self,
        r: &mut Reader<'_>,
        keep: bool,
    ) -> Result<Option<PalettedContainer>, String> {
        let wire = r.u8().map_err(|e| e.to_string())?;
        if wire == 0 {
            let raw = r.varint().map_err(|e| e.to_string())?;
            if !keep {
                return Ok(None);
            }
            return Ok(Some(PalettedContainer::new(
                BIOME_STRATEGY,
                self.map_biome(raw),
            )));
        }
        let (has_palette, bits) = if wire <= 3 {
            (true, wire)
        } else {
            (false, self.biome_bits.max(wire))
        };
        if bits > 32 {
            return Err(format!("{bits} bits per entry, more than 32"));
        }
        let word_count = storage_len(bits, 64);
        if !keep {
            if has_palette {
                let n = r.count(1 << bits).map_err(|e| e.to_string())?;
                for _ in 0..n {
                    let _ = r.varint().map_err(|e| e.to_string())?;
                }
            }
            let _ = r.bytes(word_count * 8).map_err(|e| e.to_string())?;
            return Ok(None);
        }
        if has_palette {
            let n = r.count(1 << bits).map_err(|e| e.to_string())?;
            let mut mapped = [0u32; 8];
            for slot in &mut mapped[..n] {
                *slot = self.map_biome(r.varint().map_err(|e| e.to_string())?);
            }
            let raw_words = r.bytes(word_count * 8).map_err(|e| e.to_string())?;
            let mut words = [0u64; 4];
            let (raw_chunks, _) = raw_words.as_chunks::<8>();
            for (dst, chunk) in words[..word_count].iter_mut().zip(raw_chunks) {
                *dst = u64::from_be_bytes(*chunk);
            }
            if n >= 2
                && all_distinct(&mapped[..n])
                && let Ok(c) = PalettedContainer::from_wire(
                    BIOME_STRATEGY,
                    bits,
                    mapped[..n].to_vec(),
                    words[..word_count].to_vec(),
                )
            {
                return Ok(Some(if let Some(v) = c.uniform() {
                    PalettedContainer::new(BIOME_STRATEGY, v)
                } else {
                    c
                }));
            }
            let default_biome = self.map_biome(0);
            let mut cells = [0u32; 64];
            unpack_wire_biomes(&words[..word_count], bits, &mut cells);
            if n == 0 {
                for c in &mut cells {
                    *c = self.map_biome(*c as i32);
                }
            } else {
                for c in &mut cells {
                    *c = mapped[..n]
                        .get(*c as usize)
                        .copied()
                        .unwrap_or(default_biome);
                }
            }
            return Ok(Some(biome_container_from_cells(&cells)));
        }
        let raw_words = r.bytes(word_count * 8).map_err(|e| e.to_string())?;
        let mut cells = [0u32; 64];
        unpack_wire_biomes_be(raw_words, bits, &mut cells);
        for c in &mut cells {
            *c = self.map_biome(*c as i32);
        }
        Ok(Some(biome_container_from_cells(&cells)))
    }

    fn decode_light_into(&self, light: &LightData, sections: &mut [Section]) {
        let column = sections.len();
        let column_i32 = column as i32;
        let sky_enabled = self.has_skylight;
        let mut above = [if sky_enabled { 15u8 } else { 0 }; 256];
        let mut above_uniform = Some(if sky_enabled { 15u8 } else { 0 });

        let mut stack_layers: [(RawSky<'_>, Option<&[u8]>); 32] = [(RawSky::Absent, None); 32];
        let mut heap_layers;
        let layers: &mut [(RawSky<'_>, Option<&[u8]>)] = if column <= stack_layers.len() {
            &mut stack_layers[..column]
        } else {
            heap_layers = vec![(RawSky::Absent, None); column];
            &mut heap_layers
        };

        // Light section b is the dimension's section b - 1.
        let first = (self.min_y - self.column.min_y()).div_euclid(16) - 1;
        let mut sky_iter = light.sky.iter();
        let mut block_iter = light.block.iter();
        for b in 0..self.sections + 2 {
            let s = if bit(&light.sky_mask, b) {
                sky_iter
                    .next()
                    .map_or(RawSky::Absent, |v| RawSky::Layer(v.as_slice()))
            } else if bit(&light.empty_sky_mask, b) {
                RawSky::Empty
            } else {
                RawSky::Absent
            };
            // In a full chunk packet, absent and empty block light are both all zero.
            let bl = if bit(&light.block_mask, b) {
                block_iter.next().map(Vec::as_slice)
            } else {
                None
            };
            let si = first + b as i32;
            if (0..column_i32).contains(&si) {
                layers[si as usize] = (if sky_enabled { s } else { RawSky::Absent }, bl);
            } else if si == column_i32 && sky_enabled {
                match s {
                    RawSky::Empty => above_uniform = Some(0),
                    RawSky::Layer(l) if l.len() >= 128 => {
                        above_uniform = unpack_bottom_row(l, &mut above);
                    }
                    RawSky::Layer(l) => {
                        for (i, a) in above.iter_mut().enumerate() {
                            *a = nibble_slice(l, i);
                        }
                        above_uniform = None;
                    }
                    RawSky::Absent => {}
                }
            }
        }

        for (section, &(sky, block)) in sections.iter_mut().zip(layers.iter()).rev() {
            section.light = match (sky, block, above_uniform) {
                (RawSky::Empty, None, _) => {
                    above_uniform = Some(0);
                    Light::Uniform(0)
                }
                (RawSky::Empty, Some(bl), _) if bl.len() == 2048 => {
                    above_uniform = Some(0);
                    match uniform_nibble_layer(bl) {
                        Some(bv) => Light::Uniform(bv),
                        None => Light::Block(copy_nibble_layer(bl), 0),
                    }
                }
                (RawSky::Absent, None, Some(sv)) => Light::Uniform(sv << 4),
                (RawSky::Absent, Some(bl), Some(sv)) if bl.len() == 2048 => {
                    match uniform_nibble_layer(bl) {
                        Some(bv) => Light::Uniform(sv << 4 | bv),
                        None => Light::Block(copy_nibble_layer(bl), sv),
                    }
                }
                (RawSky::Layer(sl), None, _) if sl.len() == 2048 => {
                    if let Some(sv) = uniform_nibble_layer(sl) {
                        above_uniform = Some(sv);
                        Light::Uniform(sv << 4)
                    } else {
                        above_uniform = unpack_bottom_row(sl, &mut above);
                        Light::Sky(copy_nibble_layer(sl), 0)
                    }
                }
                (RawSky::Layer(sl), Some(bl), _) if sl.len() == 2048 && bl.len() == 2048 => {
                    match (uniform_nibble_layer(sl), uniform_nibble_layer(bl)) {
                        (Some(sv), Some(bv)) => {
                            above_uniform = Some(sv);
                            Light::Uniform(sv << 4 | bv)
                        }
                        (Some(sv), None) => {
                            above_uniform = Some(sv);
                            Light::Block(copy_nibble_layer(bl), sv)
                        }
                        (None, Some(bv)) => {
                            above_uniform = unpack_bottom_row(sl, &mut above);
                            Light::Sky(copy_nibble_layer(sl), bv)
                        }
                        (None, None) => {
                            above_uniform = unpack_bottom_row(sl, &mut above);
                            Light::Full(combine_sky_and_block(sl, bl))
                        }
                    }
                }
                (sky, block, above_u) => {
                    let mut data = [0u8; 4096];
                    if let Some(sv) = above_u {
                        above.fill(sv);
                    }
                    match sky {
                        RawSky::Empty => {
                            above.fill(0);
                            above_uniform = Some(0);
                            if let Some(bl) = block {
                                for (i, packed) in data.iter_mut().enumerate() {
                                    *packed = nibble_slice(bl, i);
                                }
                            }
                        }
                        RawSky::Layer(sl) => {
                            for (i, packed) in data.iter_mut().enumerate() {
                                let s = nibble_slice(sl, i);
                                let b = block.map_or(0, |a| nibble_slice(a, i));
                                *packed = s << 4 | b;
                            }
                            for (a, &p) in above.iter_mut().zip(&data[..256]) {
                                *a = p >> 4;
                            }
                            above_uniform = None;
                        }
                        RawSky::Absent => {
                            for (i, packed) in data.iter_mut().enumerate() {
                                let s = above[i & 255];
                                let b = block.map_or(0, |a| nibble_slice(a, i));
                                *packed = s << 4 | b;
                            }
                        }
                    }
                    Light::from_slice(&data)
                }
            };
        }
    }

    /// LevelChunkSection data into the client's column.
    pub(super) fn column(&self, p: &play::LevelChunkWithLight) -> Result<Chunk, String> {
        let mut scratch = DecodeScratch::default();
        self.column_with_scratch(p, &mut scratch)
    }

    fn column_with_scratch(
        &self,
        p: &play::LevelChunkWithLight,
        scratch: &mut DecodeScratch,
    ) -> Result<Chunk, String> {
        let generation = shared_generation();
        let ours = block_strategy();
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        let column = self.column.sections();
        let column_i32 = column as i32;
        let need_below = self.sections > 0 && offset > 0;
        let need_above = self.sections > 0 && (offset + self.sections as i32) < column_i32;

        scratch.sections.clear();
        scratch.sections.resize_with(column, || Section {
            blocks: PalettedContainer::new(ours, 0),
            biomes: PalettedContainer::new(BIOME_STRATEGY, 0),
            light: Light::Uniform(0xF0),
        });

        let mut r = Reader::new(&p.chunk.buffer);
        let mut first_biomes: Option<PalettedContainer> = None;
        let mut last_biomes: Option<PalettedContainer> = None;
        for i in 0..self.sections {
            let _ = r.bytes(4).map_err(|e| e.to_string())?;
            let target = offset + i as i32;
            let in_column = (0..column_i32).contains(&target);
            let blocks = decode_block_container(&mut r, ours, in_column)?;
            let keep_first = need_below && i == 0;
            let keep_last = need_above && i == self.sections - 1;
            let biomes =
                self.decode_biome_container(&mut r, in_column || keep_first || keep_last)?;
            if in_column {
                let slot = &mut scratch.sections[target as usize];
                if let Some(blocks) = blocks {
                    slot.blocks = blocks;
                }
                if let Some(biomes) = biomes {
                    if keep_first {
                        first_biomes = Some(repeat_biome_layer(&biomes, 0));
                    }
                    if keep_last {
                        last_biomes = Some(repeat_biome_layer(&biomes, 3));
                    }
                    slot.biomes = biomes;
                }
            } else if let Some(biomes) = biomes {
                if keep_first {
                    first_biomes = Some(repeat_biome_layer(&biomes, 0));
                }
                if keep_last {
                    last_biomes = Some(repeat_biome_layer(&biomes, 3));
                }
            }
        }
        if let Some(edge) = first_biomes {
            let end = (offset as usize).min(column);
            for slot in &mut scratch.sections[..end] {
                slot.biomes = edge.clone();
            }
        }
        if let Some(edge) = last_biomes {
            let start = (offset + self.sections as i32).max(0) as usize;
            if start < column {
                for slot in &mut scratch.sections[start..] {
                    slot.biomes = edge.clone();
                }
            }
        }
        // ClientPacketListener.applyLightData: the column keeps the light
        // the server sent; nothing is relit.
        self.decode_light_into(&p.light, &mut scratch.sections);
        let mut chunk = Chunk::from_sections_with_generation(
            p.x,
            p.z,
            self.column,
            &mut scratch.sections,
            generation,
        );
        chunk.lit = true;
        chunk.light_loaded = true;
        // From the single-player server thread: its own sections where they
        // are the same.
        horizon_client::lan::handoff::take(&mut chunk);
        Ok(chunk)
    }

    /// The dimension's sections' biome containers (in the server's
    /// registry) into the column's sections. Sections outside the
    /// dimension's range take its nearest biomes (the column's top or bottom
    /// cell layer repeated).
    #[allow(dead_code)]
    fn put_biomes(&self, chunk: &mut Chunk, biomes: &[&Paletted]) {
        if biomes.is_empty() {
            return;
        }
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        // Biome table index per quart cell of the dimension's own sections.
        let biome_cells: Vec<[u32; 64]> = biomes
            .iter()
            .map(|b| {
                std::array::from_fn(|j| {
                    u32::from(self.biome_map.get(b.get(j) as usize).copied().unwrap_or(0))
                })
            })
            .collect();
        let last = biome_cells.len() - 1;
        for si in 0..self.column.sections() as i32 {
            let i = si - offset;
            let cells = if (0..biome_cells.len() as i32).contains(&i) {
                biome_cells[i as usize]
            } else {
                let (edge, layer) = if i < 0 { (0, 0) } else { (last, 3) };
                std::array::from_fn(|j| biome_cells[edge][layer * 16 + j % 16])
            };
            let mut container = PalettedContainer::from_values(BIOME_STRATEGY, &cells);
            if let Some(v) = container.uniform() {
                container = PalettedContainer::new(BIOME_STRATEGY, v);
            }
            Arc::make_mut(&mut chunk.sections[si as usize]).biomes = container;
        }
    }

    /// `LevelChunk.replaceBiomes`: each of the dimension's sections reads
    /// its biome container from the packet's buffer, in order.
    pub(super) fn replace_biomes(
        &self,
        chunk: &mut Chunk,
        data: &play::ChunkBiomeData,
    ) -> Result<(), String> {
        if self.sections == 0 {
            return Ok(());
        }
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        let column = self.column.sections();
        let column_i32 = column as i32;
        let need_below = offset > 0;
        let need_above = (offset + self.sections as i32) < column_i32;

        let mut r = Reader::new(&data.buffer);
        let mut parsed: Vec<(usize, PalettedContainer)> =
            Vec::with_capacity(self.sections.min(column));
        let mut first_biomes: Option<PalettedContainer> = None;
        let mut last_biomes: Option<PalettedContainer> = None;
        for i in 0..self.sections {
            let target = offset + i as i32;
            let in_column = (0..column_i32).contains(&target);
            let keep_first = need_below && i == 0;
            let keep_last = need_above && i == self.sections - 1;
            let biomes =
                self.decode_biome_container(&mut r, in_column || keep_first || keep_last)?;
            if let Some(biomes) = biomes {
                if keep_first {
                    first_biomes = Some(repeat_biome_layer(&biomes, 0));
                }
                if keep_last {
                    last_biomes = Some(repeat_biome_layer(&biomes, 3));
                }
                if in_column {
                    parsed.push((target as usize, biomes));
                }
            }
        }
        for (si, biomes) in parsed {
            Arc::make_mut(&mut chunk.sections[si]).biomes = biomes;
        }
        if let Some(edge) = first_biomes {
            let end = (offset as usize).min(column);
            for section in &mut chunk.sections[..end] {
                Arc::make_mut(section).biomes = edge.clone();
            }
        }
        if let Some(edge) = last_biomes {
            let start = (offset + self.sections as i32).max(0) as usize;
            if start < column {
                for section in &mut chunk.sections[start..] {
                    Arc::make_mut(section).biomes = edge.clone();
                }
            }
        }
        Ok(())
    }

    /// `ClientPacketListener.readSectionList`: the sky and block layers of a
    /// light packet, by section of the client's column, and the sky light
    /// above the column's top section (from the light section just above
    /// it, else open sky).
    pub(super) fn light(&self, light: &LightData) -> (Vec<SectionLight>, [u8; 256]) {
        let column = self.column.sections() as i32;
        let mut layers: Vec<SectionLight> = vec![(None, None); self.column.sections()];
        let sky = self.has_skylight;
        let mut above = [if sky { 15 } else { 0 }; 256];
        // Light section b is the dimension's section b - 1.
        let first = (self.min_y - self.column.min_y()).div_euclid(16) - 1;
        for (b, layer) in light.layers(self.sections + 2).into_iter().enumerate() {
            let si = first + b as i32;
            if (0..column).contains(&si) {
                layers[si as usize] = layer;
            } else if si == column
                && sky
                && let Some(l) = &layer.0
            {
                for (i, a) in above.iter_mut().enumerate() {
                    *a = (l[i >> 1] >> ((i & 1) * 4)) & 15;
                }
            }
        }
        (layers, above)
    }

    /// Borrowed zero-allocation slice view of a light update packet's layers.
    fn light_slices<'a>(&self, light: &'a LightData) -> Vec<(Option<&'a [u8]>, Option<&'a [u8]>)> {
        let column = self.column.sections() as i32;
        let mut layers: Vec<(Option<&'a [u8]>, Option<&'a [u8]>)> =
            vec![(None, None); self.column.sections()];
        // Light section b is the dimension's section b - 1.
        let first = (self.min_y - self.column.min_y()).div_euclid(16) - 1;
        let mut sky_iter = light.sky.iter();
        let mut block_iter = light.block.iter();
        for b in 0..self.sections + 2 {
            let s = if bit(&light.sky_mask, b) {
                sky_iter.next().map(Vec::as_slice)
            } else if bit(&light.empty_sky_mask, b) {
                Some(&ZERO_LIGHT_LAYER[..])
            } else {
                None
            };
            let bl = if bit(&light.block_mask, b) {
                block_iter.next().map(Vec::as_slice)
            } else if bit(&light.empty_block_mask, b) {
                Some(&ZERO_LIGHT_LAYER[..])
            } else {
                None
            };
            let si = first + b as i32;
            if (0..column).contains(&si) {
                layers[si as usize] = (s, bl);
            }
        }
        layers
    }
}

/// `ClientboundSectionBlocksUpdatePacket.runUpdates`: each changed block's position (the
/// packed local position within the section) and its state.
pub(super) fn section_blocks(
    p: &play::SectionBlocksUpdate,
) -> impl Iterator<Item = ([i32; 3], i32)> + '_ {
    let [sx, sy, sz] = p.section;
    p.changes.iter().map(move |&change| {
        let local = (change & 0xFFF) as i32;
        let pos = [
            sx * 16 + (local >> 8 & 15),
            sy * 16 + (local & 15),
            sz * 16 + (local >> 4 & 15),
        ];
        (pos, (change >> 12) as i32)
    })
}

type Decoded = (Box<C>, Result<Chunk, String>);

struct Job {
    seq: u64,
    format: Arc<ChunkFormat>,
    packet: Box<C>,
}

struct WorkResult {
    seq: u64,
    packet: Box<C>,
    column: Result<Chunk, String>,
}

struct DecoderQueue {
    jobs: VecDeque<Job>,
    stopped: bool,
}

struct Retirement {
    next_submit: u64,
    next_deliver: u64,
    ready: BTreeMap<u64, Decoded>,
    done: mpsc::Receiver<WorkResult>,
}

/// Worker threads that decode chunk packets in parallel while returning
/// results strictly in the order they arrived, one per `submit`.
pub(super) struct ChunkDecoder {
    queue: Arc<(Mutex<DecoderQueue>, Condvar)>,
    retirement: Mutex<Retirement>,
    workers_alive: Arc<AtomicUsize>,
}

impl ChunkDecoder {
    pub(super) fn new() -> Self {
        let (finished, done) = mpsc::channel();
        let queue = Arc::new((
            Mutex::new(DecoderQueue {
                jobs: VecDeque::new(),
                stopped: false,
            }),
            Condvar::new(),
        ));
        let worker_count =
            horizon_threads::sizing::for_pool(horizon_threads::sizing::Pool::ChunkDecoder);
        let workers_alive = Arc::new(AtomicUsize::new(0));

        let mut spawned = 0;
        for i in 0..worker_count {
            let queue = Arc::clone(&queue);
            let finished = finished.clone();
            let alive = Arc::clone(&workers_alive);
            let builder = std::thread::Builder::new()
                .name(format!("Chunk decoder {i}"))
                .spawn(move || {
                    horizon_threads::chunk_decoder_current();
                    let mut scratch = DecodeScratch::default();
                    'worker: loop {
                        let job = {
                            let mut q = match queue.0.lock() {
                                Ok(q) => q,
                                Err(_) => break 'worker,
                            };
                            loop {
                                if q.stopped && q.jobs.is_empty() {
                                    break 'worker;
                                }
                                if let Some(job) = q.jobs.pop_front() {
                                    break job;
                                }
                                q = match queue.1.wait(q) {
                                    Ok(guard) => guard,
                                    Err(_) => break 'worker,
                                };
                            }
                        };
                        let result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &*job
                                .packet
                            {
                                C::LevelChunkWithLight(p) => {
                                    job.format.column_with_scratch(p, &mut scratch)
                                }
                                _ => Err("not a chunk packet".into()),
                            }));
                        let column = match result {
                            Ok(Ok(column)) => Ok(column),
                            Ok(Err(err)) => {
                                scratch.sections.clear();
                                Err(err)
                            }
                            Err(_) => {
                                scratch.sections.clear();
                                Err("chunk decoding panicked".into())
                            }
                        };
                        if finished
                            .send(WorkResult {
                                seq: job.seq,
                                packet: job.packet,
                                column,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    alive.fetch_sub(1, Ordering::Relaxed);
                });
            if builder.is_ok() {
                spawned += 1;
            } else if let Err(e) = builder {
                log::warn!("Horizon: no chunk decoder thread ({e}); chunks decode in the frame");
            }
        }
        workers_alive.store(spawned, Ordering::Relaxed);
        Self {
            queue,
            retirement: Mutex::new(Retirement {
                next_submit: 0,
                next_deliver: 0,
                ready: BTreeMap::new(),
                done,
            }),
            workers_alive,
        }
    }

    /// Start decoding a chunk packet with the format it arrived under.
    /// Without a decoder thread the packet comes back undecoded.
    pub(super) fn submit(&self, format: Arc<ChunkFormat>, packet: Box<C>) -> Option<Box<C>> {
        if self.workers_alive.load(Ordering::Relaxed) == 0 {
            return Some(packet);
        }
        let mut retirement = match self.retirement.lock() {
            Ok(r) => r,
            Err(_) => return Some(packet),
        };
        let seq = retirement.next_submit;
        retirement.next_submit += 1;
        let mut q = match self.queue.0.lock() {
            Ok(q) => q,
            Err(_) => {
                retirement.next_submit -= 1;
                return Some(packet);
            }
        };
        if q.stopped {
            retirement.next_submit -= 1;
            return Some(packet);
        }
        q.jobs.push_back(Job {
            seq,
            format,
            packet,
        });
        self.queue.1.notify_one();
        None
    }

    /// The oldest submitted packet and its column, unless it is still
    /// decoding: a frame never waits for the decoder.
    pub(super) fn next(&self) -> Next {
        let mut ret = match self.retirement.lock() {
            Ok(r) => r,
            Err(_) => return Next::Lost,
        };
        while let Ok(res) = ret.done.try_recv() {
            ret.ready.insert(res.seq, (res.packet, res.column));
        }
        let next_deliver = ret.next_deliver;
        if let Some(decoded) = ret.ready.remove(&next_deliver) {
            ret.next_deliver += 1;
            return Next::Ready(decoded);
        }
        if self.workers_alive.load(Ordering::Relaxed) == 0 {
            return Next::Lost;
        }
        Next::Decoding
    }
}

impl Drop for ChunkDecoder {
    fn drop(&mut self) {
        if let Ok(mut q) = self.queue.0.lock() {
            q.stopped = true;
            self.queue.1.notify_all();
        }
    }
}

pub(super) enum Next {
    Ready(Decoded),
    Decoding,
    Lost,
}

impl Game<'_> {
    /// A chunk packet's column was adopted, with its block entities in the
    /// level: the client chunk seam (milestone 8) tells `client_chunk_load`.
    pub(in crate::game) fn client_chunk_load(&mut self, x: i32, z: i32) {
        if !mod_api::__private::listened(mod_api::HookId::ClientChunkLoad) {
            return;
        }
        if let Some(dimension) = self.client_chunk_dimension() {
            self.native_client_chunk_load(&dimension, [x, z]);
        }
    }

    /// A resident column was freed, whichever route freed it — the last
    /// event for it, its data still there. Tells `client_chunk_unload` and,
    /// when the level stays (`level_stays`), `on_chunk_unloaded`: a level that
    /// goes takes its block entities with it. Its passages go with it.
    pub(in crate::game) fn client_chunk_unload(&mut self, x: i32, z: i32, level_stays: bool) {
        self.forget_chunk_passages(x, z);
        if mod_api::__private::listened(mod_api::HookId::ClientChunkUnload)
            && let Some(dimension) = self.client_chunk_dimension()
        {
            self.native_client_chunk_unload(&dimension, [x, z]);
        }
        if level_stays && let Some(level) = self.native_client_level() {
            self.notify_chunk_block_entities_unloaded(level, (x, z));
        }
    }

    /// The client level's dimension, as the server named it, for the client
    /// chunk hooks: none off a server (an integrated world's chunks are the
    /// server's `chunk_load` and `chunk_unload`).
    fn client_chunk_dimension(&self) -> Option<String> {
        self.remote.as_ref()?.heard_level.clone()
    }
}

impl Game<'_> {
    /// `handleLevelChunkWithLight`: the column replaces any loaded one, and
    /// its block entities load their update tags. `decoded` is the column the
    /// decoder made with `format`; another format since means decode again.
    pub(super) fn remote_chunk(
        &mut self,
        p: &play::LevelChunkWithLight,
        decoded: Option<(&Arc<ChunkFormat>, Result<Chunk, String>)>,
    ) {
        // ClientChunkCache.replaceWithPacketData: a column outside the
        // storage around the view centre is ignored.
        if !self.remote_chunk_in_range(p.x, p.z) {
            log::warn!(
                "Ignoring chunk since it's not in the view range: {}, {}",
                p.x,
                p.z
            );
            return;
        }
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        let column = match decoded {
            Some((used, column)) if Arc::ptr_eq(used, &format) => column,
            _ => format.column(p),
        };
        let chunk = match column {
            Ok(chunk) => chunk,
            Err(e) => {
                log::warn!("Horizon: chunk {} {}: {e}", p.x, p.z);
                return;
            }
        };
        let (x, z) = (p.x, p.z);
        // A column sent again while it is resident (a bulk edit): only what changed is lit and meshed again.
        let resident = self.world.chunks.contains_key(&(x, z));
        if resident {
            horizon_client::spans::time("replace_remote", || {
                self.streaming.replace_remote(&mut self.world, chunk)
            });
        } else {
            self.streaming.insert_remote(&mut self.world, chunk);
        }
        self.world.block_entities.unload_chunks(&[(x, z)]);
        self.load_block_entities((x, z), &p.chunk.block_entities);
        // After the column's block entities load: an attachment keeps its block entity if the
        // packet sent it again, and one left from a forgotten column goes.
        if let Some(r) = self.remote.as_mut() {
            r.attachments.retain(|holder, _| match *holder {
                Holder::BlockEntity([bx, by, bz]) => {
                    (bx >> 4, bz >> 4) != (x, z)
                        || self.world.block_entities.contains_key(&(bx, by, bz))
                }
                _ => true,
            });
        }
        // Asked once the packet's block entities and attachments are in, so a part reading them
        // sees this column's data.
        self.ask_chunk_light_emission((x, z));
        // The client chunk seam: one load per packet adoption, a re-sent position included,
        // once the column and its block entities are in. A biome packet takes neither path.
        self.client_chunk_load(x, z);
    }

    /// `LevelChunk.replaceWithPacketData`'s block entities of column `(x, z)`, as its chunk
    /// packet lists them: each made for its block, then loading its update tag.
    pub(in crate::game) fn load_block_entities(
        &mut self,
        (x, z): (i32, i32),
        infos: &[horizon_protocol::chunk::BlockEntityInfo],
    ) {
        for info in infos {
            let pos = (
                x * 16 + i32::from(info.packed_xz as u8 >> 4),
                i32::from(info.y),
                z * 16 + i32::from(info.packed_xz as u8 & 15),
            );
            let state = self.world.block(pos.0, pos.1, pos.2);
            self.sync_block_entity(pos, None, state);
            // LevelChunk.replaceWithPacketData: the block entity loads its update tag.
            self.remote_block_entity_data(
                [pos.0, pos.1, pos.2],
                info.kind,
                mod_api::BlockEntityDataSource::Chunk,
                &info.tag,
            );
            self.bind_remote_attachment(Holder::BlockEntity([pos.0, pos.1, pos.2]));
        }
    }

    /// `ClientPacketListener.handleChunksBiomes`: each loaded column takes
    /// its new biomes (`ClientChunkCache.replaceBiomes`), then every section
    /// of it and of its neighbours is drawn again, as their colours blend
    /// across columns (`LevelExtractor.setSectionDirty`).
    pub(super) fn remote_chunks_biomes(&mut self, p: &play::ChunksBiomes) {
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        for data in &p.chunks {
            let [x, z] = data.pos;
            if !self.remote_chunk_in_range(x, z) {
                log::warn!("Ignoring chunk since it's not in the view range: {x}, {z}");
                continue;
            }
            let Some(chunk) = self.world.chunks.get_mut(&(x, z)) else {
                log::warn!("Ignoring chunk since it's not present: {x}, {z}");
                continue;
            };
            if let Err(e) = format.replace_biomes(Arc::make_mut(chunk), data) {
                log::warn!("Horizon: biomes of chunk {x} {z}: {e}");
            }
        }
        let mask = horizon_client::section_mask::SectionMask::below(self.world.layout().sections());
        for data in &p.chunks {
            let [x, z] = data.pos;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let n = (x + dx, z + dz);
                    if self.world.chunks.contains_key(&n) {
                        self.world.dirty_sections.insert_column_mask(n, mask);
                    }
                }
            }
        }
    }

    /// `handleLightUpdatePacket`: the sections it names take its light.
    pub(super) fn remote_light_update(&mut self, p: &play::LightUpdate) {
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        if !self.world.chunks.contains_key(&(p.x, p.z)) {
            return;
        }
        let layers = format.light_slices(&p.light);
        let Some(chunk) = self.world.chunks.get_mut(&(p.x, p.z)) else {
            return;
        };
        let changed = horizon_client::light::update_light_layers(
            Arc::make_mut(chunk),
            &layers,
            format.has_skylight,
        );
        Streaming::light_updated(&mut self.world, p.x, p.z, &changed);
        self.ask_updated_light_emission((p.x, p.z), &changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::chunk::{ChunkData, Paletted, Section as Wire, Strategy as Wiring};

    /// A column of `state` with biome 1 everywhere, unlit.
    fn packet(x: i32, state: i32) -> Box<C> {
        let section = Wire {
            non_empty: 4096,
            fluids: 0,
            states: Paletted::from_values(&[state; 4096], Wiring::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Wiring::Biomes, 2),
        };
        let none = LightData {
            sky_mask: Vec::new(),
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky: Vec::new(),
            block: Vec::new(),
        };
        Box::new(C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z: -3,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&vec![section; 24]),
                block_entities: Vec::new(),
            },
            light: none,
        }))
    }

    /// Columns come back one per packet, in the order the packets went in,
    /// as the frame would have decoded them itself.
    #[test]
    fn the_decoder_returns_columns_in_order() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let decoder = ChunkDecoder::new();
        for x in 0..20 {
            assert!(decoder.submit(format.clone(), packet(x, 1 + x)).is_none());
        }
        for x in 0..20 {
            let (packet, column) = loop {
                match decoder.next() {
                    Next::Ready(decoded) => break decoded,
                    Next::Decoding => std::thread::yield_now(),
                    Next::Lost => panic!("the decoder stopped"),
                }
            };
            let C::LevelChunkWithLight(p) = *packet else {
                panic!("not a chunk packet");
            };
            let (column, direct) = (column.unwrap(), format.column(&p).unwrap());
            assert_eq!((column.x, column.z), (x, -3));
            for (cx, y, cz) in [(0, -64, 0), (5, 70, 9), (15, 319, 15)] {
                assert_eq!(column.get(cx, y, cz), 1 + x as u32);
                assert_eq!(column.get(cx, y, cz), direct.get(cx, y, cz));
                // Wire biome 1 is our biome 7.
                assert_eq!(column.biome(cx, y, cz), 7);
            }
        }
        assert!(matches!(decoder.next(), Next::Decoding));
    }

    /// Large bursts of chunk packets (e.g. 128 columns arriving together)
    /// decode concurrently and retire in strict sequential FIFO order.
    #[test]
    fn the_decoder_handles_large_bursts_in_strict_fifo() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let decoder = ChunkDecoder::new();
        for x in 0..128 {
            assert!(decoder.submit(format.clone(), packet(x, 1 + x)).is_none());
        }
        for x in 0..128 {
            let (packet, column) = loop {
                match decoder.next() {
                    Next::Ready(decoded) => break decoded,
                    Next::Decoding => std::thread::yield_now(),
                    Next::Lost => panic!("the decoder stopped"),
                }
            };
            let C::LevelChunkWithLight(p) = *packet else {
                panic!("not a chunk packet");
            };
            assert_eq!(p.x, x);
            let column = column.unwrap();
            assert_eq!((column.x, column.z), (x, -3));
        }
        assert!(matches!(decoder.next(), Next::Decoding));
    }

    /// `LevelChunk.replaceBiomes`: the packet's containers go to the
    /// dimension's sections (y 0 to 255 in the Nether), mapped to our biome
    /// table; the blocks stay, and the sections outside the dimension take
    /// its nearest biomes as a chunk packet's do.
    #[test]
    fn chunks_biomes_replace_a_columns_biomes() {
        let nether = DimensionType::vanilla("minecraft:the_nether");
        let format = ChunkFormat::new(&nether, vec![0, 7, 9], 2);
        let mut chunk = Chunk::empty(2, -3);
        chunk.set(3, 70, 4, 5);
        // Wire biome 1 everywhere, with 2 in the lowest and the highest cells.
        let containers: Vec<Paletted> = (0..nether.sections())
            .map(|i| {
                let mut cells = [1; 64];
                if i == 0 {
                    cells[0] = 2;
                }
                if i == nether.sections() - 1 {
                    cells[63] = 2;
                }
                Paletted::from_values(&cells, Wiring::Biomes, 2)
            })
            .collect();
        let data = play::ChunkBiomeData::from_biomes([2, -3], &containers);
        format.replace_biomes(&mut chunk, &data).unwrap();
        assert_eq!(chunk.get(3, 70, 4), 5);
        assert_eq!(chunk.biome(8, 70, 8), 7);
        assert_eq!(chunk.biome(0, 0, 0), 9);
        assert_eq!(chunk.biome(4, 0, 0), 7);
        assert_eq!(chunk.biome(15, 255, 15), 9);
        // Below and above the Nether: its bottom and top cell layers.
        assert_eq!(chunk.biome(0, -64, 0), 9);
        assert_eq!(chunk.biome(15, 319, 15), 9);
        assert_eq!(chunk.biome(0, 319, 0), 7);
        // A buffer short of the dimension's sections changes nothing.
        let short = play::ChunkBiomeData::from_biomes([2, -3], &containers[..3]);
        let mut other = Chunk::empty(2, -3);
        assert!(format.replace_biomes(&mut other, &short).is_err());
        assert_eq!(other.biome(8, 70, 8), 0);
    }

    #[test]
    fn decoder_recovers_after_truncated_packet_and_preserves_fifo_order() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let mixed_states: [i32; 4096] = std::array::from_fn(|i| (i & 3) as i32);
        let mixed_section = Wire {
            non_empty: 3072,
            fluids: 0,
            states: Paletted::from_values(&mixed_states, Wiring::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Wiring::Biomes, 2),
        };
        let truncated = Box::new(C::LevelChunkWithLight(play::LevelChunkWithLight {
            x: 1,
            z: -3,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                // Only 4 of 24 sections so column_with_scratch populates scratch.sections before failing.
                buffer: ChunkData::from_sections(&vec![mixed_section; 4]),
                block_entities: Vec::new(),
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        }));

        let decoder = ChunkDecoder::new();
        assert!(decoder.submit(format.clone(), packet(0, 1)).is_none());
        assert!(decoder.submit(format.clone(), truncated).is_none());
        assert!(decoder.submit(format.clone(), packet(2, 3)).is_none());

        let mut results = Vec::new();
        for _ in 0..3 {
            let (pkt, col) = loop {
                match decoder.next() {
                    Next::Ready(decoded) => break decoded,
                    Next::Decoding => std::thread::yield_now(),
                    Next::Lost => panic!("the decoder stopped"),
                }
            };
            let C::LevelChunkWithLight(p) = *pkt else {
                panic!("not a chunk packet");
            };
            results.push((p.x, col));
        }
        assert_eq!(results[0].0, 0);
        assert_eq!(results[0].1.as_ref().unwrap().get(0, 0, 0), 1);
        assert_eq!(results[1].0, 1);
        assert!(results[1].1.is_err());
        assert_eq!(results[2].0, 2);
        assert_eq!(results[2].1.as_ref().unwrap().get(0, 0, 0), 3);

        // Borrowed light_slices matches the owned mod-point ChunkFormat::light layers.
        let light = LightData {
            sky_mask: vec![0b0000_0010],
            block_mask: vec![0b0000_0100],
            empty_sky_mask: vec![0b0000_0100],
            empty_block_mask: vec![0b0000_0010],
            sky: vec![vec![0x5A; 2048]],
            block: vec![vec![0x3C; 2048]],
        };
        let (owned, _) = format.light(&light);
        let borrowed = format.light_slices(&light);
        assert_eq!(owned.len(), borrowed.len());
        for (a, b) in owned.iter().zip(&borrowed) {
            assert_eq!(a.0.as_deref(), b.0);
            assert_eq!(a.1.as_deref(), b.1);
        }
    }
}
