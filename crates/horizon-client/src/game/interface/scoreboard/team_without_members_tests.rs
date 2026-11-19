use super::{tame_owner, uuid_text};
use crate::game::Game;

/// With teams but no holder on one, no mob has a team (the answer the per-entity pass gave); once
/// the player joins a team, the pass still finds it.
#[test]
#[ignore = "daily"]
fn a_team_without_members_gives_the_mobs_none() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        assert!(game.mob_teams().mobs.is_empty() && game.mob_teams().player.is_none());
        game.score.board.add_team("x");
        let teams = game.mob_teams();
        assert!(teams.mobs.is_empty());
        assert!(teams.player.is_none());
        let name = game.acting_player_name().to_owned();
        game.score.board.add_to_team(&name, "x");
        let teams = game.mob_teams();
        assert_eq!(teams.player.map(|t| t.name.as_str()), Some("x"));
    });
}

/// The member-driven pass of `mob_teams` finds the teams the per-entity pass finds, through board edits
/// between asks: a holder added or removed, a second team, a name that only looks like a UUID.
#[test]
#[ignore = "daily"]
fn team_pass_by_members_equals_the_full_pass() {
    {
        let mut owned = crate::game::native_machines::tests::server_game();
        let game = &mut owned;
        game.build_registries();
        game.set_allow_commands(true);
        for c in [
            "setblock 8 79 8 stone",
            "summon pig 8.5 80 8.5 {NoAI:1b}",
            "summon cow 9.5 80 8.5 {NoAI:1b}",
            "summon sheep 10.5 80 8.5 {NoAI:1b}",
            "summon chicken 11.5 80 8.5 {NoAI:1b}",
        ] {
            assert!(game.run_command(c), "{c}");
        }
        let ids: Vec<u32> = game.mobs.iter().map(|m| m.id).collect();
        assert!(ids.len() >= 4);
        let texts: Vec<String> = ids
            .iter()
            .map(|id| uuid_text(*game.persist.uuids.get(id).unwrap()))
            .collect();
        let snapshot = |game: &Game<'_>| -> Vec<(u32, String)> {
            let mut v: Vec<(u32, String)> = game
                .mob_teams()
                .mobs
                .iter()
                .map(|(id, t)| (*id, t.name.clone()))
                .collect();
            v.sort();
            v
        };
        let reference = |game: &Game<'_>| -> Vec<(u32, String)> {
            let mut v: Vec<(u32, String)> = game
                .persist
                .uuids
                .keys()
                .filter_map(|id| {
                    let owner = || game.mobs.iter().find(|m| m.id == *id).and_then(tame_owner);
                    Some((*id, game.team_with_owner(*id, owner)?.name.clone()))
                })
                .collect();
            v.sort();
            v
        };
        let check = |game: &mut Game<'_>, step: &str| {
            let new = snapshot(game);
            let old = reference(game);
            assert_eq!(new, old, "{step}");
            new.len()
        };
        game.score.board.add_team("a");
        game.score.board.add_team("b");
        assert_eq!(check(game, "two empty teams"), 0);
        game.score.board.add_to_team(&texts[0], "a");
        assert_eq!(check(game, "one pig on a"), 1);
        game.score.board.add_to_team(&texts[1], "b");
        game.score.board.add_to_team(&texts[2].to_uppercase(), "a");
        game.score.board.add_to_team("not a uuid", "b");
        assert_eq!(check(game, "upper-case text does not name a mob"), 2);
        game.score.board.add_to_team(&texts[0], "b");
        assert_eq!(check(game, "the pig moved to b"), 2);
        game.score.board.remove_from_team(&texts[1]);
        assert_eq!(check(game, "the cow removed"), 1);
    }
}
