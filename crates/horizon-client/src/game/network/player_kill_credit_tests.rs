//! `ServerPlayer.die`'s `killer.awardKillScore(this, source)` and
//! `this.createWitherRose(killer)` for a dying player: the killer's kill
//! counts and `player_kills` whatever killed them, and the wither rose a
//! wither's kill leaves where they fell.
use super::*;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
    scoreboard::RenderType,
    text::Text,
};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// The guest's connection: the victim of every test.
const CONN: u32 = 2;
const VICTIM: &str = "H039Victim";
/// A second guest, the killer of `a_guest_killing_another_guest_credits_the_kill`.
const KILLER_CONN: u32 = 3;
const KILLER: &str = "H039Killer";
/// The host's own body (`self.player`), the player that kills it.
const HOST: u32 = 100;

/// A host on a dirt floor, its own player at (4.5, 100, 6.5), and one
/// connected, placed guest standing on the floor at (22.5, 100, 9.5).
fn game<'a>(resources: &'a mut Option<Arc<crate::game::Resources>>) -> Game<'a> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(x, 0))
            .unwrap();
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 dirt"));
    // A hostile mob spawns and the player takes damage on any but peaceful.
    assert!(game.run_command("difficulty normal"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 6.5)).unwrap());
    game.player.as_mut().unwrap().entity_id = HOST as i32;
    game.lan_host(
        super::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::lan(0, "h039-test".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .push(guest(CONN, 2, VICTIM, [22.5, 100.0, 9.5]));
    game
}

/// A connected, placed guest at `at`, whose client has loaded the world
/// (`ServerGamePacketListenerImpl.hasClientLoaded`).
fn guest(conn: u32, entity_id: i32, name: &str, at: [f64; 3]) -> Guest {
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: u128::from(conn),
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        entity_id,
        at,
        1,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest
}

/// Hurts the guest to death from `attacker` and runs `ServerPlayer.die` on it.
/// A player's hit is noted as one (`Who::Player`), a mob's is looked up by
/// `hurt_player_from`.
fn kill(game: &mut Game<'_>, attacker: u32, ty: &str, player: bool) {
    game.as_guest(CONN, |g| {
        let known = (!player).then_some(attacker);
        if player {
            g.note_player_attacker(ty, attacker, Text::literal(super::chat::PLAYER_NAME), None);
        }
        assert!(
            g.hurt_player_from(100.0, Some([22.5, 100.0, 9.5]), ty, Some(known), false),
            "the guest is hurt"
        );
    })
    .unwrap();
    assert!(
        game.lan.as_ref().unwrap().hub.guests[0].health <= 0.0,
        "the guest is dying"
    );
    game.guest_die(0, CONN);
    assert!(
        game.lan.as_ref().unwrap().hub.guests[0].dead,
        "health 0 must kill"
    );
}

/// The score `holder` has on the objective `objective`, if any.
fn score(game: &Game<'_>, holder: &str, objective: &str) -> Option<i32> {
    game.score.board.score(holder, objective).map(|s| s.value)
}

fn add_objectives(game: &mut Game<'_>) {
    for (name, criteria) in [("kills", "totalKillCount"), ("pvp", "playerKillCount")] {
        assert!(game.score.board.add_objective(
            name,
            criteria,
            serde_json::json!(name),
            RenderType::Integer,
            false,
            None,
        ));
    }
}

/// `ServerPlayer.awardKillScore`: a player's blow that kills another player
/// counts for `KILL_COUNT_ALL`, `KILL_COUNT_PLAYERS` and `PLAYER_KILLS`.
#[test]
#[ignore = "daily"]
fn a_player_killing_another_player_credits_the_kill() {
    let mut resources = None;
    let mut game = game(&mut resources);
    add_objectives(&mut game);
    kill(&mut game, HOST, "minecraft:player_attack", true);
    assert_eq!(score(&game, super::chat::PLAYER_NAME, "kills"), Some(1));
    assert_eq!(score(&game, super::chat::PLAYER_NAME, "pvp"), Some(1));
    assert_eq!(
        game.stats.get("custom", "minecraft:player_kills"),
        1,
        "PLAYER_KILLS"
    );
    // The victim's own side: no kill, no player kill.
    assert_eq!(score(&game, VICTIM, "kills"), None);
    assert_eq!(
        game.lan
            .as_ref()
            .unwrap()
            .play
            .get(&CONN)
            .map(|p| p.stats.get("custom", "minecraft:player_kills")),
        Some(0)
    );
}

/// The same for a LAN guest's kill: a hosted world's players are all guests,
/// the owner's window too, so the killer's own record takes the kill.
#[test]
#[ignore = "daily"]
fn a_guest_killing_another_guest_credits_the_kill() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .push(guest(KILLER_CONN, 3, KILLER, [18.5, 100.0, 9.5]));
    add_objectives(&mut game);
    kill(&mut game, 3, "minecraft:player_attack", true);
    assert_eq!(score(&game, KILLER, "kills"), Some(1));
    assert_eq!(score(&game, KILLER, "pvp"), Some(1));
    assert_eq!(
        game.lan
            .as_ref()
            .unwrap()
            .play
            .get(&KILLER_CONN)
            .map(|p| p.stats.get("custom", "minecraft:player_kills")),
        Some(1),
        "PLAYER_KILLS"
    );
    // The host's own body is not the killer.
    assert_eq!(score(&game, super::chat::PLAYER_NAME, "kills"), None);
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 0);
}

