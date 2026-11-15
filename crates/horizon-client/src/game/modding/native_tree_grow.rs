// game-module-visibility: pub(crate)
//! The engine side of `on_tree_grow`: the trunk placer's callback, asking the parts of the block
//! under a growing trunk on the worldgen workers, and on the server thread when a sapling grows.
//! It reads what the server published (`native_worldgen_parts`), never a game thread's tables.
use super::api_values::block_state::Index;
use super::native_worldgen_parts;
use horizon_world::native_worldgen_hooks::TreeGrowAccess;
use horizon_world::registry::BlockStateId as EngineState;
use mod_api::{BlockState, HookId, StructureRandom};

/// The engine's `on_tree_grow` callback: the block the parts of the block at `pos` place there,
/// `Some(None)` for none, or None to leave it to the tree's own provider. Nothing is read or
/// converted while no part answers the hook.
#[allow(clippy::option_option)]
pub(crate) fn on_tree_grow(
    access: &mut dyn TreeGrowAccess,
    pos: [i32; 3],
) -> Option<Option<EngineState>> {
    if !mod_api::__private::has_part_hook(HookId::OnTreeGrow) {
        return None;
    }
    let index = native_worldgen_parts::index()?;
    let state = index.to_api(access.block_state(pos)?).ok()?;
    let mut soil = None;
    native_worldgen_parts::with_host(|host| {
        mod_api::world::ground::native_on_tree_grow(
            host,
            state,
            pos,
            &mut Random(access),
            &mut soil,
        );
    })?;
    placed(&index, state, soil?)
}

/// A part's answer for the block under the trunk, as the engine places it. A state the game's
/// registry does not have is refused, naming the mods whose parts answer for `state`'s block,
/// and the tree keeps its own provider.
#[allow(clippy::option_option)]
fn placed(
    index: &Index,
    state: BlockState,
    answer: Option<BlockState>,
) -> Option<Option<EngineState>> {
    let Some(answer) = answer else {
        return Some(None);
    };
    match index.to_engine(answer) {
        Ok(placed) => Some(Some(placed)),
        Err(error) => {
            log::warn!(
                "on_tree_grow: a part of {} answered a block state this game does not have \
                 ({error:?}); the tree places its own block",
                mod_api::__private::worldgen_part_mods(HookId::OnTreeGrow, state).join(", ")
            );
            None
        }
    }
}

