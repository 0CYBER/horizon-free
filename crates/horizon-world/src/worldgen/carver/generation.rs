//! Biome-selected neighboring carvers and their per-entry legacy seeds.
use super::{CanyonCarver, CarverError, CaveCarver};
use crate::worldgen::{
    DecodeError, Object, anchor::WorldGenerationContext, density::identifier, required,
};
use horizon_core::{
    carving::{CarverOutput, CarvingMask, CarvingMaskError},
    position::ChunkPos,
    random::{LegacyRandom, RandomSource},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// The file beside a worldgen bundle's `carvers.json` that lists the carver
/// ids a content root left out (a mod's own carver type, or load conditions
/// that fail): a biome naming one drops it while no data pack supplies it.
pub const LEFT_OUT: &str = "carvers_left_out.json";

#[derive(Clone, Debug)]
pub enum WorldCarver {
    Cave(CaveCarver),
    Canyon(CanyonCarver),
}
impl WorldCarver {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        match identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?
        .as_str()
        {
            "minecraft:cave" => Ok(Self::Cave(CaveCarver::from_json(json)?)),
            "minecraft:canyon" => Ok(Self::Canyon(CanyonCarver::from_json(json)?)),
            _ => Err(DecodeError::Field("world carver type")),
        }
    }
    pub fn is_start_chunk(&self, random: &mut impl RandomSource) -> bool {
        match self {
            Self::Cave(c) => c.is_start_chunk(random),
            Self::Canyon(c) => c.is_start_chunk(random),
        }
    }
    pub fn carve<O: CarverOutput>(
        &self,
        context: WorldGenerationContext,
        random: &mut impl RandomSource,
        target: ChunkPos,
        origin: ChunkPos,
        output: &mut O,
    ) -> Result<bool, CarverError<O::Error>> {
        match self {
            Self::Cave(c) => c.carve(context, random, target, origin, output),
            Self::Canyon(c) => c.carve(context, random, target, origin, output),
        }
    }
}
/// Immutable named carvers. Inline definitions and direct holder lists are
/// supported; tagged sets and live registry reloads remain separate work.
pub struct CarverCatalog {
    definitions: BTreeMap<String, Arc<WorldCarver>>,
}
impl CarverCatalog {
    pub fn from_json(definitions: &BTreeMap<String, String>) -> Result<Self, DecodeError> {
        if definitions.len() > 8192 {
            return Err(DecodeError::Field("carver registry size"));
        }
        let mut result = BTreeMap::new();
        for (name, json) in definitions {
            let name = identifier(name)?;
            if result
                .insert(name, Arc::new(WorldCarver::from_json(json)?))
                .is_some()
            {
                return Err(DecodeError::Field("duplicate carver name"));
            }
        }
        Ok(Self {
            definitions: result,
        })
    }
    /// Direct reads of the accepted immutable registry, shared with generic level lookups.
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.definitions.contains_key(id)
    }
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.definitions.keys().map(String::as_str)
    }
    pub fn decode_set(&self, json: &str) -> Result<Vec<Arc<WorldCarver>>, DecodeError> {
        self.decode_set_leaving_out(json, &BTreeSet::new())
            .map(|(carvers, _)| carvers)
    }
    /// [`Self::decode_set`], where a named carver this catalog lacks and
    /// `left_out` lists is dropped instead of failing: the carvers, and the
    /// ids dropped.
    pub fn decode_set_leaving_out(
        &self,
        json: &str,
        left_out: &BTreeSet<String>,
    ) -> Result<(Vec<Arc<WorldCarver>>, Vec<String>), DecodeError> {
        use serde_json::value::RawValue;
        let raw: Box<RawValue> = serde_json::from_str(json)?;
        let entries: Vec<Box<RawValue>> = if raw.get().starts_with('[') {
            serde_json::from_str(raw.get())?
        } else {
            vec![raw]
        };
        if entries.len() > 1024 {
            return Err(DecodeError::Field("biome carver count"));
        }
        let (mut carvers, mut dropped) = (Vec::with_capacity(entries.len()), Vec::new());
        for raw in &entries {
            if let Ok(name) = serde_json::from_str::<String>(raw.get()) {
                let name = identifier(&name)?;
                match self.definitions.get(&name) {
                    Some(carver) => carvers.push(carver.clone()),
                    None if left_out.contains(&name) => dropped.push(name),
                    None => return Err(DecodeError::Registry(format!("unknown carver {name}"))),
                }
            } else {
                carvers.push(Arc::new(WorldCarver::from_json(raw.get())?));
            }
        }
        Ok((carvers, dropped))
    }
}
#[derive(Debug)]
pub enum MaskGenerationError<E> {
    Source(E),
    Mask(CarvingMaskError),
    Carver(CarverError<CarvingMaskError>),
    TooManyCarvers,
}
impl<E: std::fmt::Display> std::fmt::Display for MaskGenerationError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(e) => e.fmt(f),
            Self::Mask(e) => e.fmt(f),
            Self::Carver(e) => e.fmt(f),
            Self::TooManyCarvers => f.write_str("biome carver count exceeds 1024"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for MaskGenerationError<E> {}

/// Build the union mask from a 17×17 origin neighborhood, X then Z. The provider
/// selects each origin's ordered biome carvers using its uncached biome resolver
/// at the origin's minimum quart X/Z and quart Y=0. Repeated entries keep their
/// individual ordinal seeds. The caller applies a successfully returned mask.
pub fn generate_carving_mask<'a, E>(
    world_seed: i64,
    target: ChunkPos,
    context: WorldGenerationContext,
    upgrading: bool,
    mut carvers: impl FnMut(ChunkPos) -> Result<&'a [Arc<WorldCarver>], E>,
) -> Result<CarvingMask, MaskGenerationError<E>> {
    let min = context.min_y.wrapping_add(1);
    let max = context
        .min_y
        .wrapping_add(context.height)
        .wrapping_sub(1)
        .wrapping_sub(if upgrading { 0 } else { 7 });
    let mut mask = CarvingMask::new(min, max).map_err(MaskGenerationError::Mask)?;
    for dx in -8..=8 {
        for dz in -8..=8 {
            let origin = ChunkPos::new(target.x.wrapping_add(dx), target.z.wrapping_add(dz));
            let selected = carvers(origin).map_err(MaskGenerationError::Source)?;
            if selected.len() > 1024 {
                return Err(MaskGenerationError::TooManyCarvers);
            }
            for (index, carver) in selected.iter().enumerate() {
                let mut random =
                    large_feature_random(world_seed.wrapping_add(index as i64), origin);
                if carver.is_start_chunk(&mut random) {
                    carver
                        .carve(context, &mut random, target, origin, &mut mask)
                        .map_err(MaskGenerationError::Carver)?;
                }
            }
        }
    }
    Ok(mask)
}
/// The legacy `WorldgenRandom.setLargeFeatureSeed` stream used by carving.
pub fn large_feature_random(seed: i64, origin: ChunkPos) -> LegacyRandom {
    let mut random = LegacyRandom::new(seed);
    let x = random.next_long();
    let z = random.next_long();
    random
        .set_seed(i64::from(origin.x).wrapping_mul(x) ^ i64::from(origin.z).wrapping_mul(z) ^ seed);
    random
}

