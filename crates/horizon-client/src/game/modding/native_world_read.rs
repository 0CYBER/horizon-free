//! Read-only block, tag and fluid queries for native mods.
use super::native_level_signals::ReadLevel;
use super::{Dim, Game};
use horizon_client::{items::Items, world::World};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{
    chunk::BlockState,
    fluid::{FluidHeightWorld, FluidKind, FluidState as EngineFluidState},
    level::{Behaviours, fluid_can_be_replaced_with, fluid_tick_delay},
    registry::{BlockRegistry, BlockStateId},
};
use mod_api::BlockKey;
use mod_api::{FluidKey, FluidState, LevelRef, LifecycleContext, UnresolvedFluid};

/// Why a server block cannot be read.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NativeBlockReadError {
    /// The context is stale or the call is outside its server tick hook.
    InactiveContext,
    /// The dimension is unknown or is not currently open on the server.
    UnavailableDimension,
    OutOfBounds,
    Unloaded,
    InvalidState,
}

thread_local! {
    // Like the key-to-state mapping, this snapshot belongs to the thread resolving the world.
    static BLOCK_TAGS: std::cell::RefCell<std::collections::HashMap<String, std::collections::HashSet<BlockKey>>> = Default::default();
}

/// Bind the world's merged tags alongside its block mappings, including the resource-only case.
/// Never cache on Items: its string tables also serve tools and client network snapshots.
pub(super) fn prepare_block_tags(
    tags: &std::collections::HashMap<String, std::collections::HashSet<String>>,
) -> std::collections::HashMap<String, std::collections::HashSet<BlockKey>> {
    tags.iter()
        .map(|(tag, members)| {
            let mut set = std::collections::HashSet::<BlockKey>::default();
            for name in members {
                if let Some(block) = mod_api::__private::block_key(name) {
                    set.insert(block);
                }
            }
            (tag.clone(), set)
        })
        .collect()
}

pub(super) fn install_block_tags(
    tags: std::collections::HashMap<String, std::collections::HashSet<BlockKey>>,
) {
    BLOCK_TAGS.with(|snapshot| *snapshot.borrow_mut() = tags);
}

#[cfg(test)]
pub(in crate::game) fn installed_block_tag_snapshot()
-> std::collections::HashMap<String, std::collections::HashSet<BlockKey>> {
    BLOCK_TAGS.with(|snapshot| snapshot.borrow().clone())
}

/// One availability check, one tag-name lookup and one API key membership test.
/// An unavailable owning-thread mapping is not a missing tag.
pub(super) fn block_in_tag(block: BlockKey, tag: &str) -> Option<bool> {
    super::api_values::default_state(block)?;
    let tag = horizon_client::command::args::normalized(tag);
    Some(BLOCK_TAGS.with(|snapshot| {
        snapshot
            .borrow()
            .get(tag.as_ref())
            .is_some_and(|set| set.contains(&block))
    }))
}

/// `BlockState.getFluidState`. None for an id no state has.
pub(super) fn state_fluid(
    behaviours: &Behaviours,
    registry: &BlockRegistry,
    state: BlockStateId,
) -> Option<FluidState> {
    registry.state(state)?;
    api_fluid_state(behaviours.fluid(state))
}

/// Convert the engine's state using the load-owned kind-to-key mapping.
pub(super) fn api_fluid_state(state: EngineFluidState) -> Option<FluidState> {
    Some(mod_api::__private::fluid_state(
        match state.kind().vanilla_index() {
            Some(index) => mod_api::__private::world_fluid_key(usize::from(index))?,
            None => mod_api::__private::fluid_key(state.kind().name())?,
        },
        state.amount(),
        state.falling(),
        state.is_source(),
    ))
}

/// `FluidState.is(TagKey)` over the merged fluid tags. The empty fluid is in none.
pub(super) fn fluid_in_tag(items: &Items, fluid: &str, tag: &str) -> bool {
    items.fluid_in_tag(
        &horizon_client::command::args::normalized(tag),
        &horizon_client::command::args::normalized(fluid),
    )
}

/// `FluidState.getShape`'s one box for `fluid` with `above` over it: up to a full block under
/// the same fluid (`FluidState.getHeight`), else to the fluid's own height. None for no fluid.
fn fluid_shape(fluid: EngineFluidState, above: Option<EngineFluidState>) -> Option<[f64; 6]> {
    if fluid.kind() == FluidKind::Empty {
        return None;
    }
    let height = if above.is_some_and(|above| above.kind().same_family(fluid.kind())) {
        1.0
    } else {
        f64::from(fluid.own_height())
    };
    Some([0.0, 0.0, 0.0, 1.0, height, 1.0])
}

