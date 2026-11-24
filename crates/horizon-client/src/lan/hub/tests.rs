mod bundle_sound_tests;
mod cave_air_tests;
mod chunk_ready_tests;
mod collision_exclusion_tests;
mod effect_meta_tests;
#[path = "far_tests.rs"]
mod far_tests;
mod glide_motion_tests;
mod ground_claim_tests;
mod impulse_grace_tests;
mod live_player_limit_tests;
mod main_hand_tests;
mod menu_sync_tests;
mod ordered_inbox_tests;
mod pending_move_bound_tests;
mod pending_move_scope_tests;
mod pending_move_tests;
mod pong_tests;
mod skin_outer_layer_tests;
mod sound_tests;
mod terrain_send_bench;
mod terrain_view_tests;
use super::*;
use crate::fast_hash::FxHashSet;

/// Queued one-shot fields wait out a change nobody sees, go in one built tag to
/// every viewer, and are not sent again; a column sent whole doesn't lose them.
#[test]
fn client_update_fields_reach_every_viewer_once_and_wait_without_viewers() {
    use crate::block_entity_types::BlockEntityTypes;
    use crate::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use horizon_core::nbt::{Compound, JavaString};
    use std::sync::atomic::AtomicUsize;
    static BUILT: AtomicUsize = AtomicUsize::new(0);
    let (mut hub, first) = hub_with_a_guest();
    let builtin = horizon_protocol::registry::Builtin::parse(
        r#"{"minecraft:block_entity_type":{"entries":{"test:machine":{"protocol_id":49}}}}"#,
    )
    .unwrap();
    let types = BlockEntityTypes::new(
        [("test:machine".into(), "test:machine".into())].into(),
        builtin
            .get("minecraft:block_entity_type")
            .unwrap()
            .names
            .clone(),
    );
    let states = ["minecraft:air", "test:machine"].map(|n| (Arc::<str>::from(n), false, None));
    hub.encoder = ChunkEncoder::with_types(states, &builtin, &[], &[], Some(&types));
    hub.encoder.mod_update_tag = Some(|_, entity| {
        BUILT.fetch_add(1, Ordering::Relaxed);
        Some(entity.nbt.clone())
    });
    let pos = (1, 64, 3);
    let progress = (JavaString::from("progress"), Tag::Int(7));
    let effect = (JavaString::from("effect"), Tag::Int(23));
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, 1);
    world.chunks.insert((0, 0), Arc::new(chunk));
    let machine = ModBlockEntity::new(
        "test:machine".into(),
        ModBlockEntityFields {
            nbt: Compound::from([progress.clone()]),
            ..Default::default()
        },
    );
    let revision = machine.revision();
    machine.set_client_update_fields(Compound::from([effect.clone()]));
    world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(machine)));
    world.block_log = Some(vec![pos]);
    // Nobody has the chunk yet: no tag is built, and the fields wait.
    hub.broadcast_block_changes(&mut world);
    assert_eq!(BUILT.load(Ordering::Relaxed), 0);
    assert!(first.try_recv().is_err());
    hub.guests.update(0, |g| _ = g.sent_chunks.insert((0, 0)));
    let (tx, second) = std::sync::mpsc::channel();
    let mut other = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Ada".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [0.5, 64.0, 0.5],
        8,
    );
    other.placed = true;
    other.dimension = "minecraft:overworld".into();
    other.sent_chunks.insert((0, 0));
    hub.guests.push(other);
    let tags = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        let mut tags = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::BlockEntityData(packet)) = C::decode(id, r.rest()) {
                assert_eq!(packet.pos, [pos.0, pos.1, pos.2]);
                tags.push(packet.tag);
            }
        }
        tags
    };
    // The column goes whole in the same broadcast: the change is kept for the fields.
    world.bulk_log.insert((0, 0));
    world.block_log.as_mut().unwrap().extend([pos, pos]);
    hub.broadcast_block_changes(&mut world);
    let once = Tag::Compound(Compound::from([progress.clone(), effect]));
    for viewer in [&first, &second] {
        assert_eq!(tags(viewer), std::slice::from_ref(&once));
    }
    let built = BUILT.load(Ordering::Relaxed);
    world.block_log.as_mut().unwrap().push(pos);
    hub.broadcast_block_changes(&mut world);
    let plain = Tag::Compound(Compound::from([progress.clone()]));
    for viewer in [&first, &second] {
        assert_eq!(tags(viewer), std::slice::from_ref(&plain));
    }
    assert_eq!(BUILT.load(Ordering::Relaxed), built + 1);
    let BlockEntity::Mod(machine) = &world.block_entities[&pos] else {
        unreachable!();
    };
    assert_eq!(machine.revision(), revision);
    assert_eq!(machine.nbt, Compound::from([progress]));
}

/// A glide `updateFallFlying` ends reaches the player's client with the next tick's data, its flag and its pose
/// together, as vanilla's ServerEntity.sendChanges runs before ServerPlayer.doTick: the landing glide lasts one more
/// tick on the ground, and a jump then keeps its speed (the elytra hop).
#[test]
fn a_glide_ended_this_tick_reaches_its_client_a_tick_later() {
    use crate::pose::Pose;
    fn data(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(u8, Value)> {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let C::SetEntityData(p) = C::decode(id, r.rest()).unwrap() {
                sent.extend(
                    p.values
                        .into_iter()
                        .filter(|(i, _, _)| *i == 0 || *i == 6)
                        .map(|(i, _, v)| (i, v)),
                );
            }
        }
        sent
    }
    let (mut hub, rx) = hub_with_a_guest();
    hub.guests.update(0, |g| g.fall_flying = true);
    hub.sync_player_state(0);
    assert_eq!(
        data(&rx),
        [
            (0, Value::Byte(-128)),
            (6, Value::Id(Pose::FallFlying.id()))
        ]
    );
    // The landing tick: the guest's glide tick ends the glide after the data was looked at.
    hub.guests.update(0, |g| g.end_glide());
    hub.sync_player_state(0);
    assert_eq!(data(&rx), [], "the end waits for the next tick");
    hub.sync_player_state(0);
    assert_eq!(
        data(&rx),
        [(0, Value::Byte(0)), (6, Value::Id(Pose::Standing.id()))]
    );
}

#[test]
fn self_pose_wire_lifecycle_preserves_existing_publishers() {
    use crate::pose::Pose;
    fn drain(
        rx: &std::sync::mpsc::Receiver<crate::lan::Out>,
    ) -> (Vec<Vec<(u8, i32, Value)>>, Vec<i32>) {
        let mut frames = Vec::new();
        let mut keeps = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            match C::decode(id, r.rest()).unwrap() {
                C::SetEntityData(p) if p.values.iter().any(|(i, _, _)| *i == 0 || *i == 6) => {
                    assert_eq!(p.id, 1, "actual self recipient");
                    frames.push(p.values);
                }
                C::Respawn(p) => keeps.push(p.keep as i32),
                _ => {}
            }
        }
        (frames, keeps)
    }
    let pose = |p: Pose| vec![(6, serializer::POSE, Value::Id(p.id()))];
    let flags = |f| vec![(0, serializer::BYTE, Value::Byte(f))];
    let (mut hub, rx) = hub_with_a_guest();
    hub.sync_player_state(0);
    assert!(drain(&rx).0.is_empty(), "default Standing is not dirty");
    hub.guests.update(0, |g| g.sneaking = true);
    hub.sync_player_state(0);
    assert_eq!(drain(&rx).0, [flags(2), pose(Pose::Crouching)]);
    hub.sync_player_state(0);
    assert!(drain(&rx).0.is_empty());
    // The existing refused-glide force-flags marker never forces Pose.
    hub.guests.update(0, |g| g.sent_shared ^= -128);
    hub.sync_player_state(0);
    assert_eq!(drain(&rx).0, [flags(2)]);
    hub.guests.update(0, |g| g.sneaking = false);
    hub.sync_player_state(0);
    assert_eq!(drain(&rx).0, [flags(0), pose(Pose::Standing)]);
    hub.sync_player_state(0);
    assert!(drain(&rx).0.is_empty());
    hub.guests.update(0, |g| g.bed = Some([1, 64, 2]));
    hub.guests.update(0, |g| g.sleeping = true);
    hub.sync_player_state(0);
    assert_eq!(
        drain(&rx).0,
        [vec![
            (6, serializer::POSE, Value::Id(2)),
            (
                14,
                serializer::OPTIONAL_BLOCK_POS,
                Value::OptPos(Some([1, 64, 2]))
            ),
        ]]
    );
    assert_eq!(hub.guests[0].sent_pose, Pose::Sleeping);
    hub.wake(0, false);
    assert_eq!(
        drain(&rx).0,
        [vec![
            (6, serializer::POSE, Value::Id(0)),
            (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
        ]]
    );
    hub.sync_player_state(0);
    assert!(drain(&rx).0.is_empty(), "wake already sent Standing");
    // The non-wake bed-delta publisher also owns its exact existing frame.
    hub.guests.update(0, |g| g.bed = Some([1, 64, 2]));
    hub.guests.update(0, |g| g.sleeping = true);
    hub.sync_player_state(0);
    drain(&rx);
    hub.guests.update(0, |g| g.bed = None);
    hub.guests.update(0, |g| g.sleeping = false);
    hub.sync_player_state(0);
    assert_eq!(
        drain(&rx).0,
        [vec![
            (6, serializer::POSE, Value::Id(0)),
            (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
        ]]
    );
    // A glide's pose reaches the player too, as vanilla's server sends it.
    hub.guests.update(0, |g| g.fall_flying = true);
    hub.sync_player_state(0);
    assert_eq!(drain(&rx).0, [flags(-128), pose(Pose::FallFlying)]);
    hub.guests.update(0, |g| g.fall_flying = false);
    hub.guests.update(0, |g| g.sneaking = true);
    hub.sync_player_state(0);
    drain(&rx);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
    assert_eq!(drain(&rx).1, [3]);
    assert_eq!(hub.guests[0].sent_pose, Pose::Crouching);
    hub.sync_player_state(0);
    assert!(drain(&rx).0.is_empty(), "KEEP_ALL_DATA retains crouch");
    hub.guests.update(0, |g| g.sneaking = false);
    hub.sync_player_state(0);
    assert_eq!(drain(&rx).0, [flags(0), pose(Pose::Standing)]);
    hub.guests.update(0, |g| g.sneaking = true);
    hub.sync_player_state(0);
    drain(&rx);
    hub.respawn(0, [0.5, 64.0, 0.5], Some([0.0; 2]), &ctx);
    assert_eq!(drain(&rx).1, [0]);
    assert_eq!(hub.guests[0].sent_pose, Pose::Standing);
    hub.sync_player_state(0);
    assert_eq!(
        drain(&rx).0,
        [pose(Pose::Crouching)],
        "KEEP_NONE reset makes the actual crouch dirty again"
    );
    hub.sync_player_state(0);
    assert!(drain(&rx).0.is_empty());
}

/// A hub with one placed guest, the speed effect registered, and what the
/// guest is sent.
fn hub_with_a_guest() -> (Hub, std::sync::mpsc::Receiver<crate::lan::Out>) {
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:mob_effect".to_string(),
        horizon_protocol::registry::Ids::from_names(vec!["minecraft:speed".to_string()]),
    );
    hub_with_sync(sync)
}

#[test]
fn modprofile_guest_tree_survives_refresh_with_permissions() {
    for level in 0..=4 {
        let mut sync = SyncData::empty();
        sync.commands = Some(crate::lan::commands::CommandTree {
            root: serde_json::json!({"type":"root","children":{}}),
        });
        let (mut hub, rx) = hub_with_sync(sync);
        hub.guests.update(0, |guest| guest.permission = level);
        for refresh in [false, true] {
            if refresh {
                hub.set_mod_commands(
                    7,
                    serde_json::json!({"children":{"plugin":{"type":"literal","executable":true}}}),
                );
            } else {
                hub.send_commands(0);
            }
            let crate::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
                panic!("packet");
            };
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let play::Clientbound::Commands(packet) =
                play::Clientbound::decode(id, &body[reader.position()..]).unwrap()
            else {
                panic!("commands");
            };
            let root = &packet.nodes[packet.root as usize];
            let profile = root
                .children
                .iter()
                .map(|&i| &packet.nodes[i as usize])
                .find(|n| n.name == "modprofile");
            if level < 3 {
                assert!(profile.is_none());
            } else {
                let profile = profile.expect("admins receive modprofile on join and refresh");
                assert!(profile.restricted && !profile.executable);
                let actions: Vec<_> = profile
                    .children
                    .iter()
                    .map(|&i| {
                        let node = &packet.nodes[i as usize];
                        assert!(node.executable);
                        node.name.as_str()
                    })
                    .collect();
                assert_eq!(actions, ["report", "start", "stop"]);
            }
            if refresh {
                assert!(
                    root.children
                        .iter()
                        .any(|&i| packet.nodes[i as usize].name == "plugin")
                );
            }
        }
    }
}

#[test]
fn registered_bool_rule_reaches_guest_commands() {
    let mut sync = SyncData::empty();
    sync.commands = Some(crate::lan::commands::CommandTree {
        root: serde_json::json!({"type":"root","children":{"gamerule":{"type":"literal","permissions":{"permission":{"level":"gamemasters"}}}}}),
    });
    sync.builtin.registries.insert(
        "minecraft:command_argument_type".into(),
        horizon_protocol::registry::Ids::from_names(
            play::ARGUMENT_TYPES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        ),
    );
    let (mut hub, rx) = hub_with_sync(sync);
    let rule = crate::gamerules::Rule {
        name: "example:enabled".into(),
        category: "example:machines".into(),
        default: crate::gamerules::Value::Bool(true),
        min: 0,
        max: 0,
    };
    hub.rule_registry = Arc::new(
        crate::gamerules::RuleRegistry::with_mod_rules(&["example:machines".into()], &[rule])
            .unwrap(),
    );
    hub.guests.update(0, |guest| guest.permission = 2);
    hub.send_commands(0);
    let crate::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
        panic!("packet")
    };
    let mut reader = horizon_protocol::buf::Reader::new(&body);
    let id = reader.varint().unwrap();
    let play::Clientbound::Commands(packet) =
        play::Clientbound::decode(id, &body[reader.position()..]).unwrap()
    else {
        panic!("commands")
    };
    let client = crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
    let regs = crate::command::Registries {
        permission: 2,
        ..Default::default()
    };
    let line = "gamerule example:enabled ";
    let suggestions = client
        .client_suggestions(&client.parse(line, &regs), line.len(), &regs)
        .1
        .into_iter()
        .map(|s| s.text)
        .collect::<Vec<_>>();
    assert_eq!(suggestions, ["false", "true"]);
    assert!(
        client
            .parse("gamerule example:enabled false", &regs)
            .executable()
            .is_ok()
    );
}

