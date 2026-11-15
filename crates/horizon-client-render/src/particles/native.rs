//! Opt-in, compiled particle behavior. The ordinary engine still owns the
//! particle, its sprite set, interpolation, rendering and total count limits.
use super::{Particle, ParticleLevel, ParticleOptions};
use mod_api::{ModParticle, ParticleProvider};
use std::{collections::BTreeMap, fmt::Debug, sync::Arc, sync::OnceLock};

/// Requests made by one particle tick, submitted through normal spawn limits.
#[derive(Default)]
pub struct ParticleUpdate {
    /// An index in this particle's declared sprite set. Out-of-range is clamped.
    pub frame: Option<usize>,
    pub spawned: Vec<(ParticleOptions, [f64; 3], [f64; 3])>,
}

/// Owned behavior for a compiled native particle. Cloning a particle clones its
/// controller state. Do not retain world references; inspect `level` each tick.
pub trait ParticleController: Clone + Debug + Send + Sync + 'static {
    /// Call `particle.advance_age()` for ordinary lifetime/interpolation, or
    /// explicitly maintain those fields for an overridden source tick.
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
}

pub(super) trait Controller: Debug + Send + Sync {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
    fn clone_box(&self) -> Box<dyn Controller>;
}
impl<T: ParticleController> Controller for T {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate {
        ParticleController::tick(self, particle, level)
    }
    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }
}
impl Clone for Box<dyn Controller> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// The mods' particle providers by type name: set once at start, before the resources first load,
/// and read by every engine built after.
static REGISTERED: OnceLock<BTreeMap<String, Arc<dyn ParticleProvider>>> = OnceLock::new();

/// Sets the native and wasm mods' particle providers (`register_particle_providers`), once, before
/// the resources first load; each engine then resolves its sprite sets' providers at every load.
/// False when already set.
pub fn register_particle_providers(providers: BTreeMap<String, Arc<dyn ParticleProvider>>) -> bool {
    REGISTERED.set(providers).is_ok()
}

/// What [`register_particle_providers`] set; none before.
pub(super) fn registered() -> &'static BTreeMap<String, Arc<dyn ParticleProvider>> {
    static NONE: BTreeMap<String, Arc<dyn ParticleProvider>> = BTreeMap::new();
    REGISTERED.get().unwrap_or(&NONE)
}

/// How far from the origin a provider may move a particle: the world border's limit. Past it, or
/// not a number, the particle expires rather than reach the renderer.
const LIMIT: f64 = 3.0e7;

/// The controller of a particle of a mod's code-defined type. It owns the particle's behaviour for
/// the engine: age and expiry stay the engine's, the motion is the provider's. Holds the provider
/// the type resolved to when its sprite sets loaded, so a tick dispatches straight to it.
#[derive(Clone)]
pub(super) struct Provided {
    provider: Arc<dyn ParticleProvider>,
    /// `ModParticle::state`, kept from one tick to the next.
    state: [f64; 4],
    /// The frame of the sprite set drawn, as the provider last saw it.
    frame: u32,
}
impl Debug for Provided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provided")
            .field("state", &self.state)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}