/// The fluid a registry id names, with or without `minecraft:`.
fn fluid_kind(fluid: &str) -> Option<FluidKind> {
    FluidKind::from_name(&horizon_client::command::args::normalized(fluid))
}

/// The world fluid a registered key names. None for a tank-only fluid, which has no world
/// form, and for a key the registry doesn't hold.
pub(super) fn engine_fluid_kind(fluid: FluidKey) -> Option<FluidKind> {
    FluidKind::from_name(mod_api::__private::fluid_id(fluid)?)
}

/// The blocks around one position of a level, as `FluidState.getHeight` reads them: an unloaded
/// block is empty and has no sturdy underside.
struct FluidColumn<'a, 'g> {
    game: &'a Game<'g>,
    level: LevelRef,
}
impl FluidColumn<'_, '_> {
    fn fluid_at(&self, pos: [i32; 3]) -> EngineFluidState {
        self.game
            .level_block_state(self.level, pos)
            .map_or(EngineFluidState::EMPTY, |state| {
                self.game.res.behaviours().fluid(state)
            })
    }
}
impl FluidHeightWorld for FluidColumn<'_, '_> {
    type Error = std::convert::Infallible;
    fn fluid(&mut self, pos: [i32; 3]) -> Result<EngineFluidState, Self::Error> {
        Ok(self.fluid_at(pos))
    }
    fn sturdy_down(&mut self, pos: [i32; 3]) -> Result<bool, Self::Error> {
        Ok(self
            .game
            .level_block_state(self.level, pos)
            .is_some_and(|state| {
                self.game
                    .res
                    .behaviours()
                    .is_face_sturdy(state, Direction::Down)
            }))
    }
}

