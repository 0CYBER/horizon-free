//! A skin change: the profile property is set, and the player is sent to the others again.
use super::native_guest_turn_tests::with_guests;
use mod_api::{GameProfile, ProfileProperty};

#[test]
#[ignore = "daily"]
fn a_new_skin_is_set_and_the_player_sent_again() {
    with_guests(|game| {
        let a = game.player(12).unwrap();
        assert!(game.player_set_profile_property(a, "textures", "old", None));
        assert!(game.player_set_profile_property(a, "textures", "new", Some("signed")));
        assert!(!game.player_set_profile_property(a, "", "value", None));
        assert!(!game.player_set_profile_property(a, "textures", &"x".repeat(32768), None));
        // an astral value counts two UTF-16 units a character: 16400 of them are over the limit
        assert!(!game.player_set_profile_property(
            a,
            "textures",
            &"\u{1F600}".repeat(16_400),
            None
        ));
        assert!(!game.player_set_profile_property(a, &"\u{1F600}".repeat(33), "v", None));
        let properties = |game: &super::Game<'_>| {
            let g = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            g.profile_overrides
                .iter()
                .map(|p| (p.name.clone(), p.value.clone(), p.signature.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            properties(game),
            [("textures".into(), "new".into(), Some("signed".into()))]
        );
        // and the stored properties stayed whole after the refusals above
        assert_eq!(
            properties(game),
            [("textures".into(), "new".into(), Some("signed".into()))]
        );
        // The login properties stay as they were (an offline-mode player has none), `player_profile` with
        // them; the lookups and the player list give what the mod set.
        let set = ProfileProperty::new("textures".into(), "new".into(), Some("signed".into()));
        assert!(
            game.lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .properties
                .is_empty()
        );
        assert_eq!(
            game.player_profile(a),
            Some(GameProfile::new(12, "guest12".into(), Vec::new()))
        );
        let listed = GameProfile::new(12, "guest12".into(), vec![set]);
        assert_eq!(game.profile_by_uuid(12), Some(listed.clone()));
        assert_eq!(game.profile_by_name("guest12"), Some(listed));
        // Hidden from everyone for a whole tick, then shown again.
        assert!(game.player_refresh(a));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        assert_eq!(
            hub.hidden.get(&12),
            None,
            "the refresh leaves the mod's hide level alone"
        );
        assert!(!hub.sees(4, 12));
        hub.ticks += 1;
        hub.end_refreshes();
        assert!(!hub.sees(4, 12));
        hub.ticks += 1;
        hub.end_refreshes();
        assert!(hub.sees(0, 12), "shown again");
        // A player a mod hid stays hidden at that level afterwards, also after a second refresh.
        hub.hidden.insert(13, 3);
        let b = game.player(13).unwrap();
        assert!(game.player_refresh(b));
        assert!(game.player_refresh(b));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        assert!(!hub.sees(255, 13), "out of every list while refreshed");
        hub.ticks += 2;
        hub.end_refreshes();
        assert_eq!(hub.hidden.get(&13), Some(&3));
        assert!(!hub.sees(0, 13) && hub.sees(3, 13));
        // A hide at 255 set during a pending refresh survives its end.
        hub.hidden.insert(13, u8::MAX);
        assert!(game.player_refresh(b));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.ticks += 2;
        hub.end_refreshes();
        assert_eq!(hub.hidden.get(&13), Some(&u8::MAX));
    });
}