#[test]
fn guest_rule_tree_comes_from_session_registry() {
    let mut sync = SyncData::empty();
    sync.commands = Some(crate::lan::commands::CommandTree {
        root: serde_json::json!({"type":"root", "children": {
            "gamerule": {"type":"literal", "permissions":{"permission":{"level":"gamemasters"}},
                "children":{"stale_rule":{"type":"literal"}}}
        }}),
    });
    sync.builtin.registries.insert(
        "minecraft:command_argument_type".into(),
        horizon_protocol::registry::Ids::from_names(
            play::ARGUMENT_TYPES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        ),
    );
    let (mut hub, rx) = hub_with_sync(sync);
    hub.rule_registry = Arc::new(crate::gamerules::RuleRegistry::vanilla());
    hub.guests.update(0, |g| g.permission = 2);
    hub.send_commands(0);
    let crate::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
        panic!("expected packet")
    };
    let mut reader = horizon_protocol::buf::Reader::new(&body);
    let id = reader.varint().unwrap();
    let play::Clientbound::Commands(packet) =
        play::Clientbound::decode(id, &body[reader.position()..]).unwrap()
    else {
        panic!("expected commands")
    };
    let root = packet.nodes.iter().find(|n| n.name == "gamerule").unwrap();
    let names: Vec<_> = root
        .children
        .iter()
        .map(|i| packet.nodes[*i as usize].name.clone())
        .collect();
    let mut expected: Vec<_> = hub
        .rule_registry
        .available_rules(false)
        .flat_map(|r| {
            [
                r.name.clone(),
                r.name.strip_prefix("minecraft:").unwrap().to_owned(),
            ]
        })
        .collect();
    expected.sort();
    assert_eq!(names, expected);
    let client = crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
    let regs = crate::command::Registries {
        permission: 2,
        ..Default::default()
    };
    let suggestions = |line: &str| {
        client
            .client_suggestions(&client.parse(line, &regs), line.len(), &regs)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect::<Vec<_>>()
    };
    for (name, report_bounds) in [
        ("random_tick_speed", serde_json::json!({"min": 0})),
        (
            "max_snow_accumulation_height",
            serde_json::json!({"min": 0, "max": 8}),
        ),
    ] {
        let rule = packet.nodes.iter().find(|n| n.name == name).unwrap();
        let value = &packet.nodes[rule.children[0] as usize];
        assert_eq!(
            value.parser.as_ref().unwrap().1,
            match report_bounds.get("max") {
                None => vec![1, 0, 0, 0, 0],
                Some(_) => vec![3, 0, 0, 0, 0, 0, 0, 0, 8],
            }
        );
    }
    assert!(suggestions("gamerule keep_").contains(&"keep_inventory".to_owned()));
    assert_eq!(suggestions("gamerule keep_inventory "), ["false", "true"]);
    assert!(suggestions("gamerule max_snow_accumulation_height ").is_empty());
    assert!(
        client
            .parse("gamerule max_snow_accumulation_height 8", &regs)
            .executable()
            .is_ok()
    );
    assert!(
        client
            .parse("gamerule max_snow_accumulation_height 9", &regs)
            .executable()
            .is_err()
    );
    assert!(!names.iter().any(|n| n.contains("max_minecart_speed")));
    let tree = hub.sync.commands.as_ref().unwrap();
    let enabled = tree.packet_with_features(
        2,
        false,
        &hub.sync.builtin,
        &serde_json::Value::Null,
        &[crate::gamerules::MINECART_IMPROVEMENTS.to_owned()],
        &hub.rule_registry,
    );
    let root = enabled.nodes.iter().find(|n| n.name == "gamerule").unwrap();
    let names: Vec<_> = root
        .children
        .iter()
        .map(|i| enabled.nodes[*i as usize].name.clone())
        .collect();
    let mut expected: Vec<_> = hub
        .rule_registry
        .available_rules(true)
        .flat_map(|r| {
            [
                r.name.clone(),
                r.name.strip_prefix("minecraft:").unwrap().to_owned(),
            ]
        })
        .collect();
    expected.sort();
    assert_eq!(names, expected);
    assert!(
        !hub.commands_packet(0)
            .nodes
            .iter()
            .any(|n| n.name == "gamerule")
    );
    let registry = hub.rule_registry.clone();
    hub.reload_resources(hub.sync.clone(), &mut Default::default());
    assert!(Arc::ptr_eq(&registry, &hub.rule_registry));
}

/// Every packet the guest was sent, decoded.
fn sent(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<play::Clientbound> {
    rx.try_iter()
        .filter_map(|out| match out {
            crate::lan::Out::Packet(body) => {
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().ok()?;
                play::Clientbound::decode(id, &body[reader.position()..]).ok()
            }
            _ => None,
        })
        .collect()
}

/// A second placed guest, Alice (uuid 2), whose receiver is returned.
fn add_alice(hub: &mut Hub) -> std::sync::mpsc::Receiver<crate::lan::Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut alice = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Alice".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [0.5, 64.0, 0.5],
        8,
    );
    alice.placed = true;
    alice.dimension = "minecraft:overworld".into();
    hub.guests.push(alice);
    rx
}

/// A mod's list name is kept by UUID and goes to every client that has the player's Tab entry, and
/// None takes it away again; it is kept for the next Tab list entry too. The decoded packets hold the
/// exact names (the test format hook leaves the name alone without its setting), and a viewer the
/// player is hidden from is told nothing.
#[test]
fn a_list_name_replaces_the_tab_list_name_and_goes_back() {
    let (mut hub, bob) = hub_with_a_guest();
    let alice = add_alice(&mut hub);
    hub.nicknames.insert(1, "Bobby".into());
    let names = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        sent(rx)
            .into_iter()
            .filter_map(|p| match p {
                play::Clientbound::PlayerInfoUpdate(update) if update.actions == 0x20 => Some((
                    update.entries[0].uuid,
                    update.entries[0].display_name.clone(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let red = crate::text::Text::legacy("\u{a7}cBob");
    let red_tag = crate::scoreboard::component_tag(&red.component());
    let bobby_tag =
        crate::scoreboard::component_tag(&crate::text::Text::literal("Bobby").component());
    hub.set_list_name(1, Some(red.clone()));
    assert_eq!(hub.list_names.get(&1), Some(&red));
    for rx in [&bob, &alice] {
        assert_eq!(names(rx), [(1, Some(Some(red_tag.clone())))]);
    }
    hub.set_list_name(1, None);
    assert!(!hub.list_names.contains_key(&1));
    for rx in [&bob, &alice] {
        assert_eq!(names(rx), [(1, Some(Some(bobby_tag.clone())))]);
    }
    // A viewer Bob is hidden from gets nothing; Bob's own client still does.
    hub.hidden.insert(1, 3);
    hub.set_list_name(1, Some(red.clone()));
    assert_eq!(names(&bob), [(1, Some(Some(red_tag)))]);
    assert!(names(&alice).is_empty(), "no entry, no update");
    // A player who is not on keeps the name for their return, and nothing is sent.
    hub.set_list_name(7, Some(red.clone()));
    assert!(sent(&bob).is_empty() && sent(&alice).is_empty());
    assert_eq!(hub.list_names.get(&7), Some(&red));
}

/// [`hub_with_a_guest`] over a sync the test built.
fn hub_with_sync(sync: SyncData) -> (Hub, std::sync::mpsc::Receiver<crate::lan::Out>) {
    let config = crate::lan::NetConfig {
        announce: false,
        listen: false,
        ..crate::lan::NetConfig::loopback(0, "EffectSyncTest".into(), 2)
    };
    let sync = std::sync::Arc::new(sync);
    let lan = Lan::start_with(sync.clone(), config).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(std::sync::Arc<str>, bool); 0],
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    let hub_config = HubConfig {
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
    let mut hub = Hub::new(
        lan,
        sync,
        encoder,
        hub_config,
        crate::test_resources::loop_context(horizon_world::native_load_context::LoopSide::Server),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let mut bob = Guest::new(
        crate::lan::Joined {
            conn: 1,
            uuid: 1,
            name: "Bob".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        1,
        [0.5, 64.0, 0.5],
        8,
    );
    bob.placed = true;
    bob.dimension = "minecraft:overworld".into();
    hub.guests.push(bob);
    (hub, rx)
}

#[test]
fn an_empty_advancement_tree_still_sends_a_first_reset_after_staging() {
    let (mut hub, rx) = hub_with_a_guest();
    hub.mods_loaded = true;
    hub.sync_player_state(0);
    let task = hub.native_advancement_grants.pop().unwrap();
    assert!(task.grants.is_empty());
    let before: Vec<_> = rx.try_iter().collect();
    assert!(
        !before.iter().any(|out| match out {
            crate::lan::Out::Packet(body) =>
                horizon_protocol::buf::Reader::new(body).varint().ok()
                    == Some(ids::UPDATE_ADVANCEMENTS),
            _ => false,
        })
    );
    hub.finish_native_first_tick(&task, &[], &mut AdvancementOwners::default());
    let found = rx.try_iter().any(|out| match out {
        crate::lan::Out::Packet(body) => {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            reader.varint().ok() == Some(ids::UPDATE_ADVANCEMENTS)
                && reader.bool().ok() == Some(true)
        }
        _ => false,
    });
    assert!(found, "the first packet resets the client's empty tree");
}

/// `store_mod_commands` keeps a new mod tree without sending it (the
/// game sends it after the mods filtered it: `native_refresh_commands`),
/// and `set_mod_commands` still sends a changed tree, and only a changed one.
#[test]
fn stored_mod_commands_are_not_sent_until_asked() {
    let (mut hub, rx) = hub_with_a_guest();
    let trees = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        let mut sent = 0;
        while let Ok(out) = rx.try_recv() {
            if let crate::lan::Out::Packet(body) = out {
                let mut r = horizon_protocol::buf::Reader::new(&body);
                let pid = r.varint().unwrap();
                if let Ok(C::Commands(_)) = C::decode(pid, r.rest()) {
                    sent += 1;
                }
            }
        }
        sent
    };
    let first = serde_json::json!({"hello": {}});
    assert!(hub.store_mod_commands(0, first.clone()));
    assert!(!hub.store_mod_commands(0, first.clone()));
    assert_eq!(trees(&rx), 0);
    assert_eq!(hub.mod_commands_key(), Some(0));
    hub.set_mod_commands(0, first);
    assert_eq!(trees(&rx), 0, "an unchanged tree is not sent again");
    hub.set_mod_commands(0, serde_json::json!({"hello": {}, "bye": {}}));
    assert_eq!(trees(&rx), 1);
    hub.resend_commands();
    assert_eq!(trees(&rx), 1, "every placed player gets their tree again");
}

/// `onGameRuleChanged`: a changed `reduced_debug_info` reaches each player
/// as their own entity event 22 / 23, and an unchanged one sends nothing.
#[test]
fn reduced_debug_info_changes_reach_players_as_entity_events() {
    let (mut hub, rx) = hub_with_a_guest();
    let id = hub.guests[0].entity_id;
    hub.set_reduced_debug_info(false);
    assert!(sent_entity_events(&rx).is_empty());
    hub.set_reduced_debug_info(true);
    hub.set_reduced_debug_info(true);
    assert_eq!(sent_entity_events(&rx), vec![(id, 22)]);
    hub.set_reduced_debug_info(false);
    assert_eq!(sent_entity_events(&rx), vec![(id, 23)]);
}

/// `ServerLevel.broadcastEntityEvent`: an entity event reaches the guest
/// tracking the entity and the guest whose player it is, but not a guest
/// that does not track it, even one well within 128 blocks.
#[test]
fn entity_events_reach_tracking_players_and_self() {
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:entity_type".into(),
        horizon_protocol::registry::Ids::from_names(vec!["minecraft:mooshroom".into()]),
    );
    let (mut hub, rx) = hub_with_sync(sync);
    let own = hub.guests[0].entity_id;
    hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
    let near = EntityState {
        id: 37,
        uuid: 37,
        kind: "minecraft:mooshroom".into(),
        position: [0.5, 64.0, 0.5],
        ..EntityState::default()
    };
    // In a chunk the guest was not sent, so it is not tracked.
    let untracked = EntityState {
        id: 38,
        uuid: 38,
        position: [100.5, 64.0, 0.5],
        ..near.clone()
    };
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let border = crate::worldborder::WorldBorder::default();
    let mut next_entity = 39;
    let entities = [near, untracked];
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &entities,
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    hub.track_entities(&mut ctx);
    assert!(hub.guests[0].tracker.is_tracking(37));
    assert!(!hub.guests[0].tracker.is_tracking(38));
    while rx.try_recv().is_ok() {}
    hub.entity_event(37, 2);
    hub.entity_event(38, 2);
    hub.entity_event(own, 35);
    assert_eq!(sent_entity_events(&rx), vec![(37, 2), (own, 35)]);
}

/// The entity events (id, event) the guest was sent, in order.
fn sent_entity_events(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(i32, i8)> {
    let mut sent = Vec::new();
    while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let pid = r.varint().unwrap();
        if let Ok(C::EntityEvent(p)) = C::decode(pid, r.rest()) {
            sent.push((p.id, p.event));
        }
    }
    sent
}

/// The blocks the guest was sent as `ClientboundBlockUpdatePacket`s, in order, with the
/// state each one carries.
fn sent_block_updates(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<([i32; 3], i32)> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::BlockUpdate(p)) = C::decode(id, r.rest()) {
            sent.push((p.pos, p.state));
        }
    }
    sent
}

/// A world whose blocks at y = 63 are `state` under the four chunks around the origin.
fn floor_at(state: u32) -> World {
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    for cz in -1..=0 {
        for cx in -1..=0 {
            let mut chunk = crate::chunk::Chunk::empty(cx, cz);
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, 63, z, state);
                }
            }
            chunk.lit = true;
            world.chunks.insert((cx, cz), std::sync::Arc::new(chunk));
        }
    }
    world
}

