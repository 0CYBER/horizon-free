//! Save visibility while a falling block's row is lent out (`tick_one_falling`,
//! game/blocks/falling_blocks.rs).
#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
    use horizon_core::position::ChunkPos;

    struct Observer;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observer {
        fn entity_tick_listened(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityTickListenedEvent,
        ) {
            event.listened = true;
        }
        fn entity_tick_pre(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            mod_api::Flow::Continue
        }
        fn entity_tick_post(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
        }
    }

    #[test]
    #[ignore = "daily"]
    fn terminal_falling_blocks_balance_residency() {
        crate::game::in_own_process(
            "game::falling_save::tests::terminal_falling_blocks_balance_residency",
            || {
                for listening in [false, true] {
                    let mods: mod_api::Mods = if listening {
                        vec![mod_api::ModEntry::new("observer", Box::new(Observer))]
                    } else {
                        Vec::new()
                    }
                    .into();
                    mod_platform::testing::with_mods(mods, || {
                        with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                            let sand = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:sand")
                                .unwrap();
                            for x in 0..6 {
                                assert!(game.spawn_falling_block(
                                    game.dim.id(),
                                    sand,
                                    [f64::from(x), 70.0, 0.5],
                                    [0.0; 3],
                                    mod_api::Falling::default()
                                ));
                            }
                            game.reset_entity_residency();
                            // An observer activates the same dispatch without writing entity state.
                            let round = game.native_tick_round();
                            assert_eq!(round.listening(), listening);
                            game.native_tick_post_batch(round);
                            // Expired blocks permanently leave the resident list.
                            for _ in 0..6 {
                                game.falling.update(0, |block| {
                                    block.state = horizon_world::registry::BlockStateId(0)
                                });
                                game.tick_one_falling(0, &[]).unwrap();
                            }
                            assert!(game.falling.is_empty());
                            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
                        });
                    });
                }
            },
        );
    }
}
