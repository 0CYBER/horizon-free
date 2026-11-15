//! Owned placement and loot data; none of these snapshots borrows an engine context.
use crate::{BlockPos, Direction, ItemKey};
use horizon_core::nbt::Tag;

/// Inputs to selecting a block's placement state.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlacementContext {
    /// Resolved destination of the block being placed.
    pub pos: BlockPos,
    /// Position of the clicked block.
    pub hit_pos: BlockPos,
    /// Clicked face.
    pub face: Direction,
    /// World-space click location.
    pub hit: [f64; 3],
    /// Whether the click originated inside the block's shape.
    pub inside: bool,
    /// Placer yaw in degrees.
    pub yaw: f32,
    /// Placer pitch in degrees.
    pub pitch: f32,
    /// Whether secondary use is active.
    pub sneaking: bool,
    /// The resolved item in the placing hand; `None` for an empty hand.
    pub item: Option<ItemKey>,
    /// Whether the clicked block itself is replaced.
    pub replace_clicked: bool,
}

impl PlacementContext {
    /// A placement at `pos`, with a click on `face` and an optional held item.
    /// The click starts at the block origin, with zero view angles and no flags set.
    pub fn new(pos: BlockPos, face: Direction, item: Option<ItemKey>) -> Self {
        Self {
            pos,
            hit_pos: pos,
            face,
            hit: [f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)],
            inside: false,
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
            item,
            replace_clicked: false,
        }
    }
}

/// A loot function's intermediate item candidate, before inventory materialization.
/// Counts can be non-positive while loot functions run; unlike `ItemStack`, this
/// is a roll's intermediate data and does not represent an occupied inventory slot.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootStack {
    /// Resolved candidate item.
    pub item: ItemKey,
    /// Intermediate count; a non-positive result produces no item.
    pub count: i32,
    /// Intermediate damage value.
    pub damage: i32,
    /// Enchantment identifiers and levels in loot-function order.
    pub enchantments: Vec<(String, i32)>,
    /// Potion identifier.
    pub potion: Option<String>,
    /// Whether to create the map ID component after the roll.
    pub map: bool,
    /// A deferred exploration-map search.
    pub exploration: Option<Box<ExplorationMap>>,
    /// Ominous bottle amplifier.
    pub ominous_amplifier: Option<i32>,
    /// Suspicious stew effect identifiers and durations in ticks.
    pub stew_effects: Vec<(String, i32)>,
    /// Custom or item name components, still unresolved styled trees.
    pub names: Vec<(String, crate::text::Text)>,
    /// Ordered component patches; `None` removes a component.
    pub components: Vec<(String, Option<horizon_core::nbt::Tag>)>,
}

/// A deferred exploration-map search requested by a loot function.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ExplorationMap {
    /// Destination structure tag, without `#`.
    pub destination: String,
    /// Map decoration identifier.
    pub decoration: String,
    /// Map zoom.
    pub zoom: u8,
    /// Search radius in chunks.
    pub search_radius: i32,
    /// Whether to skip previously discovered structures.
    pub skip_existing_chunks: bool,
}

/// An owned view of the parameters supplied to a mod's loot kind.
/// The game fills it only when that kind has subscribed listeners.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootContext {
    /// Queried root id, written after all nested rolls, before global modifiers (arismoko/horizon#1270).
    /// The first id wins; nested rolls never write it. Unset is `UNKNOWN_LOOT_TABLE`.
    pub queried_table_id: String,
    /// Stack whose data is queried, unchanged through nested rolls; absent in existing callers.
    pub queried_stack: Option<crate::ItemStack>,
    /// Entity named by the table, if any.
    pub entity: Option<LootEntity>,
    /// Killing damage, if any.
    pub damage: Option<LootDamage>,
    /// Player whose kill produced this roll, if any.
    pub player: Option<LootEntity>,
    /// Tool used for the roll, if any.
    pub tool: Option<LootTool>,
    /// Block identifier for a block-drop roll.
    pub block: Option<String>,
    /// Luck applied to this roll.
    pub luck: f32,
    /// Biome identifier, when available.
    pub biome: Option<String>,
    /// Seed drawn by the engine only when the mod's kind participates.
    pub seed: i64,
    /// Origin, block entity and level facts at the moment of the roll.
    pub place: Option<LootPlace>,
}

