//! The window's own ambient effects of entities, which a client makes for
//! itself (`level().isClientSide()` branches): endermen's and endermites'
//! portal particles, blazes' smoke and burn, eyes of ender's trail. They run
//! on the game that has the window, following a server, never on
//! a server: nothing of them goes over the network.
use super::Game;
use horizon_client::blocks::RenderShape;
use horizon_client::effects::effect_particle;
use horizon_client::mobs::{MobEvent, Species, ambient};
use horizon_client::motion::{LAVA, WATER};
use horizon_client::particles::{ParticleEngine, ParticleOptions, ParticleType};
use horizon_client::pose::Pose;
use horizon_core::{geometry::Vec3, random::RandomSource};
use mod_api::EntityRef;

impl Game<'_> {
    pub(super) fn tick_client_ambient(&mut self) {
        if self.serving.is_some() || self.options.dedicated.is_some() {
            return;
        }
        // A window simulating the world already runs `WitherBoss.aiStep`.
        // One following a server does not: `Level.addParticle` on the server
        // draws nothing, so the smoke, the powered effect and the charge
        // cloud have to be made here.
        let following = self.remote.is_some();
        let mut events = Vec::new();
        let random = self.particles.level_random();
        for m in self.mobs.iter_mut().filter(|m| !m.removed) {
            ambient::client_ai_step(m, &mut events);
            ambient::client_particles(m, random, &mut events);
            if following && m.species() == horizon_client::mobs::Species::Wither {
                horizon_client::mobs::wither::head_particles(m, random, &mut events);
            }
        }
        for event in events {
            match event {
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = ParticleType::from_name(kind) {
                        self.particles
                            .add(&ParticleOptions::simple(ty), at, velocity);
                    }
                }
                MobEvent::Particles { kind, at, count: 0 } => {
                    if let Some(ty) = ParticleType::from_name(kind) {
                        self.particles
                            .add(&ParticleOptions::simple(ty), at, [0.0; 3]);
                    }
                }
                MobEvent::ParticleColored {
                    at,
                    color: [r, g, b],
                } => {
                    self.particles.add(
                        &ParticleOptions::colored(ParticleType::EntityEffect, [r, g, b, 1.0]),
                        at,
                        [0.0; 3],
                    );
                }
                // Each guest of a LAN host makes its own.
                MobEvent::LocalSound(name, at, volume, pitch) => {
                    self.audio
                        .play_unshared(&format!("minecraft:{name}"), Some(at), volume, pitch)
                }
                _ => {}
            }
        }
        self.eye_particles();
        self.panda_effects();
        self.sprint_dust();
        self.effect_swirls();
    }

    /// `Entity.baseTick`'s `spawnSprintParticle` on a client: dust of the
    /// block under a sprinting runner's feet streams back from them every
    /// tick, for the window's own player and for every other client entity
    /// whose synced sprint flag is set.
    fn sprint_dust(&mut self) {
        // Each runner's feet, movement, supporting block, box width and index
        // in `mobs` (none for the window's own player).
        let mut runners: Vec<(Vec3, Vec3, Option<[i32; 3]>, f64, Option<usize>)> = Vec::new();
        // Entity.canSpawnSprintParticle, with LocalPlayer's own override: a
        // player who is flying leaves no dust.
        if let Some(p) = self.player.as_ref()
            && p.sprinting
            && !p.flying
            && !p.spectator
            && p.pose != Pose::Crouching
            && !p.health.dead_or_dying()
            && !p.fluids.was_touching_water
            && !p.is_in_lava()
            && p.fluid_height(WATER) <= 0.0
            && p.fluid_height(LAVA) <= 0.0
        {
            let support = p.ground.supporting_block.map(|b| [b.x, b.y, b.z]);
            runners.push((p.position, p.velocity, support, f64::from(0.6f32), None));
        }
        // IronGolem.canSpawnSprintParticle replaces the sprinting test: one
        // tick in five while it moves at all, in water or not, drawn from the
        // golem's own random as its client copy's Entity.baseTick does.
        for (i, m) in self.mobs.iter_mut().enumerate() {
            if &*m.kind.id != "minecraft:iron_golem" {
                continue;
            }
            let movement = m.position.subtract(m.previous);
            let moving =
                movement.x * movement.x + movement.z * movement.z > f64::from(2.5000003E-7_f32);
            if moving && m.random.next_int_bound(5).unwrap_or(1) == 0 {
                runners.push((m.position, movement, None, f64::from(m.width()), Some(i)));
            }
        }
        // Entity.canSpawnSprintParticle on any client copy: a mob's synced
        // sprint flag draws the crumb as a player's does, and a spectator's
        // never does (`!isSpectator()`). `Rabbit` overrides it to false (its
        // hop event draws its own), and the iron golem's own branch is above.
        for (i, m) in self.mobs.iter().enumerate() {
            if &*m.kind.id == "minecraft:iron_golem" || m.species() == Species::Rabbit {
                continue;
            }
            // A client copy's fluids aren't known here: none where it stands.
            let feet = [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ];
            let dry = self
                .res
                .facts()
                .get(self.world.block(feet[0], feet[1], feet[2]))
                .fluid_level
                == 0;
            if m.extra.sprinting
                && !m.extra.crouching
                && !m.extra.spectator
                && m.alive()
                && !m.removed
                && dry
            {
                let movement = m.position.subtract(m.previous);
                runners.push((m.position, movement, None, f64::from(m.width()), Some(i)));
            }
        }
        for (position, movement, support, width, mob) in runners {
            self.sprint_particle(position, movement, support, width, |game| {
                // this.random: the runner's own, so one walking out of sight
                // leaves the level's and the particles' random alone. The
                // window's player has its own body's random too.
                let r = match mob {
                    Some(i) => &mut game.mobs.get_mut(i)?.random,
                    None => &mut game.player.as_mut()?.random,
                };
                Some((r.next_double(), r.next_double()))
            });
        }
    }

    /// Spawn sprint particles when called explicitly by a client mod. A
    /// valid entity returns true even if the block beneath it is invisible.
    /// Returns false for stale, untracked, server-side, or unsupported entities.
    pub(super) fn entity_sprint_particles(&mut self, entity: EntityRef) -> bool {
        use super::commands::Target;
        if self.is_logical_server() {
            return false;
        }
        let Some(target) = self.native_client_entity_target(entity) else {
            return false;
        };
        let Some([x, y, z]) = self.entity_position(entity) else {
            return false;
        };
        let Some([vx, vy, vz]) = self.entity_velocity(entity) else {
            return false;
        };
        let Some([min, max]) = self.entity_bounding_box(entity) else {
            return false;
        };
        let support = match target {
            Target::Player => self
                .player
                .as_ref()
                .and_then(|p| p.ground.supporting_block)
                .map(|p| [p.x, p.y, p.z]),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.supporting),
            _ => None,
        };
        let mob = match target {
            Target::Mob(id) => Some(id),
            _ => None,
        };
        self.sprint_particle(
            Vec3::new(x, y, z),
            Vec3::new(vx, vy, vz),
            support,
            max[0] - min[0],
            |game| {
                let random = match mob {
                    Some(id) => &mut game.mob_index.get_mut(&mut game.mobs, id)?.random,
                    None => game.particles.random(),
                };
                Some((random.next_double(), random.next_double()))
            },
        );
        true
    }

    /// `Entity.spawnSprintParticle`: one dust crumb of the block under the
    /// feet at `position` (`getOnPosLegacy`), thrown back against `movement`,
    /// unless that block is invisible. `offsets` draws the two random
    /// `nextDouble`s for x and z (the entity's random in vanilla).
    pub(super) fn sprint_particle(
        &mut self,
        position: Vec3,
        movement: Vec3,
        support: Option<[i32; 3]>,
        width: f64,
        offsets: impl FnOnce(&mut Self) -> Option<(f64, f64)>,
    ) {
        // Entity.getOnPosLegacy: getOnPos(0.2).
        let below = (position.y - f64::from(0.2f32)).floor() as i32;
        let tall = |game: &Self, s: [i32; 3]| {
            let name = game.block_name(s);
            [
                "minecraft:fences",
                "minecraft:walls",
                "minecraft:fence_gates",
            ]
            .iter()
            .any(|tag| game.named_block_in_tag(tag, &name))
        };
        let at = match support {
            Some(s) if tall(self, s) => s,
            Some(s) => [s[0], below, s[2]],
            None => [position.x.floor() as i32, below, position.z.floor() as i32],
        };
        let state = self.world.block(at[0], at[1], at[2]);
        if self.res.facts().get(state).render_shape == RenderShape::Invisible {
            return;
        }
        let Some((rx, rz)) = offsets(self) else {
            return;
        };
        let Some((sprite, tint)) = self.particle_sprite(state, at) else {
            return;
        };
        let mut x = position.x + (rx - 0.5) * width;
        let mut z = position.z + (rz - 0.5) * width;
        // Clamped into the block when the feet are in another column.
        if position.x.floor() as i32 != at[0] {
            x = x.clamp(f64::from(at[0]), f64::from(at[0]) + 1.0);
        }
        if position.z.floor() as i32 != at[2] {
            z = z.clamp(f64::from(at[2]), f64::from(at[2]) + 1.0);
        }
        self.particles.add(
            &ParticleOptions::block(ParticleType::Block, sprite, tint)
                .with_source(&self.res.facts().get(state).name),
            [x, position.y + 0.1, z],
            [movement.x * -4.0, 1.5, movement.z * -4.0],
        );
    }

    /// `LivingEntity.tickEffects` on a client: the swirls around an entity
    /// with visible effects, from the particles its synched data lists. The
    /// window's player has its own effects, a server's mobs and players the
    /// list the server sent, and the mobs and guests of a game in the frame
    /// theirs.
    fn effect_swirls(&mut self) {
        // `ITEM_SLIME` and `ITEM_COBWEB` crumbs draw the item's model, which
        // `BreakingItemParticle`'s providers resolve as they spawn: the sprites
        // are kept per type here, and this resolves them once (it returns at
        // once after).
        self.ensure_item_particle_sprites();
        if let Some(p) = self.player.as_ref() {
            let effects = &self.potions.effects.list;
            let list: Vec<_> = effects
                .iter()
                .filter(|e| e.visible)
                .filter_map(|e| effect_particle(e.id, e.ambient))
                .collect();
            let ambient = effects.iter().filter(|e| e.visible).all(|e| e.ambient);
            let invisible = self.own_invisible();
            let size = (
                f64::from(p.dimensions.width),
                f64::from(p.dimensions.height),
            );
            swirl(
                &mut self.particles,
                p.position,
                size,
                &list,
                ambient,
                invisible,
            );
        }
        for m in self.mobs.iter() {
            // A mob with no effects swirls nothing: skipped before its size
            // is worked out, which every mob in a crowded world would pay
            // for (#14053).
            if !m.alive() || (m.extra.effect_particles.is_empty() && m.extra.effects.is_empty()) {
                continue;
            }
            let size = (f64::from(m.width()), f64::from(m.height()));
            let invisible = m.is_invisible();
            if m.extra.effect_particles.is_empty() {
                // A mob of this game: its own effects, visible and not ambient.
                let list: Vec<_> = m
                    .extra
                    .effects
                    .iter()
                    .filter(|e| !m.extra.hidden_effects.contains(&e.id))
                    .filter_map(|e| effect_particle(e.id, m.extra.ambient_effects.contains(&e.id)))
                    .collect();
                swirl(
                    &mut self.particles,
                    m.position,
                    size,
                    &list,
                    false,
                    invisible,
                );
            } else {
                let list = &m.extra.effect_particles;
                let ambient = m.extra.effect_ambient;
                swirl(
                    &mut self.particles,
                    m.position,
                    size,
                    list,
                    ambient,
                    invisible,
                );
            }
        }
    }
}

