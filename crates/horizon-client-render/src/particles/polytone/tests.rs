use super::*;
use crate::chunk::Chunk;
use crate::world::World;
use glam::DVec3;
use horizon_core::nbt::Tag;
use horizon_world::DATA_VERSION;
use horizon_world::block_collision::BlockCollisionRegistry;
use horizon_world::collision::CachedCollisionRegistry;
use horizon_world::registry::BlockStateId;
use std::collections::HashSet;

const STATES: [&str; 2] = ["minecraft:air", "minecraft:torch"];

fn blocks() -> &'static BlockRegistry {
    let names = || Tag::List(STATES.iter().map(|s| Tag::String((*s).into())).collect());
    Box::leak(Box::new(
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", names()),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:torch", Tag::Int(1)),
                ]),
            ),
        ]))
        .unwrap(),
    ))
}

/// A pack overlay root holding `files` (path, JSON).
fn pack(files: &[(&str, serde_json::Value)]) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "horizon-polytone-particles-{}-{n}",
        std::process::id()
    ));
    for (path, json) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json.to_string()).unwrap();
    }
    root
}

struct Scene {
    world: World,
    facts: crate::blocks::BlockFacts,
    collision: BlockCollisionRegistry<'static>,
}

fn scene(blocks: &'static BlockRegistry) -> Scene {
    let names = || Tag::List(STATES.iter().map(|s| Tag::String((*s).into())).collect());
    let cached = CachedCollisionRegistry::from_nbt(
        &Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", names()),
            ("shapes", Tag::List(vec![])),
            ("cached_shapes", Tag::IntArray(vec![-1, -1])),
        ]),
        blocks,
    )
    .unwrap();
    let mut world = World::new(BlockStateId(0), 0);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    world.set(1, 64, 1, BlockStateId(1)).unwrap();
    Scene {
        world,
        facts: crate::blocks::BlockFacts {
            states: vec![],
            air: BlockStateId(0),
            woods: HashMap::new(),
            sign_texts: HashMap::new(),
        },
        collision: BlockCollisionRegistry::new(Arc::new(cached)).unwrap(),
    }
}

fn engine(registry: Registry) -> ParticleEngine {
    let mut engine = ParticleEngine::new(
        &horizon_assets::resource_source::ResourceSource::directory(Path::new("").to_path_buf()),
        &HashMap::new(),
        &HashSet::new(),
    );
    engine.mod_sets.insert(
        "test:spark".into(),
        ModSet {
            set: 0,
            behaviour: None,
            provider: None,
            group: None,
        },
    );
    engine.set_sprites.push(vec![10, 11, 12]);
    engine.camera = [1.0, 65.0, 1.0];
    engine.set_polytone(Some(Arc::new(registry)));
    engine
}

fn spark_pack() -> std::path::PathBuf {
    pack(&[
        (
            "assets/test/polytone/custom_particles/spark.json",
            serde_json::json!({
                "has_physics": false,
                "initializer": {"lifetime": "10", "size": "0.5", "red": "0.25", "green": "0.5"},
                "ticker": {"dx": "o.custom('heat')", "remove_condition": "o.age >= 4"}
            }),
        ),
        (
            "assets/test/polytone/block_modifiers/torch.json",
            serde_json::json!({
                "targets": ["minecraft:torch"],
                "particle_emitters": [
                    {
                        "particle": "test:spark", "chance": "1", "spawn_location": "lower_corner",
                        "x": "0.5", "y": "1", "z": "0.5", "red": "0.75", "customs": {"Heat": "2"}
                    },
                    {"particle": "minecraft:flame", "chance": "0"}
                ]
            }),
        ),
        (
            // A lower priority modifier of the same block: the torch's above wins.
            "assets/test/polytone/block_modifiers/low.json",
            serde_json::json!({
                "targets": ["minecraft:torch"],
                "priority": -1,
                "particle_emitters": [{"particle": "minecraft:flame", "chance": "1"}]
            }),
        ),
    ])
}

