//! The sculk vein's water gate. `SculkBehaviour.attemptSpreadVein` and
//! `SculkVeinSpreaderConfig.stateCanBeReplaced` test the *fluid* of the
//! existing state (`FluidState.is(Fluids.WATER)`), so a waterlogged block
//! passes as well as the water block itself. Needs the prepared client
//! resources and is skipped when they are absent.
use super::*;
use crate::chunk::BlockState;
use crate::level::data;
use horizon_core::random::LegacyRandom;
use std::{collections::HashMap, path::PathBuf, sync::OnceLock};

struct Fixture {
    registry: BlockRegistry,
    behaviours: Behaviours,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let root = std::env::var_os("HORIZON_RESOURCES")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/horizon-client")
                        .join(horizon_core::VANILLA_VERSION)
                });
            if !root.join("block-behaviours.json").exists() {
                eprintln!(
                    "skipping sculk vein water tests: no resources at {}",
                    root.display()
                );
                return None;
            }
            let registry = data::load_registry(&root).expect("registry");
            let behaviours = Behaviours::load(&registry, &root).expect("behaviours");
            Some(Fixture {
                registry,
                behaviours,
            })
        })
        .as_ref()
}

struct Level {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    events: Vec<LevelEvent>,
}
impl crate::level::BlockEntityNeighbors for Level {}
impl LevelAccess for Level {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.blocks.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(pos, state).unwrap_or(self.air))
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        0
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
}

struct World {
    f: &'static Fixture,
    level: Level,
    ticks: Ticks,
}
impl World {
    fn new(f: &'static Fixture) -> Self {
        Self {
            level: Level {
                blocks: HashMap::new(),
                air: f.behaviours.air(),
                random: LegacyRandom::new(42),
                events: Vec::new(),
            },
            ticks: Ticks::new(0),
            f,
        }
    }
    fn id(&self, name: &str, props: &[(&str, &str)]) -> BlockStateId {
        let state = BlockState {
            name: format!("minecraft:{name}"),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        self.f
            .registry
            .resolve(&state)
            .unwrap_or_else(|_| panic!("unknown state {name} {props:?}"))
    }
    fn put(&mut self, pos: BlockPos, state: BlockStateId) {
        self.level.blocks.insert(pos, state);
    }
    fn name(&self, pos: BlockPos) -> String {
        let state = self.level.block_state(pos);
        self.f.registry.state(state).unwrap().name.clone()
    }
    fn prop(&self, pos: BlockPos, key: &str) -> String {
        self.f
            .registry
            .state(self.level.block_state(pos))
            .unwrap()
            .properties
            .get(key)
            .cloned()
            .unwrap_or_default()
    }
    /// One `SculkSpreader.ChargeCursor.update` of a cursor on `pos` that
    /// `facings` came with.
    fn update_cursor(&mut self, pos: BlockPos, facings: Vec<Direction>, charge: i32) {
        let mut spreader = sculk::SculkSpreader::default();
        spreader.add_cursor(sculk::ChargeCursor {
            pos,
            charge,
            update_delay: 0,
            decay_delay: 1,
            facings: Some(facings),
        });
        self.f.behaviours.sculk_update_cursors(
            &mut self.level,
            &mut self.ticks,
            &mut spreader,
            pos,
            true,
        );
    }
    fn sounds(&self, name: &str) -> Vec<(BlockPos, f32, f32)> {
        self.level
            .events
            .iter()
            .filter_map(|e| match e {
                LevelEvent::Sound {
                    pos,
                    sound,
                    volume,
                    pitch,
                } if &**sound == name => Some((*pos, *volume, *pitch)),
                _ => None,
            })
            .collect()
    }
}

/// `SculkBehaviour.java:21`: a waterlogged state's fluid is water, so the
/// default behaviour regrows veins over it and plays the spread sound.
#[test]
fn a_charge_regrows_veins_on_a_waterlogged_fence() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let stone = w.id("stone", &[]);
    w.put(BlockPos::new(0, 0, 0), stone);
    // Iron is sturdy but not `#minecraft:sculk_replaceable`, so
    // `SculkVeinBlock.attemptPlaceSculk` leaves it alone; with stone there it
    // would become sculk, take the vein's only face and `onDischarged` would
    // turn the vein back into water.
    w.put(BlockPos::new(0, 2, 0), w.id("iron_block", &[]));
    let fence = w.id("oak_fence", &[("waterlogged", "true")]);
    let pos = BlockPos::new(0, 1, 0);
    w.put(pos, fence);

    w.update_cursor(pos, vec![Direction::Up], 100);

    assert_eq!(
        w.name(pos),
        "minecraft:sculk_vein",
        "the charge regrew a vein over the waterlogged fence"
    );
    assert_eq!(
        w.prop(pos, "up"),
        "true",
        "the vein faces the iron block above"
    );
    assert_eq!(
        w.prop(pos, "waterlogged"),
        "true",
        "the vein is waterlogged, as SculkVeinBlock.regrow keeps the fluid"
    );
    assert_eq!(
        w.sounds("minecraft:block.sculk.spread"),
        vec![(pos, 1.0, 1.0)],
        "the spread sound plays at the vein's position"
    );
}

/// A fluid that is not water still refuses the vein.
#[test]
fn a_charge_refuses_a_vein_on_lava() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let stone = w.id("stone", &[]);
    w.put(BlockPos::new(0, 0, 0), stone);
    w.put(BlockPos::new(0, 2, 0), stone);
    let lava = w.id("lava", &[]);
    let pos = BlockPos::new(0, 1, 0);
    w.put(pos, lava);

    w.update_cursor(pos, vec![Direction::Up], 100);

    assert_eq!(w.name(pos), "minecraft:lava");
    assert!(w.sounds("minecraft:block.sculk.spread").is_empty());
}

/// `SculkVeinBlock.java:167-174`: only a fluid that is not water refuses, so
/// the waterlogged glow lichen (replaceable) takes a vein. `SculkBlock`'s
/// spreader runs over the sculk at `pos` with all three spread types.
#[test]
fn veins_spread_onto_a_waterlogged_glow_lichen() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let stone = w.id("stone", &[]);
    for x in -1..=1 {
        for z in -1..=1 {
            w.put(BlockPos::new(x, 0, z), stone);
        }
    }
    let pos = BlockPos::new(0, 1, 0);
    w.put(pos, w.id("sculk", &[]));
    // North of the sculk: a waterlogged glow lichen with no faces, with the
    // stone floor under it and stone above it to hold the vein's up face.
    let target = BlockPos::new(0, 1, -1);
    w.put(target, w.id("glow_lichen", &[("waterlogged", "true")]));
    w.put(BlockPos::new(0, 2, -1), stone);

    w.update_cursor(pos, Vec::new(), 100);

    assert_eq!(
        w.name(target),
        "minecraft:sculk_vein",
        "the vein spreads onto the waterlogged glow lichen"
    );
    assert_eq!(w.prop(target, "up"), "true");
    assert_eq!(
        w.prop(target, "waterlogged"),
        "true",
        "MultifaceBlock.getStateForPlacement waterlogs the vein"
    );
}