impl Game<'_> {
    /// The saved name for an engine-issued unresolved fluid.
    pub(super) fn unresolved_fluid_id(&self, fluid: UnresolvedFluid) -> String {
        mod_api::__private::unresolved_fluid_name(fluid)
    }

    /// The tag tables mods read: on a client the tags its server sent, once they are installed
    /// (`install_remote_tags`), elsewhere the merged tables, once the world has built them (the
    /// world lock is installed after they are built). None before that.
    pub(super) fn native_tag_items(&self) -> Option<&Items> {
        if self.options.execution_role.is_client() {
            return self.remote.as_ref()?.tags.as_ref();
        }
        self.session_lock.is_some().then(|| self.items())
    }

    /// `items`' block tags, over this game's resolved block keys.
    pub(super) fn block_tag_sets(
        &self,
        items: &Items,
    ) -> std::collections::HashMap<String, std::collections::HashSet<BlockKey>> {
        prepare_block_tags(&items.block_tags)
    }

    /// Whether the resolved block is in the world's merged block tag, on a client the tag its
    /// server sent. Unknown tags return false; unavailable tags or owning-thread block mappings
    /// return None.
    pub(super) fn block_in_tag(&self, block: BlockKey, tag: &str) -> Option<bool> {
        self.native_tag_items()?;
        block_in_tag(block, tag)
    }

    /// The outline of `state` as `BlockState.getShape` answers it with an empty getter at the
    /// origin (`EmptyBlockGetter.INSTANCE`, `BlockPos.ZERO`): its boxes, each `[min x, y, z,
    /// max x, y, z]` in the block's own space, with no random offset and none of a world's
    /// context (a state that reads its neighbours or block entity answers what its state alone
    /// gives). Empty for a state with no outline, such as air. None for an id no state has.
    pub(super) fn state_outline(&self, state: BlockStateId) -> Option<Vec<[f64; 6]>> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).outline.to_vec())
    }

    /// The fluid state `state` holds (`BlockState.getFluidState`): water for a waterlogged state,
    /// the empty fluid for most. None for an id no state has.
    pub(super) fn state_fluid_state(&self, state: BlockStateId) -> Option<FluidState> {
        state_fluid(self.res.behaviours(), self.pack.assets.registry, state)
    }

    /// Whether `fluid` is in the fluid tag `tag` (`FluidState.is(TagKey)`, `minecraft:water` or
    /// `water`), over the merged fluid tags, on a client those its server sent. An unknown tag
    /// is false. None as for `block_in_tag`.
    pub(super) fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        Some(fluid_in_tag(self.native_tag_items()?, fluid, tag))
    }

    /// The fluid state at `pos` of the level `level` names (`BlockGetter.getFluidState`). None
    /// for a stale handle, a chunk that isn't loaded or a `y` outside the world: a read never
    /// loads a chunk.
    pub(super) fn level_fluid_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<FluidState> {
        let state = self.level_block_state(level, pos)?;
        self.state_fluid_state(state)
    }

    /// The fluid's shape at `pos` (`FluidState.getShape`), block-local: none for no fluid, else
    /// the floor to `FluidState.getHeight`, a full block under the same fluid and the fluid's
    /// own height otherwise. None as for `level_block_state`; an unloaded block above is no
    /// fluid.
    pub(super) fn level_fluid_shape(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<Vec<[f64; 6]>> {
        let behaviours = self.res.behaviours();
        let fluid = behaviours.fluid(self.level_block_state(level, pos)?);
        let above = self
            .level_block_state(level, [pos[0], pos[1] + 1, pos[2]])
            .map(|state| behaviours.fluid(state));
        Some(fluid_shape(fluid, above).into_iter().collect())
    }

    /// `Fluid.getTickDelay(level)` of `fluid` at `pos` in the level in place: vanilla's by its
    /// family and the dimension (`World::fast_lava`), a mod fluid's as registered and as its
    /// `fluid_rules` parts change it there (the one rule path the flow itself reads). None for
    /// another level, a stale handle, a client, an unloaded or out-of-height position and a
    /// fluid with no world form.
    pub(super) fn level_fluid_tick_delay(
        &self,
        level: LevelRef,
        fluid: FluidKey,
        pos: [i32; 3],
    ) -> Option<i32> {
        let [x, y, z] = pos;
        if !self.level_in_place(level) || !self.world.is_loaded_at(x, y, z) {
            return None;
        }
        let kind = engine_fluid_kind(fluid)?;
        let view = ReadLevel { game: self, level };
        Some(fluid_tick_delay(kind, self.world.fast_lava, || {
            let fluid = kind.mod_fluid()?;
            Some(super::GameLevel::answered_fluid_rules(
                self.dim.level(),
                BlockPos::new(x, y, z),
                fluid,
                fluid.get().rules,
                |call| mod_api::dispatch_fluid_rules(&view, call),
            ))
        }))
    }

    /// `FluidState.getHeight(level, pos)` of the fluid at `pos`. None for a stale handle or a
    /// position that isn't loaded, as `level_block_state`.
    pub(super) fn level_fluid_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32> {
        let mut column = FluidColumn { game: self, level };
        self.level_block_state(level, pos)?;
        let Ok(height) = column.fluid_at(pos).height_at(&mut column, pos);
        Some(height)
    }

    /// The surface height the camera's immersion test reads at `pos`
    /// (`FluidState::camera_height_at`). None as for `level_fluid_height`.
    pub(super) fn level_fluid_height_for_camera(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<f32> {
        let mut column = FluidColumn { game: self, level };
        self.level_block_state(level, pos)?;
        let Ok(height) = column.fluid_at(pos).camera_height_at(&mut column, pos);
        Some(height)
    }

    /// `FluidState.canBeReplacedWith(level, pos, fluid, direction)` for the fluid at `pos`. False
    /// for a `fluid` with no world form; None as for `level_fluid_height`.
    pub(super) fn level_fluid_can_be_replaced_with(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        direction: Direction,
    ) -> Option<bool> {
        let mut column = FluidColumn { game: self, level };
        self.level_block_state(level, pos)?;
        let here = column.fluid_at(pos);
        let Some(other) = engine_fluid_kind(fluid) else {
            return Some(false);
        };
        Some(fluid_can_be_replaced_with(here, other, direction, || {
            let Ok(height) = here.height_at(&mut column, pos);
            height
        }))
    }

    /// Whether `fluid` and `other` are the same fluid (`Fluid.isSame`): a source and its
    /// flowing form are, so `minecraft:water` and `minecraft:flowing_water` are the same, water
    /// and lava not. Names are fluid ids, with or without `minecraft:`; false when either
    /// names no fluid.
    pub(super) fn fluid_is_same(&self, fluid: &str, other: &str) -> bool {
        match (fluid_kind(fluid), fluid_kind(other)) {
            (Some(fluid), Some(other)) => fluid.same_family(other),
            _ => false,
        }
    }

    /// The bucket item that picks `fluid` up (`Fluid.getBucket`), `minecraft:water_bucket` for
    /// water and flowing water. None for `minecraft:empty`, whose bucket is air, and for a name
    /// no fluid has.
    pub(super) fn fluid_bucket(&self, fluid: &str) -> Option<mod_api::ItemKey> {
        mod_api::__private::item_key(horizon_world::level::fluid_bucket(fluid_kind(fluid)?)?)
    }

    /// The sound event a bucket plays as it picks `fluid` up (`Fluid.getPickupSound`),
    /// `minecraft:item.bucket.fill_lava` for lava and flowing lava. None for
    /// `minecraft:empty`, which has none, and for a name no fluid has.
    pub(super) fn fluid_pickup_sound(&self, fluid: &str) -> Option<&'static str> {
        horizon_world::level::fluid_pickup_sound(fluid_kind(fluid)?)
    }

    /// The sound event a bucket plays as it empties `fluid` (`BucketItem.playEmptySound`),
    /// `minecraft:item.bucket.empty_lava` for lava and flowing lava. None for
    /// `minecraft:empty`, for a tank-only fluid and for a name no fluid has.
    pub(super) fn fluid_empty_sound(&self, fluid: &str) -> Option<&'static str> {
        horizon_world::level::fluid_empty_sound(fluid_kind(fluid)?)
    }

    /// Read a loaded block in a server tick hook, without loading a chunk.
    /// The returned name and properties are owned and remain valid after the hook.
    #[allow(dead_code)]
    pub(super) fn native_server_block(
        &self,
        context: &LifecycleContext,
        dimension: &str,
        pos: [i32; 3],
    ) -> Result<BlockState, NativeBlockReadError> {
        if !self.native_lifecycle.permits_read(context) {
            return Err(NativeBlockReadError::InactiveContext);
        }
        if !Dim::exists(dimension) {
            return Err(NativeBlockReadError::UnavailableDimension);
        }
        let dim = Dim::from_id(dimension);
        let world = if dim == self.dim {
            Some(&self.world)
        } else {
            self.levels.parked_world(dim)
        }
        .ok_or(NativeBlockReadError::UnavailableDimension)?;
        read_block(world, self.pack.assets.registry, pos)
    }
}

