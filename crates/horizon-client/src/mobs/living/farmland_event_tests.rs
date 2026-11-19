use super::*;
#[test]
#[ignore = "daily"]
fn a_mob_trampling_farmland_posts_block_change_from_it() {
    let ran = crate::mobs::world::with_built_test_world(
        &[],
        |world, registry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let state = registry.default_state("minecraft:farmland").unwrap();
            world.set(0, 0, 0, state).unwrap();
        },
        |w, types| {
            let dirt = w.registry.default_state("minecraft:dirt").unwrap();
            let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
            let mut mob = Mob::new(7, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
            mob.fall_distance = 5.0;
            let mut events = Vec::new();
            let mut damage = crate::health::TestLevel::new(0);
            mob.fall_on_block(w, &mut damage.level(), &mut events);
            // The write, then the event the rule posts for it.
            assert!(
                matches!(
                    events.as_slice(),
                    [
                        MobEvent::SetBlock {
                            pos: [0, 0, 0],
                            state,
                        },
                        MobEvent::LandingGameEvent {
                            event: "minecraft:block_change",
                            at: [0, 0, 0],
                            source: Some(7),
                            state: affected,
                        },
                        ..
                    ] if *state == dirt && *affected == dirt
                ),
                "{events:?}"
            );
        },
    );
    // Without the vanilla data under .local/horizon-client the world
    // cannot be built: the test has nothing to check.
    assert!(ran, "vanilla data is missing");
}