/// One tick of `LivingEntity.tickEffects`' client branch: a one in four
/// chance (one in fifteen while invisible, a fifth of either when all the
/// effects are ambient) of one of the particles at a random spot of the box.
fn swirl(
    particles: &mut ParticleEngine,
    feet: Vec3,
    size: (f64, f64),
    list: &[ParticleOptions],
    ambient: bool,
    invisible: bool,
) {
    if list.is_empty() {
        return;
    }
    let bound = if invisible { 15 } else { 4 } * if ambient { 5 } else { 1 };
    let r = particles.random();
    let Ok(0) = r.next_int_bound(bound) else {
        return;
    };
    // Util.getRandom, then getRandomX(0.5), getRandomY and getRandomZ.
    let pick = r.next_int_bound(list.len() as i32).unwrap_or_default() as usize;
    let x = feet.x + size.0 * (2.0 * r.next_double() - 1.0) * 0.5;
    let y = feet.y + size.1 * r.next_double();
    let z = feet.z + size.0 * (2.0 * r.next_double() - 1.0) * 0.5;
    particles.add(&list[pick], [x, y, z], [1.0; 3]);
}

#[cfg(test)]
mod sprint_handle_tests {
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::connection::Pipe;
    use mod_api::{ClientHost, EntityRef, WorldEpoch};
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn client_entity_sprint_particles_use_the_local_block_and_reject_stale_handles() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            let root =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let client_resources = super::super::load_resources(
                horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &super::super::Settings::default(),
                std::time::Instant::now(),
                false,
                super::super::execution_role::ExecutionRole::Client,
                &super::super::Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap();
            game.pack = client_resources.pack;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SprintParticleTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            assert_eq!(game.world.block(0, 63, 0), stone);
            assert!(
                game.particle_sprite(stone, [0, 63, 0]).is_some(),
                "stone sprite missing"
            );
            game.particles.requested = Some(Vec::new());
            let entity = game.entity(7).unwrap();
            let host: &mut dyn ClientHost = game;
            assert!(entity.sprint_particles(host));
            let requests = game.particles.requested.as_ref().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].1[1], 64.1);
            let stale = EntityRef::new(WorldEpoch::new(0), 7);
            assert!(!stale.sprint_particles(game));
            game.remote.as_mut().unwrap().logged_in = false;
            assert!(!entity.sprint_particles(game));
            assert_eq!(game.particles.requested.as_ref().unwrap().len(), 1);
        });
    }
}

#[cfg(test)]
#[path = "tests/effect_particle_tests.rs"]
mod effect_particle_tests;

#[cfg(test)]
#[path = "tests/sprint_dust_tests.rs"]
mod sprint_dust_tests;
