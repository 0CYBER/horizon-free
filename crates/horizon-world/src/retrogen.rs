//! Below-zero upgrade metadata and resident block transformations.
//! Generation scheduling remains owned by the caller.
use crate::{chunk_status::ChunkStatus, compression::invalid, nbt::Tag};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BelowZeroRetrogen {
    target: ChunkStatus,
    missing_bedrock: Vec<u64>,
}
impl BelowZeroRetrogen {
    pub fn new(target: ChunkStatus, mut missing_bedrock: Vec<u64>) -> io::Result<Self> {
        if target == ChunkStatus::Empty {
            return Err(invalid("retrogen target status cannot be empty"));
        }
        while missing_bedrock.last() == Some(&0) {
            missing_bedrock.pop();
        }
        Ok(Self {
            target,
            missing_bedrock,
        })
    }
    pub fn target_status(&self) -> ChunkStatus {
        self.target
    }
    pub fn has_bedrock_holes(&self) -> bool {
        !self.missing_bedrock.is_empty()
    }
    pub fn has_bedrock_hole(&self, x: i32, z: i32) -> bool {
        let index = ((z & 15) * 16 + (x & 15)) as usize;
        self.missing_bedrock
            .get(index / 64)
            .is_some_and(|word| word & (1_u64 << (index % 64)) != 0)
    }
    pub fn from_nbt(tag: &Tag) -> io::Result<Self> {
        if !matches!(tag, Tag::Compound(_)) {
            return Err(invalid("retrogen is not a compound"));
        }
        let target = ChunkStatus::from_nbt(
            tag.get("target_status")
                .ok_or_else(|| invalid("missing retrogen target status"))?,
        )?;
        // The native optional field is lenient: an invalid stream is absent.
        let words = tag
            .get("missing_bedrock")
            .and_then(long_stream)
            .unwrap_or_default();
        Self::new(target, words)
    }
    pub fn to_nbt(&self) -> Tag {
        let mut fields = vec![("target_status", self.target.to_nbt())];
        if self.has_bedrock_holes() {
            fields.push((
                "missing_bedrock",
                Tag::LongArray(
                    self.missing_bedrock
                        .iter()
                        .map(|word| *word as i64)
                        .collect(),
                ),
            ));
        }
        Tag::compound(fields)
    }
}
fn long_stream(tag: &Tag) -> Option<Vec<u64>> {
    match tag {
        Tag::LongArray(words) => Some(words.iter().map(|v| *v as u64).collect()),
        Tag::IntArray(words) => Some(words.iter().map(|v| i64::from(*v) as u64).collect()),
        Tag::ByteArray(words) => Some(words.iter().map(|v| i64::from(*v as i8) as u64).collect()),
        Tag::List(words) => words
            .iter()
            .map(|word| {
                Some(match word {
                    Tag::Byte(v) => i64::from(*v) as u64,
                    Tag::Short(v) => i64::from(*v) as u64,
                    Tag::Int(v) => i64::from(*v) as u64,
                    Tag::Long(v) => *v as u64,
                    Tag::Float(v) => (f32::from_bits(*v) as i64) as u64,
                    Tag::Double(v) => (f64::from_bits(*v) as i64) as u64,
                    _ => return None,
                })
            })
            .collect(),
        _ => None,
    }
}

impl BelowZeroRetrogen {
    /// Replace the legacy five-layer bedrock band in a resident proto-chunk.
    /// Local X/Z positions and native X-fastest traversal preserve write order.
    pub fn replace_old_bedrock<B: crate::worldgen::terrain::TerrainBlockStates>(
        chunk: &mut crate::worldgen::terrain::TerrainChunk<'_, B>,
    ) -> Result<(), crate::worldgen::terrain::TerrainError> {
        use crate::worldgen::terrain::TerrainError;
        let bedrock = chunk
            .registry()
            .default_state("minecraft:bedrock")
            .ok_or(TerrainError::InvalidState)?;
        let deepslate = chunk
            .registry()
            .default_state("minecraft:deepslate")
            .ok_or(TerrainError::InvalidState)?;
        for z in 0..16 {
            for y in 0..5 {
                for x in 0..16 {
                    if chunk.same_block(chunk.block(x, y, z), bedrock) {
                        chunk.set_block_worldgen(x, y, z, deepslate)?;
                    }
                }
            }
        }
        Ok(())
    }
    /// Clear marked columns through the supplied chunk's current generation
    /// height. The installed upgrade and this mask need not be the same object.
    pub fn apply_bedrock_mask<B: crate::worldgen::terrain::TerrainBlockStates>(
        &self,
        chunk: &mut crate::worldgen::terrain::TerrainChunk<'_, B>,
    ) -> Result<(), crate::worldgen::terrain::TerrainError> {
        use crate::worldgen::terrain::TerrainError;
        let air = chunk
            .registry()
            .default_state("minecraft:air")
            .ok_or(TerrainError::MissingAir)?;
        let (min_y, height) = chunk.height_for_generation();
        let max_y = min_y.wrapping_add(height).wrapping_sub(1);
        for x in 0..16 {
            for z in 0..16 {
                if self.has_bedrock_hole(x, z) {
                    for y in min_y..=max_y {
                        chunk.set_block_worldgen(x, y, z, air)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// The native decoration decision is captured when the resolver is created.
/// Removing upgrade metadata afterwards does not unwrap an existing resolver.
/// Candidate sampling runs before deciding whether to read an old column biome.
pub struct RetrogenBiomeResolver<F> {
    upgrading: bool,
    resolve: F,
}
impl<F> RetrogenBiomeResolver<F> {
    pub fn new<T, E>(upgrading: bool, resolve: F) -> Self
    where
        F: FnMut([i32; 3]) -> Result<T, E>,
    {
        Self { upgrading, resolve }
    }
    pub fn sample<T, E>(
        &mut self,
        quart: [i32; 3],
        biome_key: impl FnOnce(&T) -> Option<&str>,
        old_biome: impl FnOnce([i32; 3]) -> Result<T, E>,
    ) -> Result<T, E>
    where
        F: FnMut([i32; 3]) -> Result<T, E>,
    {
        let candidate = (self.resolve)(quart)?;
        if !self.upgrading
            || matches!(
                biome_key(&candidate),
                Some("minecraft:lush_caves" | "minecraft:dripstone_caves" | "minecraft:deep_dark")
            )
        {
            Ok(candidate)
        } else {
            old_biome([quart[0], 0, quart[2]])
        }
    }
}