/// One tick of the hub that handles a single packet from the guest.
fn tick_packet(hub: &mut Hub, world: &mut World, packet: S) {
    hub.guests
        .inbox_mut(0)
        .unwrap()
        .push_back(horizon_client::lan::guest::InboxItem::uncredited(packet));
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let _ = hub.drain_packets(&mut ctx);
}

/// One tick of the hub that handles a single position move from the guest.
fn tick_guest_move(hub: &mut Hub, world: &mut World, to: [f64; 3], flags: u8) {
    // One position move per client tick (receivedPositionThisTick).
    {
        let value = false;
        hub.guests.update(0, |g| g.position_this_tick = value);
    }
    tick_packet(
        hub,
        world,
        S::MovePlayerPos(play::MovePlayerPos {
            x: to[0],
            y: to[1],
            z: to[2],
            flags,
        }),
    );
}

/// `ClientboundRespawnPacket.KEEP_ALL_DATA`: a level change keeps the client's copy of the
/// player's shared flags, so one that clears afterwards still reaches it. (A creative player
/// who leaves lava through a dimension change kept the fire on its screen.)
#[test]
fn a_shared_flag_cleared_after_a_level_change_reaches_the_player() {
    let (mut hub, rx) = hub_with_a_guest();
    let shared_flags_sent = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::SetEntityData(packet)) = C::decode(id, r.rest()) {
                sent.extend(packet.values.iter().filter_map(|(index, _, value)| {
                    match (index, value) {
                        (0, Value::Byte(flags)) => Some(*flags),
                        _ => None,
                    }
                }));
            }
        }
        sent
    };
    hub.guests.update(0, |g| g.sneaking = true);
    hub.sync_player_state(0);
    assert_eq!(shared_flags_sent(&rx), [0x02]);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
    let _ = shared_flags_sent(&rx);
    hub.guests.update(0, |g| g.sneaking = false);
    hub.sync_player_state(0);
    assert_eq!(shared_flags_sent(&rx), [0]);
}

/// A mod menu whose part keeps it open (`menu_still_valid`) still closes
/// when its player changes level or respawns: those close it without
/// asking `stillValid` (`ServerPlayer.closeContainer`).
#[test]
fn a_menu_kept_open_by_its_part_closes_on_level_change_and_respawn() {
    let mod_type: &'static crate::containers::ModMenuType =
        Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:kept",
            slots: 27,
            data_slots: 0,
            results: vec![],
            links: None,
            crafting: None,
        }));
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let k = (0, 64, 40);
    world
        .block_entities
        .insert(k, crate::containers::BlockEntity::Chest(vec![None; 27]));
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let open = |hub: &mut Hub| {
        hub.guests.update(0, |g| {
            g.menu = Some(super::super::menu::Menu {
                window: 1,
                kind: 2,
                pos: Some(k),
                slots: super::super::menu::Slots::Block(k),
                size: 27,
                state: 0,
                sent: Vec::new(),
                drag: None,
                sent_data: Vec::new(),
                hidden: Vec::new(),
                mod_type: Some(mod_type),
                registry_id: "test:kept".into(),
                craft: None,
                links: Vec::new(),
                asks_still_valid: true,
            });
        });
    };
    open(&mut hub);
    hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
    assert!(hub.guests[0].menu.is_none());
    open(&mut hub);
    hub.respawn(0, [0.5, 64.0, 0.5], None, &ctx);
    assert!(hub.guests[0].menu.is_none());
}

/// `ServerGamePacketListenerImpl.forceSendPlayerSupportBlocks`: a client that says it stands
/// while the last move the server resolved came down onto nothing is sent the blocks under
/// its box again (its four corners, each once), so a client that missed a block change cannot
/// stay standing on air. At most once every 200 ticks of the connection.
#[test]
fn a_guest_standing_on_nothing_is_sent_the_blocks_below_it() {
    // Block state 1 is stone in the vanilla registry order.
    const STONE: u32 = 1;
    let (mut hub, rx) = hub_with_a_guest();
    {
        let value = 0;
        hub.guests.update(0, |g| g.load_timeout = value);
    }
    // Across a block corner: the box's four corners are four different blocks.
    {
        let value = [0.0, 64.0, 0.0];
        hub.guests.update(0, |g| g.position = value);
    }
    let mut world = floor_at(STONE);
    // The connection's 200th tick is the first that may send: vanilla starts the count at 0.
    hub.ticks = 199;
    tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
    assert!(
        sent_block_updates(&rx).is_empty(),
        "the first 200 ticks of a connection send nothing"
    );
    hub.ticks = 200;
    tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
    assert_eq!(
        sent_block_updates(&rx),
        vec![
            ([-1, 63, -1], STONE as i32),
            ([0, 63, -1], STONE as i32),
            ([-1, 63, 0], STONE as i32),
            ([0, 63, 0], STONE as i32),
        ],
        "the four blocks under the box, each once, with the server's states"
    );
    // A tick later: the next 200 have to pass first.
    hub.ticks = 201;
    tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
    assert!(
        sent_block_updates(&rx).is_empty(),
        "no more than once every 200 ticks"
    );
    // The last move the server resolved came down onto something: it stands on its own.
    {
        let value = true;
        hub.guests.update(0, |g| g.rests_on_ground = value);
    }
    hub.ticks = 500;
    tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
    assert!(
        sent_block_updates(&rx).is_empty(),
        "a player the server saw land is not resent the blocks"
    );
    // A client that does not claim the ground is not helped either.
    {
        let value = false;
        hub.guests.update(0, |g| g.rests_on_ground = value);
    }
    hub.ticks = 800;
    tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 0);
    assert!(
        sent_block_updates(&rx).is_empty(),
        "only a claim of standing on the ground asks for the blocks below"
    );
    // A box over one block gets that block once (the corners are the same cell).
    hub.ticks = 1100;
    {
        let value = [0.5, 64.0, 0.5];
        hub.guests.update(0, |g| g.position = value);
    }
    tick_guest_move(&mut hub, &mut world, [0.5, 64.0, 0.5], 1);
    assert_eq!(
        sent_block_updates(&rx),
        vec![([0, 63, 0], STONE as i32)],
        "one block under a box that sits across a block"
    );
}

#[test]
fn held_network_guests_keep_level_updates_without_including_fake_actors() {
    let (mut hub, rx) = hub_with_a_guest();
    {
        let value = false;
        hub.guests.update(0, |g| g.placed = value);
    }
    {
        let value = true;
        hub.guests.update(0, |g| g.held = value);
    }
    let (mut other, fake_rx) = hub_with_a_guest();
    let mut fake = other.guests.remove(0);
    fake.kind = super::super::guest::GuestKind::Fake;
    fake.conn = 2;
    fake.uuid = 2;
    fake.entity_id = 2;
    fake.position = [64.5, 64.0, 0.5];
    fake.held = true;
    hub.guests.push(fake);
    assert!(!hub.here(&hub.guests[0]));
    assert!(hub.watches(&hub.guests[0]));
    assert!(!hub.watches(&hub.guests[1]));
    assert_eq!(hub.loading_chunks(|_| true).collect::<Vec<_>>(), [(0, 0)]);
    let packet = C::GameEvent(play::GameEvent {
        event: game_event::IMMEDIATE_RESPAWN,
        param: 1.0,
    });
    hub.broadcast_level(&packet);
    hub.broadcast_watching(&packet);
    hub.broadcast(&packet);
    assert_eq!(
        rx.try_iter().count(),
        2,
        "held guests watch but are not placed online players"
    );
    assert_eq!(fake_rx.try_iter().count(), 0);
}

#[test]
fn fake_is_not_advertised_in_player_latency_updates() {
    let (mut hub, rx) = hub_with_a_guest();
    let (tx, _fake_rx) = std::sync::mpsc::channel();
    let mut fake = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Fake".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: String::new(),
            sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
            latency: std::sync::Arc::default(),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [32.0, 64.0, 0.0],
        8,
    );
    fake.kind = super::super::guest::GuestKind::Fake;
    fake.placed = true;
    hub.guests.push(fake);
    assert_eq!(hub.online_count(), 1);
    let reply = hub.list_reply(false);
    let Some(Tag::List(args)) = reply.get("with") else {
        panic!("expected list reply arguments");
    };
    assert_eq!(args[0], Tag::Int(1));
    assert_eq!(args[2], display_name(&hub.loop_context, 1, "Bob"));
    hub.guests.update(0, |g| g.sent_chunks.insert((2, 0)));
    let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    hub.track_entities(&mut ctx);
    assert!(!hub.guests[0].tracker.is_tracking(2));
    assert!(hub.guests[1].tracker.ids().is_empty());
    {
        let value = false;
        hub.guests.update(0, |g| g.placed = value);
    }
    hub.place_new_player(2, &mut ctx);
    assert!(rx.try_recv().is_err());
    hub.place_new_player(1, &mut ctx);
    let mut saw_list = false;
    while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let Ok(id) = reader.varint() else { continue };
        if let Ok(C::PlayerInfoUpdate(update)) = C::decode(id, reader.rest()) {
            saw_list = true;
            assert!(update.entries.iter().all(|entry| entry.uuid != 2));
        }
    }
    assert!(saw_list);
    hub.ticks = 601;
    hub.end_tick();
    let crate::lan::Out::Packet(body) = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
    else {
        panic!("expected player info update");
    };
    let mut reader = horizon_protocol::buf::Reader::new(&body);
    let id = reader.varint().unwrap();
    let C::PlayerInfoUpdate(update) = C::decode(id, reader.rest()).unwrap() else {
        panic!("expected player info update");
    };
    assert_eq!(update.entries.len(), 1);
    assert_eq!(update.entries[0].uuid, 1);
}

/// Exercise the compiled conversion path, including full component equality,
/// selection, removals and registry reload invalidation.
#[test]
fn equipment_is_converted_again_only_when_a_stack_changes() {
    use horizon_items::equipment::{BODY, SADDLE};
    let (mut hub, _rx) = hub_with_a_guest();
    hub.guests.update(0, |g| {
        g.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
        g.selected = 0;
        g.offhand = Some(ItemStack::new("minecraft:shield", 1));
        g.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
        g.equipment[BODY] = Some(ItemStack::new("minecraft:white_carpet", 1));
        g.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
    });
    let first = hub.guest_equipment(0);
    assert_eq!(
        first.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
        [0, 1, 5, 6, 7]
    );
    for _ in 0..3 {
        assert_eq!(hub.guest_equipment(0), first);
        for slot in 0..equipment_cache::SLOTS {
            assert!(
                !hub.guests[0]
                    .equipment_cache
                    .is_stale(slot, equipment_source(&hub.guests[0], slot))
            );
        }
    }
    hub.guests
        .update(0, |g| g.inventory[0].as_mut().unwrap().count = 3);
    let changed = hub.guest_equipment(0);
    assert_ne!(changed, first);
    let mut component_stack = ItemStack::new("minecraft:stone", 3);
    let mut data = crate::item_data::StackData::default();
    data.repair_cost = 7;
    component_stack.extra = Some(Box::new(data));
    hub.guests
        .update(0, |g| g.inventory[0] = Some(component_stack.clone()));
    assert!(
        hub.guests[0]
            .equipment_cache
            .is_stale(0, Some(&component_stack))
    );
    let expected = hub.stack(&Some(component_stack));
    assert_eq!(hub.guest_equipment(0)[0], (0, expected));
    hub.guests.update(0, |g| {
        g.selected = 1;
        g.offhand = None;
        g.equipment[BODY] = None;
        g.equipment[SADDLE] = None;
    });
    let cleared = hub.guest_equipment(0);
    assert_eq!(
        cleared.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
        [5]
    );
    assert!(!hub.guests[0].equipment_cache.is_stale(1, None));
    hub.reload_resources(hub.sync.clone(), &mut Default::default());
    assert!(
        hub.guests[0]
            .equipment_cache
            .is_stale(5, hub.guests[0].armor[0].as_ref())
    );
    assert_eq!(hub.guest_equipment(0), cleared);
}

/// `LivingEntity.collectEquipmentChanges` walks every `EquipmentSlot`: a
/// player's body (wire slot 6) and saddle (7) reach the players who see
/// them, with the hands and armor.
#[test]
fn a_players_body_and_saddle_reach_the_players_who_see_them() {
    use horizon_items::equipment::{BODY, SADDLE};
    let (mut hub, rx) = hub_with_a_guest();
    let (tx, _ann_rx) = std::sync::mpsc::channel();
    let mut ann = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Ann".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
            latency: std::sync::Arc::default(),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [4.5, 64.0, 0.5],
        8,
    );
    ann.placed = true;
    ann.dimension = "minecraft:overworld".into();
    ann.offhand = Some(ItemStack::new("minecraft:shield", 1));
    ann.equipment[BODY] = Some(ItemStack::new("minecraft:white_carpet", 1));
    ann.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
    hub.guests.push(ann);
    hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
    // The test hub syncs no entity type registry.
    hub.entity_types.insert("minecraft:player".into(), 1);
    let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    hub.track_entities(&mut ctx);
    assert!(hub.guests[0].tracker.is_tracking(2));
    let mut slots = None;
    while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let Ok(id) = reader.varint() else { continue };
        if let Ok(C::SetEquipment(p)) = C::decode(id, reader.rest())
            && p.id == 2
        {
            slots = Some(p.slots.iter().map(|(slot, _)| *slot).collect::<Vec<_>>());
        }
    }
    assert_eq!(slots, Some(vec![1, 6, 7]));
}

