//! `Pufferfish.playerTouch`: the sting game event (`ClientboundGameEventPacket.PUFFER_FISH_STING`)
//! goes out only inside the `hurtServer` branch, and only when the fish is not
//! silent; the poison follows it.
use crate::game::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::mobs::Mob;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::play::Clientbound;

/// A game whose window player touches one puffed (state 2) pufferfish. The
/// server acts for the guest `lan_acting`: `playerTouch` runs there, and the
/// event it sends reaches that player's own client.
fn game_with(silent: bool) -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        false,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.creative = false;
    game.lan_acting = Some(7);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 66.0, 8.5)).unwrap());
    let kind = game.res.mob_types()["minecraft:pufferfish"].clone();
    let mut fish = Mob::new(9001, kind, Vec3::new(8.5, 66.0, 8.5), 0.0);
    fish.extra.size = 2;
    fish.silent = silent;
    game.mobs.push(fish);
    game
}

/// The game events the stung player's client was sent, as `(event, parameter)`.
fn stings(game: &Game<'_>) -> Vec<(u8, f32)> {
    game.guest_packets
        .iter()
        .filter_map(|packet| match packet {
            Clientbound::GameEvent(e) => Some((e.event, e.param)),
            _ => None,
        })
        .collect()
}

fn health(game: &Game<'_>) -> f32 {
    game.player.as_ref().unwrap().health.health()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_sting_event_goes_out_once_per_accepted_hit() {
    let mut g = game_with(false);
    g.touch_pufferfish();
    assert_eq!(health(&g), 17.0, "1 + PuffState 2 of damage landed");
    assert_eq!(stings(&g), vec![(9, 0.0)], "PUFFER_FISH_STING, parameter 0");
    let poison = g.potions.effects.get("poison").expect("poisoned");
    assert_eq!((poison.duration, poison.amplifier), (120, 0));
    // A second touch inside the hurt cooldown is refused (`hurtServer` is
    // false): no damage and no event.
    g.touch_pufferfish();
    assert_eq!(health(&g), 17.0, "the cooldown refused the second hit");
    assert_eq!(stings(&g).len(), 1, "a refused hit sends nothing");
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_silent_fish_damages_and_poisons_without_the_sting() {
    let mut g = game_with(true);
    g.touch_pufferfish();
    assert_eq!(health(&g), 17.0, "the damage still lands");
    assert!(g.potions.effects.get("poison").is_some(), "the poison too");
    assert!(stings(&g).is_empty(), "a silent fish sends no event");
}
