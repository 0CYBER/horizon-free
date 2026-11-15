use super::*;
use horizon_client::mobs::{MobTypes, render};
use std::sync::Arc;
fn table() -> ResolvedProfessions {
    ordered_table(&["test:engineer"])
}
fn ordered_table(names: &[&str]) -> ResolvedProfessions {
    let id = |name| mod_api::Identifier::parse(name).unwrap();
    ResolvedProfessions::prepare(
        names
            .iter()
            .map(|name| {
                (
                    "test".into(),
                    mod_api::ProfessionDefinition {
                        key: mod_api::ProfessionKey(id(name)),
                        workstations: vec![],
                        held_poi: vec![],
                        acquirable_poi: vec![],
                        requested_items: vec![],
                        secondary_blocks: vec![],
                        work_sound: None,
                        trades: vec![],
                    },
                )
            })
            .collect(),
        |_, _| true,
    )
    .unwrap()
}
fn roundtrip(kind: &str, custom: bool, texture: &str) {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources"),
    );
    let types = MobTypes::load(&root).unwrap();
    let kind = Arc::new(types.types.get(kind).unwrap().clone());
    let mut server = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
    let server_table = if custom {
        table()
    } else {
        ResolvedProfessions::vanilla()
    };
    let client_table = if custom {
        table()
    } else {
        ResolvedProfessions::vanilla()
    };
    let profession = if custom {
        "test:engineer"
    } else {
        "minecraft:farmer"
    };
    let slot = server_table
        .resolve(profession)
        .expect("registered profession");
    server.extra.profession = server_table.state(slot).unwrap();
    let hub = profession_hub("ProfessionClient", true);
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    server_meta(
        &server,
        &hub,
        None,
        0,
        &mut meta,
        &mut defaults,
        &server_table,
    );
    let mut client = Mob::new(2, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
    encoded_receive(&mut client, meta, &client_table);
    render::villager_client::install(&client_table, &Default::default());
    assert_eq!(
        client.extra.profession.id(),
        if custom {
            "test:engineer"
        } else {
            "minecraft:farmer"
        }
    );
    assert!(
        render::layers(&client, 0.0)
            .iter()
            .any(|layer| layer.texture == texture)
    );
}
#[test]
#[ignore = "daily"]
fn engineer_metadata_and_villager_texture() {
    roundtrip(
        "minecraft:villager",
        true,
        "test:entity/villager/profession/engineer",
    );
}
#[test]
#[ignore = "daily"]
fn engineer_metadata_and_zombie_texture() {
    roundtrip(
        "minecraft:zombie_villager",
        true,
        "test:entity/zombie_villager/profession/engineer",
    );
}
#[test]
#[ignore = "daily"]
fn vanilla_farmer_metadata_and_texture() {
    roundtrip(
        "minecraft:villager",
        false,
        "entity/villager/profession/farmer",
    );
}

pub(in crate::game) fn meta_hub(name: &str) -> horizon_client::lan::hub::Hub {
    profession_hub(name, false)
}
/// A hub with no players and empty registries, for `mob_meta`.
fn profession_hub(name: &str, vanilla_professions: bool) -> horizon_client::lan::hub::Hub {
    use horizon_client::lan::{Lan, NetConfig, chunks::ChunkEncoder, data, hub};
    let mut sync = data::SyncData::empty();
    for (registry, id) in [
        ("minecraft:villager_type", "minecraft:plains"),
        ("minecraft:villager_profession", "minecraft:none"),
    ] {
        sync.builtin.registries.insert(
            registry.into(),
            horizon_protocol::registry::Ids::from_names(vec![id.into()]),
        );
    }
    if vanilla_professions {
        sync.builtin.registries.insert(
            "minecraft:villager_profession".into(),
            horizon_protocol::registry::Ids::from_names(
                ResolvedProfessions::vanilla()
                    .entries()
                    .iter()
                    .map(|entry| {
                        if entry.id.contains(':') {
                            entry.id.clone()
                        } else {
                            format!("minecraft:{}", entry.id)
                        }
                    })
                    .collect(),
            ),
        );
    }
    let sync = std::sync::Arc::new(sync);
    let config = NetConfig {
        announce: false,
        listen: false,
        ..NetConfig::loopback(0, name.into(), 2)
    };
    let lan = Lan::start_with(sync.clone(), config).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(std::sync::Arc<str>, bool); 0],
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    let hub_config = hub::HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 10,
        simulation_distance: 10,
        dimension: "minecraft:overworld".into(),
        allow_flight: false,
        seed: 0,
        max_players: 20,
        integrated: false,
        levels: Default::default(),
        level_seeds: Default::default(),
    };
    hub::Hub::new(
        lan,
        sync,
        encoder,
        hub_config,
        std::sync::Arc::new(
            crate::game::test_server_load()
                .loop_context(horizon_world::native_load_context::LoopSide::Server, []),
        ),
    )
}

