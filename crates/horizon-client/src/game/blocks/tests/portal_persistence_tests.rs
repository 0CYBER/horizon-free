//! Direct production handlers over an empty in-memory world; no server or renderer.
use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use std::sync::Arc;

#[test]
#[ignore = "needs explicit HORIZON_TEST_RESOURCES and HORIZON_PORTAL_REFERENCE"]
fn live_and_saved_portal_counters_follow_runtime_and_data_updates() {
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var_os("HORIZON_PORTAL_REFERENCE").unwrap()).unwrap(),
    )
    .unwrap();
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.headless = true;
    options.game_dir = Some(std::env::current_dir().unwrap());
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.options.dedicated = Some(Vec::new());
    game.world = horizon_client::world::World::new(game.res.facts().air, 1);
    for z in -2..=2 {
        for x in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    let kinds = [
        "minecraft:oak_boat",
        "minecraft:minecart",
        "minecraft:arrow",
        "minecraft:pig",
    ];
    let mut ids = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        let uuid = [11318, 10, 0, index as i32];
        let tag = Tag::compound([
            ("id", Tag::String((*kind).into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            ("Pos", doubles([4.5, 210.0, 4.5])),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
            ("PortalCooldown", Tag::Int(5)),
        ]);
        game.load_listed(kind, Some(uuid), None, tag).unwrap();
        ids.push(game.next_entity);
    }
    assert_eq!(game.boats.len(), 1);
    assert_eq!(game.carts.len(), 1);
    assert_eq!(game.creatures.projectiles.len(), 1);
    assert_eq!(game.mobs.len(), 1);
    game.tickets.update(Some(crate::ticket_levels::Sources {
        forced: vec![(0, 0)],
        ..Default::default()
    }));
    assert!(game.entity_ticking_area()(4, 4));
    game.handle_vehicle_portals();
    game.handle_entity_portals();
    // Projectiles run their portal pass after moving (`portal_after_move`).
    game.handle_portals(false, true);
    for id in &ids {
        assert_eq!(game.portals.entities.get(id).unwrap().cooldown, 4);
    }
    for id in &ids[..OWNED] {
        let target = game.level_entity(*id as i32).unwrap();
        assert_eq!(counter(&game.command_target_nbt(target).unwrap()), 4);
    }
    let mut nbt = Vec::new();
    let mut saved = Vec::new();
    for row in reference["contact"].as_array().unwrap() {
        for (&id, value) in ids.iter().zip(row.as_array().unwrap()) {
            game.restore_portal_cooldown(
                id,
                &Tag::compound([("PortalCooldown", Tag::Int(value.as_i64().unwrap() as i32))]),
            );
        }
        nbt.push(
            ids[..OWNED]
                .iter()
                .map(|id| {
                    let target = game.level_entity(*id as i32).unwrap();
                    counter(&game.command_target_nbt(target).unwrap())
                })
                .collect::<Vec<_>>(),
        );
        let chunks = game.entity_chunks(&std::collections::HashSet::from([(0, 0)]), false);
        saved.push(
            kinds[..OWNED]
                .iter()
                .map(|kind| {
                    counter(
                        chunks
                            .iter()
                            .flat_map(|(_, tags)| tags)
                            .find(|tag| {
                                horizon_client::save::text(tag.get("id")).as_deref() == Some(*kind)
                            })
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    let id = ids[0];
    let mut gateway = horizon_client::end_gateway::Traveller::default();
    gateway.set_cooldown(7);
    game.end.gateways.insert(id, gateway);
    let target = game.level_entity(id as i32).unwrap();
    assert_eq!(counter(&game.command_target_nbt(target).unwrap()), 7);
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(3))]));
    assert_eq!(game.end.gateways[&id].cooldown(), 3);
    game.end.gateways.remove(&id);
    // /data merge reads the new zero counter into the live entity, not just its raw tag.
    for id in ids[..OWNED].iter().copied() {
        let target = game.level_entity(id as i32).unwrap();
        let mut tag = game.command_target_nbt(target).unwrap();
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(2));
        }
        game.apply_entity_nbt(target, tag).unwrap();
        assert_eq!(game.portals.entities.get(&id).unwrap().cooldown, 2);
        game.portals.entities.get_mut(&id).unwrap().cooldown = 5;
        let target = game.level_entity(id as i32).unwrap();
        let mut tag = game.command_target_nbt(target).unwrap();
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(0));
        }
        game.apply_entity_nbt(target, tag).unwrap();
        assert!(!game.portals.entities.contains_key(&id));
    }
    let stand = Tag::compound([
        ("id", Tag::String("minecraft:armor_stand".into())),
        ("UUID", Tag::IntArray(vec![11318, 10, 1, 0])),
        ("Pos", doubles([4.5, 210.0, 4.5])),
        ("PortalCooldown", Tag::Int(20)),
    ]);
    game.load_listed(
        "minecraft:armor_stand",
        Some([11318, 10, 1, 0]),
        None,
        stand,
    )
    .unwrap();
    let id = game.next_entity;
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(20))]));
    game.portals.entities.get_mut(&id).unwrap().cooldown = 19;
    let target = game.level_entity(id as i32).unwrap();
    // An unrelated merge reads the live counter, not the loaded 20, and writes it back.
    let tag = game.command_target_nbt(target).unwrap();
    assert_eq!(counter(&tag), 19);
    game.apply_entity_nbt(target, tag).unwrap();
    assert_eq!(game.portals.entities[&id].cooldown, 19);
    game.portals.entities.remove(&id);
    let id = game.next_entity;
    let state = game.portals.entities.entry(id).or_default();
    state.set_inside(BlockPos::new(4, 210, 4), 300);
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(2))]));
    assert!(game.portals.entities[&id].process.is_some());
    assert_eq!(game.portals.entities[&id].cooldown, 2);
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(0))]));
    assert!(game.portals.entities[&id].process.is_some());
    assert_eq!(game.portals.entities[&id].cooldown, 0);
    let report = serde_json::json!({"nbt":nbt,"saved":saved});
    let owned: Vec<Vec<i64>> = reference["contact"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array().unwrap()[..OWNED]
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect()
        })
        .collect();
    for field in ["nbt", "saved"] {
        assert_eq!(report[field], serde_json::json!(owned), "{field}");
    }
    if let Some(path) = std::env::var_os("HORIZON_PORTAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
/// The first kinds in `kinds` (boat, minecart) are the ones whose data reads
/// and chunk saves take the live counter here; the others follow elsewhere.
const OWNED: usize = 2;
fn counter(tag: &Tag) -> i32 {
    horizon_client::save::number(tag.get("PortalCooldown")).unwrap() as i32
}