#[cfg(test)]
mod tests {
    use super::*;
    const CAVE: &str = r#"{"type":"minecraft:cave","probability":0.5,
        "count":{"type":"minecraft:uniform","min_inclusive":0,"max_inclusive":2},
        "y":{"type":"minecraft:uniform","min_inclusive":{"absolute":0},"max_inclusive":{"absolute":64}},
        "thickness":{"type":"minecraft:uniform","min_inclusive":0.5,"max_exclusive":1.5},
        "room_vertical_radius_multiplier":{"type":"minecraft:uniform","min_inclusive":0.5,"max_exclusive":1.0},
        "horizontal_radius_multiplier":{"type":"minecraft:uniform","min_inclusive":0.5,"max_exclusive":1.0},
        "vertical_radius_multiplier":{"type":"minecraft:uniform","min_inclusive":0.5,"max_exclusive":1.0},
        "floor_level":{"type":"minecraft:uniform","min_inclusive":-1.0,"max_exclusive":-0.5}}"#;

    fn catalog(ids: &[&str]) -> CarverCatalog {
        let definitions = ids
            .iter()
            .map(|id| ((*id).to_owned(), CAVE.to_owned()))
            .collect();
        CarverCatalog::from_json(&definitions).unwrap()
    }

    /// A biome's carver a content root left out drops while nothing supplies
    /// it, and is used once a data pack puts a carver under its id.
    #[test]
    fn a_left_out_carver_drops_only_while_no_pack_supplies_it() {
        let left_out = BTreeSet::from(["hollows:vent".to_owned()]);
        let biome = r#"["hollows:deep_cave", "hollows:vent"]"#;
        let (set, dropped) = catalog(&["hollows:deep_cave"])
            .decode_set_leaving_out(biome, &left_out)
            .unwrap();
        assert_eq!((set.len(), dropped), (1, vec!["hollows:vent".to_owned()]));
        let (set, dropped) = catalog(&["hollows:deep_cave", "hollows:vent"])
            .decode_set_leaving_out(biome, &left_out)
            .unwrap();
        assert_eq!((set.len(), dropped.len()), (2, 0));
        // An unknown carver nobody left out still fails, as vanilla's does.
        let other = r#"["hollows:other"]"#;
        assert!(
            catalog(&[])
                .decode_set_leaving_out(other, &left_out)
                .is_err()
        );
    }
}
