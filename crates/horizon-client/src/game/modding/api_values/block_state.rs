//! Owning-thread, load-bound numeric bridge. Workers receive prepared values instead.
//! The bridge lives in horizon-gameplay; this module keeps its tests, which need the game's registry fixtures.
#[cfg(test)]
use super::{TryIntoApi, TryIntoEngine};
pub(crate) use horizon_gameplay::api_values::block_state::*;
#[cfg(test)]
use horizon_world::registry::{BlockRegistry, BlockStateId};
#[cfg(test)]
use mod_api::{BlockState, BlockStateError};
#[cfg(test)]
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
