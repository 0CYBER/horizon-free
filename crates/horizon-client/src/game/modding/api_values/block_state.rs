//! Owning-thread, load-bound numeric bridge. Workers receive prepared values instead.
use super::{TryIntoApi, TryIntoEngine};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use mod_api::{BlockState, BlockStateError};
use std::{cell::RefCell, sync::Arc};

/// A fully checked index, inert until explicitly installed by the owning game thread.
pub(crate) struct Index {
    forward: Vec<BlockState>,
    reverse: std::collections::HashMap<mod_api::BlockKey, Vec<Option<BlockStateId>>>,
}
thread_local! {
    static INDEX: RefCell<Option<Arc<Index>>> = const { RefCell::new(None) };
}

/// Routing reads the precomputed key only: no event value is converted here.
pub(crate) fn block_listens(hook: mod_api::HookId, state: BlockStateId) -> bool {
    mod_api::__private::has_scoped_hook(hook)
        && INDEX.with_borrow(|index| {
            index
                .as_ref()
                .and_then(|index| index.forward.get(state.0 as usize))
                .is_some_and(|value| mod_api::__private::block_key_listens(hook, value.block()))
        })
}

/// Whether every part on `state` that answers `explosion_resistance` declares the answer pure.
pub(crate) fn explosion_resistance_pure(state: BlockStateId) -> bool {
    INDEX.with_borrow(|index| {
        index
            .as_ref()
            .and_then(|index| index.forward.get(state.0 as usize))
            .is_some_and(|value| mod_api::__private::explosion_resistance_key_pure(value.block()))
    })
}

pub(super) fn clear() {
    INDEX.with_borrow_mut(|index| *index = None);
}
pub(super) fn snapshot() -> Result<Box<[BlockState]>, BlockStateError> {
    Ok(current()?.forward.clone().into_boxed_slice())
}
pub(super) fn bind(registry: &BlockRegistry) -> Result<(), String> {
    clear();
    install(prepare(registry)?);
    Ok(())
}

pub(crate) fn install(prepared: Arc<Index>) {
    INDEX.with_borrow_mut(|index| *index = Some(prepared));
}

