use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before the in-world HUD (`RenderGuiEvent.Pre`): `canceled` hides it as
    /// F1 does.
    pub(in crate::game) fn native_render_gui_pre(&self, canceled: &mut bool) {
        self.fire_render_gui_pre(canceled);
    }
}

#[cfg(test)]
mod local_sound_tests {
    use mod_api::{ClientHost, Flow, Hooks, ModEntry, RenderGuiPreEvent, SoundPlay};
    use mod_platform::testing::with_mods;

    struct Mod;
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn render_gui_pre(&self, host: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
            assert!(
                host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0)
                    .is_some()
            );
            Flow::Continue
        }
    }

    /// A sound a screen hook plays through its read-only host is held, then played by the
    /// next audio update, before its tick.
    #[test]
    #[ignore = "daily"]
    fn a_screen_hooks_sound_reaches_audio_at_the_next_update() {
        let mods: mod_api::Mods = vec![ModEntry::new("click", Box::new(Mod))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::ExecutionRole::Client;
                game.audio.heard.clear();
                game.native_render_gui_pre(&mut false);
                assert_eq!(
                    game.mod_sounds.queued.borrow().len(),
                    1,
                    "held for the audio update"
                );
                assert!(!game.audio.heard.contains_key("minecraft:ui.button.click"));
                game.tick_sound_engine();
                assert!(game.mod_sounds.queued.borrow().is_empty());
                assert_eq!(game.audio.heard.get("minecraft:ui.button.click"), Some(&1));
            });
        });
    }
}
