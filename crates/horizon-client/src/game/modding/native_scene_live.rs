//! Living screen scenes (`DrawContext::draw_scene` with entities, particles or a clock):
//! a scene's blocks and mod block entities as `native_scene` draws them, with entities
//! standing in them and particles that live in them. Per calling mod and clock id, the
//! game keeps the block entities' own fields, a small level of the scene's blocks, its own
//! particle engine and its entities. None of that is the client's level or particle engine.
use super::api_values::TryIntoEngine as _;
use super::{
    Game,
    native_scene::{SceneBlock, SceneBlocks, SceneFrame, SceneQuad, face_shade, shaded},
};
use glam::{DVec3, Quat, Vec3};
use horizon_client::{
    animate_tick::{AnimateContext, animate_scene},
    chunk::Chunk,
    hud::HudBatch,
    mesher::Vertex,
    mobs::Mob,
    particles::{ParticleEngine, ParticleLevel, ParticleOptions},
    world::World,
};
use horizon_core::geometry::Vec3 as Position;
use mod_api::{
    BlockEntityTypeKey, GuiEntity, GuiEntityError, ModBlockEntity, SCENE_ANIMATE_PICKS,
    SCENE_CATCH_UP, SCENE_MAX_ANIMATING, SCENE_MAX_ENTITIES, SCENE_MAX_PARTICLES, Scene,
    SceneBlockEntity, SceneClock, SceneError, SceneView,
};
use std::{
    cell::Cell,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

/// How many scenes the game keeps: a guide page rarely shows more. The one drawn
/// longest ago goes first.
const KEPT: usize = 16;
/// An entity or particle this far out is refused, as a level's border would refuse it;
/// a block this far out draws but has no animate tick.
const FAR: f64 = 30_000_000.0;

/// An input's original fields, and its scene-owned drawing copy when its type opted in.
pub(super) struct LiveBlockEntity {
    pub(super) index: usize,
    seed: SceneBlockEntity,
    pub(super) ticking: Option<(BlockEntityTypeKey, ModBlockEntity)>,
}

impl LiveBlockEntity {
    fn new(index: usize, seed: &SceneBlockEntity) -> Self {
        let id = seed.id.to_string();
        let ticking = mod_api::__private::scene_block_entity_ticker(&id)
            .map(|kind| (kind, ModBlockEntity::new(id, seed.fields.clone())));
        Self {
            index,
            seed: seed.clone(),
            ticking,
        }
    }
}

/// One scene's kept state.
struct Live {
    /// The mod that drew it, and its clock's id.
    owner: &'static str,
    id: u32,
    /// The hash of the blocks it was made for.
    key: u64,
    /// The entities it was made for.
    entities: Vec<GuiEntity>,
    /// The supplied block entities, in input order, separate from their mutable copies.
    block_entities: Vec<LiveBlockEntity>,
    /// The last tick it ran.
    tick: u64,
    /// The frame it last drew in.
    frame: u64,
    world: World,
    /// The scene's blocks its small level holds: what the animate ticks pick among.
    picks: Vec<[i32; 3]>,
    engine: ParticleEngine,
    /// One per scene entity, in order.
    mobs: Vec<Mob>,
    /// Geometry scratch, kept so a frame allocates none.
    verts: Vec<Vertex>,
}

/// The living scenes the game's drawing keeps, and the frame's animating budget.
#[derive(Default)]
pub(in crate::game) struct LiveScenes {
    live: Vec<Live>,
    /// The frame `animating` counts.
    frame: u64,
    /// The scenes (mod, id) that animated in `frame`.
    animating: Vec<(&'static str, u32)>,
}

impl LiveScenes {
    /// Forgets every kept scene: their particles name the old atlas's sprites and their
    /// entities the old resources' types (a resource reload), or the world closed.
    pub(in crate::game) fn clear(&mut self) {
        self.live.clear();
        self.animating.clear();
    }
    /// How many scenes are kept.
    #[cfg(test)]
    pub(in crate::game) fn len(&self) -> usize {
        self.live.len()
    }
    /// Whether scene (`owner`, `id`) may animate in `frame`: the first
    /// `SCENE_MAX_ANIMATING` scenes a frame do, each as often as it is drawn.
    pub(in crate::game) fn admit(&mut self, frame: u64, owner: &'static str, id: u32) -> bool {
        if self.frame != frame {
            self.frame = frame;
            self.animating.clear();
        }
        if self.animating.contains(&(owner, id)) {
            return true;
        }
        if self.animating.len() >= SCENE_MAX_ANIMATING {
            return false;
        }
        self.animating.push((owner, id));
        true
    }
}

/// The blocks' hash: a scene whose blocks change starts over.
pub(super) fn scene_key(blocks: &[SceneBlock<'_>]) -> u64 {
    let mut h = DefaultHasher::new();
    for b in blocks {
        (b.pos, b.state.0).hash(&mut h);
    }
    h.finish()
}

/// The input payloads are compared with the original seeds, never with the animated fields.
fn same_block_entities(blocks: &[SceneBlock<'_>], kept: &[LiveBlockEntity]) -> bool {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| block.entity.map(|entity| (i, entity)))
        .eq(kept.iter().map(|entry| (entry.index, &entry.seed)))
}

/// The seed of scene `id`'s tick `tick`: the same ticks give the same particles.
pub(super) fn tick_seed(id: u32, tick: u64) -> i64 {
    let mut h = DefaultHasher::new();
    (0x5343_454e_u32, id, tick).hash(&mut h);
    h.finish() as i64
}

/// The ticks a scene last run at `last` runs to show `tick`: whether it starts over from
/// no particles first (a jump past `SCENE_CATCH_UP`), and the first and last tick to run.
/// None when nothing runs (paused, or a tick behind, which the caller starts over).
pub(super) fn ticks_to_run(last: u64, tick: u64) -> Option<(bool, u64, u64)> {
    if tick <= last {
        return None;
    }
    let restart = tick - last > SCENE_CATCH_UP;
    let first = if restart {
        tick - SCENE_CATCH_UP + 1
    } else {
        last + 1
    };
    Some((restart, first, tick))
}

/// The camera turn that shows a particle's quad flat to the screen through
/// `scene_matrix`: its up up the screen and its right to the right.
pub(super) fn facing_viewer(view: SceneView) -> Quat {
    Quat::from_rotation_y(-view.yaw.to_radians())
        * Quat::from_rotation_x(view.pitch.to_radians())
        * Quat::from_rotation_y(std::f32::consts::PI)
}

/// Whether a scene lives without any scene-only block entity tickers.
pub(super) fn lives(scene: &Scene) -> bool {
    !scene.entities.is_empty()
        || !scene.particles.is_empty()
        || scene.clock.is_some_and(|c| c.animate_blocks)
}

/// A living scene's input, checked before anything is kept or drawn: its clock.
pub(super) fn check(
    rect: [f32; 4],
    scene: &Scene,
    view: SceneView,
) -> Result<SceneClock, SceneError> {
    let finite = rect.iter().chain(&view.center).all(|v| v.is_finite())
        && view.yaw.is_finite()
        && view.pitch.is_finite()
        && view.scale.is_finite()
        && view.scale > 0.0;
    if !finite {
        return Err(SceneError::BadView);
    }
    if scene.entities.len() > SCENE_MAX_ENTITIES {
        return Err(SceneError::TooManyEntities);
    }
    if scene.particles.len() > SCENE_MAX_PARTICLES {
        return Err(SceneError::TooManyParticles);
    }
    let clock = scene.clock.ok_or(SceneError::NoClock)?;
    if !clock.partial.is_finite() {
        return Err(SceneError::BadClock);
    }
    let placed = |p: &[f64; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= FAR);
    for (index, e) in (0u32..).zip(&scene.entities) {
        if !placed(&e.pos)
            || ![e.yaw, e.head_yaw, e.head_pitch]
                .iter()
                .all(|v| v.is_finite())
        {
            return Err(SceneError::BadEntity { index });
        }
    }
    for (index, p) in (0u32..).zip(&scene.particles) {
        if !placed(&p.pos) || !p.velocity.iter().all(|v| v.is_finite()) {
            return Err(SceneError::BadParticle { index });
        }
    }
    Ok(clock)
}

/// `DrawContext::entity`'s refusal for scene entity `index`, by the same name.
fn entity_refusal(error: GuiEntityError, index: u32) -> SceneError {
    match error {
        GuiEntityError::BadData => SceneError::BadData { index },
        GuiEntityError::NoEntity => SceneError::NoEntity { index },
        GuiEntityError::Unavailable | GuiEntityError::BadView => SceneError::Unavailable,
        _ => SceneError::UnknownType { index },
    }
}

impl Game<'_> {
    /// Draws the prepared `blocks` into the given `frame`, with its textured quads.
    /// A static scene keeps no state and reads no clock. A living one is drawn with
    /// its entities and the particles its clock ticks; a clock also drives any
    /// scene-only block entity tickers. Invalid input draws nothing.
    /// Past the frame's animating budget, the blocks, textured quads and the scene's
    /// last state are drawn, its clock does not advance, and this returns `TooManyScenes`.
    pub(super) fn native_draw_scene(
        &self,
        batch: &mut HudBatch,
        blocks: &SceneBlocks<'_, '_>,
        scene: &Scene,
        frame: SceneFrame,
    ) -> Result<(), SceneError> {
        if !self.native_window() {
            return Err(SceneError::Unavailable);
        }
        let ordinary = lives(scene);
        let can_tick = scene.clock.is_some()
            && mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySceneTick);
        if !ordinary && !can_tick {
            return self.draw_scene_quads(batch, blocks, frame, None, |_| {});
        }
        let owner = mod_api::content::crash::current_native_mod().unwrap_or("");
        let mut scenes = self
            .live_scenes
            .try_borrow_mut()
            .map_err(|_| SceneError::Unavailable)?;
        let at = scene.clock.and_then(|clock| {
            scenes
                .live
                .iter()
                .position(|l| l.owner == owner && l.id == clock.id)
        });
        let key = scene_key(blocks.cells);
        let matching = at.filter(|&i| {
            let l = &scenes.live[i];
            l.key == key
                && (!can_tick || same_block_entities(blocks.cells, &l.block_entities))
                && l.entities
                    .iter()
                    .eq(scene.entities.iter().map(|e| &e.entity))
        });
        if !ordinary {
            let ticking = matching.map_or_else(
                || {
                    blocks.cells.iter().filter_map(|b| b.entity).any(|entity| {
                        mod_api::__private::scene_block_entity_ticker(&entity.id.to_string())
                            .is_some()
                    })
                },
                |i| {
                    scenes.live[i]
                        .block_entities
                        .iter()
                        .any(|entry| entry.ticking.is_some())
                },
            );
            if !ticking {
                self.draw_scene_quads(batch, blocks, frame, None, |_| {})?;
                // Removing a ticking input retires its previous animation, even when the
                // replacement no longer has anything that animates.
                if let Some(i) = at {
                    scenes.live.swap_remove(i);
                }
                return Ok(());
            }
        }
        let clock = check(frame.rect, scene, frame.view)?;
        // Checked whole before anything is kept; an empty list allocates nothing.
        let added = scene
            .particles
            .iter()
            .zip(0u32..)
            .map(|(p, index)| {
                p.options
                    .try_into_engine(&())
                    .map(|o| (o, p.pos, p.velocity))
                    .map_err(|_| SceneError::BadParticle { index })
            })
            .collect::<Result<Vec<(ParticleOptions, _, _)>, _>>()?;
        // The client's entities as they are this frame, refused before anything is kept.
        let mut existing = Vec::new();
        for (index, e) in (0u32..).zip(&scene.entities) {
            if let GuiEntity::Existing(_) = e.entity {
                let mob = self
                    .prepare_gui_entity(&e.entity)
                    .map_err(|error| entity_refusal(error, index))?;
                existing.push((index as usize, mob));
            }
        }
        let matching = matching.filter(|&i| clock.tick >= scenes.live[i].tick);
        let frame_id = self.frames;
        if !scenes.admit(frame_id, owner, clock.id) {
            // Still draw the fixed geometry, and the last state when it is this scene's.
            match matching {
                Some(i) => {
                    let l = &mut scenes.live[i];
                    l.frame = frame_id;
                    self.draw_live(batch, blocks, frame, l, clock.partial)?;
                }
                None => self.draw_scene_quads(batch, blocks, frame, None, |_| {})?,
            }
            return Err(SceneError::TooManyScenes);
        }
        let i = if let Some(i) = matching {
            i
        } else {
            let made = self.live_scene(blocks.cells, scene, owner, clock, key, can_tick)?;
            match at {
                Some(i) => {
                    scenes.live[i] = made;
                    i
                }
                None => {
                    if scenes.live.len() == KEPT
                        && let Some(oldest) = (0..KEPT).min_by_key(|&i| scenes.live[i].frame)
                    {
                        scenes.live.swap_remove(oldest);
                    }
                    scenes.live.push(made);
                    scenes.live.len() - 1
                }
            }
        };
        let l = &mut scenes.live[i];
        l.frame = frame_id;
        for (index, mob) in existing {
            l.mobs[index] = mob;
        }
        self.advance_live_scene(l, blocks.cells, clock, &added);
        for (mob, e) in l.mobs.iter_mut().zip(&scene.entities) {
            pose(mob, e, clock);
        }
        self.draw_live(batch, blocks, frame, l, clock.partial)
    }

    /// `l`'s blocks, entities and particles as they are, `partial` into the next tick.
    fn draw_live(
        &self,
        batch: &mut HudBatch,
        blocks: &SceneBlocks<'_, '_>,
        frame: SceneFrame,
        l: &mut Live,
        partial: f32,
    ) -> Result<(), SceneError> {
        let Live {
            tick,
            block_entities,
            engine,
            mobs,
            verts,
            ..
        } = l;
        let view = frame.view;
        self.draw_scene_quads(
            batch,
            blocks,
            frame,
            Some((*tick, partial, block_entities)),
            |quads| {
                for mob in mobs.iter() {
                    verts.clear();
                    // `DrawContext::entity`'s submission, in the scene's coordinates.
                    self.gui_entity_geometry_at(mob, partial, DVec3::ZERO, verts);
                    entity_quads(verts, quads);
                }
                verts.clear();
                let base = facing_viewer(view);
                engine.render(DVec3::ZERO, base, partial, &|_| 0xFF, verts);
                for q in verts.as_chunks::<4>().0 {
                    quads.push(SceneQuad {
                        corners: q.each_ref().map(|v| Vec3::from_array(v.pos)),
                        uv: q.each_ref().map(|v| v.uv),
                        color: q[0].color,
                        normal: Vec3::Z,
                        cull: false,
                    });
                }
            },
        )
    }

    /// A new kept scene: a level of `blocks`, an empty particle engine on the GUI atlas
    /// held to `SCENE_MAX_PARTICLES`, and `scene`'s entities prepared as
    /// `DrawContext::entity` prepares them (made for drawing only, nothing joins a level).
    fn live_scene(
        &self,
        blocks: &[SceneBlock<'_>],
        scene: &Scene,
        owner: &'static str,
        clock: SceneClock,
        key: u64,
        can_tick: bool,
    ) -> Result<Live, SceneError> {
        let mut mobs = Vec::with_capacity(scene.entities.len());
        for (index, e) in (0u32..).zip(&scene.entities) {
            let mob = self
                .prepare_gui_entity(&e.entity)
                .map_err(|error| entity_refusal(error, index))?;
            mobs.push(mob);
        }
        let mut world = World::new(self.res.facts().air, 0);
        let mut picks = Vec::new();
        let near = |v: i32| f64::from(v).abs() <= FAR;
        for b in blocks {
            if !(near(b.pos[0]) && near(b.pos[2])) {
                continue;
            }
            let at = (b.pos[0] >> 4, b.pos[2] >> 4);
            if world.chunks.get(&at).is_none() {
                world.chunks.insert(at, Arc::new(Chunk::empty(at.0, at.1)));
            }
            // Outside the level's height a block draws but has no animate tick.
            if world.set_stored(b.pos[0], b.pos[1], b.pos[2], b.state) {
                picks.push(b.pos);
            }
        }
        let mut engine = ParticleEngine::new(
            &self.pack.source,
            &self.gui.atlas.sprites,
            &self.gui.atlas.translucent,
        );
        engine.live_limit = Some(SCENE_MAX_PARTICLES);
        // The spawn limiter's camera at the blocks' middle: the view never changes what lives.
        if !picks.is_empty() {
            let n = picks.len() as f64;
            let sum = picks.iter().fold([0.0; 3], |s, p| {
                [
                    s[0] + f64::from(p[0]),
                    s[1] + f64::from(p[1]),
                    s[2] + f64::from(p[2]),
                ]
            });
            engine.camera = sum.map(|v| v / n + 0.5);
        }
        Ok(Live {
            owner,
            id: clock.id,
            key,
            entities: scene.entities.iter().map(|e| e.entity.clone()).collect(),
            block_entities: if can_tick {
                blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| b.entity.map(|entity| LiveBlockEntity::new(i, entity)))
                    .collect()
            } else {
                Vec::new()
            },
            tick: clock.tick,
            frame: self.frames,
            world,
            picks,
            engine,
            mobs,
            verts: Vec::new(),
        })
    }

    /// Runs `l`'s ticks up to `clock.tick` as the client's tick runs them: the blocks'
    /// animate ticks (`SCENE_ANIMATE_PICKS` picks among the scene's own blocks), scene-only
    /// block entity ticks, then the particles. A jump past `SCENE_CATCH_UP` resets the
    /// block entity fields to their seeds and clears particles before running the last
    /// `SCENE_CATCH_UP` ticks. The first tick also adds `added`. Particles' sounds are dropped.
    fn advance_live_scene(
        &self,
        l: &mut Live,
        blocks: &[SceneBlock<'_>],
        clock: SceneClock,
        added: &[(ParticleOptions, [f64; 3], [f64; 3])],
    ) {
        let Some((restart, first, last)) = ticks_to_run(l.tick, clock.tick) else {
            return;
        };
        let Live {
            block_entities,
            world,
            engine,
            picks,
            ..
        } = l;
        if restart {
            engine.clear();
            for entry in &mut *block_entities {
                if let Some((_, entity)) = &mut entry.ticking {
                    **entity = entry.seed.fields.clone();
                }
            }
        }
        engine.particle_level = self.settings.particles;
        let facts = self.res.facts();
        let tables = &self.pack.context.tables;
        let model_offset = |state, x, z| tables.offset(state, x, z);
        let ctx = AnimateContext {
            enchant_power: &self.ambience.enchant_power,
            world,
            facts,
            collision: self.collision,
            biomes: world_biomes!(self),
            registry: self.pack.assets.registry,
            raining_at: &|_| false,
            height_no_leaves: &|_| i32::MIN,
            firefly_bush_sounds: false,
            creaking_active: false,
            sky_darken: 0,
            marker: None,
            model_offset: &model_offset,
            gateway_particles: &|_, _| 0,
            thundering: false,
            ultrawarm: false,
        };
        for tick in first..=last {
            let seed = tick_seed(clock.id, tick);
            engine.seed_randoms(seed, seed.rotate_left(21), seed.rotate_left(42));
            engine.seed_particle_randoms(seed.rotate_left(53));
            if tick == first {
                for (options, at, velocity) in added {
                    engine.add(options, *at, *velocity);
                }
            }
            if clock.animate_blocks {
                animate_scene(self.res.animate(), engine, &ctx, picks, SCENE_ANIMATE_PICKS);
            }
            for entry in &mut *block_entities {
                if let Some((kind, entity)) = &mut entry.ticking {
                    let block = &blocks[entry.index];
                    Self::native_block_entity_scene_tick(
                        *kind,
                        block.pos,
                        block.state,
                        tick,
                        entity,
                    );
                }
            }
            engine.tick(&ParticleLevel {
                world,
                facts,
                collision: self.collision,
                player: None,
                cached_chunk: Cell::new(None),
            });
            engine.sounds.clear();
        }
        l.tick = clock.tick;
    }
}

/// `mob` placed and turned as `e` says, at the clock's tick (its age, which idle
/// animations read): no tick moves it.
fn pose(mob: &mut Mob, e: &mod_api::SceneEntity, clock: SceneClock) {
    let [x, y, z] = e.pos;
    mob.position = Position::new(x, y, z);
    mob.previous = mob.position;
    (mob.yaw, mob.previous_yaw) = (e.yaw, e.yaw);
    (mob.body_yaw, mob.previous_body_yaw) = (e.yaw, e.yaw);
    (mob.head_yaw, mob.previous_head_yaw) = (e.head_yaw, e.head_yaw);
    (mob.head_pitch, mob.previous_head_pitch) = (e.head_pitch, e.head_pitch);
    mob.age = clock.tick as u32;
}

/// An entity's quads as scene quads: shaded by face as the scene's blocks are, the
/// faces looking away dropped.
fn entity_quads(verts: &[Vertex], quads: &mut Vec<SceneQuad>) {
    for q in verts.as_chunks::<4>().0 {
        let corners = q.each_ref().map(|v| Vec3::from_array(v.pos));
        let normal = (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .normalize_or_zero();
        quads.push(SceneQuad {
            corners,
            uv: q.each_ref().map(|v| v.uv),
            color: shaded(q[0].color, face_shade(normal)),
            normal,
            cull: normal != Vec3::ZERO,
        });
    }
}

#[cfg(test)]
impl Game<'_> {
    /// The live particles of the mod's scene `id`: positions, previous positions and ages.
    pub(super) fn scene_particles(&self, owner: &str, id: u32) -> Vec<([f64; 3], [f64; 3], i32)> {
        let scenes = self.live_scenes.borrow();
        scenes
            .live
            .iter()
            .find(|l| l.owner == owner && l.id == id)
            .map(|l| {
                l.engine
                    .live()
                    .map(|p| (p.position, p.previous, p.age))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// The tick the mod's scene `id` last ran, when it is kept.
    pub(super) fn scene_tick(&self, owner: &str, id: u32) -> Option<u64> {
        let scenes = self.live_scenes.borrow();
        scenes
            .live
            .iter()
            .find(|l| l.owner == owner && l.id == id)
            .map(|l| l.tick)
    }
}
