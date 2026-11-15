use super::*;

#[test]
#[ignore = "daily"]
fn biome_ambient_particle_is_emitted_by_the_animate_tick() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let collision = crate::collisions::load(&root, assets.registry).unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items =
        crate::items::Items::tags_only(&root, &horizon_world::loot::PackOverlay::NONE, &defaults)
            .unwrap();
    let tables = AnimateTables::load(&root, assets.registry).unwrap();
    let enchant_power = EnchantPower::default();
    enchant_power.rebuild(
        assets.registry.len(),
        |state| items.default_enchant_power(&facts.get(state).name),
        |state| {
            items.block_in_tag(
                "minecraft:enchantment_power_transmitter",
                &facts.get(state).name,
            )
        },
    );
    let mut biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .unwrap();
    biomes.list[0].ambient_particles = vec![(T::Ash, 1.0)];
    let mut world = World::new(facts.air, 0);
    world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
    );
    let mut engine = ParticleEngine::new(
        &horizon_assets::resource_source::ResourceSource::directory(root),
        &Default::default(),
        &Default::default(),
    );
    engine.created_requests = Some(Vec::new());
    engine.camera = [0.5, 64.5, 0.5];
    let ctx = AnimateContext {
        enchant_power: &enchant_power,
        world: &world,
        facts: &facts,
        collision: &collision,
        biomes: &biomes,
        registry: assets.registry,
        raining_at: &|_| false,
        height_no_leaves: &|_| 0,
        firefly_bush_sounds: false,
        creaking_active: false,
        sky_darken: 0,
        marker: None,
        model_offset: &|_, _, _| None,
        gateway_particles: &|_, _| 0,
        thundering: false,
        ultrawarm: false,
    };
    do_animate_tick(&tables, &mut engine, &ctx, [0, 64, 0], 1, true);
    let requests = engine.created_requests.take().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0.ty, T::Ash);
}