/// A mod's named effect particle goes out as id -1, which vanilla's
/// `ParticleTypes.STREAM_CODEC` cannot read: the entity data sent to a
/// Horizon client has it, a vanilla client's has the rest of the list. As
/// the bytes on the wire, when a client starts tracking the entity, when
/// the list changes, and when the entity is paired as it joins.
#[test]
fn a_named_effect_particle_is_sent_only_to_horizon_clients() {
    // id, name length, name.
    const NAMED: [u8; 19] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 13, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b':', b's',
        b'w', b'i', b'r', b'l',
    ];
    // ClientboundSetEntityDataPacket: the packet's id, then the entity's.
    let data = |entity: u8, entries: &[&[u8]]| {
        let mut body = vec![101, entity];
        body.extend(entries.concat());
        body.push(0xFF);
        body
    };
    // Entry 10, PARTICLES (17), then the count.
    let particles = |list: &[&[u8]]| {
        let mut entry = vec![10, 17, list.len() as u8];
        entry.extend(list.concat());
        entry
    };
    let ambient: &[u8] = &[11, 8, 1];
    let plain: &[u8] = &[0];

    let (mut hub, bob_rx) = hub_with_a_guest();
    let (tx, ann_rx) = std::sync::mpsc::channel();
    let mut ann = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Ann".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "horizon".into(),
            sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
            latency: std::sync::Arc::default(),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [4.5, 64.0, 0.5],
        8,
    );
    ann.placed = true;
    ann.dimension = "minecraft:overworld".into();
    hub.guests.push(ann);
    hub.guests.update_all(|g| {
        g.sent_chunks.insert((0, 0));
    });
    // The test hub syncs no entity type registry.
    hub.entity_types.insert("minecraft:zombie".into(), 1);

    // The entity data a client was sent since the last look.
    let sent = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        let mut bodies = Vec::new();
        while let Ok(out) = rx.try_recv() {
            match out {
                crate::lan::Out::Packet(body) => bodies.push(body),
                crate::lan::Out::Packets(list) => bodies.extend(list),
                crate::lan::Out::Disconnect(_) => panic!("disconnected"),
            }
        }
        bodies.retain(|body| body.first() == Some(&101));
        bodies
    };
    let zombie = |id: i32, meta: Vec<(u8, i32, Value)>| EntityState {
        id,
        uuid: id as u128,
        kind: "minecraft:zombie".into(),
        position: [2.5, 64.0, 0.5],
        meta,
        ..EntityState::default()
    };
    let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 200;
    let border = crate::worldborder::WorldBorder::default();
    let mut track = |hub: &mut Hub, state: EntityState| {
        let entities = [state];
        hub.track_entities(&mut TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &entities,
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            spawn_dimension: "minecraft:overworld",
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        });
    };

    // What a listener's replacement becomes.
    let named = crate::particles::ParticleOptions::named("example:swirl");
    let only_named = hub.particle_meta(std::slice::from_ref(&named), false);
    let Value::Particles(list) = &only_named[0].2 else {
        panic!("expected particles");
    };
    let mixed = vec![
        (
            10,
            serializer::PARTICLES,
            Value::Particles(vec![list[0].clone(), Particle { id: 0, data: None }]),
        ),
        (11, serializer::BOOLEAN, Value::Bool(true)),
    ];

    // Starting to track it.
    track(&mut hub, zombie(100, mixed.clone()));
    assert_eq!(
        sent(&ann_rx),
        [data(100, &[&particles(&[&NAMED, plain]), ambient])]
    );
    assert_eq!(sent(&bob_rx), [data(100, &[&particles(&[plain]), ambient])]);

    // The change leaves only the named particle: vanilla's list empties.
    let mut changed = only_named.clone();
    changed.push(mixed[1].clone());
    track(&mut hub, zombie(100, changed));
    assert_eq!(sent(&ann_rx), [data(100, &[&particles(&[&NAMED])])]);
    assert_eq!(sent(&bob_rx), [data(100, &[&particles(&[])])]);

    // No named particle: both clients get the same bytes.
    let ordinary = vec![(
        10,
        serializer::PARTICLES,
        Value::Particles(vec![Particle { id: 0, data: None }]),
    )];
    track(&mut hub, zombie(100, ordinary));
    let expected = [data(100, &[&particles(&[plain])])];
    assert_eq!(sent(&ann_rx), expected);
    assert_eq!(sent(&bob_rx), expected);

    // Paired as it joins, after the tracking pass.
    hub.note_joined(&[zombie(101, only_named)]);
    assert_eq!(sent(&ann_rx), [data(101, &[&particles(&[&NAMED])])]);
    assert_eq!(sent(&bob_rx), [data(101, &[&particles(&[])])]);
}

/// `handleChatSessionUpdate` shares `chatMessageChain` with chat: a new
/// session is announced after a message that came before it. With mods
/// loaded that message waits for the game (`ChatSubmitted`), so the
/// announcement waits behind it; with nothing held it goes out at once.
#[test]
fn a_new_chat_session_waits_behind_chat_the_mods_hold() {
    let (mut hub, rx) = hub_with_a_guest();
    let update = || play::PlayerInfoUpdate {
        actions: 2,
        entries: vec![play::PlayerInfo {
            uuid: 1,
            ..Default::default()
        }],
    };
    let mut events = Vec::new();
    hub.announce_chat_session(update(), &mut events);
    assert!(events.is_empty());
    assert_eq!(rx.try_iter().count(), 1, "announced at once");

    let mut events = vec![HostEvent::ChatSubmitted {
        message: Box::new(chat::ChatMessage {
            sender: 1,
            index: 0,
            signature: None,
            content: "hi".into(),
            timestamp: 0,
            salt: 0,
            last_seen: Vec::new(),
            unsigned_content: None,
        }),
        bound: Box::new(play::ChatTypeBound {
            chat_type: 0,
            name: horizon_core::nbt::Tag::String("Bob".into()),
            target: None,
        }),
        name: "Bob".into(),
    }];
    hub.announce_chat_session(update(), &mut events);
    assert_eq!(rx.try_iter().count(), 0, "held behind the chat");
    assert!(matches!(
        events.as_slice(),
        [HostEvent::ChatSubmitted { .. }, HostEvent::ChatSessionStarted(u)] if **u == update()
    ));
}
/// The tabs the guest was sent in `ClientboundSelectAdvancementsTabPacket`s, in order.
fn sent_tabs(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<Option<String>> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::SelectAdvancementsTab(p)) = C::decode(id, r.rest()) {
            sent.push(p.tab);
        }
    }
    sent
}

/// A data folder holding two advancements: `minecraft:root`, a root with a display, and
/// `minecraft:task`, which hangs off it.
fn advancement_data() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-advancement-tab-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let folder = root.join("data").join("minecraft").join("advancement");
    std::fs::create_dir_all(&folder).unwrap();
    let display = |icon: &str, title: &str| {
        format!(r#"{{"icon":{{"id":"{icon}"}},"title":"{title}","description":"{title}"}}"#)
    };
    std::fs::write(
        folder.join("root.json"),
        format!(
            r#"{{"display":{},"criteria":{{}}}}"#,
            display("minecraft:stone", "Root")
        ),
    )
    .unwrap();
    std::fs::write(
        folder.join("task.json"),
        format!(
            r#"{{"parent":"minecraft:root","display":{},"criteria":{{}}}}"#,
            display("minecraft:dirt", "Task")
        ),
    )
    .unwrap();
    root
}

/// ServerGamePacketListenerImpl.handleSeenAdvancements with PlayerAdvancements.setSelectedTab:
/// the tab a player opens is kept when it names a root with a display, and the player is told
/// the tab the server keeps — none for anything else — whenever that changed. A closed screen
/// says nothing about the tab.
#[test]
fn an_advancement_tab_a_guest_opens_is_kept_and_answered() {
    let data = advancement_data();
    let mut sync = SyncData::empty();
    sync.advancements = crate::lan::advancements::ServerAdvancements::load(
        &data,
        &horizon_world::loot::PackOverlay::NONE,
        Default::default(),
        Default::default(),
    );
    assert_eq!(
        sync.advancements.selectable("minecraft:root"),
        Some(true),
        "the fixture's root is selectable"
    );
    let (mut hub, rx) = hub_with_sync(sync);
    {
        let value = 0;
        hub.guests.update(0, |g| g.load_timeout = value);
    }
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let open = |tab: &str| {
        S::SeenAdvancements(play::SeenAdvancements {
            action: 0,
            tab: Some(tab.to_owned()),
        })
    };
    // The root the client opens is the tab the server keeps now.
    tick_packet(&mut hub, &mut world, open("minecraft:root"));
    assert_eq!(sent_tabs(&rx), vec![Some("minecraft:root".to_owned())]);
    assert_eq!(
        hub.guests[0].selected_tab.as_deref(),
        Some("minecraft:root")
    );
    // Opening it again changes nothing, so nothing is sent.
    tick_packet(&mut hub, &mut world, open("minecraft:root"));
    assert!(
        sent_tabs(&rx).is_empty(),
        "an unchanged tab is not sent again"
    );
    // A tab that is not a root takes the selection away, and the client is told so.
    tick_packet(&mut hub, &mut world, open("minecraft:task"));
    assert_eq!(sent_tabs(&rx), vec![None]);
    assert_eq!(hub.guests[0].selected_tab, None);
    // The root again, then an id the server has no advancement for: vanilla's
    // `ServerAdvancementManager.get` is null, `setSelectedTab` is not called, and the
    // tab stays.
    tick_packet(&mut hub, &mut world, open("minecraft:root"));
    assert_eq!(sent_tabs(&rx), vec![Some("minecraft:root".to_owned())]);
    tick_packet(&mut hub, &mut world, open("minecraft:nowhere"));
    assert!(sent_tabs(&rx).is_empty(), "an unknown id changes nothing");
    assert_eq!(
        hub.guests[0].selected_tab.as_deref(),
        Some("minecraft:root")
    );
    // The closed screen keeps the tab the server has.
    tick_packet(
        &mut hub,
        &mut world,
        S::SeenAdvancements(play::SeenAdvancements {
            action: 1,
            tab: None,
        }),
    );
    assert!(sent_tabs(&rx).is_empty(), "a closed screen keeps the tab");
    assert_eq!(
        hub.guests[0].selected_tab.as_deref(),
        Some("minecraft:root")
    );
    let _ = std::fs::remove_dir_all(&data);
}

/// `handlePlayerCommand` START_RIDING_JUMP (3) and STOP_RIDING_JUMP (4): the
/// hub used to drop both, so a guest's released jump bar never reached the
/// mount they ride, and neither did the horse's rear or its jump sound.
#[test]
fn a_guests_riding_jump_reaches_the_game() {
    let (mut hub, _rx) = hub_with_a_guest();
    // hasClientLoaded: the handlers ignore a player's actions until then.
    {
        let value = 0;
        hub.guests.update(0, |g| g.load_timeout = value);
    }
    let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    for action in [3, 4] {
        hub.guests.inbox_mut(0).unwrap().push_back(
            horizon_client::lan::guest::InboxItem::uncredited(S::PlayerCommand(
                play::PlayerCommand {
                    id: 1,
                    action,
                    data: 75,
                },
            )),
        );
        let events = hub.tick(&mut ctx);
        let reached = events.iter().any(|event| {
            matches!(event, HostEvent::Packet(1, packet) if matches!(&**packet, S::PlayerCommand(c) if c.action == action && c.data == 75))
        });
        assert!(reached, "action {action} reached the game");
    }
}

/// The durations of the effect packets the guest was sent.
fn sent_durations(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<i32> {
    let mut durations = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::UpdateMobEffect(p)) = C::decode(id, r.rest()) {
            durations.push(p.duration);
        }
    }
    durations
}

fn sent_frozen(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<i32> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::SetEntityData(p)) = C::decode(id, r.rest()) {
            for (index, _, value) in p.values {
                if let (7, Value::Int(ticks)) = (index, value) {
                    sent.push(ticks);
                }
            }
        }
    }
    sent
}

#[test]
fn a_guests_frozen_ticks_go_to_its_client() {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        return;
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut body =
        crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO).unwrap();
    body.ticks_frozen = 140;
    let (mut hub, rx) = hub_with_a_guest();
    {
        let value = Some(Box::new(body));
        hub.guests.update(0, |g| g.motion = value);
    }
    hub.sync_player_state(0);
    assert_eq!(sent_frozen(&rx), vec![140]);
    hub.sync_player_state(0);
    assert_eq!(sent_frozen(&rx), Vec::<i32>::new());
    hub.guests
        .update(0, |g| g.motion.as_mut().unwrap().ticks_frozen = 0);
    hub.sync_player_state(0);
    assert_eq!(sent_frozen(&rx), vec![0]);
}

/// The host and port of the transfer packets the guest was sent.
fn sent_transfers(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(String, i32)> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::Transfer(p)) = C::decode(id, r.rest()) {
            sent.push((p.host, p.port));
        }
    }
    sent
}

/// The relatives of the position packets the guest was sent.
fn sent_position_relatives(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<i32> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::PlayerPosition(p)) = C::decode(id, r.rest()) {
            sent.push(p.relatives);
        }
    }
    sent
}

/// A command's teleport carries its relative axes as the position
/// packet's DELTA flags (`TeleportCommand.performTeleport` builds them
/// with `Relative.direction`, and `ServerPlayer.teleport` passes them to
/// `ClientboundPlayerPositionPacket`): a relative axis keeps its motion
/// on the client, an absolute axis is a zero change.
#[test]
fn a_command_teleport_carries_its_relative_axes() {
    use play::relative::{DELTA_X, DELTA_Z};
    let (mut hub, rx) = hub_with_a_guest();
    hub.teleport_command(0, [10.5, 70.0, 10.5], [true, false, true]);
    hub.teleport_command(0, [12.5, 70.0, 12.5], [false; 3]);
    assert_eq!(sent_position_relatives(&rx), vec![DELTA_X | DELTA_Z, 0]);
}

