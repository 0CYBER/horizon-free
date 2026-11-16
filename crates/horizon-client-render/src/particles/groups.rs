//! The render groups of mods' particles (`register_particle_groups`). The engine keeps each group's
//! live particles apart from the built-in ones, as `ParticleEngine.particles` keeps one
//! `ParticleGroup` per `ParticleRenderType`, and resolves each particle type's group when its
//! sprite sets load, so a spawn, a tick and a draw never look a group up by name.
use super::Particle;
use mod_api::{Identifier, ParticleBlend, ParticleGroup};
use std::sync::OnceLock;

/// The mods' particle groups, set once at start, before the resources first load, and read by every
/// engine built after.
static REGISTERED: OnceLock<Vec<ParticleGroup>> = OnceLock::new();

/// Sets the native and wasm mods' particle groups (`register_particle_groups`), once, before the
/// resources first load; each engine then resolves its sprite sets' groups at every load. A group
/// whose id is not `<namespace>:<path>` in a namespace other than vanilla's, or that another group
/// has already, is logged and left out. False when already set.
pub fn register_particle_groups(groups: Vec<ParticleGroup>) -> bool {
    REGISTERED.set(checked(groups)).is_ok()
}

/// What [`register_particle_groups`] set, in registration order; none before. A group's index
/// here is its handle in every engine.
pub(super) fn registered() -> &'static [ParticleGroup] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

fn checked(groups: Vec<ParticleGroup>) -> Vec<ParticleGroup> {
    let mut kept: Vec<ParticleGroup> = Vec::new();
    for group in groups {
        let own =
            Identifier::parse_namespaced(&group.id).is_some_and(|id| id.namespace() != "minecraft");
        if !own {
            log::warn!(
                "particle group {:?} is not <namespace>:<path> in a mod's namespace",
                group.id
            );
        } else if kept.iter().any(|k| k.id == group.id) {
            log::warn!(
                "particle group {} is registered twice; the first is kept",
                group.id
            );
        } else {
            kept.push(group);
        }
    }
    kept
}

/// One registered group in an engine: how it blends and its live particles. Vanilla's
/// `ParticleGroup` keeps its own queue and its own limit the same way.
pub(super) struct Group {
    pub(super) blend: ParticleBlend,
    pub(super) particles: Vec<Particle>,
}