/// The tree's own random, as a part draws from it.
struct Random<'a>(&'a mut dyn TreeGrowAccess);
impl StructureRandom for Random<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.0.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.next_boolean()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::{block_state, resolve_block_definitions};
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    use horizon_world::native_worldgen_hooks::WorldgenRandom;
    use horizon_world::registry::BlockRegistry;
    use mod_api::{
        BlockKey, BlockPart, BlockStateRecord, Hooks, ModEntry, Mods, OnTreeGrowEvent, WorldgenHost,
    };
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, Mutex, PoisonError};

    use native_worldgen_parts::PUBLISHING;

    const GRASS: EngineState = EngineState(1);
    const COARSE_DIRT: EngineState = EngineState(2);
    const FACTS: &str = r#"{"defaults":{"minecraft:air":0,"minecraft:grass_block":1,"minecraft:coarse_dirt":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:grass_block"],["s",false,false,"","minecraft:coarse_dirt"]]}"#;

    /// A trunk over grass, counting the draws from its random.
    #[derive(Default)]
    struct Trunk {
        draws: u32,
    }
    impl TreeGrowAccess for Trunk {
        fn block_state(&mut self, _: [i32; 3]) -> Option<EngineState> {
            Some(GRASS)
        }
    }
    impl WorldgenRandom for Trunk {
        fn next_int_bound(&mut self, _: i32) -> i32 {
            self.draws += 1;
            0
        }
        fn next_long(&mut self) -> i64 {
            self.draws += 1;
            0
        }
        fn next_float(&mut self) -> f32 {
            self.draws += 1;
            0.0
        }
        fn next_double(&mut self) -> f64 {
            self.draws += 1;
            0.0
        }
        fn next_boolean(&mut self) -> bool {
            self.draws += 1;
            false
        }
    }

    /// Coarse dirt under the trunk, after one draw from the tree's random.
    struct Coarse(BlockKey);
    #[mod_api::horizon_part]
    impl BlockPart for Coarse {
        fn on_tree_grow(
            &self,
            _: &dyn WorldgenHost,
            event: &mut OnTreeGrowEvent<'_>,
        ) -> Option<Option<BlockState>> {
            event.random.next_int_bound(4);
            Some(Some(BlockStateRecord::new(self.0, 0).into()))
        }
    }
    struct Soil;
    #[mod_api::__subscriptions]
    impl Hooks for Soil {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let grass = r.block_key("minecraft:grass_block");
            let coarse = r.block_key("minecraft:coarse_dirt");
            r.block_part(grass, Coarse(coarse));
        }
    }

    /// Publish `parts` resolved against `tags` with `registry`'s index, and an empty worldgen
    /// context.
    fn publish(registry: &'static BlockRegistry, tags: &[(&str, &[&str])]) {
        let tags = tags
            .iter()
            .map(|(tag, members)| {
                let members = members.iter().map(|member| member.to_string()).collect();
                (tag.to_string(), members)
            })
            .collect::<HashMap<_, _>>();
        let parts = mod_api::__private::prepare_parts(
            &tags,
            &HashMap::new(),
            mod_api::__private::KnownIds {
                block: &|id| registry.default_state(id).is_some(),
                item: &|_| false,
            },
        )
        .unwrap();
        native_worldgen_parts::publish(parts.as_ref(), &block_state::current().unwrap(), registry);
        let root = std::env::temp_dir().join("horizon-tree-grow-no-settings");
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(BTreeMap::new(), root)),
            [],
        );
        horizon_world::native_worldgen_hooks::publish_context(Arc::new(load.worldgen_context([])));
    }

    #[test]
    fn the_soils_part_answers_from_the_published_tables_and_draws_from_the_tree() {
        let _publishing = PUBLISHING.lock().unwrap_or_else(PoisonError::into_inner);
        let mods: Mods = vec![ModEntry::new("soil", Box::new(Soil))].into();
        let mut trunk = Trunk::default();
        let answer = mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_facts_json(FACTS.as_bytes()).unwrap(),
            ));
            resolve_block_definitions(registry).unwrap();
            publish(registry, &[]);
            mod_api::__private::worldgen_feature(|| on_tree_grow(&mut trunk, [0, 63, 0]))
        });
        assert_eq!(answer, Some(Some(COARSE_DIRT)));
        assert_eq!(trunk.draws, 1);
    }

    /// The parts that heard `on_tree_grow`, in the order they were asked.
    static HEARD: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    fn heard() -> Vec<&'static str> {
        std::mem::take(&mut *HEARD.lock().unwrap_or_else(PoisonError::into_inner))
    }
    /// Answers nothing, recording that it was asked as `"key"` or `"tag"`.
    struct Listening;
    /// A second part type: a tag part of the same type as a key part on the block is dropped as
    /// attached twice.
    struct TagListening;
    fn listen(name: &'static str) -> Option<Option<BlockState>> {
        HEARD
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        None
    }
    #[mod_api::horizon_part]
    impl BlockPart for Listening {
        fn on_tree_grow(
            &self,
            _: &dyn WorldgenHost,
            _: &mut OnTreeGrowEvent<'_>,
        ) -> Option<Option<BlockState>> {
            listen("key")
        }
    }
    #[mod_api::horizon_part]
    impl BlockPart for TagListening {
        fn on_tree_grow(
            &self,
            _: &dyn WorldgenHost,
            _: &mut OnTreeGrowEvent<'_>,
        ) -> Option<Option<BlockState>> {
            listen("tag")
        }
    }
    struct KeyAndTag;
    #[mod_api::__subscriptions]
    impl Hooks for KeyAndTag {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let grass = r.block_key("minecraft:grass_block");
            r.block_part(grass, Listening);
            r.block_part_tag("minecraft:dirt", TagListening);
        }
    }

    #[test]
    fn the_published_key_and_tag_parts_answer_and_a_feature_keeps_its_tables() {
        let _publishing = PUBLISHING.lock().unwrap_or_else(PoisonError::into_inner);
        let mods: Mods = vec![ModEntry::new("soils", Box::new(KeyAndTag))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_facts_json(FACTS.as_bytes()).unwrap(),
            ));
            resolve_block_definitions(registry).unwrap();
            let grow = || {
                let mut trunk = Trunk::default();
                assert_eq!(on_tree_grow(&mut trunk, [0, 63, 0]), None);
                assert_eq!(trunk.draws, 0);
            };
            publish(registry, &[("minecraft:dirt", &["minecraft:grass_block"])]);
            grow();
            assert_eq!(heard(), ["key", "tag"]);
            mod_api::__private::worldgen_feature(|| {
                // A `/reload` drops grass from the tag while the feature runs.
                publish(registry, &[("minecraft:dirt", &[])]);
                grow();
                assert_eq!(heard(), ["key", "tag"]);
            });
            grow();
            assert_eq!(heard(), ["key"]);
        });
    }

    #[test]
    fn a_state_the_registry_lacks_is_refused_and_the_tree_keeps_its_own_block() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let registry = BlockRegistry::from_facts_json(FACTS.as_bytes()).unwrap();
        resolve_block_definitions(&registry).unwrap();
        let index = block_state::current().unwrap();
        let grass = index.to_api(GRASS).unwrap();
        let coarse = index.to_api(COARSE_DIRT).unwrap();
        let missing = BlockStateRecord::new(coarse.block(), u16::MAX).into();
        assert_eq!(placed(&index, grass, Some(missing)), None);
        assert_eq!(placed(&index, grass, None), Some(None));
        assert_eq!(placed(&index, grass, Some(coarse)), Some(Some(COARSE_DIRT)));
    }
}