/// `TransferCommand.transfer`: the target's client is sent the play-state
/// `ClientboundTransferPacket` with the hostname and the port, and nothing
/// else disconnects it — the client closes and logs in there itself
/// (`ClientPacketListener.handleTransfer`).
#[test]
fn a_transferred_player_is_sent_the_host_and_the_port() {
    let (hub, rx) = hub_with_a_guest();
    hub.transfer(1, "example.net", 25566);
    assert_eq!(sent_transfers(&rx), vec![("example.net".to_owned(), 25566)]);
    // A connection the hub does not know gets nothing.
    hub.transfer(9, "example.net", 25566);
    assert_eq!(
        sent_transfers(&rx),
        Vec::<(String, i32)>::new(),
        "an unknown connection is not a player of this server"
    );
}

/// `ServerPlayer.acceptsSystemMessages`: a guest whose chat is hidden (2) is sent no system message, only an overlay
/// one; a guest who shows chat (0) or only commands (1) is sent both.
#[test]
fn a_system_message_skips_a_guest_whose_chat_is_hidden() {
    let (mut hub, rx) = hub_with_a_guest();
    let count = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        let (mut plain, mut bar) = (0, 0);
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let C::SystemChat(p) = C::decode(id, r.rest()).unwrap() {
                if p.overlay {
                    bar += 1;
                } else {
                    plain += 1;
                }
            }
        }
        (plain, bar)
    };
    for (visibility, expected) in [(0, (3, 1)), (1, (3, 1)), (2, (0, 1))] {
        hub.guests
            .update(0, |g| g.info.chat_visibility = visibility);
        hub.system_message(literal("all"));
        hub.system_message_to(literal("by name"), |name| name == "Bob");
        hub.system_message_to_player(1, literal("to one"));
        hub.overlay_message(literal("bar"));
        assert_eq!(count(&rx), expected, "chat visibility {visibility}");
    }
}

/// The packet `id` with nothing in it: the shortest all-zero body it decodes from.
fn blank_packet(id: i32) -> S {
    (0..=64)
        .find_map(|n| S::decode(id, &vec![0u8; n]).ok())
        .unwrap_or_else(|| panic!("no all-zero body decodes as packet {id}"))
}

/// Whether `packet` sets the idle timer (`ServerPlayer.resetLastActionTime`) of a guest who has
/// or has not loaded and whose vehicle was last accepted at `vehicle_good`.
fn resets_the_idle_timer(packet: S, loaded: bool, vehicle_good: Option<[f64; 3]>) -> bool {
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = floor_at(0);
    hub.guests.update(0, |g| {
        g.load_timeout = if loaded { 0 } else { 60 };
        g.vehicle_good = vehicle_good;
        g.last_action_tick = 5;
    });
    hub.ticks = 100;
    tick_packet(&mut hub, &mut world, packet);
    hub.guests[0].last_action_tick == hub.ticks
}

/// The handlers of `ServerGamePacketListenerImpl` that do not call `resetLastActionTime` leave the
/// idle timer alone: the answers to what the server asked, the client's settings and the
/// requests that are not the player acting (checked one by one with `javap -c -p`).
#[test]
fn packets_that_vanilla_does_not_count_as_activity_leave_the_idle_timer_alone() {
    use horizon_protocol::ids::play::serverbound as id;
    for (name, packet) in [
        ("ChatAck", id::CHAT_ACK),
        ("ContainerClose", id::CONTAINER_CLOSE),
        ("CustomPayload", id::CUSTOM_PAYLOAD),
        ("SeenAdvancements", id::SEEN_ADVANCEMENTS),
        ("ChatSessionUpdate", id::CHAT_SESSION_UPDATE),
        ("CookieResponse", id::COOKIE_RESPONSE),
        ("SetCreativeModeSlot", id::SET_CREATIVE_MODE_SLOT),
        ("PaddleBoat", id::PADDLE_BOAT),
        ("PickItemFromBlock", id::PICK_ITEM_FROM_BLOCK),
        ("KeepAlive", id::KEEP_ALIVE),
        ("ClientTickEnd", id::CLIENT_TICK_END),
        ("AcceptTeleportation", id::ACCEPT_TELEPORTATION),
    ] {
        assert!(
            !resets_the_idle_timer(blank_packet(packet), true, None),
            "{name} reset the idle timer"
        );
    }
}

/// The 17 handlers that call `resetLastActionTime`, for a client that has loaded.
#[test]
fn the_packets_that_vanilla_counts_as_activity_reset_the_idle_timer() {
    use horizon_protocol::ids::play::serverbound as id;
    for (name, packet) in [
        ("Attack", id::ATTACK),
        ("Interact", id::INTERACT),
        ("UseItemOn", id::USE_ITEM_ON),
        ("UseItem", id::USE_ITEM),
        ("PlayerAction", id::PLAYER_ACTION),
        ("PlayerCommand", id::PLAYER_COMMAND),
        ("SpectatorAction", id::SPECTATOR_ACTION),
        ("Chat", id::CHAT),
        ("ChatCommand", id::CHAT_COMMAND),
        ("ChatCommandSigned", id::CHAT_COMMAND_SIGNED),
        ("SignUpdate", id::SIGN_UPDATE),
        ("SetCarriedItem", id::SET_CARRIED_ITEM),
        ("Punch", id::PUNCH),
        ("PlaceRecipe", id::PLACE_RECIPE),
        ("ContainerClick", id::CONTAINER_CLICK),
        ("ContainerButtonClick", id::CONTAINER_BUTTON_CLICK),
        ("ClientCommand", id::CLIENT_COMMAND),
    ] {
        assert!(
            resets_the_idle_timer(blank_packet(packet), true, None),
            "{name} did not reset the idle timer"
        );
    }
    // A packet Horizon does not model counts, as it always did.
    assert!(resets_the_idle_timer(S::Unknown(250, vec![]), true, None));
}

/// `hasClientLoaded` comes before the reset in seven of them and in `handlePlayerInput`, and
/// `handleSetCarriedItem` resets only for a slot of the hotbar.
#[test]
fn the_idle_timer_waits_for_a_loaded_client_and_a_slot_of_the_hotbar() {
    use horizon_protocol::ids::play::serverbound as id;
    for (name, packet) in [
        ("Attack", id::ATTACK),
        ("Interact", id::INTERACT),
        ("UseItemOn", id::USE_ITEM_ON),
        ("UseItem", id::USE_ITEM),
        ("PlayerAction", id::PLAYER_ACTION),
        ("PlayerCommand", id::PLAYER_COMMAND),
        ("SpectatorAction", id::SPECTATOR_ACTION),
        ("PlayerInput", id::PLAYER_INPUT),
    ] {
        assert!(
            !resets_the_idle_timer(blank_packet(packet), false, None),
            "{name} reset the idle timer of a client that has not loaded"
        );
        assert!(
            resets_the_idle_timer(blank_packet(packet), true, None),
            "{name} did not reset the idle timer of a client that has loaded"
        );
    }
    let slot =
        |slot| resets_the_idle_timer(S::SetCarriedItem(play::SetCarriedItem { slot }), true, None);
    assert!(slot(0) && slot(8));
    assert!(!slot(9) && !slot(12) && !slot(-1));
}

/// A vehicle's move that repeats the position the game last accepted is the client standing
/// still (it sends one every tick for a vehicle it drives): no activity. A move away counts, and
/// so does any move when no position was kept for the vehicle.
#[test]
fn a_vehicle_that_stands_still_does_not_reset_the_idle_timer() {
    let at = [0.5, 64.0, 0.5];
    let moves = |to: [f64; 3]| {
        S::MoveVehicle(play::MoveVehicleRequest {
            position: to,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        })
    };
    // The same place, and a shift of 0.002 (squared 4.0E-6, under 1.0E-5).
    assert!(!resets_the_idle_timer(moves(at), true, Some(at)));
    assert!(!resets_the_idle_timer(
        moves([0.502, 64.0, 0.5]),
        true,
        Some(at)
    ));
    // A shift of 0.004 (squared 1.6E-5) and a block away.
    assert!(resets_the_idle_timer(
        moves([0.504, 64.0, 0.5]),
        true,
        Some(at)
    ));
    assert!(resets_the_idle_timer(
        moves([1.5, 64.0, 0.5]),
        true,
        Some(at)
    ));
    // No position kept for this vehicle: the move counts.
    assert!(resets_the_idle_timer(moves(at), true, None));
}

/// `/effect give Bob minecraft:speed 30 1` from a dedicated server's
/// console (#7115). Vanilla runs the console after the players' tick
/// (`DedicatedServer.tickConnection`) and `ServerPlayer.onEffectAdded`
/// sends the effect at once: 30 s is 600 ticks, and the tick after, which
/// counts it down to 599, sends nothing (`LivingEntity.tickEffects` only
/// refreshes it at a multiple of 600 left). The console syncs right after
/// its command (`console_command`) for the same packet.
#[test]
fn an_effect_synced_after_its_command_is_sent_once_at_its_full_duration() {
    let (mut hub, rx) = hub_with_a_guest();
    hub.guests.update(0, |g| {
        g.effects.add(EffectInstance::new("speed", 30 * 20, 1));
    });
    // The console's command, then its sync.
    hub.sync_all_effects();
    // The next tick: the guests' effects count down (`tick_guests`), then
    // the hub syncs them (`tick_level`).
    hub.guests.update(0, |g| g.effects.tick());
    hub.sync_all_effects();
    hub.guests.update(0, |g| g.effects.tick());
    hub.sync_all_effects();
    assert_eq!(sent_durations(&rx), vec![600]);
    // Without the sync after the command, the first sync comes after the
    // tick has counted it down: what #7115's transcript shows.
    let (mut late, rx) = hub_with_a_guest();
    late.guests.update(0, |g| {
        g.effects.add(EffectInstance::new("speed", 30 * 20, 1));
    });
    late.guests.update(0, |g| g.effects.tick());
    late.sync_all_effects();
    assert_eq!(sent_durations(&rx), vec![599]);
}

/// Minecraft.startAttack sends the attack, then the arm swing, and the next client tick
/// a turn. Vanilla handles both in the tick they came in; the swing starts the attack
/// strength again, so a swing left for the next tick took a tick off every hit's
/// charge: a guest's fists, 5 ticks apart, no longer broke a minecart (#7817).
#[test]
fn an_attacks_swing_goes_with_it_past_the_move_behind() {
    let (mut hub, _rx) = hub_with_a_guest();
    {
        let value = 0;
        hub.guests.update(0, |g| g.load_timeout = value);
    }
    let inbox = hub.guests.inbox_mut(0).unwrap();
    inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(
        S::Attack(play::Attack { id: 7 }),
    ));
    inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(S::Punch(
        horizon_protocol::packets::Empty,
    )));
    inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(
        S::MovePlayerRot(play::MovePlayerRot {
            yaw: 90.0,
            pitch: 0.0,
            flags: 1,
        }),
    ));
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let events = hub.drain_packets(&mut ctx);
    let handled: Vec<&S> = events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Packet(_, p) => Some(&**p),
            _ => None,
        })
        .collect();
    assert!(
        matches!(handled[..], [S::Attack(_), S::Punch(_)]),
        "the game was handed {handled:?}"
    );
    assert!(
        matches!(
            hub.guests[0].inbox.iter().collect::<Vec<_>>()[..],
            [InboxItem::Packet(S::MovePlayerRot(_), _)]
        ),
        "the turn did not wait for the next tick"
    );
}

/// Drains guest 0's inbox holding a player action (`action`) and the move
/// its client sent after it in the same client tick: the packets handed to
/// the game, and the guest's position and known movement before.
fn drain_action_then_move(action: i32) -> (Hub, Vec<S>, ([f64; 3], [f64; 3])) {
    let action = S::PlayerAction(play::PlayerAction {
        action,
        pos: [0; 3],
        direction: 0,
        sequence: 0,
    });
    drain_then_move(action, |_| {})
}

/// [`drain_action_then_move`] for any packet, with `setup` applied to
/// the guest first.
fn drain_then_move(
    first: S,
    setup: impl FnOnce(&mut Guest),
) -> (Hub, Vec<S>, ([f64; 3], [f64; 3])) {
    let (mut hub, _rx) = hub_with_a_guest();
    let before = (hub.guests[0].position, hub.guests[0].known_movement);
    hub.guests.update(0, |guest| {
        guest.load_timeout = 0;
        setup(guest);
        let inbox = &mut guest.inbox;
        inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(first));
        inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(
            S::MovePlayerPos(play::MovePlayerPos {
                x: before.0[0] + 0.3,
                y: before.0[1],
                z: before.0[2],
                flags: 1,
            }),
        ));
    });
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let handled = hub
        .drain_packets(&mut ctx)
        .into_iter()
        .filter_map(|e| match e {
            HostEvent::Packet(_, p) => Some(*p),
            _ => None,
        })
        .collect();
    (hub, handled, before)
}

/// #8423: a RELEASE_USE_ITEM shoots from the player's position and known
/// movement of the last move the server handled; the move the client sent
/// after it waits for the next tick, as vanilla handles the two in order.
#[test]
fn a_release_shoots_before_the_move_behind_it() {
    let (hub, handled, before) = drain_action_then_move(6);
    assert!(
        matches!(handled[..], [S::PlayerAction(_)]),
        "the game was handed {handled:?}"
    );
    assert!(
        matches!(
            hub.guests[0].inbox.iter().collect::<Vec<_>>()[..],
            [InboxItem::Packet(S::MovePlayerPos(_), _)]
        ),
        "the move did not wait for the next tick"
    );
    let g = &hub.guests[0];
    assert_eq!(
        (g.position, g.known_movement),
        before,
        "the shot's position"
    );
}

