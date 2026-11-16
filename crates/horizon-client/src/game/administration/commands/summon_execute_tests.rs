//! `execute summon` goes through `SummonCommand.createEntity` as `/summon`
//! does: the same refusals, and a mob finalized (`Mob.finalizeSpawn`) before
//! it joins; a summoned rabbit rolls its variant there.
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::mobs::{
    Species,
    finalize::{Attr, modifier_amount},
};

#[test]
#[ignore = "daily"]
fn execute_summon_checks_and_finalizes_as_summon_does() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        // Mob.finalizeSpawn's random_spawn_bonus: never on an entity that only loaded.
        let finalized = |m: &horizon_client::mobs::Mob| {
            modifier_amount(m, Attr::FollowRange, "minecraft:random_spawn_bonus").is_some()
        };
        assert!(game.run_command("execute positioned 0.5 100 0.5 summon minecraft:pig run say ok"));
        let pig = game.mobs.last().expect("the pig joined");
        assert_eq!(pig.species(), Species::Pig);
        assert!(finalized(pig), "an execute-summoned pig is finalized");

        let count = game.mobs.len();
        for refused in [
            // ResourceArgument.getSummonableEntityType.
            "execute summon minecraft:player run say ok",
            // Level.isInSpawnableBounds: y below 20000000.
            "execute positioned 0.5 20000000 0.5 summon minecraft:pig run say ok",
            "execute positioned 30000000 100 0.5 summon minecraft:pig run say ok",
        ] {
            assert!(!game.run_command(refused), "{refused}");
        }
        game.ui.difficulty = 0;
        assert!(!game.run_command("execute positioned 0.5 100 0.5 summon minecraft:zombie run say ok"));
        assert!(game.run_command("execute positioned 0.5 100 0.5 summon minecraft:pig run say ok"));
        assert_eq!(game.mobs.len(), count + 1, "only the Peaceful pig joined");

        // Rabbit.finalizeSpawn: forty summoned rabbits are not all the default
        // brown, whatever this biome's rabbits are (brown/salt/black, white, gold).
        for command in [
            "summon minecraft:rabbit 0.5 100 0.5",
            "execute positioned 0.5 100 0.5 summon minecraft:rabbit run say ok",
        ] {
            let from = game.mobs.len();
            for _ in 0..40 {
                assert!(game.run_command(command), "{command}");
            }
            let variants: Vec<u8> = game.mobs[from..].iter().map(|m| m.extra.variant).collect();
            assert!(variants.iter().any(|&v| v != 0), "{command}: {variants:?}");
        }
    });
}
