//! Registered server-player context versus a console's execute-on-vehicle source.
use super::vehicle_damage_command_tests::with_game;
use super::{Game, Target};
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

fn query(game: &mut Game<'_>, key: &str, action: &str) -> (i32, i8) {
    use horizon_core::nbt::Tag;
    assert!(
        game.score.storage.get("h007:unit").get(key).is_none(),
        "fresh phase receipt: {key}"
    );
    assert!(game.run_command(&format!(
        "data modify storage h007:unit {key} set value {{result:91,success:17b}}"
    )));
    assert!(game.run_command(&format!("execute store result storage h007:unit {key}.result int 1 store success storage h007:unit {key}.success byte 1 run {action}")));
    let storage = game.score.storage.get("h007:unit");
    let receipt = storage.get(key).unwrap();
    let Some(Tag::Int(result)) = receipt.get("result") else {
        panic!("typed result receipt")
    };
    let Some(Tag::Byte(success)) = receipt.get("success") else {
        panic!("typed success receipt")
    };
    (*result, *success)
}

#[test]
#[ignore = "daily"]
fn console_guest_vehicle_relation_uses_the_registered_rider() {
    with_game(|game| {
        game.world_root = game.options.game_dir.as_ref().unwrap().join("world");
        std::fs::create_dir_all(&game.world_root).unwrap();
        // A world is open: entity handles (`/ride`'s riding checks) need its epoch.
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
        assert!(
            game.player.is_none(),
            "the server source has no local player body"
        );
        assert!(game.source_entity().is_none());
        game.lan_host(
            crate::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "h007".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel();
        let uuid = horizon_protocol::crypto::offline_uuid("Alice");
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid,
                name: "Alice".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            31,
            [4.5, 100.0, 4.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.game_mode = 0;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        let host = game.lan.as_mut().unwrap();
        host.hub.ops.insert(uuid, 4);
        host.hub.refresh_permissions();
        assert_eq!(
            host.hub.guest(2).unwrap().permission,
            4,
            "actual operator binding matches the connected fixture"
        );
        let clock = game.world.game_time;
        let source = game.server_source();
        assert!(source.entity.is_none());
        let pig_uuid = "00000049-0000-0007-0000-00090000000b";
        assert!(game.run_command(
            "summon minecraft:pig 8.5 100 8.5 {UUID:[I;73,7,9,11],NoAI:1b,NoGravity:1b}"
        ));
        let pig = game
            .mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([73, 7, 9, 11]))
            .unwrap();
        assert_eq!(pig.kind.id, "minecraft:pig");
        let pig_id = pig.id;
        let relation =
            "execute as Alice on vehicle if entity @s[type=minecraft:pig,nbt={UUID:[I;73,7,9,11]}]";
        let own_relation =
            "execute on vehicle if entity @s[type=minecraft:pig,nbt={UUID:[I;73,7,9,11]}]";
        let mut observed = Vec::new();
        for (phase, player_mount) in [("console_mounted", false), ("player_mounted", true)] {
            let mounted = if player_mount {
                game.for_player(Target::Guest(2), |g| {
                    g.execute_line(&format!("ride @s mount {pig_uuid}"), g.player_source())
                        .unwrap()
                })
                .unwrap()
            } else {
                game.execute_line(&format!("ride Alice mount {pig_uuid}"), source)
                    .unwrap()
            };
            assert_eq!(
                mounted, 1,
                "actual landed mount must succeed before the relation test"
            );
            let actual = game.command_guest(2).unwrap();
            assert_eq!(actual.uuid, uuid);
            assert_eq!(actual.entity_id, 31);
            assert!(
                actual.placed && actual.loaded && actual.health == 20.0 && actual.game_mode == 0
            );
            assert_eq!(actual.vehicle, Some(pig_id));
            assert!(game.player.is_none() && game.riding.is_none());
            let from_console =
                game.with_source(source, |g| query(g, &format!("{phase}_console"), relation));
            let from_player = game
                .for_player(Target::Guest(2), |g| {
                    assert_eq!(g.lan_acting, Some(2));
                    assert_eq!(g.player.as_ref().unwrap().entity_id, 31);
                    assert_eq!(g.riding, Some(pig_id));
                    g.with_source(g.player_source(), |g| {
                        query(g, &format!("{phase}_player"), own_relation)
                    })
                })
                .unwrap();
            assert_eq!(
                from_player,
                (1, 1),
                "registered rider's own source is a positive control"
            );
            observed.push((from_console, from_player));
            let dismounted = if player_mount {
                game.execute_line("ride Alice dismount", source).unwrap()
            } else {
                game.for_player(Target::Guest(2), |g| {
                    g.execute_line("ride @s dismount", g.player_source())
                        .unwrap()
                })
                .unwrap()
            };
            assert_eq!(dismounted, 1);
            assert_eq!(game.command_guest(2).unwrap().vehicle, None);
            assert_eq!(
                game.with_source(source, |g| query(
                    g,
                    &format!("{phase}_console_empty"),
                    relation
                )),
                (91, 17)
            );
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g.with_source(
                    g.player_source(),
                    |g| query(g, &format!("{phase}_player_empty"), own_relation)
                )),
                Some((91, 17))
            );
            assert!(
                game.player.is_none() && game.riding.is_none() && game.source_entity().is_none()
            );
        }
        assert_eq!(game.world.game_time, clock);
        assert_eq!(
            observed,
            [((1, 1), (1, 1)), ((1, 1), (1, 1))],
            "console relation must resolve the actual registered rider's pig UUID"
        );
    });
}