/// A STAB's move runs in the same tick: held for the next one, the jump it
/// carries came a tick after the Lunge's impulse (lunge_motion).
#[test]
fn a_stab_keeps_the_move_behind_it_in_its_tick() {
    let (hub, handled, before) = drain_action_then_move(8);
    assert!(
        matches!(handled[..], [S::PlayerAction(_)]),
        "the game was handed {handled:?}"
    );
    let g = &hub.guests[0];
    assert!(g.inbox.is_empty(), "the move waited for the next tick");
    assert_eq!(g.position[0], before.0[0] + 0.3, "the move was not handled");
}

/// #17206's review: a rider's MoveVehicle and its ClientTickEnd in one drain.
/// The hub hands the game the move and queues the tick-end reset behind it, so
/// the game sets the rider's movement before the reset keeps it for the tick
/// (vanilla runs both handlers in packet order on the main thread,
/// `ServerGamePacketListenerImpl.handleMoveVehicle` then `handleClientTickEnd`).
/// The reset used to run inline ahead of the queued move, leaving the flag set
/// and a stale movement kept into the next tick.
#[test]
fn a_vehicle_move_and_its_tick_end_keep_packet_order() {
    let (mut hub, _rx) = hub_with_a_guest();
    hub.guests.update(0, |g| {
        g.load_timeout = 0;
        g.vehicle = Some(7);
        // A move from an earlier tick, which the inline reset used to zero
        // before the queued move of this drain ever ran.
        g.known_movement = [0.9, 0.0, 0.0];
        g.movement_this_tick = false;
        g.inbox.push_back(InboxItem::uncredited(S::MoveVehicle(
            play::MoveVehicleRequest {
                position: [0.5, 64.0, 0.5],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: true,
            },
        )));
        g.inbox.push_back(InboxItem::uncredited(S::ClientTickEnd(
            horizon_protocol::packets::Empty,
        )));
    });
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let events = hub.drain_packets(&mut ctx);
    // The move goes to the game first; the tick-end reset follows it in order.
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Packet(_, p) if matches!(&**p, S::MoveVehicle(_)) => Some("move"),
            HostEvent::TickEnd(_) => Some("tick-end"),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        ["move", "tick-end"],
        "the reset follows the move in packet order: {kinds:?}"
    );
    // The hub deferred the movement reset (the stale move is still there for
    // the game's dispatch to overwrite), but the position flag reset inline:
    // no queued move bears on it.
    assert_eq!(
        hub.guests[0].known_movement,
        [0.9, 0.0, 0.0],
        "the movement reset waits for the queued move"
    );
    assert!(
        !hub.guests[0].position_this_tick,
        "the position flag reset inline"
    );
    // The game dispatches the move (sets the movement and the flag), then the
    // queued tick end keeps that move for the tick and clears the flag.
    let conn = hub.guests[0].conn;
    hub.guests.update(0, |g| {
        g.known_movement = [0.4, 0.0, 0.0];
        g.movement_this_tick = true;
    });
    hub.client_tick_end(conn);
    assert_eq!(
        hub.guests[0].known_movement,
        [0.4, 0.0, 0.0],
        "the move's own tick keeps it"
    );
    assert!(!hub.guests[0].movement_this_tick);
    // The next tick has no move of either: its tick end zeroes the movement.
    hub.client_tick_end(conn);
    assert_eq!(
        hub.guests[0].known_movement, [0.0; 3],
        "a client tick with no move of either has none"
    );
}

/// STOP_SLEEPING goes to the game, which wakes the guest after the mods
/// hear it (`player_bed_leave`); the move the client sent after it waits
/// for the next tick, so it never runs while the guest still sleeps.
#[test]
fn a_move_after_leave_bed_waits_for_the_wake() {
    let leave = S::PlayerCommand(play::PlayerCommand {
        id: 0,
        action: 0,
        data: 0,
    });
    let (hub, handled, before) = drain_then_move(leave, |guest| {
        guest.bed = Some([5, 64, 5]);
        guest.sleeping = true;
    });
    assert!(
        matches!(
            handled[..],
            [S::PlayerCommand(play::PlayerCommand { action: 0, .. })]
        ),
        "the game was handed {handled:?}"
    );
    let g = &hub.guests[0];
    assert!(
        matches!(
            g.inbox.iter().collect::<Vec<_>>()[..],
            [InboxItem::Packet(S::MovePlayerPos(_), _)]
        ),
        "the move did not wait for the wake"
    );
    assert_eq!(
        (g.bed, g.sleeping),
        (Some([5, 64, 5]), true),
        "the hub woke the guest"
    );
    assert_eq!(
        (g.position, g.known_movement),
        before,
        "the move ran in bed"
    );
}

/// The owner's own shoulder parrots are sent when a slot changes.
#[test]
fn the_owner_is_sent_the_shoulder_slots_that_changed() {
    assert!(shoulder_changes([None; 2], [None; 2]).is_empty());
    let landed = shoulder_changes([Some(1), None], [None; 2]);
    assert_eq!(landed.len(), 1);
    assert_eq!((landed[0].0, &landed[0].2), (19, &Value::Id(2)));
    let left = shoulder_changes([None; 2], [Some(1), Some(4)]);
    assert_eq!(left.len(), 2);
    assert_eq!((left[1].0, &left[1].2), (20, &Value::Id(0)));
}

/// The counts of what is stuck in a player or a mob are sent when set, and a
/// count back at zero is sent as the default.
#[test]
fn stuck_counts_are_synched_as_entity_data() {
    let mut state = EntityState::default();
    stuck_meta(&mut state.meta, &mut state.defaults, (3, 0));
    assert_eq!(state.meta, vec![(12, serializer::INT, Value::Int(3))]);
    assert_eq!(
        state.defaults,
        vec![
            (12, serializer::INT, Value::Int(0)),
            (13, serializer::INT, Value::Int(0))
        ]
    );
}

/// Joined states are used in the server tick after the one that noted
/// them, never later.
#[test]
fn joined_states_last_only_into_the_next_tick() {
    let joined = vec![EntityState {
        id: 7,
        ..EntityState::default()
    }];
    assert_eq!(noted_last_tick(Some((5, joined.clone())), 6), joined);
    assert!(noted_last_tick(Some((5, joined.clone())), 5).is_empty());
    assert!(noted_last_tick(Some((5, joined)), 9).is_empty());
    assert!(noted_last_tick(None, 6).is_empty());
}

/// `getMaximumFlyingTicks`: 80 ticks at normal gravity, ceil(80 * 0.08 /
/// gravity) where it is weaker, and never a kick without gravity.
#[test]
fn maximum_flying_ticks_follow_gravity() {
    assert_eq!(maximum_flying_ticks(0.08), 80);
    assert_eq!(maximum_flying_ticks(0.16), 80);
    assert_eq!(maximum_flying_ticks(0.04), 160);
    assert_eq!(maximum_flying_ticks(0.03), 214);
    assert_eq!(maximum_flying_ticks(0.0), i32::MAX);
    assert_eq!(maximum_flying_ticks(1.0E-6), i32::MAX);
}

fn textures(value: &str) -> horizon_protocol::packets::login::Property {
    horizon_protocol::packets::login::Property {
        name: "textures".into(),
        value: value.into(),
        signature: Some("c2lnbmVk".into()),
    }
}

/// While a player is sent again the entity tracker hides them from everyone (watching clients get the
/// removal and the respawn around the profile update); afterwards it hides them at the level a mod set.
#[test]
fn a_refresh_hides_the_player_from_the_entity_tracker_and_gives_back_the_mods_level() {
    let (mut hub, _rx) = hub_with_a_guest();
    assert_eq!(hub.tracker_hidden(1), None);
    hub.hidden.insert(1, 3);
    assert_eq!(hub.tracker_hidden(1), Some(3));
    hub.refresh_player(1);
    assert_eq!(
        hub.tracker_hidden(1),
        Some(u8::MAX),
        "out of every client's tracking"
    );
    assert!(!hub.sees(255, 1));
    hub.ticks += 2;
    hub.end_refreshes();
    assert_eq!(hub.tracker_hidden(1), Some(3), "the mod's level again");
    hub.hidden.remove(&1);
    assert_eq!(hub.tracker_hidden(1), None);
}

/// The owner is listed with the account's skin once the fetch answered,
/// with none before (no wait) or when it failed; other players keep the
/// properties they logged in with, and none of the owner's.
#[test]
fn the_owner_is_listed_with_the_accounts_skin_once_fetched() {
    let (answer, fetched) = std::sync::mpsc::channel();
    let profile = super::super::OwnerProfile::new(move || fetched.recv().unwrap_or_default());
    let start = std::time::Instant::now();
    assert!(listed_properties(&[], &[], true, Some(&profile)).is_empty());
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    // A property a mod set before the fetch answered is listed alone, and gives way to nothing later.
    let badge = horizon_protocol::packets::login::Property {
        name: "example:badge".into(),
        value: "1".into(),
        signature: None,
    };
    assert_eq!(
        listed_properties(&[], std::slice::from_ref(&badge), true, Some(&profile)),
        vec![badge.clone()]
    );
    answer.send(vec![textures("owner")]).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while listed_properties(&[], &[], true, Some(&profile)).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the fetch never landed"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        listed_properties(&[], &[], true, Some(&profile)),
        vec![textures("owner")]
    );
    // A mod's property is an override by name: the account's others stay, its own textures replace.
    assert_eq!(
        listed_properties(&[], std::slice::from_ref(&badge), true, Some(&profile)),
        vec![textures("owner"), badge.clone()]
    );
    assert_eq!(
        listed_properties(&[], &[textures("mod")], true, Some(&profile)),
        vec![textures("mod")]
    );
    // A guest: its own properties (online mode), or none (offline mode).
    assert_eq!(
        listed_properties(&[textures("guest")], &[], false, Some(&profile)),
        vec![textures("guest")]
    );
    assert!(listed_properties(&[], &[], false, Some(&profile)).is_empty());
    // No signed-in account (no fetch): as before.
    assert!(listed_properties(&[], &[], true, None).is_empty());
    let failed = super::super::OwnerProfile::new(Vec::new);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while failed.properties.get().is_none() {
        let _ = failed.properties();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(listed_properties(&[], &[], true, Some(&failed)).is_empty());
}

fn property(name: &str, value: &str) -> horizon_protocol::packets::login::Property {
    horizon_protocol::packets::login::Property {
        name: name.into(),
        value: value.into(),
        signature: None,
    }
}

/// An owner profile whose fetch has answered with `properties`.
fn fetched(
    properties: Vec<horizon_protocol::packets::login::Property>,
) -> super::super::OwnerProfile {
    let profile = super::super::OwnerProfile::new(move || properties.clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while profile.properties().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the fetch never landed"
        );
        std::thread::yield_now();
    }
    profile
}

fn names(listed: &[horizon_protocol::packets::login::Property]) -> Vec<&str> {
    listed.iter().map(|p| p.name.as_str()).collect()
}

/// A client refuses a player list update with a 17th property: when a mod set 16 before the owner's
/// account arrived, the late merge lists the mod's 16 only, and in general the overrides win and the
/// account's other properties fill what is left, in their order.
#[test]
fn a_late_account_fetch_never_lists_more_than_16_properties() {
    let account = fetched(vec![
        property("textures", "owner"),
        property("cape", "owner"),
        property("extra", "owner"),
    ]);
    let mods: Vec<_> = (0..16).map(|i| property(&format!("m{i}"), "v")).collect();
    let listed = listed_properties(&[], &mods, true, Some(&account));
    assert_eq!(listed, mods, "16 overrides leave no slot for the account");
    // 15 overrides leave one slot: the account's first property that is not overridden.
    let listed = listed_properties(&[], &mods[..15], true, Some(&account));
    assert_eq!(listed.len(), 16);
    assert_eq!(names(&listed)[..2], ["textures", "m0"]);
    // An overridden property of the account takes its place and costs no extra slot.
    let mut overrides = vec![property("cape", "mod")];
    overrides.extend(mods[..14].iter().cloned());
    let listed = listed_properties(&[], &overrides, true, Some(&account));
    assert_eq!(listed.len(), 16);
    assert_eq!(names(&listed)[..2], ["textures", "cape"]);
    assert_eq!(listed[1].value, "mod");
    assert!(
        !names(&listed).contains(&"extra"),
        "the account's last gave way"
    );
    // Overrides over the 16 are cut (the setter refuses them, so only a stale list is cut).
    let many: Vec<_> = (0..20).map(|i| property(&format!("m{i}"), "v")).collect();
    assert_eq!(
        listed_properties(&[], &many, true, Some(&account)),
        many[..16]
    );
    // A name set twice is listed once, as the later one.
    let twice = [property("textures", "a"), property("textures", "b")];
    assert_eq!(
        listed_properties(&[], &twice, true, Some(&account))[0],
        property("textures", "b")
    );
    // A guest's login properties are cut at 16 too, and overridden by name.
    let login: Vec<_> = (0..20).map(|i| property(&format!("l{i}"), "v")).collect();
    let listed = listed_properties(&login, &[property("l3", "mod")], false, None);
    assert_eq!(listed.len(), 16);
    assert_eq!(listed[3].value, "mod");
    assert_eq!(names(&listed)[..3], ["l0", "l1", "l2"]);
}

/// Several valid profiles of 16 properties each pass the frame limit in one `PlayerInfoUpdate`; the
/// batches keep the order and each encodes under the limit, so no connection is closed by the framing.
#[test]
fn a_player_list_is_split_into_packets_that_fit_the_frame() {
    let entry = |i: u128| play::PlayerInfo {
        uuid: i,
        name: Some(format!("player{i}")),
        properties: (0..16)
            .map(|n| horizon_protocol::packets::login::Property {
                name: format!("p{n}"),
                value: "v".repeat(32767),
                signature: Some("s".repeat(1024)),
            })
            .collect(),
        game_mode: Some(0),
        listed: Some(true),
        latency: Some(0),
        display_name: Some(None),
        list_order: Some(0),
        show_hat: Some(true),
        ..Default::default()
    };
    let entries: Vec<_> = (0..40).map(entry).collect();
    let batches = info_batches(Hub::INFO_ACTIONS, entries.clone());
    assert!(batches.len() > 1, "{} batches", batches.len());
    for batch in &batches {
        assert!(!batch.is_empty());
        let bytes = C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Hub::INFO_ACTIONS,
            entries: batch.clone(),
        })
        .encode();
        assert!(
            bytes.len() <= horizon_protocol::frame::MAX_FRAME,
            "a batch of {} entries is {} bytes",
            batch.len(),
            bytes.len()
        );
    }
    let flat: Vec<_> = batches.into_iter().flatten().collect();
    assert_eq!(flat, entries, "every entry once, in order");
    // A short list stays one packet, and no entries still give the empty list the join always sent.
    assert_eq!(
        info_batches(Hub::INFO_ACTIONS, entries[..3].to_vec()).len(),
        1
    );
    assert_eq!(
        info_batches(Hub::INFO_ACTIONS, Vec::new()),
        vec![Vec::<play::PlayerInfo>::new()]
    );
}

