//! Ordered column obstruction checks, layer truncation and provider writes.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::BlockPredicate,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::{BlockStateProvider, StateProviderError},
    value_provider::IntProvider,
};
use horizon_core::{
    geometry::Direction,
    random::{InvalidBound, RandomSource},
};
use serde_json::value::RawValue;

#[derive(Clone, Debug)]
struct Layer {
    height: IntProvider,
    provider: BlockStateProvider,
}
#[derive(Clone, Debug)]
pub struct BlockColumnFeature {
    layers: Vec<Layer>,
    direction: Direction,
    allowed: BlockPredicate,
    prioritize_tip: bool,
}
#[derive(Debug)]
pub enum BlockColumnError<E> {
    Random(InvalidBound),
    Provider(StateProviderError<E>),
    Query(PlacementQueryError),
    World(E),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for BlockColumnError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::Provider(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("block column work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlockColumnError<E> {}
impl BlockColumnFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        if depth > 128 || *remaining == 0 {
            return Err(DecodeError::Field("block column depth or work"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let values: Vec<Box<RawValue>> = serde_json::from_str(required(&object, "layers")?.get())?;
        let mut layers = Vec::with_capacity(values.len());
        for raw in values {
            let layer: Object = serde_json::from_str(raw.get())?;
            let height = IntProvider::decode(required(&layer, "height")?, depth + 1, remaining)?;
            if height.bounds()[0] < 0 {
                return Err(DecodeError::Field("negative block column height"));
            }
            let (providers, catalog, _) = bindings
                .simple_blocks
                .as_ref()
                .ok_or(DecodeError::Field("block column provider bindings"))?;
            let provider = catalog.provider_with_budget(
                required(&layer, "provider")?.get(),
                providers,
                depth + 1,
                remaining,
            )?;
            layers.push(Layer { height, provider });
        }
        let name: String = serde_json::from_str(required(&object, "direction")?.get())?;
        let direction = match name.as_str() {
            "down" => Direction::Down,
            "up" => Direction::Up,
            "north" => Direction::North,
            "south" => Direction::South,
            "west" => Direction::West,
            "east" => Direction::East,
            _ => return Err(DecodeError::Field("block column direction")),
        };
        let allowed = BlockPredicate::decode_with_blocks(
            required(&object, "allowed_placement")?.get(),
            depth + 1,
            remaining,
            Some(bindings),
        )?;
        let prioritize_tip = serde_json::from_str(required(&object, "prioritize_tip")?.get())?;
        Ok(Self {
            layers,
            direction,
            allowed,
            prioritize_tip,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BlockColumnError<W::Error>> {
        let mut heights = Vec::with_capacity(self.layers.len());
        let mut total = 0_i32;
        for layer in &self.layers {
            let height = layer
                .height
                .sample(random)
                .map_err(BlockColumnError::Random)?;
            total = total.wrapping_add(height);
            heights.push(height);
        }
        if total == 0 {
            return Ok(false);
        }
        let step = self.direction.normal();
        let advance = |p: [i32; 3]| std::array::from_fn(|i| p[i].wrapping_add(step[i]));
        let mut query_position = advance(origin);
        let mut budget = 1_048_576_usize;
        for index in 0..total {
            spend(&mut budget)?;
            if !self
                .allowed
                .test(world, query_position)
                .map_err(BlockColumnError::Query)?
            {
                truncate(&mut heights, total, index, self.prioritize_tip);
                break;
            }
            query_position = advance(query_position);
        }
        let mut position = origin;
        for (layer, height) in self.layers.iter().zip(heights) {
            for _ in 0..height {
                spend(&mut budget)?;
                let state = layer
                    .provider
                    .get_state(world, random, position)
                    .map_err(BlockColumnError::Provider)?;
                world
                    .set_block(position, state, 2)
                    .map_err(BlockColumnError::World)?;
                position = advance(position);
            }
        }
        Ok(true)
    }
}
fn spend<E>(budget: &mut usize) -> Result<(), BlockColumnError<E>> {
    if *budget == 0 {
        return Err(BlockColumnError::WorkLimit);
    }
    *budget -= 1;
    Ok(())
}
fn truncate(heights: &mut [i32], total: i32, allowed: i32, prioritize_tip: bool) {
    let mut remove = total.wrapping_sub(allowed);
    for offset in 0..heights.len() {
        if remove <= 0 {
            break;
        }
        let index = if prioritize_tip {
            offset
        } else {
            heights.len() - 1 - offset
        };
        let count = heights[index].min(remove);
        remove = remove.wrapping_sub(count);
        heights[index] = heights[index].wrapping_sub(count);
    }
}

#[cfg(test)]
mod ceiling_tests {
    use super::*;
    use crate::{
        block_placement::BlockPlacementBindings,
        block_properties::BlockPropertiesRegistry,
        collision::CachedCollisionRegistry,
        registry::BlockRegistry,
        worldgen::state_provider::{StateProviderBindings, StateProviderCatalog},
    };
    use horizon_core::nbt::Tag;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn column_layer_ceiling() {
        let names = [
            "minecraft:air",
            "minecraft:cave_air",
            "minecraft:void_air",
            "minecraft:stone",
        ];
        let mut states: Vec<Tag> = names
            .iter()
            .map(|name| Tag::String((*name).into()))
            .collect();
        // Placement binds all 2 * 3^4 moss transitions, even for columns that
        // only use stone. These are synthetic states, not prepared game data.
        for bottom in ["false", "true"] {
            for code in 0..81 {
                let mut n = code;
                let mut props = BTreeMap::from([("bottom".into(), Tag::String(bottom.into()))]);
                for side in ["north", "east", "south", "west"] {
                    props.insert(
                        side.into(),
                        Tag::String(["none", "low", "tall"][n % 3].into()),
                    );
                    n /= 3;
                }
                states.push(Tag::compound([
                    ("id", Tag::String("minecraft:pale_moss_carpet".into())),
                    ("properties", Tag::Compound(props)),
                ]));
            }
        }
        let mut defaults: BTreeMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(i, name)| ((*name).into(), Tag::Int(i as i32)))
            .collect();
        defaults.insert("minecraft:pale_moss_carpet".into(), Tag::Int(4));
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(states.clone())),
            ("defaults", Tag::Compound(defaults)),
        ]))
        .unwrap();
        let empty = Tag::compound([
            ("x", Tag::LongArray(vec![0])),
            ("y", Tag::LongArray(vec![0])),
            ("z", Tag::LongArray(vec![0])),
            ("occupied", Tag::ByteArray(vec![])),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![0; 3])),
        ]);
        let collision_tag = Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(states.clone())),
            ("shapes", Tag::List(vec![empty])),
            ("cached_shapes", Tag::IntArray(vec![0; states.len()])),
        ]);
        let collisions = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
        let blocks = names
            .iter()
            .copied()
            .chain(["minecraft:pale_moss_carpet"])
            .map(|name| (name.into(), Tag::Byte(0)))
            .collect();
        let properties = BlockPropertiesRegistry::from_nbt(
            &Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states)),
                ("blocks", Tag::Compound(blocks)),
                ("occlusion", collision_tag),
            ]),
            &collisions,
        )
        .unwrap();
        let providers =
            StateProviderBindings::new(&registry, BTreeSet::new(), &BTreeMap::new()).unwrap();
        let catalog = StateProviderCatalog::from_resources(&BTreeMap::new()).unwrap();
        let placement =
            BlockPlacementBindings::new(&registry, &properties, &BTreeSet::new(), &BTreeSet::new())
                .unwrap();
        let bindings = FeatureBlockBindings::new(&registry, BTreeSet::new())
            .unwrap()
            .with_state_providers(providers, catalog, placement)
            .unwrap();
        let layer = r#"{"height":0,"provider":{"id":"minecraft:stone"}}"#;
        let json = format!(
            r#"{{"layers":[{}],"direction":"up","allowed_placement":{{"type":"true"}},"prioritize_tip":false}}"#,
            vec![layer; 8193].join(",")
        );
        let feature = BlockColumnFeature::from_json(&json, &bindings).unwrap();
        assert_eq!(feature.layers.len(), 8193);
        assert_eq!(feature.layers.last().unwrap().height.bounds(), [0, 0]);
    }
}
