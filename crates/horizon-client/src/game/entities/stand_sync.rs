//! The test that a stand nobody has borrowed mutably keeps the network entry it had (see
//! `lan_entities.rs`, `Joined::touch`).
#[cfg(test)]
mod stand_sync_flag_tests {
    use horizon_core::{geometry::Vec3, position::BlockPos};

    /// A stand nobody has touched since its network entry was built has the entry it had: with
    /// `syncverify` every clean stand is rebuilt anyway and compared to the kept entry. The script
    /// touches stands in every way a command or a tick can: an armor piece, invisibility, a name,
    /// a pose, health, a teleport, the floor taken away (it falls), a stand killed, a new one.
    #[test]
    #[ignore = "daily"]
    fn a_stand_nobody_touched_has_the_entry_it_had() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.level.health = 20.0;
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(14.5, 200.0, 14.5))
                .unwrap(),
        );
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        // Above the guest's fixed entity id.
        game.next_entity = 100;
        let _rx = crate::game::lan::world_spawn_sync_tests::host_for(&mut game);
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        for n in 0..5 {
            assert!(game.run_command(&format!(
                "summon armor_stand {}.5 200.0 {}.5 {{Tags:[\"s{n}\"],Health:20f}}",
                2 + n * 2,
                3 + n
            )));
        }
        crate::game::lan_entities::VERIFY_STAND_STATES
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut states = horizon_client::lan::entity::States::default();
        let mut clean = 0usize;
        let mut step = |game: &mut crate::game::Game<'_>, ticks: u32, clean: &mut usize| {
            for _ in 0..ticks {
                game.tick_armor_stands().unwrap();
                game.lan_entities(&mut states, false, 0, None);
                *clean += game.armor_stands.iter().filter(|a| !a.sync_dirty).count();
            }
        };
        step(&mut game, 20, &mut clean);
        for command in [
            "item replace entity @e[tag=s0,limit=1] armor.head with minecraft:diamond_helmet",
            "data merge entity @e[tag=s1,limit=1] {Invisible:1b}",
            "data merge entity @e[tag=s2,limit=1] {CustomName:'\"Bob\"',CustomNameVisible:1b}",
            "data merge entity @e[tag=s3,limit=1] {Pose:{Head:[10f,20f,30f]}}",
            "data merge entity @e[tag=s4,limit=1] {Health:5f}",
            "tp @e[tag=s0,limit=1] 5.5 200.0 9.5",
            "setblock 6 199 4 air",
            "summon armor_stand 1.5 200.0 12.5 {Tags:[\"new\"]}",
            "kill @e[tag=s2,limit=1]",
            "reload",
        ] {
            assert!(game.run_command(command), "{command}");
            if command == "reload" {
                // Every kept entry is dropped and every stand rebuilt.
                assert!(
                    game.stand_states.is_empty(),
                    "a reload clears the kept entries"
                );
                assert!(
                    game.armor_stands.iter().all(|a| a.sync_dirty),
                    "and marks every stand dirty"
                );
            }
            step(&mut game, 12, &mut clean);
        }
        assert!(
            clean > 100,
            "stands were clean (and verified) most ticks: {clean}"
        );
        crate::game::lan_entities::VERIFY_STAND_STATES
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