/// Resolve an existing world state, preserving unloaded and invalid states.
fn read_block(
    world: &World,
    registry: &BlockRegistry,
    [x, y, z]: [i32; 3],
) -> Result<BlockState, NativeBlockReadError> {
    if !(-30_000_000..30_000_000).contains(&x)
        || !(-30_000_000..30_000_000).contains(&z)
        || !world.shape.contains_y(y)
    {
        return Err(NativeBlockReadError::OutOfBounds);
    }
    if !world.loaded(x, z) {
        return Err(NativeBlockReadError::Unloaded);
    }
    registry
        .state(world.block(x, y, z))
        .cloned()
        .ok_or(NativeBlockReadError::InvalidState)
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::chunk::Chunk;
    use horizon_client::items::TagRegistry;
    use mod_api::LevelHost;
    use std::sync::Arc;

    #[test]
    fn a_fluid_shape_is_full_under_the_same_fluid_and_its_own_height_otherwise() {
        let source = EngineFluidState::WATER;
        let shallow = EngineFluidState::new(FluidKind::FlowingWater, 3, false);
        let lava = EngineFluidState::new(FluidKind::Lava, 8, false);
        assert_eq!(fluid_shape(EngineFluidState::EMPTY, Some(source)), None);
        // FlowingFluid.getOwnHeight: amount / 9, so a source is 8/9 high under air.
        let top = |shape: Option<[f64; 6]>| shape.map(|b| b[4]);
        assert_eq!(
            top(fluid_shape(source, None)),
            Some(f64::from(8.0f32 / 9.0))
        );
        assert_eq!(
            top(fluid_shape(shallow, None)),
            Some(f64::from(3.0f32 / 9.0))
        );
        // Flowing water under a source is the same fluid: a full block.
        assert_eq!(top(fluid_shape(shallow, Some(source))), Some(1.0));
        assert_eq!(
            top(fluid_shape(source, Some(lava))),
            Some(f64::from(8.0f32 / 9.0))
        );
        assert_eq!(
            fluid_shape(source, Some(source)),
            Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
        );
    }

    #[test]
    fn fluid_state_conversion_preserves_source_and_empty_invariants() {
        mod_platform::__private::install(Default::default());
        crate::game::api_values::register_game_keys([]);
        for kind in FluidKind::ALL {
            let engine =
                EngineFluidState::new(kind, if kind == FluidKind::Empty { 0 } else { 8 }, true);
            let api = api_fluid_state(engine).unwrap();
            assert_eq!(api.is_empty(), kind == FluidKind::Empty);
            assert_eq!(api.is_source(), engine.is_source());
            assert_eq!(mod_api::__private::fluid_id(api.fluid), Some(kind.name()));
        }
        let source = api_fluid_state(EngineFluidState::new(FluidKind::Water, 8, true)).unwrap();
        let flowing =
            api_fluid_state(EngineFluidState::new(FluidKind::FlowingWater, 8, false)).unwrap();
        assert!(source.falling && source.is_source());
        assert_eq!(flowing.amount, 8);
        assert!(!flowing.is_source());
    }

    #[test]
    fn api_block_tags_rebind_the_merged_snapshot_without_changing_keys() {
        use mod_platform::__private::part_tests;
        mod_platform::__private::install(Default::default());
        crate::game::api_values::register_game_keys([
            "minecraft:stone".into(),
            "test:machine".into(),
        ]);
        let stone = part_tests::block_key("stone");
        let machine = part_tests::block_key("test:machine");
        let mut items = Items::for_tests(Default::default());
        items
            .block_tags
            .insert("test:machines".into(), ["test:machine".into()].into());
        items.block_tags.insert(
            "minecraft:base_stone_overworld".into(),
            ["minecraft:stone".into()].into(),
        );
        for facts in [
            r#"{"defaults":{"minecraft:air":2,"minecraft:stone":0,"test:machine":1},"states":[["s",false,false,"","minecraft:stone"],["m",false,false,"","test:machine"],["a",true,false,"","minecraft:air"]]}"#,
            r#"{"defaults":{"minecraft:air":2,"minecraft:stone":1,"test:machine":0},"states":[["m",false,false,"","test:machine"],["s",false,false,"","minecraft:stone"],["a",true,false,"","minecraft:air"]]}"#,
        ] {
            // Build reordered numeric registries from the same API definitions.
            let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            crate::game::api_values::resolve_block_definitions(&registry).unwrap();
            install_block_tags(prepare_block_tags(&items.block_tags));
            assert_eq!(block_in_tag(machine, "test:machines"), Some(true));
            assert_eq!(block_in_tag(stone, "test:machines"), Some(false));
            assert_eq!(block_in_tag(stone, "base_stone_overworld"), Some(true));
            assert_eq!(
                block_in_tag(machine, "minecraft:base_stone_overworld"),
                Some(false)
            );
            assert_eq!(block_in_tag(machine, "test:missing"), Some(false));
            assert_eq!(machine, part_tests::block_key("test:machine"));
        }
        // A replaced merged tag snapshot must discard previous membership, not union it.
        items
            .block_tags
            .insert("test:machines".into(), ["minecraft:stone".into()].into());
        install_block_tags(prepare_block_tags(&items.block_tags));
        assert_eq!(block_in_tag(machine, "test:machines"), Some(false));
        assert_eq!(block_in_tag(stone, "test:machines"), Some(true));
        // Valid key, no owning world: never answer false using another thread's tags.
        assert_eq!(
            std::thread::spawn(move || block_in_tag(stone, "test:machines"))
                .join()
                .unwrap(),
            None
        );
    }

    #[test]
    fn reads_only_loaded_in_bounds_states() {
        let facts = br#"{"defaults":{"minecraft:air":0,"test:pillar":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["pillar",false,false,"",{"id":"test:pillar","properties":{"axis":"x"}}]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        world.set(-1, 64, 1, BlockStateId(1)).unwrap();
        // The Nether's build range starts at y 0 (its level's shape).
        let mut nether = World::new(BlockStateId(0), 0);
        nether.shape = horizon_client::chunk::LevelShape::NETHER;
        nether.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        let block = read_block(&world, &registry, [-1, 64, 1]).unwrap();
        assert_eq!(block.name, "test:pillar");
        assert_eq!(block.properties["axis"], "x");
        assert_eq!(
            read_block(&world, &registry, [-1, 65, 1]).unwrap().name,
            "minecraft:air"
        );
        assert_eq!(
            read_block(&world, &registry, [0, 64, 1]),
            Err(NativeBlockReadError::Unloaded)
        );
        assert_eq!(
            read_block(&nether, &registry, [-1, -1, 1]),
            Err(NativeBlockReadError::OutOfBounds)
        );
        assert_eq!(
            read_block(&world, &registry, [30_000_000, 64, 1]),
            Err(NativeBlockReadError::OutOfBounds)
        );
        world.set(-1, 64, 1, BlockStateId(99)).unwrap();
        assert_eq!(
            read_block(&world, &registry, [-1, 64, 1]),
            Err(NativeBlockReadError::InvalidState)
        );
        assert_eq!(world.chunks.len(), 1, "reads must not load chunks");
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn recipes_of_type_answers_vanilla_types_from_the_live_table() {
        with_game(|game| {
            let smelting = game.recipes_of_type("minecraft:smelting");
            let iron = smelting
                .iter()
                .find(|recipe| recipe.id == "minecraft:iron_ingot_from_smelting_raw_iron")
                .expect("vanilla's smelting recipe of raw iron");
            assert_eq!(iron.json["cookingtime"], 200);
            assert_eq!(iron.cooking_time, Some(200));
            assert_eq!(iron.experience, Some(0.7));
            assert_eq!(iron.ingredients.len(), 1);
            assert!(iron.ingredients[0].items.contains("minecraft:raw_iron"));
            assert_eq!(iron.results[0].item.as_ref(), "minecraft:iron_ingot");
            let shaped = game.recipes_of_type("minecraft:crafting");
            let bucket = shaped
                .iter()
                .find(|recipe| recipe.id == "minecraft:bucket")
                .expect("vanilla's bucket recipe");
            assert_eq!(bucket.ingredients.len(), 3, "its pattern's three cells");
            assert_eq!(bucket.results[0].item.as_ref(), "minecraft:bucket");
            assert_eq!(bucket.cooking_time, None);
            assert!(
                !game
                    .recipes_of_type("minecraft:campfire_cooking")
                    .is_empty()
            );
            assert!(game.recipes_of_type("nope:missing").is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn tag_and_fluid_reads_follow_vanilla_and_load_nothing() {
        with_game(|game| {
            let dir =
                std::env::temp_dir().join(format!("horizon-tag-reads-{}", std::process::id()));
            game.session_lock = Some(horizon_client::save::SessionLock::acquire(&dir).unwrap());
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let slab = game
                .block_state_id("minecraft:oak_slab[waterlogged=true]")
                .unwrap();
            let dry_slab = game.block_state_id("minecraft:oak_slab").unwrap();
            let lava = registry.default_state("minecraft:lava").unwrap();

            // Members come in the tag file's order, in each registry, with or without the
            // namespace; an unknown tag lists nothing.
            let oak = [
                "oak_log",
                "oak_wood",
                "stripped_oak_log",
                "stripped_oak_wood",
            ]
            .map(|name| format!("minecraft:{name}"));
            let engine_members = |registry, tag: &str| {
                game.native_tag_items().map(|items| {
                    items
                        .tag_members(registry, &horizon_client::command::args::normalized(tag))
                        .unwrap_or_default()
                        .to_vec()
                })
            };
            for registry in [TagRegistry::Block, TagRegistry::Item] {
                assert_eq!(
                    engine_members(registry, "oak_logs").as_deref(),
                    Some(&oak[..])
                );
            }
            {
                let host: &dyn mod_api::Host = game;
                let tags = host.tags();
                let oak_tag = tags
                    .key(&mod_api::Identifier::parse("minecraft:oak_logs").unwrap())
                    .unwrap();
                assert_eq!(
                    tags.items(oak_tag)
                        .map(|keys| keys.into_iter().map(|key| key.id(host)).collect::<Vec<_>>())
                        .as_deref(),
                    Some(&oak[..])
                );
                let item = host
                    .item_key(&mod_api::Identifier::parse("minecraft:oak_log").unwrap())
                    .unwrap();
                assert_eq!(tags.is_in_tag(item, oak_tag), Some(true));
            }
            assert_eq!(
                engine_members(TagRegistry::Fluid, "minecraft:water").unwrap(),
                ["minecraft:water", "minecraft:flowing_water"]
            );
            assert_eq!(
                engine_members(TagRegistry::Item, "nope:missing"),
                Some(Vec::new())
            );

            // A full block is one unit box, a bottom slab half of it, air none, and an id no
            // state has is unknown.
            assert_eq!(
                game.state_outline(stone),
                Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
            );
            assert_eq!(
                game.state_outline(dry_slab),
                Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]])
            );
            let air = registry.default_state("minecraft:air").unwrap();
            assert_eq!(game.state_outline(air), Some(Vec::new()));
            assert_eq!(game.state_outline(BlockStateId(u32::MAX)), None);

            assert_eq!(
                game.block_in_tag(
                    mod_platform::__private::part_tests::block_key("minecraft:stone"),
                    "minecraft:base_stone_overworld"
                ),
                Some(true)
            );
            assert_eq!(
                game.block_in_tag(
                    mod_platform::__private::part_tests::block_key("stone"),
                    "base_stone_overworld"
                ),
                Some(true)
            );
            assert_eq!(
                game.block_in_tag(
                    mod_platform::__private::part_tests::block_key("minecraft:stone"),
                    "minecraft:logs"
                ),
                Some(false)
            );
            assert_eq!(
                game.block_in_tag(
                    mod_platform::__private::part_tests::block_key("minecraft:stone"),
                    "nope:missing"
                ),
                Some(false)
            );

            assert_eq!(
                game.state_fluid_state(water),
                api_fluid_state(EngineFluidState::WATER)
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:water", "minecraft:water"),
                Some(true)
            );
            assert_eq!(game.fluid_in_tag("water", "water"), Some(true));
            assert_eq!(
                game.fluid_in_tag("minecraft:water", "minecraft:lava"),
                Some(false)
            );
            assert_eq!(
                mod_api::__private::fluid_id(game.state_fluid_state(lava).unwrap().fluid).unwrap(),
                "minecraft:lava"
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:lava", "minecraft:lava"),
                Some(true)
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:lava", "minecraft:water"),
                Some(false)
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:empty", "minecraft:water"),
                Some(false)
            );
            assert_eq!(
                game.state_fluid_state(slab),
                api_fluid_state(EngineFluidState::WATER)
            );
            assert_eq!(
                game.state_fluid_state(dry_slab),
                api_fluid_state(EngineFluidState::EMPTY)
            );
            assert_eq!(
                game.state_fluid_state(stone),
                api_fluid_state(EngineFluidState::EMPTY)
            );
            assert_eq!(game.state_fluid_state(BlockStateId(u32::MAX)), None);

            // The world holds a chunk with water in it; a neighbour is not loaded.
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(1, 64, 1, slab.0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let level = game.level(game.dim.id()).unwrap();
            let chunks = game.world.chunks.len();
            assert_eq!(
                game.level_fluid_state(level, [1, 64, 1]),
                api_fluid_state(EngineFluidState::WATER)
            );
            assert_eq!(
                game.level_fluid_state(level, [2, 64, 1]),
                api_fluid_state(EngineFluidState::EMPTY)
            );
            assert_eq!(game.level_fluid_state(level, [16, 64, 1]), None);
            assert_eq!(game.level_fluid_state(level, [1, 4000, 1]), None);
            game.with_level(|_, host, _| {
                // Not an authoritative level: tags are not the server's, so they read None.
                assert_eq!(
                    host.block_in_tag(
                        mod_platform::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:base_stone_overworld"
                    ),
                    None
                );
                assert_eq!(
                    host.fluid_in_tag("minecraft:water", "minecraft:water"),
                    None
                );
                assert_eq!(
                    host.fluid_state([1, 64, 1]),
                    api_fluid_state(EngineFluidState::WATER)
                );
            });
            game.options.dedicated = Some(Vec::new());
            let bounds = game.level_height(level).unwrap();
            use super::super::api_values::TryIntoApi;
            let stone = stone.try_into_api().unwrap();
            let slab = slab.try_into_api().unwrap();
            let invalid = mod_api::BlockStateRecord::new(stone.block(), u16::MAX).into();
            let parsed = game
                .block_state_id("minecraft:oak_slab[waterlogged=true]")
                .try_into_api()
                .unwrap();
            let default = game
                .block_state_default(mod_platform::__private::part_tests::block_key(
                    "minecraft:oak_slab",
                ))
                .try_into_api()
                .unwrap();
            game.with_level(|_, host, ticks| {
                assert_eq!(
                    host.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    host.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(host.block_state_id("minecraft:oak_slab[bad=true]"), None);

                let wrapped = super::super::mod_api_host::LevelTicks {
                    level: &*host,
                    ticks,
                };
                assert_eq!(
                    wrapped.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    wrapped.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(wrapped.block_state_id("minecraft:oak_slab[bad=true]"), None);

                assert_eq!(host.state_string(stone).as_deref(), Some("minecraft:stone"));
                assert!(
                    host.state_string(slab)
                        .unwrap()
                        .contains("waterlogged=true")
                );
                assert_eq!(host.state_string(invalid), None);
                assert_eq!((host.min_y(), host.height()), bounds);
                assert_eq!(
                    host.block_in_tag(
                        mod_platform::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:base_stone_overworld"
                    ),
                    Some(true)
                );
                assert_eq!(
                    host.block_in_tag(
                        mod_platform::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:logs"
                    ),
                    Some(false)
                );
                assert_eq!(
                    host.fluid_state([1, 64, 1]),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(host.fluid_state([16, 64, 1]), None);
                assert_eq!(
                    host.state_fluid_state(slab),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(
                    host.fluid_in_tag("minecraft:water", "minecraft:water"),
                    Some(true)
                );
                assert_eq!(
                    host.fluid_in_tag("minecraft:empty", "minecraft:water"),
                    Some(false)
                );
            });
            {
                use mod_api::LevelHost;
                let hit =
                    super::super::native_hooks::HitLevel(horizon_client::health::DamageLevel::of(
                        &mut game.world,
                        game.pack.assets.registry,
                        Some(world_items!(game)),
                        Some(&game.pack.block_registries),
                        &game.damage_hooks,
                    ));
                assert_eq!(
                    hit.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    hit.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(hit.block_state_id("minecraft:oak_slab[bad=true]"), None);

                assert_eq!(hit.state_string(stone).as_deref(), Some("minecraft:stone"));
                assert!(hit.state_string(slab).unwrap().contains("waterlogged=true"));
                assert_eq!(hit.state_string(invalid), None);
                assert_eq!((hit.min_y(), hit.height()), bounds);
                assert_eq!(
                    hit.block_in_tag(
                        mod_platform::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:base_stone_overworld"
                    ),
                    Some(true)
                );
                assert_eq!(
                    hit.fluid_in_tag("minecraft:water", "minecraft:water"),
                    Some(true)
                );
            }
            {
                use mod_api::LevelHost;
                let hit =
                    super::super::native_hooks::HitLevel(horizon_client::health::DamageLevel::of(
                        &mut game.world,
                        game.pack.assets.registry,
                        None,
                        None,
                        &game.damage_hooks,
                    ));
                assert_eq!(hit.block_state_id("minecraft:stone"), None);
                assert_eq!(
                    hit.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:stone"
                    )),
                    None
                );
                assert_eq!(
                    hit.block_in_tag(
                        mod_platform::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:logs"
                    ),
                    None
                );
                assert_eq!(hit.fluid_in_tag("minecraft:water", "minecraft:water"), None);
            }
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            let host: &dyn mod_api::Host = game;
            let tags = host.tags();
            let oak_tag = tags
                .key(&mod_api::Identifier::parse("minecraft:oak_logs").unwrap())
                .unwrap();
            let item = host
                .item_key(&mod_api::Identifier::parse("minecraft:oak_log").unwrap())
                .unwrap();
            assert_eq!(tags.items(oak_tag), None);
            assert_eq!(tags.is_in_tag(item, oak_tag), None);
            game.session_lock = None;
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    /// Sources and their flowing forms are one fluid, the empty fluid has no bucket and no
    /// pickup sound, and a name no fluid has answers nothing, through a mod's `Host`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_sameness_buckets_and_bucket_sounds_follow_vanilla() {
        with_game(|game| {
            let host: &dyn mod_api::Host = game;
            assert!(host.fluid_is_same("minecraft:water", "minecraft:flowing_water"));
            assert!(host.fluid_is_same("flowing_lava", "lava"));
            assert!(host.fluid_is_same("minecraft:empty", "minecraft:empty"));
            assert!(!host.fluid_is_same("minecraft:water", "minecraft:lava"));
            assert!(!host.fluid_is_same("minecraft:water", "minecraft:empty"));
            assert!(!host.fluid_is_same("nope:missing", "nope:missing"));

            assert_eq!(
                host.fluid_bucket("minecraft:flowing_water")
                    .and_then(mod_api::__private::item_key_id),
                Some("minecraft:water_bucket")
            );
            assert_eq!(
                host.fluid_bucket("lava")
                    .and_then(mod_api::__private::item_key_id),
                Some("minecraft:lava_bucket")
            );
            assert_eq!(host.fluid_bucket("minecraft:empty"), None);
            assert_eq!(host.fluid_bucket("nope:missing"), None);

            assert_eq!(
                host.fluid_pickup_sound("minecraft:water"),
                Some("minecraft:item.bucket.fill")
            );
            assert_eq!(
                host.fluid_pickup_sound("minecraft:flowing_lava"),
                Some("minecraft:item.bucket.fill_lava")
            );
            assert_eq!(host.fluid_pickup_sound("minecraft:empty"), None);
            assert_eq!(host.fluid_pickup_sound("nope:missing"), None);

            assert_eq!(
                host.fluid_empty_sound("flowing_water"),
                Some("minecraft:item.bucket.empty")
            );
            assert_eq!(
                host.fluid_empty_sound("minecraft:lava"),
                Some("minecraft:item.bucket.empty_lava")
            );
            assert_eq!(host.fluid_empty_sound("minecraft:empty"), None);
            assert_eq!(host.fluid_empty_sound("nope:missing"), None);
        });
    }
}