fn encoded_receive(client: &mut Mob, values: Vec<(u8, i32, Value)>, table: &ResolvedProfessions) {
    use horizon_protocol::{
        buf::Reader,
        packets::play::{Clientbound, SetEntityData},
    };
    let bytes = Clientbound::SetEntityData(SetEntityData { id: 99, values }).encode();
    let mut reader = Reader::new(&bytes);
    let id = reader.varint().unwrap();
    let Clientbound::SetEntityData(packet) =
        Clientbound::decode(id, &bytes[bytes.len() - reader.remaining()..]).unwrap()
    else {
        panic!("entity metadata");
    };
    receive(
        client,
        &packet.values,
        &Builtin::default(),
        packet.id,
        table,
    );
}

#[test]
#[ignore = "daily"]
fn villager_client_packet_slots_and_unresolved_identity() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let types = MobTypes::load(&root).unwrap();
    let server_table = ordered_table(&["test:surveyor", "test:engineer"]);
    let client_table = ordered_table(&["test:engineer", "test:surveyor"]);
    let hub = profession_hub("ProfessionPacket", true);
    for species in ["minecraft:villager", "minecraft:zombie_villager"] {
        let kind = Arc::new(types.types[species].clone());
        for (name, expected_slot, expected_id, layer) in [
            ("minecraft:weaponsmith", 14, "minecraft:weaponsmith", true),
            ("test:engineer", 15, "test:engineer", true),
            ("test:surveyor", 16, "test:surveyor", true),
            ("missing:retired", 0, "minecraft:none", false),
        ] {
            let mut server = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            server.extra.profession = server_table
                .resolve(name)
                .and_then(|slot| server_table.state(slot))
                .unwrap_or_else(|| horizon_client::mobs::villager::ProfessionState::from_id(name));
            let mut client = Mob::new(2, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            let (mut meta, mut defaults) = (Vec::new(), Vec::new());
            server_meta(
                &server,
                &hub,
                None,
                0,
                &mut meta,
                &mut defaults,
                &server_table,
            );
            let field = if species == "minecraft:villager" {
                19
            } else {
                20
            };
            assert!(meta.iter().any(|(index, _, value)| *index == field
                && matches!(value, Value::Ids(ids) if ids == &[0, expected_slot, 1])));
            encoded_receive(&mut client, meta, &client_table);
            assert_eq!(client.extra.profession.id(), expected_id);
            assert_eq!(
                client_table.state_index(&client.extra.profession),
                Some(expected_slot as usize)
            );
            match &client.extra.profession {
                horizon_client::mobs::villager::ProfessionState::Registered { slot, .. } => {
                    assert_eq!(*slot, expected_slot as u16);
                }
                state => assert_eq!(state.index(), Some(expected_slot as u8)),
            }
            assert_eq!(server.extra.profession.id(), name);
            let saved = horizon_client::mobs::villager::professions::save::villager_data(&server);
            assert_eq!(
                saved.get("profession"),
                Some(&horizon_core::nbt::Tag::String(name.into()))
            );
            render::villager_client::install(&client_table, &Default::default());
            assert_eq!(
                render::layers(&client, 0.0)
                    .iter()
                    .any(|layer| layer.texture.contains("/profession/")),
                layer
            );
        }
    }
}
