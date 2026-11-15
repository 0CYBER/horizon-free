use super::{Game, commands::Target};
use crate::game::api_values::TryIntoEngine;

impl Game<'_> {
    pub(super) fn post_contextual_game_event(
        &mut self,
        event: &'static str,
        at: [f64; 3],
        context: mod_api::GameEventContext,
    ) -> bool {
        let state = match context
            .affected_state
            .map(|state| state.try_into_engine(&()))
            .transpose()
        {
            Ok(state) => state,
            Err(_) => return false,
        };
        let Some(entity) = context.source_entity else {
            let context = self.who_context(None, state);
            self.post_game_event(event, at, context);
            return true;
        };
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if !self.native_entity_here(entity) {
            return false;
        }
        if let Target::Guest(conn) = target {
            let Some(uuid) = self.command_guest(conn).map(|guest| guest.uuid) else {
                return false;
            };
            // Capture and post inside the same guest scope: Who::Player and the
            // cause id must still identify the acting guest when listeners run.
            return self.native_as_player(uuid, |game| {
                let context = game.player_context(state);
                game.post_game_event(event, at, context);
            });
        }
        let context = match target {
            Target::Player => self.player_context(state),
            Target::Mob(id) => self.mob_context(id, state),
            _ => self.entity_event_context(entity.id(), state),
        };
        self.post_game_event(event, at, context);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::game::api_values::TryIntoApi;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{chunk::Chunk, items::ItemStack};
    use horizon_client_core::who::EventSource;
    use mod_api::{EntityRef, GameEventContext};
    use std::sync::Arc;

    fn sensor<'a>(
        game: &'a mut crate::game::Game<'_>,
    ) -> Option<&'a mut horizon_client::sculk::SculkEntity> {
        match game.world.block_entities.get_mut(&(1, 64, 1)) {
            Some(horizon_client::containers::BlockEntity::Sculk(entity)) => Some(entity),
            _ => None,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn contextual_game_event_keeps_guest_cause_until_emission() {
        use crate::game::native_item_places::tests::open_to_lan;
        use crate::game::native_sound_refs::tests::guest;
        use std::sync::Mutex;
        struct Heard(Arc<Mutex<Vec<Option<u32>>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Heard {
            fn vanilla_game_event(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::VanillaGameEventEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push(event.cause);
                mod_api::Flow::Continue
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "guest_events",
                Box::new(Heard(heard.clone())),
            )]
            .into(),
            || {
                with_game(|game| {
                    open_to_lan(game);
                    let (guest, _receiver) = guest(7, 99, [2.5, 64.0, 1.5]);
                    let id = guest.entity_id as u32;
                    game.lan.as_mut().unwrap().hub.guests.push(guest);
                    let source = game.entity(id).unwrap();
                    let level = game.level(game.dim.id()).unwrap();
                    heard.lock().unwrap().clear();
                    assert!(game.lan_acting_entity.is_none());
                    assert!(game.level_game_event(
                        level,
                        "minecraft:step",
                        [3.5, 64.0, 1.5],
                        GameEventContext {
                            source_entity: Some(source),
                            affected_state: None
                        }
                    ));
                    assert!(
                        game.lan_acting_entity.is_none(),
                        "guest scope must be restored"
                    );
                    assert_eq!(*heard.lock().unwrap(), [Some(id)]);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn sensor_receives_context_source_and_affected_state_at_requested_position() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.sculk_prepare();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = &game.pack.assets.registry;
            let sensor_state = registry.default_state("minecraft:sculk_sensor").unwrap();
            let stone = registry
                .default_state("minecraft:stone")
                .unwrap()
                .try_into_api()
                .unwrap();
            let wool = registry
                .default_state("minecraft:white_wool")
                .unwrap()
                .try_into_api()
                .unwrap();
            game.world.set(1, 64, 1, sensor_state).unwrap();
            game.sculk_sync_index();
            game.spawn_item(
                ItemStack::new("minecraft:stone", 1),
                [12.5, 64.0, 1.5],
                [0.0; 3],
                0,
            );
            let source = game.entity(game.dropped[0].id).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let at = [3.5, 64.0, 1.5];
            assert!(game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext {
                    source_entity: Some(source),
                    affected_state: Some(stone)
                }
            ));
            let vibration = &mut sensor(game).unwrap().vibration;
            let (info, _) = vibration.selector.current.take().unwrap();
            assert_eq!(info.source, Some(EventSource::Entity(source.id())));
            assert_eq!(
                info.pos, at,
                "the explicit event position must not become the source position"
            );
            assert!(game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext {
                    source_entity: Some(source),
                    affected_state: Some(wool)
                }
            ));
            assert!(
                sensor(game).unwrap().vibration.selector.current.is_none(),
                "affected wool must dampen the vibration"
            );
            let stale = EntityRef::new(
                mod_api::WorldEpoch::new(source.epoch().get().wrapping_add(1)),
                source.id(),
            );
            assert!(!game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext {
                    source_entity: Some(stale),
                    affected_state: None
                }
            ));
            assert!(sensor(game).unwrap().vibration.selector.current.is_none());
            assert!(game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext::default()
            ));
            let (info, _) = sensor(game)
                .unwrap()
                .vibration
                .selector
                .current
                .take()
                .unwrap();
            assert_eq!(info.source, None);
        });
    }
}