/// Prepare without activating thread-local state. A refused reload keeps its old live tables.
pub(crate) fn prepare(registry: &BlockRegistry) -> Result<Arc<Index>, String> {
    let forward = (0..registry.len())
        .map(|id| {
            registry
                .state(BlockStateId(id as u32))
                .expect("registry index")
                .try_into_api()
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("block state index: {error:?}"))?;
    let mut reverse = std::collections::HashMap::<_, Vec<Option<BlockStateId>>>::new();
    for (id, state) in forward.iter().enumerate() {
        let states = reverse.entry(state.block()).or_default();
        states.resize(states.len().max(state.ordinal() as usize + 1), None);
        if states[state.ordinal() as usize]
            .replace(BlockStateId(id as u32))
            .is_some()
        {
            return Err("duplicate block state in numeric registry".into());
        }
    }
    Ok(Arc::new(Index { forward, reverse }))
}
#[cfg(test)]
#[allow(clippy::items_after_test_module)] // TODO(#2273): move the module to the end of the file.
mod tests {
    use super::*;
    #[test]
    fn numeric_block_index_routes_only_subscribed_blocks() {
        struct Part;
        #[mod_api::horizon_part]
        impl mod_api::BlockPart for Part {
            fn analog_output(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut mod_api::AnalogOutputEvent,
            ) {
                event.output += 1;
            }
        }
        struct Listener;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listener {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                let key = registrar.block_key("minecraft:stone");
                registrar.block_part(key, Part);
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("routing", Box::new(Listener))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = r#"{"defaults":{"minecraft:air":2,"minecraft:stone":0,"minecraft:dirt":1},"states":[["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"],["a",true,false,"","minecraft:air"]]}"#;
            let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            super::super::resolve_block_definitions(&registry).unwrap();
            mod_api::__private::resolve_parts(
                &Default::default(),
                &Default::default(),
                mod_api::__private::KnownIds {
                    block: &|_| true,
                    item: &|_| false,
                },
            )
            .unwrap();
            assert!(block_listens(
                mod_api::HookId::AnalogOutput,
                BlockStateId(0)
            ));
            let start = std::time::Instant::now();
            for _ in 0..1_000 {
                assert!(!block_listens(
                    mod_api::HookId::AnalogOutput,
                    std::hint::black_box(BlockStateId(1))
                ));
            }
            println!(
                "actual Index and Parts: 1000 unrelated-block guards in {} ns",
                start.elapsed().as_nanos()
            );
        });
    }

    #[test]
    fn numeric_block_index_prepared_token_activates_only_on_fresh_owner() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();
        let stone = BlockStateId(1).try_into_api().unwrap();
        let prepared = prepare(&registry).unwrap();
        let reload = prepare(&registry).unwrap();
        std::thread::spawn(move || {
            assert_eq!(
                BlockStateId(1).try_into_api(),
                Err(BlockStateError::Unavailable)
            );
            install(prepared);
            assert_eq!(BlockStateId(1).try_into_api(), Ok(stone));
            assert_eq!(stone.try_into_engine(&()), Ok(BlockStateId(1)));
            install(reload);
            assert_eq!(BlockStateId(1).try_into_api(), Ok(stone));
        })
        .join()
        .unwrap();
    }
    #[test]
    fn numeric_block_index_reorders_refuses_and_clears_failed_rebind() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]]}"#;
        let first = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&first).unwrap();
        let north = BlockStateId(1).try_into_api().unwrap();
        let east = BlockStateId(2).try_into_api().unwrap();
        assert_eq!((north.ordinal(), east.ordinal()), (0, 1));
        assert_eq!(east.try_into_engine(&()), Ok(BlockStateId(2)));
        assert_eq!(
            BlockStateId(u32::MAX).try_into_api(),
            Err(BlockStateError::UnknownState)
        );
        let bad = BlockState::from(mod_api::BlockStateRecord::new(east.block(), u16::MAX));
        assert_eq!(bad.try_into_engine(&()), Err(BlockStateError::UnknownState));
        std::thread::spawn(|| {
            assert_eq!(
                BlockStateId(0).try_into_api(),
                Err(BlockStateError::Unavailable)
            )
        })
        .join()
        .unwrap();
        let reordered = facts
            .replace("north", "TEMP")
            .replace("east", "north")
            .replace("TEMP", "east");
        let reordered = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&reordered).unwrap();
        assert_eq!(BlockStateId(1).try_into_api(), Ok(east));
        assert_eq!(north.try_into_engine(&()), Ok(BlockStateId(2)));
        assert_eq!(snapshot().unwrap()[1], east);
        let broken = facts.replace("east", "south");
        let broken = BlockRegistry::from_facts_json(broken.as_bytes()).unwrap();
        assert!(super::super::resolve_block_definitions(&broken).is_err());
        assert_eq!(east.try_into_engine(&()), Err(BlockStateError::Unavailable));
        assert_eq!(
            BlockStateId(1).try_into_api(),
            Err(BlockStateError::Unavailable)
        );
        assert!(snapshot().is_err());
    }
}
/// Capture this registry's immutable index explicitly; never resolves on a worker.
pub(crate) fn current() -> Result<Arc<Index>, BlockStateError> {
    INDEX.with_borrow(|index| index.clone().ok_or(BlockStateError::Unavailable))
}
impl Index {
    /// Route on workers using the feature-pinned published Parts table, without conversion.
    pub(crate) fn worldgen_listens(&self, hook: mod_api::HookId, state: BlockStateId) -> bool {
        self.forward.get(state.0 as usize).is_some_and(|value| {
            mod_api::__private::worldgen_block_key_listens(hook, value.block())
        })
    }

    pub(crate) fn to_api(&self, state: BlockStateId) -> Result<BlockState, BlockStateError> {
        self.forward
            .get(state.0 as usize)
            .copied()
            .ok_or(BlockStateError::UnknownState)
    }
    pub(crate) fn to_engine(&self, state: BlockState) -> Result<BlockStateId, BlockStateError> {
        let states = self
            .reverse
            .get(&state.block())
            .filter(|states| !states.is_empty())
            .ok_or(BlockStateError::UnknownBlock)?;
        states
            .get(state.ordinal() as usize)
            .copied()
            .flatten()
            .ok_or(BlockStateError::UnknownState)
    }
}
impl TryIntoApi for BlockStateId {
    type Api = BlockState;
    type Error = BlockStateError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        INDEX.with_borrow(|index| {
            index
                .as_ref()
                .ok_or(BlockStateError::Unavailable)?
                .to_api(*self)
        })
    }
}
impl<C: ?Sized> TryIntoEngine<C> for BlockState {
    type Engine = BlockStateId;
    type Error = BlockStateError;
    fn try_into_engine(&self, _: &C) -> Result<Self::Engine, Self::Error> {
        INDEX.with_borrow(|index| {
            index
                .as_ref()
                .ok_or(BlockStateError::Unavailable)?
                .to_engine(*self)
        })
    }
}