/// Tool facts supplied by the item engine to a loot roll.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootTool {
    /// Resolved tool item.
    pub item: ItemKey,
    /// Enchantment identifiers and levels.
    pub enchantments: Vec<(String, i32)>,
}

/// Entity predicate facts captured for a loot roll, not a live entity handle.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootEntity {
    /// Entity type identifier.
    pub kind: String,
    /// Whether the entity is living.
    pub living: bool,
    /// Whether the entity is on fire.
    pub on_fire: bool,
    /// Whether the entity is a baby.
    pub baby: bool,
    /// Its vehicle at the moment of the roll.
    pub vehicle: Option<Box<LootEntity>>,
    /// Cube mob size, if applicable.
    pub cube_size: Option<i32>,
    /// Sheep colour and sheared state, if applicable.
    pub sheep: Option<(String, bool)>,
    /// Raider captain state, if applicable.
    pub captain: Option<bool>,
    /// Frog variant identifier, if applicable.
    pub frog_variant: Option<String>,
    /// Chicken variant identifier, if applicable.
    pub chicken_variant: Option<String>,
    /// Charged creeper state.
    pub powered: bool,
    /// Equipment slot names and their intermediate loot candidates.
    pub equipment: Vec<(String, LootStack)>,
}

/// Damage-source facts captured for a loot roll.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootDamage {
    /// Damage type identifier.
    pub kind: String,
    /// Attacker, such as a projectile's shooter.
    pub source: Option<LootEntity>,
    /// Direct attacker, such as the projectile itself.
    pub direct: Option<LootEntity>,
}

/// Place facts captured around a loot roll.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootPlace {
    /// World-space origin.
    pub origin: Option<[f64; 3]>,
    /// Block entity saved before removal, with its full metadata.
    pub block_entity: Option<Tag>,
    /// Level facts, when available.
    pub level: Option<LootLevel>,
}

/// Level facts captured around a loot roll.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LootLevel {
    /// Dimension identifier, including custom dimensions.
    pub dimension: String,
    /// Elapsed game ticks.
    pub game_time: i64,
    /// Day time; the time of day is its remainder modulo 24,000.
    pub day_time: i64,
    /// Whether it is raining.
    pub raining: bool,
    /// Whether it is thundering.
    pub thundering: bool,
}

impl LootStack {
    /// A candidate with signed count, before inventory materialization.
    pub fn new(item: ItemKey, count: i32) -> Self {
        Self {
            item,
            count,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: false,
            exploration: None,
            ominous_amplifier: None,
            stew_effects: Vec::new(),
            names: Vec::new(),
            components: Vec::new(),
        }
    }
}
impl LootContext {
    /// An empty roll context with its captured random seed.
    pub fn new(seed: i64) -> Self {
        Self {
            queried_table_id: horizon_core::UNKNOWN_LOOT_TABLE.into(),
            queried_stack: None,
            entity: None,
            damage: None,
            player: None,
            tool: None,
            block: None,
            luck: 0.0,
            biome: None,
            seed,
            place: None,
        }
    }
}
impl LootTool {
    /// Tool facts for a resolved item.
    pub fn new(item: ItemKey, enchantments: Vec<(String, i32)>) -> Self {
        Self { item, enchantments }
    }
}
impl ExplorationMap {
    /// A deferred vanilla exploration-map request, without running its search.
    pub fn new(
        destination: String,
        decoration: String,
        zoom: u8,
        search_radius: i32,
        skip_existing_chunks: bool,
    ) -> Self {
        Self {
            destination,
            decoration,
            zoom,
            search_radius,
            skip_existing_chunks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loot_snapshot_retains_unknown_patch_removal_and_order() {
        let patches = vec![
            ("test:unknown".to_owned(), Some(Tag::Int(42))),
            ("minecraft:damage".to_owned(), None),
            ("test:unknown".to_owned(), Some(Tag::String("later".into()))),
        ];
        let stack = LootStack {
            item: ItemKey(0),
            count: -1,
            damage: 0,
            enchantments: vec![],
            potion: None,
            map: false,
            exploration: None,
            ominous_amplifier: None,
            stew_effects: vec![],
            names: vec![],
            components: patches,
        };
        assert_eq!(
            stack.components[0],
            ("test:unknown".into(), Some(Tag::Int(42)))
        );
        assert_eq!(stack.components[1], ("minecraft:damage".into(), None));
        assert_eq!(
            stack.components[2],
            ("test:unknown".into(), Some(Tag::String("later".into())))
        );
    }
}