impl Provided {
    pub(super) fn new(provider: Arc<dyn ParticleProvider>, frame: u32) -> Self {
        Self {
            provider,
            state: [0.0; 4],
            frame,
        }
    }
}
impl ParticleController for Provided {
    fn tick(&mut self, particle: &mut Particle, _: &ParticleLevel<'_>) -> ParticleUpdate {
        if !particle.advance_age() {
            return ParticleUpdate::default();
        }
        let mut view = ModParticle {
            position: particle.position,
            previous: particle.previous,
            velocity: particle.velocity,
            age: particle.age,
            lifetime: particle.lifetime,
            size: particle.size,
            rgba: particle.rgba,
            frame: self.frame,
            removed: false,
            state: self.state,
        };
        self.provider.tick(&mut view);
        let finite = |v: &[f64; 3]| v.iter().all(|c| c.abs() <= LIMIT);
        if view.removed
            || !finite(&view.position)
            || !finite(&view.previous)
            || !finite(&view.velocity)
            || !view.size.is_finite()
            || !view.rgba.iter().all(|c| c.is_finite())
        {
            particle.remove();
            return ParticleUpdate::default();
        }
        particle.position = view.position;
        particle.previous = view.previous;
        particle.velocity = view.velocity;
        particle.lifetime = view.lifetime;
        // The declarative range of a mod particle's size (`horizon:behaviour`) and of its colour.
        particle.size = view.size.clamp(0.0, 16.0);
        particle.rgba = view.rgba.map(|c| c.clamp(0.0, 1.0));
        self.state = view.state;
        let frame = (view.frame != self.frame).then_some(view.frame as usize);
        self.frame = view.frame;
        ParticleUpdate {
            frame,
            ..ParticleUpdate::default()
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::*;
    use mod_api::{ModParticle, ParticleProvider};
    use std::sync::Arc;
    use {
        horizon_core::nbt::Tag,
        horizon_world::{
            DATA_VERSION,
            collision::CachedCollisionRegistry,
            registry::{BlockRegistry, BlockStateId},
        },
    };
    #[derive(Clone, Debug)]
    struct Moving(u32);
    impl ParticleController for Moving {
        fn tick(&mut self, p: &mut Particle, _: &ParticleLevel<'_>) -> ParticleUpdate {
            if p.advance_age() {
                self.0 += 1;
                p.position[0] += f64::from(self.0);
            }
            ParticleUpdate {
                frame: Some(self.0 as usize),
                ..Default::default()
            }
        }
    }
    /// An empty world with no blocks, which a tick reads nothing of.
    pub(in crate::particles) struct Scene {
        world: World,
        facts: BlockFacts,
        collision: BlockCollisionRegistry<'static>,
    }
    impl Scene {
        pub(in crate::particles) fn new() -> Self {
            // Leaked: the collision registry borrows it for as long as the scene lives.
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_nbt(&Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    (
                        "states",
                        Tag::List(vec![Tag::String("minecraft:air".into())]),
                    ),
                    ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
                ]))
                .unwrap(),
            ));
            let cached = CachedCollisionRegistry::from_nbt(
                &Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    (
                        "states",
                        Tag::List(vec![Tag::String("minecraft:air".into())]),
                    ),
                    ("shapes", Tag::List(vec![])),
                    ("cached_shapes", Tag::IntArray(vec![-1])),
                ]),
                registry,
            )
            .unwrap();
            let collision = BlockCollisionRegistry::new(std::sync::Arc::new(cached)).unwrap();
            let facts = BlockFacts {
                states: vec![],
                air: BlockStateId(0),
                woods: HashMap::new(),
                sign_texts: HashMap::new(),
            };
            Self {
                world: World::new(BlockStateId(0), 0),
                facts,
                collision,
            }
        }
        pub(in crate::particles) fn level(&self) -> ParticleLevel<'_> {
            ParticleLevel {
                world: &self.world,
                facts: &self.facts,
                collision: &self.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            }
        }
    }
    fn engine() -> ParticleEngine {
        let mut e = ParticleEngine::new(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        e.mod_sets.insert(
            "example:orbit".into(),
            ModSet {
                set: 0,
                behaviour: None,
                provider: None,
                group: None,
            },
        );
        e.set_sprites.push(vec![0, 1]);
        e.rects = vec![
            Some(Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16
            });
            2
        ];
        e
    }
    #[test]
    fn controlled_particle_keeps_owned_motion_sprite_interpolation_and_expiry() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = e
            .controlled("example:orbit", [0.0; 3], true, Moving(0))
            .unwrap();
        p.lifetime = 2;
        p.rgba[3] = 0.5;
        p.velocity = [99.0; 3]; // no vanilla drift may leak into the custom tick
        let mut copy = p.clone();
        e.tick_particle(&mut p, &level);
        e.tick_particle(&mut p, &level);
        e.tick_particle(&mut copy, &level);
        assert_eq!((p.position, p.previous), ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        assert_eq!(copy.position, [1.0, 0.0, 0.0]); // controller state cloned independently
        assert!(matches!(p.sprite, Sprite::Index(1))); // clamped frame
        p.size = 0.1;
        e.particles.push(p.clone());
        let mut vertices = Vec::new();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(
            |v| v.color >> 24 == 127 && v.layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0
        ));
        e.particles[0].set_visible(false);
        vertices.clear();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert!(vertices.is_empty());
        assert_eq!(e.particles[0].age, 2);
        e.push(copy);
        e.reload(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(e.pending.is_empty());
        vertices.clear();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert!(vertices.is_empty());
        e.tick_particle(&mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, [3.0, 0.0, 0.0]);
    }
    /// Counts its ticks, so a test sees which calls a particle's expiry skipped.
    #[derive(Default)]
    struct Probe(std::sync::atomic::AtomicUsize);
    impl ParticleProvider for Probe {
        fn tick(&self, particle: &mut ModParticle) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            particle.position[0] = f64::from(particle.age) * 2.0;
            particle.position[1] = particle.state[0];
            particle.state[0] += 0.5;
            particle.frame = particle.age as u32;
            if particle.age == 3 {
                particle.removed = true;
                particle.position[0] = 999.0;
            }
        }
    }
    struct Broken(fn(&mut ModParticle));
    impl ParticleProvider for Broken {
        fn tick(&self, particle: &mut ModParticle) {
            self.0(particle);
        }
    }
    fn provided(e: &mut ParticleEngine, provider: Arc<dyn ParticleProvider>) -> Particle {
        e.mod_sets.get_mut("example:orbit").unwrap().provider = Some(provider);
        e.make(
            &ParticleOptions::named("example:orbit"),
            [1.0; 3],
            [0.0; 3],
            5,
        )
        .unwrap()
    }
    #[test]
    fn a_providers_particle_moves_changes_frame_and_expires_by_its_tick() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 20;
        let mut copy = p.clone();
        e.tick_particle(&mut p, &level);
        assert_eq!((p.position, p.previous), ([2.0, 0.0, 1.0], [1.0; 3]));
        e.tick_particle(&mut p, &level);
        assert_eq!((p.position, p.previous), ([4.0, 0.5, 1.0], [2.0, 0.0, 1.0]));
        assert!(matches!(p.sprite, Sprite::Index(1))); // frame 2, clamped to the 2-sprite set
        // State is per particle: the copy starts from zero state, not the original's.
        e.tick_particle(&mut copy, &level);
        assert_eq!(copy.position, [2.0, 0.0, 1.0]);
        // The third tick of `p` removes it before its 20-tick lifetime, and its writes are dropped.
        e.tick_particle(&mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, [4.0, 0.5, 1.0]);
    }
    #[test]
    fn the_engine_expires_a_providers_particle_at_its_lifetime_without_calling_the_provider() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 2;
        for _ in 0..2 {
            e.tick_particle(&mut p, &level);
            assert!(!p.removed);
        }
        e.tick_particle(&mut p, &level);
        assert!(p.removed);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
    #[test]
    fn a_provider_cannot_hand_the_renderer_a_bad_number() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let bad: [fn(&mut ModParticle); 5] = [
            |p| p.position[1] = f64::NAN,
            |p| p.position[0] = f64::INFINITY,
            |p| p.position[2] = 1.0e300,
            |p| p.velocity[0] = f64::NEG_INFINITY,
            |p| p.size = f32::NAN,
        ];
        for tick in bad {
            let mut p = provided(&mut e, Arc::new(Broken(tick)));
            e.tick_particle(&mut p, &level);
            assert!(p.removed);
            assert_eq!(p.position, [1.0; 3]);
        }
        let mut p = provided(
            &mut e,
            Arc::new(Broken(|p| {
                p.size = 1.0e9;
                p.rgba = [2.0, -1.0, 0.5, 9.0];
                p.frame = u32::MAX;
                p.lifetime = i32::MIN;
                p.age = i32::MAX;
            })),
        );
        e.tick_particle(&mut p, &level);
        assert!(!p.removed);
        assert_eq!((p.size, p.rgba), (16.0, [1.0, 0.0, 0.5, 1.0]));
        assert!(matches!(p.sprite, Sprite::Index(1)));
        assert_eq!((p.lifetime, p.age), (i32::MIN, 1)); // the age stays the engine's
        e.tick_particle(&mut p, &level);
        assert!(p.removed);
    }
    #[test]
    fn a_type_resolves_to_its_provider_at_load_and_other_types_stay_declarative() {
        let dir =
            std::env::temp_dir().join(format!("horizon-provider-sets-{}", std::process::id()));
        let particles = dir.join("assets/mymod/particles");
        std::fs::create_dir_all(&particles).unwrap();
        for name in ["orbit", "plain"] {
            std::fs::write(
                particles.join(format!("{name}.json")),
                br#"{"textures":["mymod:one","mymod:two"],"horizon:behaviour":{"lifetime":7,"size":[0.3,0.3]}}"#,
            )
            .unwrap();
        }
        let mut e = engine();
        e.providers
            .insert("mymod:orbit".into(), Arc::new(Probe::default()));
        e.providers
            .insert("mymod:unmatched".into(), Arc::new(Probe::default()));
        e.reload(
            &ResourceSource::directory(dir.clone()),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(e.mod_sets["mymod:orbit"].provider.is_some());
        assert!(e.mod_sets["mymod:plain"].provider.is_none());
        assert!(!e.mod_sets.contains_key("mymod:unmatched"));
        let orbit = e
            .make(
                &ParticleOptions::named("mymod:orbit"),
                [0.0; 3],
                [0.0; 3],
                3,
            )
            .unwrap();
        // The declared spawn values seed it, and its sprite is drawn by frame from the set.
        assert!(orbit.controller.is_some());
        assert_eq!((orbit.lifetime, orbit.size), (7, 0.3));
        assert_eq!(orbit.set, Some(e.mod_sets["mymod:orbit"].set));
        let plain = e
            .make(
                &ParticleOptions::named("mymod:plain"),
                [0.0; 3],
                [0.0; 3],
                3,
            )
            .unwrap();
        assert!(plain.controller.is_none());
        assert!(matches!(plain.behaviour, Behaviour::Mod { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn controlled_spawn_respects_distance_settings_and_declared_sprite_sets() {
        let mut e = engine();
        assert!(
            e.controlled("example:missing", [0.0; 3], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [33.0, 0.0, 0.0], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [f64::NAN, 0.0, 0.0], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [32.0, 0.0, 0.0], true, Moving(0))
                .is_some()
        );
        e.particle_level = 2;
        assert!(
            e.controlled("example:orbit", [0.0; 3], true, Moving(0))
                .is_none()
        );
    }
}
