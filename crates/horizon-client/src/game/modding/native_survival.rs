// game-module-visibility: pub(crate)
//! The engine side of `can_survive_v2`, `may_place_on_v2` and `can_sustain_plant_v2`: the
//! survival questions a feature asks while it places a block in world generation. They ask the
//! parts of the state's block (and of a plant's soil) on the worldgen workers and on the server
//! thread alike, reading what the server published (`native_worldgen_parts`), so a feature and a
//! player get one answer (#17943). A hand placement still goes through the published
//! `can_survive`, `may_place_on` and `can_sustain_plant`, which read the level.
use super::native_worldgen_parts;
use horizon_world::registry::BlockStateId as EngineState;
use mod_api::HookId;

/// The engine's `can_survive_v2` callback: the parts' answer for `state` at `pos`, `Some`
/// replacing the block's own rule and `None` leaving it to vanilla. Nothing is read or
/// converted while no part answers the hook; a panicking part keeps the block's own rule.
pub(crate) fn can_survive_v2(state: EngineState, pos: [i32; 3], survives: &mut Option<bool>) {
    if !mod_api::__private::has_part_hook(HookId::CanSurviveV2) {
        return;
    }
    let Some(index) = native_worldgen_parts::index() else {
        return;
    };
    if !index.worldgen_listens(HookId::CanSurviveV2, state) {
        return;
    }
    let Ok(asked) = index.to_api(state) else {
        return;
    };
    let mut answer = None;
    if let Err(error) = horizon_client::crash::contain(|| {
        native_worldgen_parts::with_host(|host| {
            mod_api::world::survival::native_can_survive_v2(
                host,
                mod_api::BlockPos::new(pos[0], pos[1], pos[2]),
                asked,
                &mut answer,
            );
        });
    }) {
        log::warn!("can_survive_v2 callback failed: {error}; the block keeps its own rule");
        return;
    }
    *survives = answer;
}

/// The engine's `may_place_on_v2` callback: the parts' answer for the plant `state` on `ground`
/// at `ground_pos`, `Some` replacing the plant's own rule. Asked and contained as
/// `can_survive_v2`.
pub(crate) fn may_place_on_v2(
    state: EngineState,
    ground: EngineState,
    ground_pos: [i32; 3],
    may: &mut Option<bool>,
) {
    if !mod_api::__private::has_part_hook(HookId::MayPlaceOnV2) {
        return;
    }
    let Some(index) = native_worldgen_parts::index() else {
        return;
    };
    if !index.worldgen_listens(HookId::MayPlaceOnV2, state) {
        return;
    }
    let (Ok(asked), Ok(on)) = (index.to_api(state), index.to_api(ground)) else {
        return;
    };
    let mut answer = None;
    if let Err(error) = horizon_client::crash::contain(|| {
        native_worldgen_parts::with_host(|host| {
            mod_api::world::survival::native_may_place_on_v2(
                host,
                asked,
                on,
                mod_api::BlockPos::new(ground_pos[0], ground_pos[1], ground_pos[2]),
                &mut answer,
            );
        });
    }) {
        log::warn!("may_place_on_v2 callback failed: {error}; the plant keeps its own rule");
        return;
    }
    *may = answer;
}

