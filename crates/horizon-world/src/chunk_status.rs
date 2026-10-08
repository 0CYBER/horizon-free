//! Pinned generation-status identities and ordering. A status describes progress;
//! assigning one does not execute its generation task or implement lighting.
use crate::{compression::invalid, heightmap::HeightmapType, nbt::Tag};
use std::io;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ChunkStatus {
    #[default]
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Terrain,
    Features,
    InitializeLight,
    Light,
    Spawn,
    Full,
}
impl ChunkStatus {
    pub const MAX_STRUCTURE_DISTANCE: i32 = 8;
    pub const FINAL_HEIGHTMAPS: [HeightmapType; 4] = [
        HeightmapType::WorldSurface,
        HeightmapType::OceanFloor,
        HeightmapType::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves,
    ];
    pub const ALL: [Self; 10] = [
        Self::Empty,
        Self::StructureStarts,
        Self::StructureReferences,
        Self::Biomes,
        Self::Terrain,
        Self::Features,
        Self::InitializeLight,
        Self::Light,
        Self::Spawn,
        Self::Full,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn parent(self) -> Self {
        Self::ALL[self.index().saturating_sub(1)]
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Empty => "minecraft:empty",
            Self::StructureStarts => "minecraft:structure_starts",
            Self::StructureReferences => "minecraft:structure_references",
            Self::Biomes => "minecraft:biomes",
            Self::Terrain => "minecraft:terrain",
            Self::Features => "minecraft:features",
            Self::InitializeLight => "minecraft:initialize_light",
            Self::Light => "minecraft:light",
            Self::Spawn => "minecraft:spawn",
            Self::Full => "minecraft:full",
        }
    }
    /// Registry-codec lookup: unknown or invalid identifiers are errors.
    pub fn parse(name: &str) -> io::Result<Self> {
        let path = match name.split_once(':') {
            Some(("" | "minecraft", path)) => path,
            None => name,
            _ => return Err(invalid("unknown chunk status namespace")),
        };
        Self::ALL
            .into_iter()
            .find(|status| status.name().strip_prefix("minecraft:") == Some(path))
            .ok_or_else(|| invalid("unknown chunk status"))
    }
    /// Direct native byName lookup uses the registry's empty-status fallback.
    pub fn by_name(name: &str) -> Self {
        Self::parse(name).unwrap_or_default()
    }
    pub fn from_nbt(tag: &Tag) -> io::Result<Self> {
        let Tag::String(name) = tag else {
            return Err(invalid("chunk status is not a string"));
        };
        Self::parse(
            &name
                .to_string()
                .map_err(|error| invalid(error.to_string()))?,
        )
    }
    pub fn to_nbt(self) -> Tag {
        Tag::String(self.name().into())
    }
    pub fn is_level_chunk(self) -> bool {
        self == Self::Full
    }
    pub fn heightmaps(self) -> &'static [HeightmapType] {
        if self >= Self::Terrain {
            &Self::FINAL_HEIGHTMAPS
        } else {
            &[HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg]
        }
    }
}
