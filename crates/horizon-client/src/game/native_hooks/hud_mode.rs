use super::*;

impl Game<'_> {
    /// Effective HUD and first-person hand visibility.
    pub(in crate::game) fn hud_mode(&self) -> mod_api::HudMode {
        self.ui.hud_mode()
    }

    /// Set HUD and hand visibility outside a borrowed server guest's turn.
    pub(in crate::game) fn set_hud_mode(&mut self, mode: mod_api::HudMode) {
        if self.native_window() && self.lan_acting.is_none() {
            self.ui.set_hud_mode(mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{ClientHost, ClientTickPreEvent, Hooks, HudMode, ModEntry};
    use mod_platform::testing::with_mods;

    struct ThreeState;
    #[mod_api::__subscriptions]
    impl Hooks for ThreeState {
        fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            host.set_hud_mode(match host.hud_mode() {
                HudMode::AllVisible => HudMode::NoHud,
                HudMode::NoHud => HudMode::AllHidden,
                _ => HudMode::AllVisible,
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn hud_mode_game_bridge_respects_client_and_guest_guards() {
        let mods: mod_api::Mods = vec![ModEntry::new("three-state", Box::new(ThreeState))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                for role in [
                    ExecutionRole::IntegratedServer,
                    ExecutionRole::DedicatedServer,
                ] {
                    game.options.execution_role = role;
                    game.set_hud_mode(HudMode::NoHud);
                    assert_eq!(game.hud_mode(), HudMode::AllVisible);
                }
                game.options.execution_role = ExecutionRole::Client;
                for mode in [HudMode::NoHud, HudMode::AllHidden, HudMode::AllVisible] {
                    game.fire_client_tick_pre();
                    assert_eq!(game.hud_mode(), mode);
                }
                game.lan_acting = Some(17);
                game.fire_client_tick_pre();
                assert_eq!(game.hud_mode(), HudMode::AllVisible);
                game.lan_acting = None;
                game.fire_client_tick_pre();
                assert_eq!(game.hud_mode(), HudMode::NoHud);
                assert!(game.ui.hands_drawn(false, false, true, true, false));
                game.toggle_hide_gui();
                assert_eq!(game.hud_mode(), HudMode::AllVisible);
                game.toggle_hide_gui();
                assert_eq!(game.hud_mode(), HudMode::AllHidden);
            });
        });
    }
}