/// The engine's `can_sustain_plant_v2` callback: the soil `ground`'s parts answer for the plant
/// `state` above it at `ground_pos`, `Some` replacing the plant's own rule. The parts asked are
/// the soil's, so the subscription is looked up on `ground`. Asked and contained as
/// `can_survive_v2`.
pub(crate) fn can_sustain_plant_v2(
    state: EngineState,
    ground: EngineState,
    ground_pos: [i32; 3],
    sustains: &mut Option<bool>,
) {
    if !mod_api::__private::has_part_hook(HookId::CanSustainPlantV2) {
        return;
    }
    let Some(index) = native_worldgen_parts::index() else {
        return;
    };
    if !index.worldgen_listens(HookId::CanSustainPlantV2, ground) {
        return;
    }
    let (Ok(asked), Ok(on)) = (index.to_api(state), index.to_api(ground)) else {
        return;
    };
    let mut answer = None;
    if let Err(error) = horizon_client::crash::contain(|| {
        native_worldgen_parts::with_host(|host| {
            mod_api::world::survival::native_can_sustain_plant_v2(
                host,
                asked,
                on,
                mod_api::BlockPos::new(ground_pos[0], ground_pos[1], ground_pos[2]),
                &mut answer,
            );
        });
    }) {
        log::warn!("can_sustain_plant_v2 callback failed: {error}; the plant keeps its own rule");
        return;
    }
    *sustains = answer;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::{block_state, resolve_block_definitions};
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    use horizon_world::registry::BlockRegistry;
    use mod_api::{
        BlockKey, BlockPart, CanSurviveV2Event, CanSustainPlantV2Event, Hooks, MayPlaceOnV2Event,
        ModEntry, Mods, WorldgenHost,
    };
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, PoisonError};

    use native_worldgen_parts::PUBLISHING;

    const FACTS: &str = r#"{"defaults":{"minecraft:air":0,"minecraft:end_stone":1,"minecraft:grass_block":2,"mymod:crystal":3},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:end_stone"],["s",false,false,"","minecraft:grass_block"],["s",false,false,"","mymod:crystal"]]}"#;

    const END_STONE: EngineState = EngineState(1);
    const GRASS: EngineState = EngineState(2);
    const CRYSTAL: EngineState = EngineState(3);

    /// A mod's plant: it stands on end stone alone, and it places anywhere it is asked.
    struct Crystal(BlockKey);
    #[mod_api::horizon_part]
    impl BlockPart for Crystal {
        fn can_survive_v2(&self, _: &dyn WorldgenHost, _: &mut CanSurviveV2Event) -> Option<bool> {
            Some(true)
        }
        fn may_place_on_v2(
            &self,
            _: &dyn WorldgenHost,
            event: &mut MayPlaceOnV2Event,
        ) -> Option<bool> {
            Some(event.ground.block() == self.0)
        }
    }
    /// A mod's soil: it holds a plant standing on it.
    struct Soil(BlockKey);
    #[mod_api::horizon_part]
    impl BlockPart for Soil {
        fn can_sustain_plant_v2(
            &self,
            _: &dyn WorldgenHost,
            event: &mut CanSustainPlantV2Event,
        ) -> Option<bool> {
            Some(event.ground.block() == self.0)
        }
    }
    /// A soil that refuses every plant, on a block of the game's.
    struct RefusingSoil;
    #[mod_api::horizon_part]
    impl BlockPart for RefusingSoil {
        fn can_sustain_plant_v2(
            &self,
            _: &dyn WorldgenHost,
            _: &mut CanSustainPlantV2Event,
        ) -> Option<bool> {
            Some(false)
        }
    }
    struct Standing;
    #[mod_api::__subscriptions]
    impl Hooks for Standing {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let crystal = r.block_key("mymod:crystal");
            let end_stone = r.block_key("minecraft:end_stone");
            let grass = r.block_key("minecraft:grass_block");
            r.block_part(crystal, Crystal(end_stone));
            r.block_part(grass, Soil(grass));
            r.block_part(end_stone, RefusingSoil);
        }
    }

    /// Publish this mod's parts, resolved against `tags`, and an empty worldgen context: what a
    /// server does when its world opens.
    fn publish(registry: &'static BlockRegistry) {
        let tags = HashMap::new();
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
        let root = std::env::temp_dir().join("horizon-survival-no-settings");
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(BTreeMap::new(), root)),
            [],
        );
        horizon_world::native_worldgen_hooks::publish_context(Arc::new(load.worldgen_context([])));
    }

    /// Install the mod and a registry of its block beside the game's, published as a server
    /// publishes them.
    fn with_published<R>(f: impl FnOnce() -> R) -> R {
        let _publishing = PUBLISHING.lock().unwrap_or_else(PoisonError::into_inner);
        let mods: Mods = vec![ModEntry::new("standing", Box::new(Standing))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_facts_json(FACTS.as_bytes()).unwrap(),
            ));
            resolve_block_definitions(registry).unwrap();
            publish(registry);
            f()
        })
    }

    #[test]
    fn a_part_answers_whether_its_block_survives() {
        with_published(|| {
            let mut survives = None;
            can_survive_v2(CRYSTAL, [0, 64, 0], &mut survives);
            assert_eq!(survives, Some(true), "the mod's own block");
            // A block no part answers for keeps the game's own rule.
            let mut survives = None;
            can_survive_v2(GRASS, [0, 64, 0], &mut survives);
            assert_eq!(survives, None);
            // A state the published tables do not have is not converted.
            let mut survives = None;
            can_survive_v2(EngineState(u32::MAX), [0, 64, 0], &mut survives);
            assert_eq!(survives, None);
        });
    }

    #[test]
    fn a_plant_answers_the_ground_it_stands_on() {
        with_published(|| {
            let mut may = None;
            may_place_on_v2(CRYSTAL, END_STONE, [0, 63, 0], &mut may);
            assert_eq!(may, Some(true), "end stone under the mod's plant");
            let mut may = None;
            may_place_on_v2(CRYSTAL, GRASS, [0, 63, 0], &mut may);
            assert_eq!(may, Some(false), "grass under the mod's plant");
            // The ground is asked of the plant's parts, not of the ground block's.
            let mut may = None;
            may_place_on_v2(GRASS, END_STONE, [0, 63, 0], &mut may);
            assert_eq!(may, None);
        });
    }

    #[test]
    fn a_soil_answers_the_plant_above_it() {
        with_published(|| {
            // The parts asked are the planted-on block's: the mod's own soil answers for the
            // plant standing on it.
            let mut sustains = None;
            can_sustain_plant_v2(CRYSTAL, GRASS, [0, 63, 0], &mut sustains);
            assert_eq!(sustains, Some(true), "the mod's own soil");
            // A soil of the mod's on a block of the game's refuses the plant.
            let mut sustains = None;
            can_sustain_plant_v2(CRYSTAL, END_STONE, [0, 63, 0], &mut sustains);
            assert_eq!(sustains, Some(false), "a soil that refuses every plant");
            // A soil no part answers for keeps the plant's own rule.
            let mut sustains = None;
            can_sustain_plant_v2(END_STONE, CRYSTAL, [0, 63, 0], &mut sustains);
            assert_eq!(sustains, None);
        });
    }
}