/// `LivingEntity.createWitherRose`: a player a wither kills leaves a rose where
/// they died, and a mob's kill credits no score.
#[test]
#[ignore = "daily"]
fn a_wither_killing_a_player_leaves_a_rose() {
    let mut resources = None;
    let mut game = game(&mut resources);
    add_objectives(&mut game);
    assert!(
        game.run_command("summon wither 8.5 100 4.5 {NoAI:1b,Silent:1b,PersistenceRequired:1b}")
    );
    let wither = game.mobs.last().unwrap().id;
    kill(&mut game, wither, "minecraft:mob_attack", false);
    let name = &*game.res.facts().get(game.world.block(22, 100, 9)).name;
    assert_eq!(name, "minecraft:wither_rose", "the rose block");
    assert_eq!(
        score(&game, super::chat::PLAYER_NAME, "kills"),
        None,
        "a mob's kill credits no score"
    );
    assert_eq!(game.stats.get("custom", "minecraft:player_kills"), 0);
    assert!(game.dropped.is_empty(), "the block leaves no item");
}

/// The same with mobGriefing off: the rose is an item at the death position.
#[test]
#[ignore = "daily"]
fn a_wither_killing_a_player_without_mob_griefing_drops_the_rose() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // Experience to drop: `dropAllDeathLoot` drops it before the rose.
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.xp_level = 5);
    assert!(game.run_command("gamerule mob_griefing false"));
    assert!(
        game.run_command("summon wither 8.5 100 4.5 {NoAI:1b,Silent:1b,PersistenceRequired:1b}")
    );
    let wither = game.mobs.last().unwrap().id;
    kill(&mut game, wither, "minecraft:mob_attack", false);
    let rose = game.dropped.last().expect("a rose item");
    assert_eq!(&*rose.stack.item, "minecraft:wither_rose");
    assert_eq!(rose.stack.count, 1);
    let p = rose.position;
    assert_eq!((p.x, p.y, p.z), (22.5, 100.0, 9.5));
    // `dropAllDeathLoot` drops the experience first: the orbs take their ids
    // and their own randoms' seeds before the rose's item entity.
    assert!(!game.orbs.is_empty(), "the experience orbs");
    assert!(
        game.orbs.iter().all(|o| o.id < rose.id),
        "the rose's item entity comes after the orbs"
    );
}
