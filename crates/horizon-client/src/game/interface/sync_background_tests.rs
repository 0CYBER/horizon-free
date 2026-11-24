//! Regressions for the typed bindings carried by the upstream sync.
use super::native_entity_refs::tests::{with_client_game, with_game_at};
use horizon_client::chunk::Chunk;
use mod_api::{ClientHost, Flow, Hooks, ModEntry, ServerHost};
use std::{path::PathBuf, sync::Arc};

mod background {
    use super::*;
    #[derive(Default)]
    struct Background;
    #[mod_api::horizon_mod]
    impl Hooks for Background {
        fn screen_transparent_background(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::ScreenTransparentBackgroundEvent<'_>,
        ) -> Flow {
            if event.screen != "inventory" {
                return Flow::Continue;
            }
            assert!(host.apply_screen_blur());
            assert!(!host.apply_screen_blur());
            event.batch.fill([3.0, 4.0, 5.0, 6.0], 0xff123456);
            Flow::Cancel
        }
    }
}
#[test]
#[ignore = "daily"]
fn cancelled_background_keeps_custom_geometry_and_one_blur_request() {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("background", background::hooks())].into(),
        || {
            with_client_game(|game| {
                let mut batch = horizon_client::hud::HudBatch::scratch([800, 600]);
                game.render_transparent_background("inventory", &mut batch);
                assert_eq!(
                    batch.vertices.len(),
                    6,
                    "custom quad only, no vanilla gradient"
                );
                assert!(game.screen_blur_requested.get());
                batch.vertices.clear();
                game.render_transparent_background("chest", &mut batch);
                assert_eq!(
                    batch.vertices.len(),
                    6,
                    "uncancelled screen keeps its gradient"
                );
                game.screen_blur_requested.set(false);
                game.render_transparent_background("inventory", &mut batch);
                assert_eq!(
                    batch.vertices.len(),
                    12,
                    "the next frame may request blur again"
                );
            })
        },
    );
}

/// `DrawContext::blur_behind`: what was drawn before the call is the blurred background, and the
/// frame asks for the menu blur at the player's radius.
#[test]
#[ignore = "daily"]
fn blur_behind_marks_the_drawn_vertices_and_asks_for_the_menu_blur() {
    with_client_game(|game| {
        let mut batch = horizon_client::hud::HudBatch::scratch([800, 600]);
        batch.fill(&game.gui.atlas, 0.0, 0.0, 10.0, 10.0, 0xff00_00ff);
        game.settings.menu_blur = 5;
        assert_eq!(
            game.menu_blur(),
            None,
            "no screen blurs the world behind it"
        );
        game.native_blur_behind(&mut batch);
        batch.fill(&game.gui.atlas, 20.0, 20.0, 10.0, 10.0, 0xffff_ffff);
        assert_eq!(
            batch.blur_at, 6,
            "the first quad is behind the blur, the second sharp"
        );
        assert_eq!(game.menu_blur(), Some(5.0));
        game.settings.menu_blur = 0;
        assert_eq!(game.menu_blur(), None, "a radius of 0 blurs nothing");
    });
}

mod claims {
    use super::*;
    #[derive(Default)]
    struct Claims;
    #[mod_api::horizon_mod]
    impl Hooks for Claims {
        fn entity_change_block_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::EntityChangeBlockPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.player, Some(7));
            assert_eq!(event.entity, None);
            assert_eq!(event.kind, "minecraft:player");
            assert_eq!(Some(event.from), host.block_state_id("minecraft:farmland"));
            assert_eq!(Some(event.to), host.block_state_id("minecraft:dirt"));
            Flow::Cancel
        }
    }
}
#[test]
#[ignore = "daily"]
fn cancelled_landing_restores_farmland_before_its_crop_is_notified() {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    mod_platform::testing::with_mods(
        vec![ModEntry::new("claims", claims::hooks())].into(),
        || {
            with_game_at(root, |game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let (farmland, dirt, wheat) = (
                    state("minecraft:farmland"),
                    state("minecraft:dirt"),
                    state("minecraft:wheat"),
                );
                game.world.set(1, 64, 1, dirt).unwrap();
                game.world.set(1, 65, 1, wheat).unwrap();
                game.landing_block_updates(
                    Some(7),
                    &horizon_client::landing::FallEvent::Write {
                        position: horizon_core::position::BlockPos::new(1, 64, 1),
                        from: farmland,
                        state: dirt,
                        flags: 3,
                    },
                );
                assert_eq!(game.world.block(1, 64, 1), farmland);
                assert_eq!(game.world.block(1, 65, 1), wheat);
            })
        },
    );
}