#[test]
fn disable_particles_stops_only_its_blocks_own_particles() {
    let blocks = blocks();
    let root = pack(&[(
        "assets/minecraft/polytone/block_modifiers/torch.json",
        serde_json::json!({"disable_particles": true}),
    )]);
    let engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    let state = |name| blocks.default_state(name).unwrap();
    assert!(engine.polytone_disables_particles(state("minecraft:torch")));
    assert!(!engine.polytone_disables_particles(state("minecraft:air")));
    // Without a Polytone pack nothing is stopped.
    let plain = ParticleEngine::new(
        &horizon_assets::resource_source::ResourceSource::directory(Path::new("").to_path_buf()),
        &HashMap::new(),
        &HashSet::new(),
    );
    assert!(!plain.polytone_disables_particles(state("minecraft:torch")));
}

#[test]
fn a_block_emitter_spawns_a_custom_particle_its_values_seed() {
    let blocks = blocks();
    let root = spark_pack();
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    // Air has no emitters; the torch fires its first emitter only.
    engine.polytone_animate(&level, [1, 64, 2]);
    assert!(engine.pending.is_empty());
    engine.polytone_animate(&level, [1, 64, 1]);
    assert_eq!(engine.pending.len(), 1);
    let p = engine.pending[0].ordinary().unwrap();
    assert!(p.controller.is_some());
    assert_eq!(p.position, [1.5, 65.0, 1.5]);
    // The initializer's values, the emitter's red over its own.
    assert_eq!((p.lifetime, p.size), (10, 0.5));
    assert_eq!(&p.rgba[..2], &[0.75, 0.5]);
    assert!(matches!(p.sprite, Sprite::Index(10)));
    std::fs::remove_dir_all(root).unwrap();
}

/// `light_level` is a floor under the block light a custom particle is drawn with (as the
/// real mod draws a lit `light_level` 8 particle at night, measured beside it): the world's
/// light where it is brighter.
#[test]
fn light_level_is_a_floor_under_the_block_light() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/spark.json",
            serde_json::json!({"has_physics": false, "light_level": 8, "initializer": {"lifetime": "10"}}),
        ),
        (
            "assets/test/polytone/block_modifiers/torch.json",
            serde_json::json!({"targets": ["minecraft:torch"], "particle_emitters": [{"particle": "test:spark", "chance": "1"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    engine.polytone_animate(&level, [1, 64, 1]);
    let PendingParticle::Ordinary(p) = engine.pending.pop().unwrap() else {
        panic!("ordinary particle");
    };
    // Dark: the floor, 8 (light coordinates count 16 a level). The sky light is the world's.
    assert_eq!(p.light_coords(0.0, 240, 0), (240, 128));
    // A brighter block light is kept.
    assert_eq!(p.light_coords(0.0, 0, 208), (0, 208));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_ticker_moves_the_particle_and_removes_it() {
    let blocks = blocks();
    let root = spark_pack();
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    engine.polytone_animate(&level, [1, 64, 1]);
    let PendingParticle::Ordinary(mut p) = engine.pending.pop().unwrap() else {
        panic!("expected ordinary particle")
    };
    // `dx` is the emitter's named custom each tick: two blocks east per tick.
    for tick in 1..=3 {
        engine.tick_particle(&mut p, &level);
        assert_eq!(p.position[0], 1.5 + 2.0 * f64::from(tick));
        assert!(!p.removed);
    }
    // Through the sprite set over its life: age 3 of 10 over 3 sprites.
    assert!(matches!(p.sprite, Sprite::Index(10)));
    engine.tick_particle(&mut p, &level);
    assert!(p.removed, "removed once o.age >= 4");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn entity_emitters_run_on_their_entity_near_the_camera_up_to_the_limit() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/spark.json",
            serde_json::json!({"has_physics": false, "limit": 1}),
        ),
        (
            "assets/test/polytone/custom_particles/dot.json",
            serde_json::json!({"render_type": "invisible"}),
        ),
        (
            // No `targets`: the file name, `minecraft:player`.
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [
                {"particle": "test:spark", "chance": "p.onGround() ? 1 : 0", "x": "1", "y": "2"},
                // A chance that fails to run never fires.
                {"particle": "test:dot", "chance": "o.nope()"}
            ]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let registry = Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    );
    let frame = registry.frame.clone();
    let mut engine = engine(registry);
    let feet = [1.0, 64.0, 1.0];
    // In the air the chance is 0.
    engine.polytone_entity(&level, "minecraft:player", feet);
    assert!(engine.pending.is_empty());
    frame.write().unwrap().player_on_ground = true;
    engine.polytone_entity(&level, "minecraft:zombie", feet);
    // Beyond the default 32 blocks from the camera.
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 40.0]);
    assert!(engine.pending.is_empty());
    engine.polytone_entity(&level, "minecraft:player", feet);
    assert_eq!(engine.pending.len(), 1);
    assert_eq!(
        engine.pending[0].ordinary().unwrap().position,
        [2.0, 66.0, 1.0]
    );
    // `limit` 1: no second one while the first lives.
    engine.polytone_entity(&level, "minecraft:player", feet);
    assert_eq!(engine.pending.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_particle_spawns_within_the_exclusion_radius_of_its_type() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/splash.json",
            serde_json::json!({"has_physics": false, "exclusion_radius": 1, "render_type": "invisible",
                "initializer": {"lifetime": "100"}}),
        ),
        (
            "assets/test/polytone/custom_particles/drop.json",
            serde_json::json!({"has_physics": false, "render_type": "invisible"}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [
                {"particle": "test:splash", "x": "0", "chance": "1"},
                // 0.5 from the first, in the same tick: the first isn't in the world yet,
                // so it spawns (measured on the mod).
                {"particle": "test:splash", "x": "0.5", "chance": "1"},
                // 3 away: spawns.
                {"particle": "test:splash", "x": "3", "chance": "1"},
                // Another type is not excluded by it.
                {"particle": "test:drop", "x": "0.5", "chance": "1"}
            ]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let registry = Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    );
    let mut engine = engine(registry);
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 1.0]);
    let mut xs: Vec<f64> = engine
        .pending
        .iter()
        .filter_map(PendingParticle::ordinary)
        .map(|p| p.position[0])
        .collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, [1.0, 1.5, 1.5, 4.0]);
    // Once they are in the world, the next tick's splashes near them are excluded; ones
    // farther than the radius from all of them spawn, again without excluding each other.
    engine.tick(&level);
    let splashes = |p: &&PendingParticle| {
        p.ordinary()
            .is_some_and(|p| p.polytone_type.as_deref() == Some("test:splash"))
    };
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 1.0]);
    assert_eq!(engine.pending.iter().filter(splashes).count(), 0);
    engine.polytone_entity(&level, "minecraft:player", [9.0, 64.0, 1.0]);
    assert_eq!(engine.pending.iter().filter(splashes).count(), 3);
    std::fs::remove_dir_all(root).unwrap();
}