/// `requeue` as it was, scanning the queue for each chunk of the view.
fn requeue_scan(
    queue: &mut Vec<(i32, i32)>,
    sent: &FxHashSet<(i32, i32)>,
    (cx, cz): (i32, i32),
    r: i32,
) {
    queue.retain(|&(x, z)| tracked(cx, cz, r, x, z));
    for dz in -r - 2..=r + 2 {
        for dx in -r - 2..=r + 2 {
            let key = (cx + dx, cz + dz);
            if tracked(cx, cz, r, key.0, key.1) && !sent.contains(&key) && !queue.contains(&key) {
                queue.push(key);
            }
        }
    }
    queue.sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
}

/// A player walking a loop, sending a few chunks a tick: the queue
/// after every move is the one the scan built.
#[test]
#[ignore = "daily"]
fn requeue_matches_the_scan() {
    for r in [2, 5, 10, 12, 32] {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut sent = FxHashSet::default();
        for step in 0..60i32 {
            let center = ((step % 17) - 8, (step * 3 % 11) - 5);
            requeue(&mut a, &sent, center, r);
            requeue_scan(&mut b, &sent, center, r);
            assert_eq!(a, b, "radius {r}, step {step}");
            for key in a.drain(..a.len().min(7)) {
                sent.insert(key);
            }
            b.drain(..b.len().min(7));
            sent.retain(|&(x, z)| tracked(center.0, center.1, r, x, z));
        }
    }
}

/// The far ring's queue is what the view's leaves of the chunks tracked at the reach, closest
/// first, with the sent ones out of both.
#[test]
fn the_far_ring_queues_what_the_view_leaves() {
    let mut sent = FxHashSet::default();
    let (mut near, mut far) = (Vec::new(), Vec::new());
    for step in 0..12 {
        let center = (step, -step / 2);
        requeue(&mut near, &sent, center, 10);
        requeue_ring(&mut far, &sent, center, 10, 24);
        let mut all = Vec::new();
        requeue(&mut all, &sent, center, 24);
        assert!(far.iter().all(|key| !near.contains(key)));
        let mut both = [near.clone(), far.clone()].concat();
        both.sort_unstable();
        all.sort_unstable();
        assert_eq!(both, all);
        let distance =
            |&(x, z): &(i32, i32)| i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2);
        assert!(far.windows(2).all(|w| distance(&w[0]) <= distance(&w[1])));
        // A few of each sent.
        for key in near
            .drain(..near.len().min(40))
            .chain(far.drain(..far.len().min(90)))
        {
            sent.insert(key);
        }
    }
    // Without a ring, nothing.
    requeue_ring(&mut far, &sent, (0, 0), 10, 10);
    assert!(far.is_empty());
}

/// `MinecraftServer.onGameRuleChanged`: setting `limited_crafting` updates the hub's state
/// and broadcasts `ClientboundGameEventPacket` (event 12) with 1.0 (on) or 0.0 (off) to guests.
#[test]
fn set_limited_crafting_broadcasts_game_event() {
    let (mut hub, rx) = hub_with_a_guest();
    assert!(!hub.limited_crafting);

    // Setting to true broadcasts game event 12 with param 1.0
    hub.set_limited_crafting(true);
    assert!(hub.limited_crafting);

    let out = rx.try_recv().expect("packet should be sent to guest");
    let body = match out {
        crate::lan::Out::Packet(b) => b,
        _ => panic!("expected Out::Packet"),
    };
    let mut reader = horizon_protocol::buf::Reader::new(&body);
    let id = reader.varint().unwrap();
    let packet = play::Clientbound::decode(id, &body[reader.position()..]).unwrap();
    match packet {
        play::Clientbound::GameEvent(p) => {
            assert_eq!(p.event, game_event::LIMITED_CRAFTING);
            assert_eq!(p.param, 1.0);
        }
        other => panic!("expected GameEvent, got {other:?}"),
    }

    // Setting to the same state should not broadcast another packet
    hub.set_limited_crafting(true);
    assert!(rx.try_recv().is_err());

    // Setting to false broadcasts game event 12 with param 0.0
    hub.set_limited_crafting(false);
    assert!(!hub.limited_crafting);

    let out = rx.try_recv().expect("packet should be sent to guest");
    let body = match out {
        crate::lan::Out::Packet(b) => b,
        _ => panic!("expected Out::Packet"),
    };
    let mut reader = horizon_protocol::buf::Reader::new(&body);
    let id = reader.varint().unwrap();
    let packet = play::Clientbound::decode(id, &body[reader.position()..]).unwrap();
    match packet {
        play::Clientbound::GameEvent(p) => {
            assert_eq!(p.event, game_event::LIMITED_CRAFTING);
            assert_eq!(p.param, 0.0);
        }
        other => panic!("expected GameEvent, got {other:?}"),
    }
}

/// cargo test --release -p horizon-client --lib lan::hub::tests::requeue_cost -- --ignored --nocapture
#[test]
#[ignore]
fn requeue_cost() {
    for r in [10, 16, 32] {
        for (name, f) in [
            ("scan", requeue_scan as fn(&mut _, &_, _, _)),
            ("set", requeue as fn(&mut _, &_, _, _)),
        ] {
            let sent = FxHashSet::default();
            let mut queue = Vec::new();
            let start = std::time::Instant::now();
            let moves = 200;
            for step in 0..moves {
                // Nothing sent yet (an explorer waiting on generation).
                f(&mut queue, &sent, (step, 0), r);
            }
            let per = start.elapsed().as_secs_f64() * 1e6 / f64::from(moves);
            println!(
                "view {r}: {name} {per:.1} us per chunk crossed, {} queued",
                queue.len()
            );
        }
    }
}

/// Listens to `chunk_watch`, so the hub records the watch trio.
struct Watcher;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Watcher {
    fn chunk_watch(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::ChunkWatchEvent) {}
}

/// The hub's chunk step for its one guest, as many batches as the level holds.
fn send_chunks(hub: &mut Hub, world: &mut World) {
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    for _ in 0..3 {
        hub.ticks += 10;
        {
            let value = 0;
            hub.guests.update(0, |g| g.unacknowledged = value);
        }
        hub.update_chunks(&ctx);
    }
}

/// The recorded `(kind, dimension, chunk)`s since the last call, sorted
/// within each kind's run.
fn watches(hub: &mut Hub) -> Vec<(super::super::chunk_watch::ChunkWatchKind, String, [i32; 2])> {
    hub.chunk_watches
        .take()
        .into_iter()
        .flat_map(|run| {
            run.watches.into_iter().map(move |w| {
                assert_eq!((w.uuid, w.entity_id), (1, 1));
                (w.kind, run.dimension.clone(), w.chunk)
            })
        })
        .collect()
}

/// With no mod listening, a registered passage (`record_for_passages`) makes the hub
/// record the chunks it sends from then on, for the passages the game sends with them.
#[test]
fn a_passage_makes_the_hub_record_the_chunks_it_sends() {
    use super::super::chunk_watch::ChunkWatchKind::{Sent, Watch};
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = floor_at(1);
    {
        let value = 2;
        hub.guests.update(0, |g| g.view_distance = value);
    }
    hub.chunk_watches.record_for_passages();
    send_chunks(&mut hub, &mut world);
    let mut got = watches(&mut hub);
    got.sort_by_key(|(k, _, c)| (*k as u8, *c));
    let overworld = "minecraft:overworld".to_string();
    let four = [[-1, -1], [-1, 0], [0, -1], [0, 0]];
    let expected: Vec<_> = [Watch, Sent]
        .into_iter()
        .flat_map(|kind| four.map(|c| (kind, overworld.clone(), c)))
        .collect();
    assert_eq!(got, expected);
}

/// `ChunkWatchEvent`: each near chunk sent is watched then sent, once;
/// leaving the view, changing level and disconnecting unwatch it, in the
/// level it was watched in. Nothing is recorded while no mod listens.
#[test]
fn the_watch_trio_follows_each_sent_chunk() {
    use super::super::chunk_watch::ChunkWatchKind::{Sent, Unwatch, Watch};
    let overworld = || "minecraft:overworld".to_string();
    let four = [[-1, -1], [-1, 0], [0, -1], [0, 0]];
    let run = |kind, dimension: &str| -> Vec<_> {
        four.iter()
            .map(|&c| (kind, dimension.to_string(), c))
            .collect()
    };
    let sorted = |mut v: Vec<_>| {
        v.sort_by_key(|(k, _, c): &(_, String, [i32; 2])| (*k as u8, *c));
        v
    };

    // No listener: chunks go out, nothing is recorded or kept.
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = floor_at(1);
    {
        let value = 2;
        hub.guests.update(0, |g| g.view_distance = value);
    }
    send_chunks(&mut hub, &mut world);
    assert_eq!(hub.guests[0].sent_chunks.len(), 4);
    assert!(hub.chunk_watches.is_empty() && hub.guests[0].watched.is_empty());

    let mods = vec![mod_api::ModEntry::new("watch", Box::new(Watcher))].into();
    mod_platform::testing::with_mods(mods, || {
        let (mut hub, _rx) = hub_with_a_guest();
        {
            let value = 2;
            hub.guests.update(0, |g| g.view_distance = value);
        }
        send_chunks(&mut hub, &mut world);
        let got = watches(&mut hub);
        // The batch's watches come before its sends.
        assert_eq!(sorted(got[..4].to_vec()), run(Watch, &overworld()));
        assert_eq!(sorted(got[4..].to_vec()), run(Sent, &overworld()));
        // A chunk crossing that leaves the four behind.
        {
            let value = [400.5, 64.0, 0.5];
            hub.guests.update(0, |g| g.position = value);
        }
        send_chunks(&mut hub, &mut world);
        assert_eq!(sorted(watches(&mut hub)), run(Unwatch, &overworld()));
        assert!(hub.guests[0].watched.is_empty());
        // Back, then through a portal: unwatched in the level left, in
        // vanilla's view order.
        {
            let value = [0.5, 64.0, 0.5];
            hub.guests.update(0, |g| g.position = value);
        }
        send_chunks(&mut hub, &mut world);
        assert_eq!(watches(&mut hub).len(), 8);
        hub.set_dimension("minecraft:the_nether");
        let mut next_entity = 100;
        let border = crate::worldborder::WorldBorder::default();
        let ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            spawn_dimension: "minecraft:overworld",
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        hub.change_level(0, [0.5, 64.0, 0.5], 0.0, None, true, 0, &ctx);
        // The nether's chunks: the level left and the new one are a run
        // each, which names its level once.
        send_chunks(&mut hub, &mut world);
        let runs = hub.chunk_watches.take();
        let levels: Vec<_> = runs
            .iter()
            .map(|r| (r.dimension.as_str(), r.watches.len()))
            .collect();
        assert_eq!(
            levels,
            [("minecraft:overworld", 4), ("minecraft:the_nether", 8)]
        );
        let left: Vec<_> = runs[0]
            .watches
            .iter()
            .map(|w| (w.kind, overworld(), w.chunk))
            .collect();
        assert_eq!(left, run(Unwatch, &overworld()));
        // Then a disconnect.
        let _ = hub.remove_guest(0, "bye");
        assert_eq!(watches(&mut hub), run(Unwatch, "minecraft:the_nether"));
    });
}

