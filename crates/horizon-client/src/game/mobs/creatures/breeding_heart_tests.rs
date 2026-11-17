use super::*;
/// `Turtle.TurtleBreedGoal.breed` overrides `Animal.breed`, so a bred turtle
/// never runs `finalizeSpawnChildFromBreeding`'s heart burst (entity event
/// 18); `Frog.spawnChildFromBreeding` still does.
#[test]
#[ignore = "daily"]
fn a_bred_turtle_posts_no_heart_burst_but_a_bred_frog_does() {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    game.audio.capture = Some(Vec::new());
    for (kind_id, hearts) in [("minecraft:turtle", 0), ("minecraft:frog", 1)] {
        let kind = game.res.mob_types().get(kind_id).unwrap().clone();
        game.mobs
            .push(Mob::new(1, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0));
        game.mobs
            .push(Mob::new(2, kind, Vec3::new(9.5, 64.0, 8.5), 0.0));
        game.lan_entity_events.clear();
        game.breed(
            1,
            2,
            None,
            None,
            None,
            horizon_client::mobs::BreedExperience::Deferred,
        );
        let burst = game
            .lan_entity_events
            .iter()
            .filter(|(id, event)| *id == 1 && *event == 18)
            .count();
        assert_eq!(
            burst, hearts,
            "{kind_id} heartbeat events: {:?}",
            game.lan_entity_events
        );
        game.mobs.clear();
        game.orbs.clear();
    }
}