/// Measured in Java: a custom particle's `movement_aligned` quad has its texture's top
/// against its motion, so the freepack's light rays, rising toward the noon sun, hang
/// below the leaves they start at instead of over them.
#[test]
fn a_rising_movement_aligned_particle_hangs_its_texture_below() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/spark.json",
            serde_json::json!({"has_physics": false, "rotation_mode": "movement_aligned",
                "initializer": {"lifetime": "100", "size": "1"}}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:spark", "chance": "1", "dy": "0.01"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    let rect = crate::hud::Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    engine.rects = vec![Some(rect); 13];
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 1.0]);
    engine.tick(&level);
    let p = engine.particles.last().unwrap();
    // The camera 5 blocks south, at the particle's height.
    let camera = DVec3::new(p.position[0], p.position[1], p.position[2] + 5.0);
    let mut out = Vec::new();
    engine.render(camera, glam::Quat::IDENTITY, 0.0, &|_| 0xF0, &mut out);
    assert_eq!(out.len(), 4);
    for v in &out {
        // The texture's top row (v 0) under the centre, its bottom row over it.
        if v.uv[1] == 0.0 {
            assert!(v.pos[1] < 0.0, "top row at {:?}", v.pos);
        } else {
            assert!(v.pos[1] > 0.0, "bottom row at {:?}", v.pos);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// An `additive_translucent` custom particle takes the fog in Java, as vanilla's particle
/// shader gives it: its quads say so to the additive pass.
#[test]
fn an_additive_custom_particle_takes_the_particle_shader_overlay() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/spark.json",
            serde_json::json!({"has_physics": false, "render_type": "additive_translucent",
                "initializer": {"lifetime": "100", "size": "1"}}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:spark", "chance": "1"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    let rect = crate::hud::Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    engine.rects = vec![Some(rect); 13];
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 1.0]);
    engine.tick(&level);
    let mut out = Vec::new();
    engine.render(
        DVec3::new(1.0, 64.0, 6.0),
        glam::Quat::IDENTITY,
        0.0,
        &|_| 0xF0,
        &mut out,
    );
    assert!(out.is_empty());
    let additive = engine.take_additive();
    assert_eq!(additive.len(), 4);
    assert!(
        additive
            .iter()
            .all(|v| v.layer & 0xffff == crate::gfx::ADDITIVE_PARTICLE_OVERLAY)
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A particle's emitter spawns while the live particles tick: its children's
/// `exclusion_radius` sees the ones alive, so a ray emitted every tick at one spot stays one.
#[test]
fn a_particles_emitter_respects_the_exclusion_radius_of_its_children() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/holder.json",
            serde_json::json!({"has_physics": false, "render_type": "invisible",
                "initializer": {"lifetime": "100"},
                "particle_emitters": [{"particle": "test:ray", "chance": "1"}]}),
        ),
        (
            "assets/test/polytone/custom_particles/ray.json",
            serde_json::json!({"has_physics": false, "render_type": "invisible",
                "exclusion_radius": 1, "initializer": {"lifetime": "100"}}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:holder", "chance": "1"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    engine.polytone_entity(&level, "minecraft:player", [1.0, 70.0, 1.0]);
    for _ in 0..10 {
        engine.tick(&level);
    }
    let rays = engine
        .particles
        .iter()
        .chain(engine.pending.iter().filter_map(PendingParticle::ordinary))
        .filter(|p| p.polytone_type.as_deref() == Some("test:ray"))
        .count();
    assert_eq!(rays, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_gpu_particle_is_a_record_drawn_until_its_life_ends() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/rain.json",
            serde_json::json!({
                "type": "gpu", "texture": "test:textures/particle/rain.png",
                "render_type": "translucent", "rotation_mode": "movement_aligned",
                "initializer": {"lifetime": "3", "size": "0.5"},
                "gravity": 0.1, "area": {"size": [4, 4, 4], "count": 3}
            }),
        ),
        (
            // Additive: drawn in the frame's additive pass instead.
            "assets/test/polytone/custom_particles/glow.json",
            serde_json::json!({
                "type": "gpu", "texture": "test:textures/particle/rain.png",
                "render_type": "additive_translucent", "initializer": {"lifetime": "3"}
            }),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:rain"}, {"particle": "test:glow"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    let rect = crate::hud::Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    engine
        .atlas
        .insert("test:particle/rain".into(), (rect, true));
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 1.0]);
    // No engine particle: a record each; the rain drawn as its area's three quads.
    assert!(engine.pending.is_empty());
    assert_eq!(engine.polytone_gpu_records(), 2);
    let mut out = Vec::new();
    engine.render(
        DVec3::new(1.0, 70.0, 1.0),
        glam::Quat::IDENTITY,
        0.0,
        &|_| 0xF0,
        &mut out,
    );
    assert_eq!(out.len(), 3 * 4);
    assert_eq!(engine.take_additive().len(), 4);
    assert!(
        out.iter()
            .all(|v| v.layer == crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY)
    );
    for _ in 0..3 {
        engine.tick(&level);
    }
    assert_eq!(
        engine.polytone_gpu_records(),
        0,
        "gone once its 3 ticks are over"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_colormap_tints_the_particle_and_sound_emitters_play() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/leaf.json",
            serde_json::json!({
                "has_physics": false, "colormap": "minecraft:foliage_color",
                "initializer": {"red": "1", "green": "0.5", "blue": "1"},
                "sound_emitters": [{"sound": "block.note_block.harp", "chance": "1", "volume": "0.5"}]
            }),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:leaf"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let registry = Arc::new(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    // The chunk's biome is id 0: plains, whose foliage is #336699.
    registry.set_biomes(&Arc::new(crate::biome::Biomes::from_jsons(&[(
        "minecraft:plains",
        serde_json::json!({"effects": {"foliage_color": 0x336699}}),
    )])));
    let mut engine = ParticleEngine::new(
        &horizon_assets::resource_source::ResourceSource::directory(Path::new("").to_path_buf()),
        &HashMap::new(),
        &HashSet::new(),
    );
    engine.mod_sets.insert(
        "test:leaf".into(),
        ModSet {
            set: 0,
            behaviour: None,
            provider: None,
            group: None,
        },
    );
    engine.set_sprites.push(vec![10]);
    engine.camera = [1.0, 65.0, 1.0];
    engine.set_polytone(Some(registry.clone()));
    engine.polytone_entity(&level, "minecraft:player", [1.0, 64.0, 1.0]);
    let PendingParticle::Ordinary(mut p) = engine.pending.pop().unwrap() else {
        panic!("expected ordinary particle")
    };
    let tinted = [
        0x33 as f32 / 255.0,
        0.5 * 0x66 as f32 / 255.0,
        0x99 as f32 / 255.0,
    ];
    for _ in 0..2 {
        engine.tick_particle(&mut p, &level);
        // Multiplied into its own colour, every tick, without compounding.
        assert_eq!(&p.rgba[..3], &tinted);
    }
    let sounds = registry.take_sounds();
    assert_eq!(sounds.len(), 2);
    assert_eq!(
        (sounds[0].0, sounds[0].2, sounds[0].3),
        ("minecraft:block.note_block.harp", 0.5, 1.0)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_colours_recolour_their_particle_type() {
    let root = pack(&[(
        "assets/obs/polytone/colors.json",
        serde_json::json!({"particle": {"minecraft:flame": "0x0000ff"}}),
    )]);
    let registry = Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks(),
    );
    let colour = |kind: &str| {
        let mut p = Particle::blank(ParticleType::Flame, [0.0; 3]);
        modify(&registry, kind, None, &mut p);
        p.rgba
    };
    assert_eq!(colour("minecraft:flame"), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(colour("minecraft:smoke"), [1.0; 4]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn particle_modifiers_scale_power_and_recolour_their_particles_once() {
    let root = pack(&[
        (
            "assets/minecraft/polytone/particle_modifiers/flame.json",
            serde_json::json!({"size": "2", "life": "LIFETIME + 5", "red": "0.5", "speed": "2"}),
        ),
        (
            "assets/pack/polytone/particle_modifiers/smoky.json",
            serde_json::json!({"targets": ["minecraft:smoke"], "color": "0x336699"}),
        ),
    ]);
    let registry = Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks(),
    );
    let flame = || {
        let mut p = Particle::blank(ParticleType::Flame, [0.0; 3]);
        p.size = 0.1;
        p.lifetime = 10;
        p.velocity = [0.25, f64::from(0.1f32), -0.5];
        p
    };
    let mut p = flame();
    modify(&registry, "minecraft:flame", None, &mut p);
    // `size` multiplies, `speed` is vanilla's power: x and z doubled, a y of 0.1 kept.
    assert_eq!((p.size, p.lifetime, p.rgba[0]), (0.2, 15, 0.5));
    assert_eq!(p.velocity, [0.5, f64::from(0.1f32), -1.0]);
    // Another type's modifier, by its targets: the packed colour.
    let mut p = flame();
    modify(&registry, "minecraft:smoke", None, &mut p);
    assert_eq!(
        &p.rgba[..3],
        &[
            0x33 as f32 / 255.0,
            0x66 as f32 / 255.0,
            0x99 as f32 / 255.0
        ]
    );
    assert_eq!((p.size, p.lifetime), (0.1, 10));
    // No modifier aimed at it: untouched.
    let mut p = flame();
    modify(&registry, "minecraft:soul_fire_flame", None, &mut p);
    assert_eq!(
        (p.size, p.lifetime, p.velocity),
        (0.1, 10, [0.25, f64::from(0.1f32), -0.5])
    );
}

/// The freepack's light rays come from an emitter with physics that rests beside a leaf and
/// emits every tick of its life: resting is not being still, only a stopped fall is.
#[test]
fn a_resting_particle_with_physics_lives_out_its_life() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/rest.json",
            serde_json::json!({"has_physics": true, "render_type": "invisible",
                "initializer": {"lifetime": "20"}}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:rest", "chance": "1"}]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    engine.polytone_entity(&level, "minecraft:player", [1.5, 70.0, 1.5]);
    let PendingParticle::Ordinary(mut p) = engine.pending.pop().unwrap() else {
        panic!("ordinary particle");
    };
    for _ in 0..10 {
        engine.tick_particle(&mut p, &level);
        assert!(!p.removed);
    }
    // Stopped by a collision, it goes.
    p.stopped_by_collision = true;
    engine.tick_particle(&mut p, &level);
    assert!(p.removed);
    std::fs::remove_dir_all(root).unwrap();
}

/// `scene`, with the torch a full block of collision: something a particle's box can start in.
fn solid_scene(blocks: &'static BlockRegistry) -> Scene {
    use horizon_core::geometry::{Axis, VoxelShape};
    let tag = |shape: VoxelShape| {
        let sizes = shape.sizes();
        let occupied = (0..sizes[2])
            .flat_map(|z| (0..sizes[1]).flat_map(move |y| (0..sizes[0]).map(move |x| (x, y, z))))
            .map(|(x, y, z)| u8::from(shape.is_full(x as i32, y as i32, z as i32)))
            .collect();
        let (min, max) = shape.discrete_bounds();
        let axes = shape
            .cube_coordinate_axes()
            .iter()
            .enumerate()
            .fold(0, |mask, (i, on)| mask | (i8::from(*on) << i));
        let mut fields = vec![
            ("occupied", Tag::ByteArray(occupied)),
            ("cube", Tag::Byte(i8::from(shape.is_cube()))),
            ("cube_axes", Tag::Byte(axes)),
            ("discrete_min", Tag::IntArray(min.to_vec())),
            ("discrete_max", Tag::IntArray(max.to_vec())),
        ];
        for (axis, name) in [(Axis::X, "x"), (Axis::Y, "y"), (Axis::Z, "z")] {
            let bits = shape.coordinates(axis).iter().map(|v| v.to_bits() as i64);
            fields.push((name, Tag::LongArray(bits.collect())));
        }
        Tag::compound(fields)
    };
    let names = || Tag::List(STATES.iter().map(|s| Tag::String((*s).into())).collect());
    let cached = CachedCollisionRegistry::from_nbt(
        &Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", names()),
            (
                "shapes",
                Tag::List(vec![tag(VoxelShape::empty()), tag(VoxelShape::block())]),
            ),
            ("cached_shapes", Tag::IntArray(vec![0, 1])),
        ]),
        blocks,
    )
    .unwrap();
    Scene {
        collision: BlockCollisionRegistry::new(Arc::new(cached)).unwrap(),
        ..scene(blocks)
    }
}

#[test]
fn a_particle_with_physics_born_inside_a_block_never_joins() {
    let blocks = blocks();
    let leaf = |physics| {
        serde_json::json!({"has_physics": physics, "render_type": "invisible",
            "initializer": {"lifetime": "20", "hitbox_size": "0.08"},
            "particle_emitters": [{"particle": "test:mark", "chance": "1"}]})
    };
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/leaf.json",
            leaf(true),
        ),
        (
            "assets/test/polytone/custom_particles/ghost.json",
            leaf(false),
        ),
        (
            "assets/test/polytone/custom_particles/mark.json",
            serde_json::json!({"render_type": "invisible", "has_physics": false}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [{"particle": "test:leaf", "chance": "1"}]}),
        ),
        (
            "assets/test/polytone/custom_particles/wide.json",
            serde_json::json!({"has_physics": true, "render_type": "invisible",
                "initializer": {"lifetime": "20", "hitbox_size": 10},
                "particle_emitters": [{"particle": "test:mark", "chance": "1"}]}),
        ),
        (
            "assets/test/polytone/custom_particles/parent.json",
            serde_json::json!({"has_physics": true, "render_type": "invisible",
                "initializer": {"lifetime": "20", "hitbox_size": "0.08"},
                "particle_emitters": [{"particle": "test:wide", "chance": "1", "y": "-0.05"}]}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/skeleton.json",
            serde_json::json!({"emitters": [{"particle": "test:parent", "chance": "1", "x": "-4.5"}]}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/creeper.json",
            serde_json::json!({"emitters": [{"particle": "test:wide", "chance": "1", "x": "-4.5"}]}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/zombie.json",
            serde_json::json!({"emitters": [{"particle": "test:ghost", "chance": "1"}]}),
        ),
    ]);
    let scene = solid_scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    // The solid block is at (1, 64, 1): its bottom face is y 64.
    // Spawns one particle, lets the engine take it in and tick it: whether it joined, and
    // how many particles it emitted.
    let mut spawn = |entity: &str, y: f64| {
        engine.particles.clear();
        engine.pending.clear();
        engine.polytone_entity(&level, entity, [1.5, y, 1.5]);
        engine.tick(&level);
        let joined = engine.particles.len();
        engine.tick(&level);
        (
            joined,
            engine.particles.len() + engine.pending.len() - joined,
        )
    };
    // The solid block is at (1, 64, 1): its bottom face is y 64.
    // 0.05 under the block, a 0.08 box reaches into it: it never joins, nothing emitted.
    assert_eq!(spawn("minecraft:player", 63.95), (0, 0));
    // 0.01 clear of it, it joins and emits.
    assert_eq!(spawn("minecraft:player", 63.91), (1, 1));
    // Without physics nothing collides.
    assert_eq!(spawn("minecraft:zombie", 63.95), (1, 1));
    // A wide box reaching a block from five blocks away does not join either.
    assert_eq!(spawn("minecraft:creeper", 60.0), (0, 0));
    // Nor does one a clear particle emits there: the small one joins, its wide copy never.
    assert_eq!(spawn("minecraft:skeleton", 60.0), (1, 0));
    std::fs::remove_dir_all(root).unwrap();
}

/// Measured on the mod: with `limit` 1, one particle born inside a block and one clear of it
/// in the same tick, the clear one shows. The one that never joins takes no place.
#[test]
fn a_particle_that_never_joins_takes_no_place_under_its_limit() {
    let blocks = blocks();
    let lim = |x: &str| serde_json::json!({"particle": "test:lim", "chance": "1", "x": x});
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/lim.json",
            serde_json::json!({"has_physics": true, "render_type": "invisible", "limit": 1,
                "initializer": {"lifetime": "20", "hitbox_size": "0.08"}}),
        ),
        // The first inside the block's reach, the second three blocks clear of it.
        (
            "assets/minecraft/polytone/entity_modifiers/player.json",
            serde_json::json!({"emitters": [lim("0"), lim("3")]}),
        ),
        // Two clear of it: the limit still keeps one.
        (
            "assets/minecraft/polytone/entity_modifiers/zombie.json",
            serde_json::json!({"emitters": [lim("3"), lim("6")]}),
        ),
        // `exclusion_radius` with physics: the one inside the block doesn't keep the clear
        // one a block away from spawning.
        (
            "assets/test/polytone/custom_particles/exc.json",
            serde_json::json!({"has_physics": true, "render_type": "invisible",
                "exclusion_radius": 2,
                "initializer": {"lifetime": "20", "hitbox_size": "0.08"}}),
        ),
        (
            "assets/minecraft/polytone/entity_modifiers/skeleton.json",
            serde_json::json!({"emitters": [
                {"particle": "test:exc", "chance": "1", "x": "0"},
                {"particle": "test:exc", "chance": "1", "x": "1"}]}),
        ),
    ]);
    let scene = solid_scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    let mut joined = |entity: &str| {
        engine.particles.clear();
        engine.pending.clear();
        // The solid block is at (1, 64, 1): 0.05 under it, a 0.08 box reaches into it.
        engine.polytone_entity(&level, entity, [1.5, 63.95, 1.5]);
        engine.tick(&level);
        engine
            .particles
            .iter()
            .map(|p| p.position[0])
            .collect::<Vec<_>>()
    };
    assert_eq!(joined("minecraft:player"), vec![4.5]);
    assert_eq!(joined("minecraft:zombie"), vec![4.5]);
    assert_eq!(joined("minecraft:skeleton"), vec![2.5]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_pack_custom_particle_replaces_the_vanilla_type_of_its_id() {
    let root = pack(&[(
        "assets/minecraft/polytone/custom_particles/flame.json",
        serde_json::json!({"has_physics": false, "initializer": {"lifetime": "33", "size": "0.75"}}),
    )]);
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks(),
    ));
    // The vanilla flame's sprite set (what the pack's particles/flame.json lists).
    engine.sets.insert(ParticleType::Flame, 1);
    engine.set_sprites.push(vec![20, 21]);
    engine.create(
        &ParticleOptions::simple(ParticleType::Flame),
        [1.0, 65.0, 1.0],
        [0.0; 3],
    );
    assert_eq!(engine.pending.len(), 1);
    let p = engine.pending[0].ordinary().unwrap();
    assert!(
        p.controller.is_some(),
        "the pack's particle, not vanilla's flame"
    );
    assert_eq!((p.lifetime, p.size), (33, 0.75));
    assert!(matches!(p.sprite, Sprite::Index(20)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn block_and_biome_tags_pick_where_an_emitter_runs() {
    let blocks = blocks();
    let root = pack(&[
        (
            "assets/test/polytone/custom_particles/spark.json",
            serde_json::json!({"has_physics": false}),
        ),
        (
            "assets/test/polytone/block_modifiers/shiny.json",
            serde_json::json!({
                "targets": ["#test:shiny"],
                "particle_emitters": [{
                    "particle": "test:spark", "chance": "1", "biomes": "#test:warm",
                    "spawn_location": "lower_corner", "y": "1"
                }]
            }),
        ),
        (
            "data/test/tags/block/shiny.json",
            serde_json::json!({"values": ["minecraft:torch"]}),
        ),
        (
            "data/test/tags/worldgen/biome/warm.json",
            serde_json::json!({"values": ["#test:hot", "minecraft:plains"]}),
        ),
        (
            "data/test/tags/worldgen/biome/hot.json",
            serde_json::json!({"values": ["minecraft:desert"]}),
        ),
    ]);
    let scene = scene(blocks);
    let level = ParticleLevel {
        world: &scene.world,
        facts: &scene.facts,
        collision: &scene.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    let mut engine = engine(Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks,
    ));
    // The test chunk's biome is the table's first: a cold one first, then a warm one.
    let first = |name: &str| {
        Arc::new(crate::biome::Biomes::from_jsons(&[(
            name,
            serde_json::json!({}),
        )]))
    };
    let registry = engine.polytone.clone().unwrap();
    registry.set_biomes(&first("minecraft:snowy_plains"));
    engine.polytone_animate(&level, [1, 64, 1]);
    assert!(engine.pending.is_empty(), "outside #test:warm");
    // `#test:warm` holds the desert through the nested `#test:hot`.
    registry.set_biomes(&first("minecraft:desert"));
    engine.polytone_animate(&level, [1, 64, 1]);
    assert_eq!(engine.pending.len(), 1);
    assert_eq!(
        engine.pending[0].ordinary().unwrap().position,
        [1.0, 65.0, 1.0]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_filtered_modifier_changes_only_the_blocks_and_items_it_names() {
    let root = pack(&[
        (
            "assets/minecraft/polytone/particle_modifiers/block.json",
            serde_json::json!({"size": "3", "filter": {"block": ["stone", "minecraft:granite"]}}),
        ),
        (
            "assets/minecraft/polytone/particle_modifiers/item.json",
            serde_json::json!({"life": "50", "filter": {"item": "minecraft:apple"}}),
        ),
    ]);
    let registry = Registry::read(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        Default::default(),
        blocks(),
    );
    let run = |id: &str, source: Option<&str>| {
        let mut p = Particle::blank(ParticleType::Block, [0.0; 3]);
        p.size = 0.1;
        p.lifetime = 10;
        modify(&registry, id, source, &mut p);
        (p.size, p.lifetime)
    };
    assert_eq!(run("minecraft:block", Some("minecraft:stone")), (0.3, 10));
    assert_eq!(run("minecraft:block", Some("minecraft:granite")), (0.3, 10));
    assert_eq!(run("minecraft:block", Some("minecraft:dirt")), (0.1, 10));
    // A particle that carries no block (the filter can't pass).
    assert_eq!(run("minecraft:block", None), (0.1, 10));
    assert_eq!(run("minecraft:item", Some("minecraft:apple")), (0.1, 50));
    assert_eq!(run("minecraft:item", Some("minecraft:bread")), (0.1, 10));
    std::fs::remove_dir_all(root).unwrap();
}
