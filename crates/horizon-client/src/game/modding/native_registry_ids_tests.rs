//! `registry_protocol_id` on a server and on a client, with the ids written out: those of the
//! pinned 26.3 registries report, a native mod's block after vanilla's, and a data registry in the
//! order a server synced it.
use super::lan::LanSettings;
use super::native_entity_refs::tests::{declared_part_root, with_client_game, with_game_at};
use horizon_client::lan::{NetConfig, hub::HubConfig};
use horizon_core::identifier::Identifier;
use horizon_protocol::{
    connection::Pipe,
    packets::configuration::{RegistryData, RegistryEntry},
    registry::Dynamic,
};

fn id(text: &str) -> Identifier {
    Identifier::parse_namespaced(text).unwrap()
}

fn lookup(game: &super::Game<'_>, registry: &str, name: &str) -> Option<i32> {
    game.registry_protocol_id(&id(registry), &id(name))
}

/// Opens `game` to the network, as `with_guests` does, without guests.
fn serve(game: &mut super::Game<'_>) {
    let settings = LanSettings {
        port: 0,
        guest_commands: false,
        force_game_mode: false,
    };
    let net = NetConfig {
        listen: false,
        announce: false,
        lod_distance: 0,
        ..NetConfig::lan(0, String::new(), 2)
    };
    let config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: true,
        seed: 0,
        max_players: 2,
        integrated: true,
        levels: game.level_shapes,
        level_seeds: game.levels.runtime.seeds(),
    };
    game.lan_host(settings, net, config).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_server_answers_vanilla_and_mod_ids() {
    let Some(root) =
        declared_part_root("game::native_registry_ids_tests::a_server_answers_vanilla_and_mod_ids")
    else {
        return;
    };
    with_game_at(root, |game| {
        assert_eq!(
            lookup(game, "minecraft:block", "minecraft:stone"),
            None,
            "no connection yet"
        );
        serve(game);
        assert_eq!(
            lookup(game, "minecraft:particle_type", "minecraft:flame"),
            Some(39)
        );
        assert_eq!(
            lookup(game, "minecraft:item", "minecraft:diamond"),
            Some(1012)
        );
        assert_eq!(lookup(game, "minecraft:block", "minecraft:stone"), Some(1));
        // The fixture's native mod's blocks follow vanilla's 1286, by name: `battery_gives_more`
        // first, `gauge` sixth.
        assert_eq!(
            lookup(game, "minecraft:block", "test:battery_gives_more"),
            Some(1286)
        );
        assert_eq!(lookup(game, "minecraft:block", "test:gauge"), Some(1291));
        assert_eq!(
            lookup(game, "minecraft:particle_type", "minecraft:no_such"),
            None
        );
        assert_eq!(
            lookup(game, "minecraft:no_such_registry", "minecraft:flame"),
            None
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_client_answers_its_own_connections_registries() {
    with_client_game(|game| {
        assert_eq!(
            lookup(game, "minecraft:particle_type", "minecraft:flame"),
            None,
            "no connection"
        );
        let (pipe, _server) = Pipe::pair();
        game.connect_local(pipe, "Registries").unwrap();
        assert_eq!(
            lookup(game, "minecraft:particle_type", "minecraft:flame"),
            Some(39)
        );
        // Not synced yet: a data registry answers nothing.
        assert_eq!(
            lookup(game, "minecraft:damage_type", "minecraft:cactus"),
            None
        );
        // Synced out of alphabetical order: the ids are the order the server sent.
        let mut synced = Dynamic::default();
        synced.add(RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: ["minecraft:in_fire", "minecraft:cactus", "minecraft:arrow"]
                .into_iter()
                .map(|name| RegistryEntry {
                    id: name.into(),
                    data: None,
                })
                .collect(),
        });
        game.remote.as_mut().unwrap().registries = std::sync::Arc::new(synced);
        assert_eq!(
            lookup(game, "minecraft:damage_type", "minecraft:in_fire"),
            Some(0)
        );
        assert_eq!(
            lookup(game, "minecraft:damage_type", "minecraft:cactus"),
            Some(1)
        );
        assert_eq!(
            lookup(game, "minecraft:damage_type", "minecraft:arrow"),
            Some(2)
        );
        assert_eq!(
            lookup(game, "minecraft:damage_type", "minecraft:no_such"),
            None
        );
    });
}
