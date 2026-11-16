use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Which hearts the local player's health bar draws (`PlayerHeartTypeEvent`): `kind` is
    /// vanilla's pick, and a mod may set another.
    pub(in crate::game) fn native_player_heart_type(
        &mut self,
        kind: &mut mod_api::client::hud_layout::HeartKind,
    ) {
        self.fire_player_heart_type(kind);
    }
}

#[cfg(test)]
mod player_heart_type_tests {
    use mod_api::client::hud_layout::HeartKind;
    use mod_api::{ClientHost, Hooks, ModEntry, PlayerHeartTypeEvent};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// Draws frozen hearts for normal ones and withered for frozen ones, logging each pick.
    struct Swap(Arc<Mutex<Vec<HeartKind>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Swap {
        fn player_heart_type(&self, _: &mut dyn ClientHost, event: &mut PlayerHeartTypeEvent) {
            self.0.lock().unwrap().push(*event.kind);
            *event.kind = match *event.kind {
                HeartKind::Normal => HeartKind::Frozen,
                HeartKind::Frozen => HeartKind::Withered,
                other => other,
            };
        }
    }

    fn add_player(game: &mut super::Game<'_>) {
        let at = horizon_core::geometry::Vec3::ZERO;
        game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap());
    }

    /// `PlayerHeartTypeEvent`: the mod gets vanilla's pick and its answer is drawn; without a
    /// listener the pick is vanilla's.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_changes_the_health_bars_hearts() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::ExecutionRole::Client;
            assert_eq!(game.player_heart_kind(), None, "no player, no hearts");
            add_player(game);
            assert_eq!(game.player_heart_kind(), Some(HeartKind::Normal));
            game.player.as_mut().unwrap().ticks_frozen = 1000;
            assert_eq!(game.player_heart_kind(), Some(HeartKind::Frozen));
            game.player.as_mut().unwrap().ticks_frozen = 0;
        });
        let seen = Arc::default();
        let mods: mod_api::Mods =
            vec![ModEntry::new("swap", Box::new(Swap(Arc::clone(&seen))))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::ExecutionRole::Client;
                add_player(game);
                assert_eq!(game.player_heart_kind(), Some(HeartKind::Frozen));
                game.player.as_mut().unwrap().ticks_frozen = 1000;
                assert_eq!(game.player_heart_kind(), Some(HeartKind::Withered));
                assert_eq!(
                    *seen.lock().unwrap(),
                    [HeartKind::Normal, HeartKind::Frozen]
                );
            });
        });
    }
}
