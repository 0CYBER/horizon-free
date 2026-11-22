use super::Game;

impl Game<'_> {
    /// The normal hotbar's attack indicator progress before validation and layout.
    pub(in crate::game) fn native_hotbar_attack_indicator_progress(&self, progress: &mut f32) {
        self.fire_hotbar_attack_indicator_progress(progress);
    }
}

#[cfg(test)]
mod tests {
    use super::Game;
    use crate::game::{Screen, native_entity_refs::tests::with_client_game};
    use horizon_client::hud::{HudAtlas, HudBatch};
    use mod_api::{ClientHost, Hooks, HotbarAttackIndicatorProgressEvent, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    struct Override {
        progress: f32,
        seen: Arc<Mutex<Vec<f32>>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Override {
        fn hotbar_attack_indicator_progress(
            &self,
            _: &dyn ClientHost,
            event: &mut HotbarAttackIndicatorProgressEvent,
        ) {
            self.seen.lock().unwrap().push(event.progress);
            event.progress = self.progress;
        }
    }

    fn sprite_size(batch: &HudBatch, atlas: &HudAtlas, sprite: &str) -> Option<[f32; 2]> {
        let rect = atlas.sprites[sprite];
        let mut left = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut top = f32::INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        for vertex in &batch.vertices {
            let [u, v] = vertex.uv;
            if (rect.u0..=rect.u1).contains(&u) && (rect.v0..=rect.v1).contains(&v) {
                let [x, y] = vertex.pos;
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        (left < right && top < bottom).then_some([right - left, bottom - top])
    }

    const HOTBAR_PROGRESS: &str = "gui/sprites/hud/hotbar_attack_indicator_progress";
    const CROSSHAIR_PROGRESS: &str = "gui/sprites/hud/crosshair_attack_indicator_progress";

    fn frame(game: &mut Game<'_>) -> HudBatch {
        game.hud_layers([320, 240])
    }

    #[test]
    #[ignore = "daily"]
    fn hotbar_indicator_uses_a_mods_progress_without_changing_the_crosshair_or_charge() {
        with_client_game(|game| {
            game.title = None;
            game.screen = Screen::Playing;
            game.inventory.fill(None);
            game.hand_strength.ticker = 2;
            game.settings.attack_indicator = 2;

            let batch = frame(game);
            assert_eq!(
                sprite_size(&batch, &game.gui.atlas, HOTBAR_PROGRESS),
                Some([18.0, 7.0])
            );

            game.hand_strength.ticker = 5;
            let batch = frame(game);
            assert_eq!(sprite_size(&batch, &game.gui.atlas, HOTBAR_PROGRESS), None);

            let seen = Arc::default();
            let mods = vec![ModEntry::new(
                "indicator",
                Box::new(Override {
                    progress: 0.99,
                    seen: Arc::clone(&seen),
                }),
            )]
            .into();
            with_mods(mods, || {
                let batch = frame(game);
                assert_eq!(
                    sprite_size(&batch, &game.gui.atlas, HOTBAR_PROGRESS),
                    Some([18.0, 18.0])
                );
                assert_eq!(game.hand_strength.ticker, 5);
                assert_eq!(*seen.lock().unwrap(), [1.0]);

                game.hand_strength.ticker = 2;
                game.settings.attack_indicator = 1;
                let batch = frame(game);
                assert_eq!(sprite_size(&batch, &game.gui.atlas, HOTBAR_PROGRESS), None);
                assert_eq!(
                    sprite_size(&batch, &game.gui.atlas, CROSSHAIR_PROGRESS),
                    Some([6.0, 4.0])
                );

                game.settings.attack_indicator = 0;
                let batch = frame(game);
                assert_eq!(sprite_size(&batch, &game.gui.atlas, HOTBAR_PROGRESS), None);
                assert_eq!(
                    sprite_size(&batch, &game.gui.atlas, CROSSHAIR_PROGRESS),
                    None
                );

                game.settings.attack_indicator = 2;
                game.ui.hide_gui = true;
                frame(game);
                assert_eq!(*seen.lock().unwrap(), [1.0]);
            });
            game.ui.hide_gui = false;

            for (progress, expected) in [
                (1.0, None),
                (f32::NAN, Some([18.0, 7.0])),
                (2.0, Some([18.0, 7.0])),
            ] {
                let mods = vec![ModEntry::new(
                    "indicator",
                    Box::new(Override {
                        progress,
                        seen: Arc::clone(&seen),
                    }),
                )]
                .into();
                with_mods(mods, || {
                    let batch = frame(game);
                    assert_eq!(
                        sprite_size(&batch, &game.gui.atlas, HOTBAR_PROGRESS),
                        expected
                    );
                });
            }
        });
    }
}