impl Group {
    /// An empty group for each of `defs`, in its handle's order.
    pub(super) fn all(defs: &[ParticleGroup]) -> Vec<Self> {
        defs.iter()
            .map(|group| Self {
                blend: group.blend,
                particles: Vec::new(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{native::tests::Scene, *};
    use super::{Group, ParticleGroup, checked};
    use mod_api::{ModParticle, ParticleProvider};

    fn group(id: &str, blend: ParticleBlend, particles: &[&str]) -> ParticleGroup {
        ParticleGroup {
            id: id.into(),
            blend,
            particles: particles.iter().map(|&p| p.into()).collect(),
        }
    }

    #[test]
    fn registration_keeps_mod_groups_in_order_and_drops_bad_ids_and_repeats() {
        let opaque = ParticleBlend::Opaque;
        let kept = checked(vec![
            group("mymod:glow", opaque, &["mymod:a"]),
            group("minecraft:glow", opaque, &[]),
            group("noprefix", opaque, &[]),
            group("mymod:smoke", opaque, &["mymod:b"]),
            group("mymod:glow", opaque, &["mymod:c"]),
        ]);
        let ids: Vec<(&str, &[String])> = kept
            .iter()
            .map(|g| (g.id.as_str(), g.particles.as_slice()))
            .collect();
        assert_eq!(
            ids,
            [
                ("mymod:glow", &["mymod:a".to_string()][..]),
                ("mymod:smoke", &["mymod:b".to_string()][..]),
            ]
        );
    }

    /// Moves a particle to a fixed spot on each tick, so a drawn quad says where it is.
    struct Stay;
    impl ParticleProvider for Stay {
        fn tick(&self, particle: &mut ModParticle) {
            particle.position = particle.previous;
        }
    }

    /// A pack with the vanilla `flame` and mod sets `plain`, `glow`, `shade` and `orbit`, each with
    /// the one sprite `<ns>:particle/one`, and an atlas that holds every sprite.
    struct Pack {
        dir: std::path::PathBuf,
        atlas: HashMap<String, Rect>,
    }
    impl Pack {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("horizon-groups-{name}-{}", std::process::id()));
            let json = |ns: &str, path: &str, textures: &str| {
                let folder = dir.join(format!("assets/{ns}/particles"));
                std::fs::create_dir_all(&folder).unwrap();
                std::fs::write(folder.join(path), textures).unwrap();
            };
            json(
                "minecraft",
                "flame.json",
                r#"{"textures":["minecraft:flame"]}"#,
            );
            for set in ["plain", "glow", "shade", "orbit"] {
                json(
                    "mymod",
                    &format!("{set}.json"),
                    r#"{"textures":["mymod:one"],"horizon:behaviour":{"lifetime":2,"size":[0.5,0.5]}}"#,
                );
            }
            let rect = Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16,
            };
            let atlas = HashMap::from([
                ("particle/flame".to_owned(), rect),
                ("mymod:particle/one".to_owned(), rect),
            ]);
            Self { dir, atlas }
        }
        /// An engine whose registered groups are `defs`, resolved over this pack.
        fn engine(&self, defs: Vec<ParticleGroup>) -> ParticleEngine {
            let mut e = ParticleEngine::new(
                &ResourceSource::directory(self.dir.clone()),
                &self.atlas,
                &HashSet::new(),
            );
            e.group_defs = Box::leak(defs.into_boxed_slice());
            e.groups = Group::all(e.group_defs);
            self.reload(&mut e);
            e
        }
        fn reload(&self, e: &mut ParticleEngine) {
            e.reload(
                &ResourceSource::directory(self.dir.clone()),
                &self.atlas,
                &HashSet::new(),
            );
        }
    }
    impl Drop for Pack {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn spawn(e: &mut ParticleEngine, options: &ParticleOptions, x: f64) {
        let mut p = e.make(options, [x, 0.0, 0.0], [0.0; 3], 1).unwrap();
        p.lifetime = 2;
        p.size = 0.5;
        p.velocity = [0.0; 3];
        e.push(p);
    }
    /// The x of the centre of each quad `render` draws, in draw order, and its layer.
    fn drawn(e: &ParticleEngine) -> Vec<(i32, bool)> {
        let mut vertices = Vec::new();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.0,
            &|_| 0,
            &mut vertices,
        );
        vertices
            .chunks(4)
            .map(|quad| {
                let x = quad.iter().map(|v| v.pos[0]).sum::<f32>() / 4.0;
                let translucent = quad[0].layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0;
                (x.round() as i32, translucent)
            })
            .collect()
    }

    #[test]
    fn a_registered_groups_particles_draw_after_the_built_in_ones_and_expire_normally() {
        let (scene, pack) = (Scene::new(), Pack::new("lifecycle"));
        let level = scene.level();
        let mut e = pack.engine(vec![
            group(
                "mymod:glow",
                ParticleBlend::Translucent,
                &["mymod:glow", "mymod:orbit"],
            ),
            group(
                "mymod:shade",
                ParticleBlend::Opaque,
                &["mymod:shade", "mymod:glow"],
            ),
        ]);
        // Each type resolved once at load: the first group to list a type keeps it.
        let handle = |name: &str| e.mod_sets[name].group;
        assert_eq!(
            [
                handle("mymod:glow"),
                handle("mymod:orbit"),
                handle("mymod:shade"),
                handle("mymod:plain")
            ],
            [Some(0), Some(0), Some(1), None]
        );
        // A provider's particle is blended on its own, and an orbit type's declaration is opaque:
        // each takes its group's blend.
        e.providers.insert("mymod:shade".into(), Arc::new(Stay));
        pack.reload(&mut e);
        // Spawned in the reverse of the order the groups draw in.
        for (name, x) in [
            ("mymod:shade", 3.0),
            ("mymod:glow", 2.0),
            ("mymod:orbit", 4.0),
        ] {
            spawn(&mut e, &ParticleOptions::named(name), x);
        }
        spawn(&mut e, &ParticleOptions::named("mymod:plain"), 1.0);
        spawn(&mut e, &ParticleOptions::simple(ParticleType::Flame), 0.0);
        e.tick(&level);
        assert_eq!(
            (
                e.particles.len(),
                e.groups[0].particles.len(),
                e.groups[1].particles.len()
            ),
            (2, 2, 1)
        );
        assert_eq!(e.len(), 5);
        // The built-in group first, as before (the plain mod particle and the flame, opaque); then
        // group 0, translucent; then group 1, opaque, though its provider's particle is blended.
        assert_eq!(
            drawn(&e),
            [(1, false), (0, false), (2, true), (4, true), (3, false)]
        );
        // Lifetime 2: gone after the same ticks, from every group, and nothing is left drawn.
        e.tick(&level);
        e.tick(&level);
        assert_eq!(e.len(), 5);
        e.tick(&level);
        assert_eq!((e.len(), e.group_count()), (0, 0));
        assert!(e.is_empty() && drawn(&e).is_empty());
    }

    #[test]
    fn each_group_has_its_own_limit_and_a_reload_drops_only_provider_particles_of_a_group() {
        let (scene, pack) = (Scene::new(), Pack::new("limits"));
        let level = scene.level();
        let mut e = pack.engine(vec![
            group("mymod:glow", ParticleBlend::Translucent, &["mymod:glow"]),
            group("mymod:shade", ParticleBlend::Opaque, &["mymod:shade"]),
        ]);
        e.providers.insert("mymod:shade".into(), Arc::new(Stay));
        pack.reload(&mut e);
        let options = ParticleOptions::named("mymod:glow");
        let mut full = e.make(&options, [0.0; 3], [0.0; 3], 1).unwrap();
        full.lifetime = 100;
        e.groups[0].particles = vec![full; MAX_PARTICLES];
        for name in ["mymod:glow", "mymod:shade", "mymod:plain"] {
            spawn(&mut e, &ParticleOptions::named(name), 0.0);
        }
        e.tick(&level);
        // Group 0 is full, so its newcomer is refused; the other groups and the built-in one
        // still admit theirs.
        let sizes = (
            e.groups[0].particles.len(),
            e.groups[1].particles.len(),
            e.particles.len(),
        );
        assert_eq!(sizes, (MAX_PARTICLES, 1, 1));
        // A reload drops the particles a controller moves (the provider's sprite-set handles
        // belong to the load) in a group as in the built-in one; the declarative particles stay and
        // still draw in their group.
        pack.reload(&mut e);
        assert_eq!(
            (
                e.groups[0].particles.len(),
                e.groups[1].particles.len(),
                e.particles.len()
            ),
            (MAX_PARTICLES, 0, 1)
        );
        assert_eq!(e.mod_sets["mymod:shade"].group, Some(1));
    }
}