/// A single-player memory connection sends every ready view column before
/// `PlayerLoaded`. After that, on a respawn, and for a socket guest, the
/// batch is `chunks_per_tick` again (64). An unlit column stays out, and
/// the gates that send nothing still do.
#[test]
fn memory_connection_first_view_sends_every_ready_column() {
    let (mut hub, _rx) = hub_with_a_guest();
    hub.ticks = 3;
    {
        hub.guests.update(0, |g| {
            g.chunks_per_tick = 64.0;
            g.batch_quota = 0.0;
            g.address = crate::lan::conn::MEMORY_ADDRESS;
            g.loaded = false;
        });
    }
    // The unlit column first, then 80 lit ones (`floor_at`'s empty column, lit).
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut dark = crate::chunk::Chunk::empty(100, 100);
    dark.lit = false;
    world.chunks.insert((100, 100), std::sync::Arc::new(dark));
    hub.guests.update(0, |g| {
        g.chunk_queue.push((100, 100));
    });
    for i in 0..80 {
        let (x, z) = (i % 16, i / 16);
        let mut chunk = crate::chunk::Chunk::empty(x, z);
        chunk.lit = true;
        world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        hub.guests.update(0, |g| {
            g.chunk_queue.push((x, z));
        });
    }

    let batch = hub
        .next_batch(0, &world, None)
        .expect("the first view sends");
    assert_eq!(
        batch.near.len(),
        80,
        "every ready column leaves in one batch"
    );
    assert!(
        batch.near.iter().all(|(_, c)| c.lit),
        "an unlit column is not ready"
    );
    assert!(batch.far.is_empty());
    assert!(hub.guests[0].batch_quota >= 80.0);

    hub.guests.update(0, |g| {
        g.loaded = true;
    });
    let loaded = hub
        .next_batch(0, &world, None)
        .expect("a loaded client still receives chunks");
    assert_eq!(loaded.near.len(), 64, "PlayerLoaded puts the cap back");

    hub.guests.update(0, |g| {
        g.address = "127.0.0.1:25565".parse().unwrap();
    });
    hub.guests.update(0, |g| {
        g.loaded = false;
    });
    hub.guests.update(0, |g| {
        g.batch_quota = 0.0;
    });
    let socket = hub
        .next_batch(0, &world, None)
        .expect("a socket guest still receives chunks");
    assert_eq!(socket.near.len(), 64, "a socket guest keeps the quota");

    hub.guests.update(0, |g| {
        g.address = crate::lan::conn::MEMORY_ADDRESS;
    });
    hub.guests.update(0, |g| {
        g.loaded = true;
    });
    hub.guests.update(0, |g| {
        g.loading = true;
    });
    hub.guests.update(0, |g| {
        g.batch_quota = 0.0;
    });
    let respawn = hub
        .next_batch(0, &world, None)
        .expect("a respawned player still receives chunks");
    assert_eq!(
        respawn.near.len(),
        64,
        "a respawn stays on the vanilla pace"
    );

    // Seventy lit columns, the unlit one still queued: all seventy, in one batch.
    hub.guests.update(0, |g| {
        g.loaded = false;
    });
    hub.guests.update(0, |g| {
        g.batch_quota = 0.0;
    });
    hub.guests.update(0, |g| {
        g.chunk_queue.truncate(71);
    });
    let seventy = hub
        .next_batch(0, &world, None)
        .expect("seventy ready columns send");
    assert_eq!(seventy.near.len(), 70);
    assert!(seventy.near.iter().all(|(_, c)| c.lit));

    hub.guests.update(0, |g| {
        g.unacknowledged = g.max_unacknowledged;
    });
    assert!(
        hub.next_batch(0, &world, None).is_none(),
        "too many batches still waiting sends nothing"
    );

    hub.guests.update(0, |g| {
        g.unacknowledged = 0;
    });
    hub.ticks = hub.guests[0].placed_at + 1;
    assert!(
        hub.next_batch(0, &world, None).is_none(),
        "the tick after placement still waits"
    );

    hub.ticks = 3;
    hub.guests.update(0, |g| {
        g.chunk_queue.clear();
    });
    hub.guests.update(0, |g| {
        g.far_queue.clear();
    });
    assert!(
        hub.next_batch(0, &world, None).is_none(),
        "an empty queue sends nothing"
    );
}
/// `ServerGamePacketListenerImpl.handleAcceptTeleportPacket`: an accept whose id is
/// the last teleport's (`awaitingTeleport`) while none is awaited any more
/// (`awaitingPositionFromClient` is null) disconnects with
/// `multiplayer.disconnect.invalid_player_movement`; an accept of any other id is
/// ignored, and the first accept of the awaited one moves the player as before.
#[test]
fn a_second_accept_of_the_same_teleport_disconnects() {
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = floor_at(0);
    let accept = |id: i32| {
        S::AcceptTeleportation(play::AcceptTeleportation {
            id,
            x: 5.0,
            y: 70.0,
            z: 5.0,
            yaw: 0.0,
            pitch: 0.0,
        })
    };
    hub.teleport(0, Some([5.0, 70.0, 5.0]));
    let id = hub.guests[0].teleport_counter;
    assert!(hub.guests[0].awaiting_teleport.is_some());
    // An id nobody sent is ignored while one is awaited.
    tick_packet(&mut hub, &mut world, accept(id + 1));
    assert!(hub.guests[0].awaiting_teleport.is_some());
    assert!(hub.guests[0].left_reason.is_empty());
    // The awaited one is accepted once.
    tick_packet(&mut hub, &mut world, accept(id));
    assert!(hub.guests[0].awaiting_teleport.is_none());
    assert!(hub.guests[0].left_reason.is_empty());
    // Another id with nothing awaited is ignored.
    tick_packet(&mut hub, &mut world, accept(id + 1));
    assert!(hub.guests[0].left_reason.is_empty());
    // The same id again, with nothing awaited: disconnected.
    tick_packet(&mut hub, &mut world, accept(id));
    assert!(!hub.guests[0].left_reason.is_empty());
}
/// A guest's `/tp`: `ServerGamePacketListenerImpl.teleport` does not call `ServerChunkCache.move`,
/// so its tickets stay at the chunk it left (also over a second teleport) until the client's
/// ack of the teleport (`handleAcceptTeleportPacket` -> `handlePlayerPositionChange`) moves them.
/// A camera change moves them at once (`ServerPlayer.setCamera`), also back to the player.
#[test]
fn a_teleported_guests_tickets_stay_where_it_left_until_the_teleport_is_acked() {
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = floor_at(0);
    let left = hub.guests[0].chunk();
    hub.teleport(0, Some([104.5, 70.0, 8.5]));
    assert_eq!(hub.guests[0].ticket_chunk(), left);
    hub.teleport(0, Some([200.5, 70.0, 8.5]));
    assert_eq!(
        (hub.guests[0].chunk(), hub.guests[0].ticket_chunk()),
        ((12, 0), left)
    );
    let id = hub.guests[0].teleport_counter;
    tick_packet(
        &mut hub,
        &mut world,
        S::AcceptTeleportation(play::AcceptTeleportation {
            id,
            x: 200.5,
            y: 70.0,
            z: 8.5,
            yaw: 0.0,
            pitch: 0.0,
        }),
    );
    assert_eq!(hub.guests[0].ticket_chunk(), (12, 0));
    // `guest_set_camera`: watching, the teleport to the camera, the tickets released.
    hub.guests.update(0, |g| g.watching = true);
    hub.teleport(0, Some([8.5, 70.0, 8.5]));
    hub.guests.update(0, |g| g.release_tickets());
    hub.guests.update(0, |g| g.watching = false);
    assert_eq!(hub.guests[0].ticket_chunk(), left);
}

/// A client-only entity reaches only its viewer: spawn, move and remove are one packet each to that
/// client, and a move or removal of an id the viewer does not hold is refused. Uuid 1 is Bob,
/// uuid 2 is Alice.
#[test]
fn a_viewer_entity_reaches_that_viewer_only() {
    let (mut hub, bob) = hub_with_a_guest();
    hub.entity_types.insert("minecraft:shulker".into(), 1);
    let alice = add_alice(&mut hub);
    sent(&alice);
    sent(&bob);
    assert!(hub.spawn_viewer_entity(2, 9001, "minecraft:shulker", [1.0, 2.0, 3.0]));
    let got = sent(&alice);
    assert!(
        matches!(&got[0], play::Clientbound::AddEntity(e) if e.id == 9001 && e.position == [1.0, 2.0, 3.0])
    );
    assert_eq!(got.len(), 1);
    assert!(sent(&bob).is_empty(), "the other players are not told");
    assert!(hub.move_viewer_entity(2, 9001, [4.0, 5.0, 6.0]));
    assert!(
        matches!(&sent(&alice)[0], play::Clientbound::TeleportEntity(t) if t.id == 9001 && t.position == [4.0, 5.0, 6.0])
    );
    assert!(hub.remove_viewer_entity(2, 9001));
    assert!(matches!(&sent(&alice)[0], play::Clientbound::RemoveEntities(r) if r.ids == [9001]));
    // Gone for good: a second removal or move is refused, and nothing is sent.
    assert!(!hub.remove_viewer_entity(2, 9001));
    assert!(!hub.move_viewer_entity(2, 9001, [0.0; 3]));
    // Not for a player who is not on, and not for an unknown kind.
    assert!(!hub.spawn_viewer_entity(7, 9002, "minecraft:shulker", [0.0; 3]));
    assert!(!hub.spawn_viewer_entity(2, 9003, "minecraft:no_such_entity", [0.0; 3]));
}

#[test]
fn departing_guest_releases_multiplayer_wire_cache_when_batches_are_empty() {
    let (mut hub, _rx1) = hub_with_a_guest();
    let (tx2, _rx2) = std::sync::mpsc::channel();
    let mut alice = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Alice".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx2, std::sync::Arc::default()),
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [0.5, 64.0, 0.5],
        8,
    );
    alice.placed = true;
    alice.dimension = "minecraft:overworld".into();
    hub.guests.push(alice);
    hub.encoder = ChunkEncoder::new(
        ["minecraft:air", "minecraft:stone"].map(|n| (std::sync::Arc::<str>::from(n), false)),
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    hub.encoder.tables = Some(std::sync::Arc::new(
        crate::save::SaveTables::for_heightmaps(crate::assets::Properties::for_tests(&[
            (true, false, false, false),
            (false, false, true, true),
        ])),
    ));
    let mut world = floor_at(1);
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    for _ in 0..3 {
        hub.ticks += 10;
        for i in 0..hub.guests.len() {
            hub.guests.update(i, |g| g.unacknowledged = 0);
        }
        hub.update_chunks(&ctx);
    }
    let [(_, sections_multiplayer), (heights_multiplayer, _)] = hub.encoder.cache_census();
    assert!(heights_multiplayer > 0);
    // The second guest leaves while the remaining player has no pending batches.
    let _ = hub.remove_guest(1, "bye");
    hub.ticks += 10;
    hub.guests.update(0, |g| g.unacknowledged = 0);
    hub.update_chunks(&ctx);
    let [(_, sections_solo), (heights_solo, _)] = hub.encoder.cache_census();
    assert_eq!(heights_solo, 0);
    assert!(sections_solo < sections_multiplayer);
}
/// A mod's view centre (horizon-rust/horizon#19219): the chunk tracking moves there, the client
/// is told the new cache centre, the chunks around it load as the player's own do, and the
/// player stays where they are. `None` puts the tracking back on the player.
#[test]
fn a_view_centre_moves_the_chunk_tracking_but_not_the_player() {
    let (mut hub, rx) = hub_with_a_guest();
    let uuid = hub.guests[0].uuid;
    let own = hub.guests[0].chunk();
    let position = hub.guests[0].position;
    hub.update_view(0, false);
    let centres = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| -> Vec<(i32, i32)> {
        rx.try_iter()
            .flat_map(|out| match out {
                crate::lan::Out::Packet(body) => vec![body],
                crate::lan::Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .filter_map(|body| {
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                match play::Clientbound::decode(id, &body[reader.position()..]) {
                    Ok(C::SetChunkCacheCenter(p)) => Some((p.x, p.z)),
                    _ => None,
                }
            })
            .collect()
    };
    let _ = centres(&rx);

    assert_eq!(hub.set_view_center(uuid, Some((40, -12))), Ok(()));
    hub.update_view(0, false);
    assert_eq!(hub.guests[0].center, Some((40, -12)));
    assert_eq!(centres(&rx), vec![(40, -12)]);
    assert_eq!(hub.guests[0].position, position);
    assert_eq!(hub.guests[0].ticket_chunk(), own);
    let loading: Vec<_> = hub.loading_chunks(|_| true).collect();
    assert!(loading.contains(&own) && loading.contains(&(40, -12)));
    assert_eq!(hub.view_center(uuid), Some((40, -12)));

    // Refused, changing nothing.
    assert_eq!(
        hub.set_view_center(uuid, Some((1_875_001, 0))),
        Err(ViewCenterRefusal::OutOfRange)
    );
    assert_eq!(
        hub.set_view_center(uuid + 1, Some((0, 0))),
        Err(ViewCenterRefusal::NotOnline)
    );
    assert_eq!(hub.view_center(uuid), Some((40, -12)));

    assert_eq!(hub.set_view_center(uuid, None), Ok(()));
    hub.update_view(0, false);
    assert_eq!(hub.guests[0].center, Some(own));
    assert_eq!(centres(&rx), vec![own]);
}

/// `ServerGamePacketListenerImpl.handleMovePlayer` while a teleport awaits its accept
/// (`updateAwaitingTeleport`): the move's position is ignored but its turn is not,
/// `absSnapRotationTo(Mth.wrapDegrees(yRot), Mth.wrapDegrees(xRot))` (`Entity` clamps the pitch
/// to 90 degrees). Expected values worked out from `Mth.wrapDegrees`, not from the code.
#[test]
fn a_moves_turn_still_counts_while_a_teleport_awaits() {
    let (mut hub, _rx) = hub_with_a_guest();
    hub.guests.update(0, |g| g.load_timeout = 0);
    let mut world = floor_at(0);
    hub.teleport(0, Some([5.0, 70.0, 5.0]));
    assert!(hub.guests[0].awaiting_teleport.is_some());
    let at = hub.guests[0].position;
    let turned = |hub: &Hub| (hub.guests[0].yaw, hub.guests[0].pitch);
    let turn = |yaw, pitch| {
        S::MovePlayerRot(play::MovePlayerRot {
            yaw,
            pitch,
            flags: 0,
        })
    };
    // 450 degrees is 90; a pitch past the top clamps to 90.
    tick_packet(&mut hub, &mut world, turn(450.0, 100.0));
    assert_eq!(turned(&hub), (90.0, 90.0));
    // -270 wraps to 90 as well, and -540 to -180; the pitch keeps what it is in range.
    tick_packet(&mut hub, &mut world, turn(-540.0, -30.0));
    assert_eq!(turned(&hub), (-180.0, -30.0));
    // A move with a position turns the player too and still does not move them.
    hub.guests.update(0, |g| g.position_this_tick = false);
    tick_packet(
        &mut hub,
        &mut world,
        S::MovePlayerPosRot(play::MovePlayerPosRot {
            x: 50.0,
            y: 70.0,
            z: 50.0,
            yaw: 10.0,
            pitch: 20.0,
            flags: 0,
        }),
    );
    assert_eq!(turned(&hub), (10.0, 20.0));
    assert_eq!(hub.guests[0].position, at);
    // A move without a turn keeps the one they have, and the teleport is still awaited.
    hub.guests.update(0, |g| g.position_this_tick = false);
    tick_guest_move(&mut hub, &mut world, [50.0, 70.0, 50.0], 0);
    assert_eq!(turned(&hub), (10.0, 20.0));
    assert!(hub.guests[0].awaiting_teleport.is_some());
    assert!(hub.guests[0].left_reason.is_empty());
}
