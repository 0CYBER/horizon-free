//! The raid bar's viewers are `ServerBossEvent.players`, which
//! `Raid.updatePlayers` refreshes on the 20-tick beats: walking out of the
//! raid keeps the red bar until the next beat.
use super::super::Game;
use horizon_client::chunk::Chunk;
use horizon_core::{geometry::Vec3, position::ChunkPos};
use std::sync::Arc;

/// The red, ten-notched bars on this client's overlay: the raid's.
fn raid_bars(game: &Game<'_>) -> usize {
    game.boss_overlay
        .bars(horizon_client::boss_overlay::now_millis())
        .into_iter()
        .filter(|b| b.color == 2 && b.overlay == 2)
        .count()
}

#[test]
#[ignore = "daily"]
fn the_raid_bar_keeps_a_viewer_until_the_next_twenty_tick_beat() {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    game.world_root =
        std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("raid-bar-world");
    for x in -5..5 {
        for z in -5..5 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
    }
    let registry = game.pack.assets.registry;
    // A village: a claimed bed, which `raid_center` turns into the raid's.
    let foot = registry.default_state("minecraft:white_bed").unwrap();
    let mut head_state = registry.state(foot).unwrap().clone();
    head_state.properties.insert("part".into(), "head".into());
    let head = registry.id(&head_state).unwrap();
    game.world.set(2, 200, 2, foot).unwrap();
    game.world.set(2, 200, 1, head).unwrap();
    let kind = game
        .res
        .mob_types()
        .get("minecraft:villager")
        .unwrap()
        .clone();
    let mut villager = horizon_client::mobs::Mob::new(162, kind, Vec3::new(3.5, 200.0, 2.5), 0.0);
    villager.no_ai = true;
    villager.extra.bed = Some([2, 200, 1]);
    game.mobs.push(villager);
    // The local player, at the raid's centre.
    let ints = [0, 0, 0, 1];
    game.level.player_uuid = Some(ints);
    game.player =
        Some(super::super::PlayerMotion::new(game.motion, Vec3::new(8.5, 200.0, 8.5)).unwrap());
    assert!(game.raid_command_create([8, 200, 8], 1));
    // The first tick is a beat (the countdown is 300 ticks): the player joins.
    game.tick_raids();
    game.publish_boss_bars();
    assert_eq!(raid_bars(&game), 1, "the player stands in the raid");
    // Walking out of `getRaidAt` (96 blocks) must not drop the bar before the
    // next beat: ticks 2 to 19 are not beats.
    game.player.as_mut().unwrap().position = Vec3::new(200.5, 200.0, 8.5);
    for tick in 2..20 {
        game.tick_raids();
        game.publish_boss_bars();
        assert_eq!(raid_bars(&game), 1, "the bar is still shown at tick {tick}");
    }
    // Tick 20 is (`ticksActive % 20 == 0`): the bar goes.
    game.tick_raids();
    game.publish_boss_bars();
    assert_eq!(raid_bars(&game), 0);
    // Walking back in waits for a beat too: tick 21 is the countdown's
    // (cooldown 280), ticks 22 to 39 are not.
    game.tick_raids();
    game.publish_boss_bars();
    game.player.as_mut().unwrap().position = Vec3::new(8.5, 200.0, 8.5);
    for tick in 22..40 {
        game.tick_raids();
        game.publish_boss_bars();
        assert_eq!(raid_bars(&game), 0, "the bar waits at tick {tick}");
    }
    // Tick 40 is (`ticksActive % 20 == 0`): the player is back in the raid.
    game.tick_raids();
    game.publish_boss_bars();
    assert_eq!(raid_bars(&game), 1, "the player stands in the raid again");
    // Dying leaves the set on a beat as well (`validPlayer`'s `isAlive`): tick
    // 41 is the countdown's (cooldown 260), ticks 42 to 59 are not.
    game.tick_raids();
    game.publish_boss_bars();
    game.player
        .as_mut()
        .unwrap()
        .set_health(0.0, false)
        .unwrap();
    for tick in 42..60 {
        game.tick_raids();
        game.publish_boss_bars();
        assert_eq!(raid_bars(&game), 1, "the bar outlives death at tick {tick}");
    }
    // Tick 60 is (`ticksActive % 20 == 0`): the dead player leaves the bar.
    game.tick_raids();
    game.publish_boss_bars();
    assert_eq!(raid_bars(&game), 0);
}
