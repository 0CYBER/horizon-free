use horizon_client::motion::{MotionWorld, PlayerMotion};
use horizon_core::geometry::Vec3;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
#[ignore = "daily"]
fn replayed_landing_reads_restitution_again_and_moves_identically() {
    struct Glass(Arc<AtomicUsize>);
    #[mod_api::horizon_part]
    impl mod_api::BlockPart for Glass {
        fn bounce_restitution(
            &self,
            _: mod_api::LevelSide<'_>,
            event: &mut mod_api::BounceRestitutionEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
            event.factor = 0.5;
        }
    }
    struct Mod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Mod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Glass(self.0.clone()));
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "restitution-replay",
        Box::new(Mod(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        crate::game::native_hooks::install_block_contact_hooks();
        super::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let glass = game
                .pack
                .assets
                .registry
                .default_state("minecraft:glass")
                .unwrap();
            game.world.set(3, 64, 5, glass).unwrap();
            let mut run = |replay| {
                let mut player =
                    PlayerMotion::new(game.motion, Vec3::new(3.5, 65.25, 5.5)).unwrap();
                player.velocity.y = -1.0;
                let mut world = MotionWorld {
                    entities: &[],
                    player: &mut player,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    tag_items: Some(game.res.items()),
                    block_registries: Some(&game.pack.block_registries),
                };
                if replay {
                    world.replayed_tick(Default::default(), false).unwrap();
                } else {
                    world
                        .controlled_simulation_tick(Default::default())
                        .unwrap();
                }
                (player.position, player.velocity.y)
            };
            let first = run(false);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            let replay = run(true);
            assert_eq!(first, replay);
            assert_eq!(first.0, Vec3::new(3.5, 65.0, 5.5));
            // The collision uses 1/4 of the tick: (1 + 0.25 * 0.08) *
            // (1 + 0.25 * (0.98f32 - 1)) * 0.5, then gravity 0.08 and drag 0.98f32.
            assert_eq!(first.1, 0.41890101053619394);
            assert_eq!(calls.load(Ordering::Relaxed), 2);
        });
    });
}
