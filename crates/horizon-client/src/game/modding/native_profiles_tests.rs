//! Game profiles the server knows by UUID or name.
use super::native_guest_turn_tests::with_guests;
use horizon_protocol::packets::login::Property;
use mod_api::{GameProfile, ProfileProperty};

#[test]
#[ignore = "daily"]
fn profiles_come_from_the_players_on_without_a_lookup() {
    with_guests(|game| {
        let i = game.guest_index(12).unwrap();
        game.lan.as_mut().unwrap().hub.guests.update(i, |g| {
            g.properties = vec![Property {
                name: "textures".into(),
                value: "e30=".into(),
                signature: Some("sig".into()),
            }];
        });
        let skin = GameProfile::new(
            12,
            "guest12".into(),
            vec![ProfileProperty::new(
                "textures".into(),
                "e30=".into(),
                Some("sig".into()),
            )],
        );
        let a = game.player(12).unwrap();
        assert_eq!(game.player_profile(a), Some(skin.clone()));
        assert_eq!(game.profile_by_uuid(12), Some(skin.clone()));
        assert_eq!(game.profile_by_name("GUEST12"), Some(skin));
        assert_eq!(
            game.profile_by_uuid(13),
            Some(GameProfile::new(13, "guest13".into(), Vec::new()))
        );
        // Not on and not a dedicated server's user cache: unknown, never made up.
        assert_eq!(game.profile_by_uuid(99), None);
        assert_eq!(game.profile_by_name("Notch"), None);
    });
}
