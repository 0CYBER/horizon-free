use super::*;

/// A pearl's water test is `Entity.isInWater`: `wasTouchingWater` from the
/// last `updateFluidInteraction`, water whose surface reaches its 0.25
/// box (arismoko/horizon#3072), not the block at its feet.
#[test]
fn a_pearl_feels_the_water_its_box_touches() {
    // The throwable air drag is the float 0.99 widened to a double.
    let air = f64::from(0.99f32);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("entity-types.json").exists()
        || !root.join("movement-properties.nbt").exists()
    {
        return;
    }
    let assets = horizon_client::assets::Assets::load(&root).unwrap();
    let render = assets
        .reload(&horizon_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .unwrap();
    let registry = assets.registry;
    let facts = horizon_client::blocks::BlockFacts::load(&root, registry).unwrap();
    let motion = horizon_client::motion::MotionResources::load(&root, &assets).unwrap();
    let data = horizon_client::mobs::MobData::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &facts,
    );
    let biomes = horizon_client::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
    )
    .unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = horizon_client::items::Items::load(
        &root,
        registry,
        &facts,
        &horizon_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let collision =
        std::sync::Arc::new(horizon_client::collisions::load(&root, registry).unwrap());
    let suffocation =
        horizon_client::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation)
            .unwrap();
    let mut world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
    // A pool of still water one block deep at y = 4, from x = 20 on (clear of the
    // scene's pyramid): its
    // surface is at 4 + 8/9.
    let water = registry.default_state("minecraft:water").unwrap();
    for x in 20..=40 {
        for z in 0..=2 {
            world.set(x, 4, z, water).unwrap();
        }
    }
    let lid_openers = std::collections::HashMap::new();
    let w = MobWorld {
        mob_maps: horizon_client::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),
        goal_host: None,
        live_mobs: None,
        teams: Default::default(),
        professions: horizon_client::mobs::villager::profession_table(),
        loop_context: &crate::game::test_server_load()
            .loop_context(horizon_world::native_load_context::LoopSide::Server, []),
        world: &world,
        registry,
        facts: &facts,
        fluid_tables: &motion.fluids,
        attributes: &motion.attributes,
        movement: &motion.blocks,
        collision: &collision,
        suffocation: &suffocation,
        data: &data,
        player: None,
        player_identity: None,
        player_selection: None,
        controlling_player_movement: None,
        nearest_player: None,
        others: &[],
        dragon: None,
        leash_boats: &[],
        who_lookup: None,
        foreign: None,
        others_grid: None,
        others_index: None,
        passenger_objects: &[],
        others_villagers: None,
        village: None,
        species_slots: None,
        others_solid: None,
        daylight: true,
        monsters_burn: false,
        game_time: 1000,
        day_time: 1000,
        raining: false,
        thundering: false,
        level: horizon_core::LevelKey::OVERWORLD,
        block_tags: &items.block_tags,
        landing: &motion.landing,
        biomes: &biomes,
        mob_griefing: true,
        universal_anger: false,
        mob_drops: true,
        max_entity_cramming: 24,
        difficulty: spawning::LocalDifficulty::new(2, 1000, 0, 0.0),
        dimension_type: horizon_world::dimension_type::DimensionFacts::overworld(),
        world_seed: 0,
        sea_level: 63,
        items: Some(&items),
        lid_openers: &lid_openers,
        mod_effects: None,
        mob_hooks: None,
        asks_animal_tame: false,
        asks_living_destroy_block: false,
        asks_baby_spawn: false,
        border: None,
        section_writer: None,
    };
    let pw = ProjWorld {
        w: &w,
        targets: &[],
        border: None,
    };
    let pearl = |x: f64, y: f64, velocity: Vec3| {
        let mut p = Projectile::new(ProjectileKind::EnderPearl, Vec3::new(x, y, 1.5), None);
        p.velocity = velocity;
        p
    };
    // Skimming the pool: its feet are in the water block, its box above
    // the surface. ThrowableProjectile.applyInertia keeps the air drag.
    let mut p = pearl(25.5, 4.95, Vec3::new(0.3, 0.0, 0.0));
    p.tick(&pw);
    assert!(p.position.y > 4.9, "the pearl dipped: {p:?}");
    p.tick(&pw);
    assert!(
        (p.velocity.x - 0.3 * air * air).abs() < 1e-9,
        "a pearl above the surface slowed as in water: {p:?}"
    );
    // At the pool's edge: its feet over the dry block at x = 19, a side
    // of its box in the water at x = 20. The water drag after the first
    // tick, which still reads `wasTouchingWater` as false.
    let mut p = pearl(19.95, 4.3, Vec3::ZERO);
    p.tick(&pw);
    assert!(
        (p.velocity.y - (-0.03 * air)).abs() < 1e-9,
        "the first tick has the air drag: {p:?}"
    );
    p.tick(&pw);
    assert!(
        (p.velocity.y - (-0.03 * air - 0.03) * 0.8).abs() < 1e-9,
        "a pearl whose box is in the water kept the air drag: {p:?}"
    );
    // Thrown under water, a pearl moves with the air drag on its first
    // tick and the water drag after it.
    let mut p = pearl(30.5, 4.2, Vec3::ZERO);
    p.tick(&pw);
    assert!(
        (p.velocity.y - (-0.03 * air)).abs() < 1e-9,
        "the first tick under water has the air drag: {p:?}"
    );
    p.tick(&pw);
    assert!(
        (p.velocity.y - (-0.03 * air - 0.03) * 0.8).abs() < 1e-9,
        "the second tick under water has the water drag: {p:?}"
    );
}
