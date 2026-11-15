use super::*;
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use horizon_world::{
    block_entity_push::BlockPushWorld,
    block_landing::{
        BlockLandingWorld, DamageSourceOwner, LandingDamage, LandingEntityKind, LandingEvent,
        LandingSound,
    },
    entity_obstruction::EntityIdentity,
    level::{Behaviours, LevelAccess, Ticks},
    movement_properties::MovementProperties,
    registry::{BlockStateId, BlockTableFile},
};
use std::{collections::HashMap, convert::Infallible, result::Result};

struct World {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    time: i64,
}
impl horizon_world::level::BlockEntityNeighbors for World {}
impl LevelAccess for World {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.blocks.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(pos, state).unwrap_or(self.air))
    }
    fn is_loaded(&self, _: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
}
impl BlockPushWorld for World {
    type Error = Infallible;
    fn collision_shape(&mut self, _: BlockStateId, _: BlockPos) -> Result<VoxelShape, Self::Error> {
        Ok(VoxelShape::empty())
    }
    fn entities(&mut self, _: Aabb) -> Result<Vec<EntityIdentity>, Self::Error> {
        Ok(Vec::new())
    }
    fn entity_bounds(&self, _: EntityIdentity) -> Aabb {
        unreachable!()
    }
    fn teleport_relative(&mut self, _: EntityIdentity, _: Vec3) -> Result<(), Self::Error> {
        unreachable!()
    }
}
impl BlockLandingWorld for World {
    type DamageSource = ();
    type Sound = ();
    fn is_server_level(&self) -> bool {
        true
    }
    fn is_client_side(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn random_float(&mut self) -> Result<f32, Self::Error> {
        Ok(0.0)
    }
    fn random_int(&mut self, _: i32) -> Result<i32, Self::Error> {
        Ok(0)
    }
    fn under_spawn_protection(&mut self, _: BlockPos) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn mob_griefing(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn entity_dimensions(&self) -> (f32, f32) {
        (1.0, 2.0)
    }
    fn suppressing_bounce(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn damage_source(&mut self, _: DamageSourceOwner, _: LandingDamage) -> Result<(), Self::Error> {
        Ok(())
    }
    fn cause_fall_damage(&mut self, _: f64, _: f32, _: ()) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn known_sound(&self, _: LandingSound) {}
    fn living_fall_sound(&mut self, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn entity_sound(&mut self, _: (), _: f32, _: f32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn block_sound(&mut self, _: BlockPos, _: (), _: f32, _: f32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn broadcast_entity_event(&mut self, _: u8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn destroy_block(&mut self, _: BlockPos, _: bool) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn set_block(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        _: u32,
    ) -> Result<bool, Self::Error> {
        self.blocks.insert(pos, state);
        Ok(true)
    }
    fn block_event(
        &mut self,
        _: LandingEvent,
        _: BlockPos,
        _: BlockStateId,
        _: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn level_event(&mut self, _: i32, _: BlockPos, _: i32) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn unknown_farmland_base_is_a_typed_load_error() {
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-farmland-unknown-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch,
        &[
            (
                "horizon-mod.toml",
                "id = \"soil\"\nname = \"Soil\"\nversion = \"0.1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
            ),
            (
                "data/soil/horizon/block/farm.json",
                r#"{"template":"minecraft:farmland","base_block":"minecraft:missing_farmland_base"}"#,
            ),
        ],
    );
    let error = Plan::new(&base, &[m]).err().unwrap();
    assert!(error.downcast_ref::<UnknownBaseBlock>().is_some());
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "daily"]
fn farmland_base_reaches_drying_cover_and_trample() {
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-farmland-base-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            (
                "horizon-mod.toml",
                "id = \"soil\"\nname = \"Soil\"\nversion = \"0.1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
            ),
            (
                "data/soil/horizon/block/farm.json",
                r#"{"template":"minecraft:farmland","base_block":"soil:base"}"#,
            ),
            (
                "data/soil/horizon/block/base.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/soil/horizon/block/default.json",
                r#"{"template":"minecraft:farmland"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let behaviours = Behaviours::load(registry, &root).unwrap();
    let table = BlockTableFile::open(&root.join("movement-properties.nbt"), registry).unwrap();
    let properties = MovementProperties::from_table(table.table("movement").unwrap()).unwrap();
    let landing = crate::landing::load(&root, &properties).unwrap();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let dirt = registry.default_state("minecraft:dirt").unwrap();
    let mod_base = registry.default_state("soil:base").unwrap();
    let farm = registry.default_state("soil:farm").unwrap();
    let default = registry.default_state("soil:default").unwrap();
    let pos = BlockPos::new(0, 64, 0);
    let mut world = World {
        blocks: HashMap::new(),
        air,
        random: LegacyRandom::new(0),
        time: 0,
    };
    let mut ticks = Ticks::new(0);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    for (state, expected) in [(farm, mod_base), (default, dirt)] {
        world.blocks.clear();
        world.blocks.insert(pos, state);
        behaviours.random_tick(&mut world, &mut ticks, pos);
        assert_eq!(world.block_state(pos), expected, "drying");
        world.blocks.clear();
        world.blocks.insert(pos, state);
        behaviours.set_block(&mut world, &mut ticks, BlockPos::new(0, 65, 0), stone, 3);
        world.time += 1;
        behaviours.tick_scheduled(&mut world, &mut ticks);
        assert_eq!(world.block_state(pos), expected, "solid block above");
        world.blocks.clear();
        world.blocks.insert(pos, state);
        landing
            .fall_on(&mut world, state, pos, LandingEntityKind::Player, 2.0)
            .unwrap();
        assert_eq!(world.block_state(pos), expected, "trampling");
    }
    fs::remove_dir_all(scratch).unwrap();
}
