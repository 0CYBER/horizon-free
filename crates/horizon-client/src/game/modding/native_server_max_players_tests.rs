//! `server_max_players` / `server_set_max_players`: the live login limit and each refusal.
use super::native_guest_turn_tests::with_guests;
use mod_api::PlayerLimitRefusal;

/// A limit is read, set live and read back; `i32::MAX` is the highest taken and zero is taken.
#[test]
#[ignore = "daily"]
fn a_mod_reads_and_sets_the_player_limit() {
    with_guests(|game| {
        let before = game.server_max_players().unwrap();
        assert_eq!(game.server_set_max_players(before + 40), Ok(()));
        assert_eq!(game.server_max_players(), Some(before + 40));
        let shared = &game.lan.as_ref().unwrap().hub.lan.shared;
        assert_eq!(shared.max_players(), before as usize + 40);
        assert_eq!(game.server_set_max_players(i32::MAX as u32), Ok(()));
        assert_eq!(game.server_max_players(), Some(i32::MAX as u32));
        assert_eq!(game.server_set_max_players(0), Ok(()));
        assert_eq!(game.server_max_players(), Some(0));
    });
}

/// Each refusal is its own result and changes nothing: a limit above `i32::MAX`, and no server
/// open to players (the read has the one reason, `None`).
#[test]
#[ignore = "daily"]
fn each_player_limit_refusal_has_its_own_name_and_changes_nothing() {
    with_guests(|game| {
        assert_eq!(game.server_set_max_players(7), Ok(()));
        let over = i32::MAX as u32 + 1;
        assert_eq!(
            game.server_set_max_players(over),
            Err(PlayerLimitRefusal::TooHigh {
                maximum: i32::MAX as u32
            })
        );
        assert_eq!(
            game.server_set_max_players(u32::MAX),
            Err(PlayerLimitRefusal::TooHigh {
                maximum: i32::MAX as u32
            })
        );
        assert_eq!(game.server_max_players(), Some(7), "nothing was written");
        let host = game.lan.take();
        assert_eq!(game.server_max_players(), None);
        assert_eq!(
            game.server_set_max_players(9),
            Err(PlayerLimitRefusal::NotOpen)
        );
        assert_eq!(
            game.server_set_max_players(over),
            Err(PlayerLimitRefusal::NotOpen),
            "no server is the first reason"
        );
        game.lan = host;
        assert_eq!(game.server_max_players(), Some(7));
    });
}
