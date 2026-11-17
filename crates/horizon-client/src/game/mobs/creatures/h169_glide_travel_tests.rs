use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
use std::sync::Arc;
fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    with_role_game(
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        test,
    );
}
fn with_role_game(
    role: crate::game::execution_role::ExecutionRole,
    test: impl FnOnce(&mut Game<'_>),
) {
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
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("h169-{:?}", std::thread::current().id())),
    );
    options.execution_role = role;
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
    game.set_allow_commands(true);

    let mut center = Chunk::empty(0, 0);
    center.lit = true;
    game.world.chunks.insert((0, 0), Arc::new(center));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("difficulty normal"));
    assert!(game.run_command("forceload add 0 0"));
    for x in -1..=1 {
        for z in -1..=1 {
            if x == 0 && z == 0 {
                continue;
            }
            let mut chunk = Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
    }
    game.load_ticking_neighbourhood(0, 0);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
    game.world.random.set_seed(169);
    game.update_chunk_tickets();
    assert!(crate::game::ticking_chunk(&game.world, 0, 0));
    assert!(game.tickets.levels().block_ticking(0, 0));
    test(&mut game);
}

#[test]
#[ignore = "daily"]
fn h169_registered_eligible_husk_actually_travels_as_a_glider() {
    with_game(|game| {
        game.world.game_time = 0;
        for (n, no_ai, glide, x) in [(1, 0, 1, 4.5), (2, 1, 1, 8.5), (3, 0, 0, 12.5)] {
            assert!(game.run_command(&format!(
                "summon husk {x} 300 8.5 {{UUID:[I;169,0,0,{n}],NoAI:{no_ai}b,PersistenceRequired:1b,FallFlying:{glide}b,Rotation:[0f,0f],Motion:[0d,0d,0d],equipment:{{chest:{{id:\"minecraft:elytra\",count:1}}}}}}"
            )));
        }
        let indexes: Vec<_> = (1..=3)
            .map(|n| {
                let id = game
                    .persist
                    .uuids
                    .iter()
                    .find_map(|(id, u)| (*u == [169, 0, 0, n]).then_some(*id))
                    .unwrap();
                game.mobs.iter().position(|m| m.id == id).unwrap()
            })
            .collect();
        let main = indexes[0];
        let id = game.mobs[main].id;
        assert!(game.mobs[main].extra.fall_flying);
        assert!(!game.mobs[main].no_ai && !game.mobs[main].on_ground);
        let stack = horizon_client::mobs::finalize::equipment_stack(
            &game.mobs[main],
            horizon_core::equipment_slot::EquipmentSlot::Chest,
        )
        .unwrap();
        assert!(horizon_client::item_sync::can_glide_using(
            &stack,
            "chest",
            game.items()
        ));
        game.world.game_time += 1;
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(4, 8));
        game.tick_creature_entities().unwrap();
        assert_eq!(game.mobs[main].id, id);
        assert_eq!(game.uuid_for(id), [169, 0, 0, 1]);
        assert!(
            game.mobs[main].extra.fall_flying,
            "real modeled flag remains eligible after the tick"
        );
        assert_eq!(
            game.mobs[indexes[1]].position,
            Vec3::new(8.5, 300.0, 8.5),
            "NoAI is a stationary control"
        );
        assert!(!game.mobs[indexes[2]].extra.fall_flying);
        assert_eq!(game.world.game_time, 1);
        // Independent fresh Java first-tick values, not native helper output.
        assert_eq!(
            game.mobs[main].position,
            Vec3::new(4.5, 299.9823599996567, 8.501782000017165)
        );
        assert_eq!(
            game.mobs[main].velocity,
            Vec3::new(0.0, -0.017640000343322755, 0.0017820000171661376)
        );
    });
}

// Added source/unit control, not part of the frozen dry-air Java runtime pair.
#[test]
#[ignore = "daily"]
fn h169_added_actual_wall_collision_uses_mob_damage_source() {
    with_game(|game| {
        assert!(game.run_command("fill 6 298 7 6 303 10 stone"));
        assert!(game.run_command(r#"summon husk 4.5 300 8.5 {UUID:[I;169,0,0,4],PersistenceRequired:1b,FallFlying:1b,Rotation:[-90f,0f],Motion:[2d,0d,0d],equipment:{chest:{id:"minecraft:elytra",count:1}}}"#));
        let id = game
            .persist
            .uuids
            .iter()
            .find_map(|(id, u)| (*u == [169, 0, 0, 4]).then_some(*id))
            .unwrap();
        let index = game.mobs.iter().position(|m| m.id == id).unwrap();
        assert!(
            game.mobs[index].kind.armor > 0.0,
            "real Husk armor is the bypass control"
        );
        assert_eq!(game.mobs[index].health, 20.0);
        game.world.game_time += 1;
        game.update_chunk_tickets();
        game.tick_creature_entities().unwrap();
        let m = &game.mobs[index];
        assert!(m.horizontal_collision, "actual wall collision");
        assert!(m.position.x > 4.5 && m.position.x < 6.0);
        assert_eq!(m.velocity.x, 0.0, "resolved collision, no damage knockback");
        assert_eq!(m.health, 3.0, "(2 - 0) * 10 - 3, bypassing the real armor");
        let damage = m.last_damage.unwrap().0;
        assert_eq!(damage.kind, horizon_client::mobs::DamageKind::FlyIntoWall);
        assert!(damage.bypasses_armor() && damage.no_knockback());
        assert!(!damage.is_fire() && !damage.is_projectile());
        assert_eq!(source_damage_type(damage), "minecraft:fly_into_wall");
        assert_eq!(game.uuid_for(id), [169, 0, 0, 4]);
    });
}

#[test]
fn hostile_mobs_in_non_entity_ticking_chunks_do_not_despawn() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    // The unopened fixture starts in Peaceful, which refuses /summon zombie.
    assert!(game.run_command("difficulty normal"));
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5))
            .unwrap(),
    );
    game.settings.simulation_distance = 4;
    game.settings.render_distance = 24;
    let cx = 20;
    let pos_x = (cx * 16 + 8) as f64;
    game.world.chunks.insert(
        (cx, 0),
        Arc::new(horizon_client::chunk::Chunk::empty(cx, 0)),
    );
    assert!(game.run_command(&format!(
        "summon zombie {pos_x} 65.0 8.5 {{UUID:[I;16553,0,0,1],PersistenceRequired:0b}}"
    )));
    let mob_id = game
        .mobs
        .iter()
        .find(|m| game.persist.uuid_of(m.id) == Some([16553, 0, 0, 1]))
        .map(|m| m.id)
        .expect("zombie spawned");

    // The unopened server has no ticket levels yet. Establish the player's
    // real loading/simulation boundary before ticking the distant zombie.
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(0, 0));
    assert!(game.world.loaded(pos_x as i32, 8));
    assert!(!game.entity_ticking_area()(pos_x as i32, 8));

    game.tick_creature_entities().unwrap();

    assert!(
        game.mobs.iter().any(|m| m.id == mob_id),
        "hostile mob in non-entity-ticking chunk must not despawn (#16553)"
    );
}
