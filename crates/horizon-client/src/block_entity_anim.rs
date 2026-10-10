//! Client-ticked block-entity animation state, as vanilla's block entities
//! keep it: `ShulkerBoxBlockEntity` (open count, animation status, progress),
//! `BellBlockEntity` (ring ticks, shake direction, resonation),
//! `ConduitBlockEntity.clientTick` (tick count, shape check, rotation,
//! hunting, nautilus particles), `SkullBlockEntity.animation` (powered dragon
//! and piglin heads) and `DecoratedPotBlockEntity` wobbles.
//!
//! [`BlockEntityAnims`] holds the state per position and turns it into the
//! renderer's [`BlockAnim`]. Its tick methods take what the level answers
//! (blocks, entities, game time) as closures, so the parity fixture can drive
//! them with scripted inputs (`tools/test_block_entity_anim_parity.py`).
use crate::block_entity_render::BlockAnim;
use horizon_core::{
    geometry::trig::Mth,
    random::{LegacyRandom, RandomSource},
};
use std::collections::HashMap;

/// `Direction` names by `get3DDataValue`.
pub const DIRECTIONS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

fn step(direction: u8) -> [i32; 3] {
    match direction {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

/// `Direction.from3DDataValue`: `BY_3D_DATA[Mth.abs(data % 6)]`.
pub fn from_3d_data_value(data: i32) -> u8 {
    (data % 6).unsigned_abs() as u8
}

/// The 3D data value of a direction name.
pub fn direction_index(name: &str) -> Option<u8> {
    DIRECTIONS.iter().position(|d| *d == name).map(|i| i as u8)
}

/// `ShulkerBoxBlockEntity.AnimationStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShulkerStatus {
    #[default]
    Closed,
    Opening,
    Opened,
    Closing,
}

/// `ShulkerBoxBlockEntity` lid state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shulker {
    pub open_count: i32,
    pub status: ShulkerStatus,
    pub progress: f32,
    pub progress_old: f32,
}

/// Side effects of one [`Shulker::tick`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShulkerTick {
    /// `doNeighborUpdates` calls (shape and neighbour updates around the box).
    pub neighbor_updates: u32,
    /// `moveCollidedEntities`: the swept lid box (min x, y, z, max x, y, z);
    /// entities in it move out along the facing.
    pub push: Option<[f64; 6]>,
}

/// `Shulker.getProgressDeltaAabb(size, direction, from, to, position)`.
pub fn progress_delta_aabb(
    size: f32,
    direction: u8,
    from: f32,
    to: f32,
    position: [f64; 3],
) -> [f64; 6] {
    let size_d = f64::from(size);
    let mut b = [
        -size_d * 0.5,
        0.0,
        -size_d * 0.5,
        size_d * 0.5,
        size_d,
        size_d * 0.5,
    ];
    let max = f64::from(from.max(to));
    let min = f64::from(from.min(to));
    let s = step(direction);
    for axis in 0..3 {
        // expandTowards
        let a = f64::from(s[axis]) * max * size_d;
        if a < 0.0 {
            b[axis] += a;
        } else if a > 0.0 {
            b[axis + 3] += a;
        }
    }
    for axis in 0..3 {
        // contract
        let a = -f64::from(s[axis]) * (1.0 + min) * size_d;
        if a < 0.0 {
            b[axis] -= a;
        } else if a > 0.0 {
            b[axis + 3] -= a;
        }
    }
    // new AABB keeps min <= max; then move to the position.
    let mut out = [0.0; 6];
    for axis in 0..3 {
        out[axis] = b[axis].min(b[axis + 3]) + position[axis];
        out[axis + 3] = b[axis].max(b[axis + 3]) + position[axis];
    }
    out
}

impl Shulker {
    /// `triggerEvent(1, openCount)`.
    pub fn trigger_event(&mut self, b0: i32, b1: i32) -> bool {
        if b0 != 1 {
            return false;
        }
        self.open_count = b1;
        if b1 == 0 {
            self.status = ShulkerStatus::Closing;
        }
        if b1 == 1 {
            self.status = ShulkerStatus::Opening;
        }
        true
    }
    /// `startOpen`/`stopOpen` for a non-spectator user: the new open count to
    /// send as block event 1, and whether this was the first opener (open) or
    /// the last closer (close), which play the sound and game event.
    pub fn start_open(&mut self) -> (i32, bool) {
        if self.open_count < 0 {
            self.open_count = 0;
        }
        self.open_count += 1;
        (self.open_count, self.open_count == 1)
    }
    pub fn stop_open(&mut self) -> (i32, bool) {
        self.open_count -= 1;
        (self.open_count, self.open_count <= 0)
    }
    /// `updateAnimation` (`ShulkerBoxBlockEntity.tick`); `facing` is the
    /// block's facing as a 3D data value.
    pub fn tick(&mut self, pos: [i32; 3], facing: u8) -> ShulkerTick {
        let mut out = ShulkerTick::default();
        self.progress_old = self.progress;
        match self.status {
            ShulkerStatus::Closed => self.progress = 0.0,
            ShulkerStatus::Opening => {
                self.progress += 0.1;
                if self.progress_old == 0.0 {
                    out.neighbor_updates += 1;
                }
                if self.progress >= 1.0 {
                    self.status = ShulkerStatus::Opened;
                    self.progress = 1.0;
                    out.neighbor_updates += 1;
                }
                let bottom = [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]),
                    f64::from(pos[2]) + 0.5,
                ];
                out.push = Some(progress_delta_aabb(
                    1.0,
                    facing,
                    self.progress_old,
                    self.progress,
                    bottom,
                ));
            }
            ShulkerStatus::Opened => self.progress = 1.0,
            ShulkerStatus::Closing => {
                self.progress -= 0.1;
                if self.progress_old == 1.0 {
                    out.neighbor_updates += 1;
                }
                if self.progress <= 0.0 {
                    self.status = ShulkerStatus::Closed;
                    self.progress = 0.0;
                    out.neighbor_updates += 1;
                }
            }
        }
        out
    }
    /// `getProgress(a)`: `Mth.lerp(a, progressOld, progress)`.
    pub fn get_progress(&self, a: f32) -> f32 {
        self.progress_old + a * (self.progress - self.progress_old)
    }
    /// `ShulkerBoxBlock.canOpen`: a closed box only opens when its lid has room
    /// (`level.noCollision` of the half-block lid box, deflated by 1e-6).
    pub fn lid_box(pos: [i32; 3], facing: u8) -> [f64; 6] {
        let bottom = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let mut b = progress_delta_aabb(1.0, facing, 0.0, 0.5, bottom);
        for axis in 0..3 {
            b[axis] += 1.0e-6;
            b[axis + 3] -= 1.0e-6;
        }
        b
    }
}

/// What the bell's tick asks about an entity it gathered when rung.
#[derive(Clone, Copy, Debug)]
pub struct BellEntity {
    pub position: [f64; 3],
    /// `isAlive() && !isRemoved()`.
    pub alive: bool,
    /// In `EntityTypeTags.RAIDERS`.
    pub raider: bool,
}

/// `BellBlockEntity`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bell {
    pub ticks: i32,
    pub shaking: bool,
    /// `clickDirection` as a 3D data value.
    pub click_direction: Option<u8>,
    pub last_ring_timestamp: i64,
    /// `nearbyEntities`: the living entities around the bell when it was last
    /// gathered, as caller-defined handles.
    pub nearby: Option<Vec<u64>>,
    pub resonating: bool,
    pub resonation_ticks: i32,
}

/// Side effects of one [`Bell::tick`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BellTick {
    /// Resonation started: `BELL_RESONATE` (played by the server).
    pub resonate: bool,
    /// Resonation ended (`ResonationEndAction`): the client's
    /// `showBellParticles` entity-effect particles (position and colour) and
    /// the server's `makeRaidersGlow` targets.
    pub ended: Option<(Vec<([f64; 3], i32)>, Vec<u64>)>,
}

fn closer_to_center_than(pos: [i32; 3], p: [f64; 3], distance: f64) -> bool {
    let dx = f64::from(pos[0]) + 0.5 - p[0];
    let dy = f64::from(pos[1]) + 0.5 - p[1];
    let dz = f64::from(pos[2]) + 0.5 - p[2];
    dx * dx + dy * dy + dz * dz < distance * distance
}

impl Bell {
    /// `triggerEvent(1, direction)`. `gather` lists the living entities whose
    /// position is inside the bell's block box inflated by 48
    /// (`updateEntities`, refreshed at most every 60 ticks).
    pub fn trigger_event(
        &mut self,
        b0: i32,
        b1: i32,
        game_time: i64,
        gather: impl FnOnce() -> Vec<u64>,
    ) -> bool {
        if b0 != 1 {
            return false;
        }
        if game_time > self.last_ring_timestamp + 60 || self.nearby.is_none() {
            self.last_ring_timestamp = game_time;
            self.nearby = Some(gather());
        }
        self.resonation_ticks = 0;
        self.click_direction = Some(from_3d_data_value(b1));
        self.ticks = 0;
        self.shaking = true;
        true
    }
    /// `BellBlockEntity.tick` with the client's resonation end action.
    pub fn tick(&mut self, pos: [i32; 3], entity: impl Fn(u64) -> Option<BellEntity>) -> BellTick {
        let mut out = BellTick::default();
        if self.shaking {
            self.ticks += 1;
        }
        if self.ticks >= 50 {
            self.shaking = false;
            self.ticks = 0;
        }
        let nearby = self.nearby.clone().unwrap_or_default();
        let raider_in = |e: &BellEntity, distance: f64| {
            e.alive && closer_to_center_than(pos, e.position, distance) && e.raider
        };
        if self.ticks >= 5
            && self.resonation_ticks == 0
            && nearby
                .iter()
                .filter_map(|&h| entity(h))
                .any(|e| raider_in(&e, 32.0))
        {
            self.resonating = true;
            out.resonate = true;
        }
        if self.resonating {
            if self.resonation_ticks < 40 {
                self.resonation_ticks += 1;
            } else {
                // showBellParticles / makeRaidersGlow.
                let mut color = 16_700_985;
                let views: Vec<(u64, BellEntity)> = nearby
                    .iter()
                    .filter_map(|&h| Some((h, entity(h)?)))
                    .collect();
                let count = views
                    .iter()
                    .filter(|(_, e)| closer_to_center_than(pos, e.position, 48.0))
                    .count() as i32;
                let per = ((count - 21) / -2).clamp(3, 15);
                let mut particles = Vec::new();
                let mut glow = Vec::new();
                for (h, e) in &views {
                    if !raider_in(e, 48.0) {
                        continue;
                    }
                    glow.push(*h);
                    let (bx, bz) = (f64::from(pos[0]), f64::from(pos[2]));
                    let dist = ((e.position[0] - bx) * (e.position[0] - bx)
                        + (e.position[2] - bz) * (e.position[2] - bz))
                        .sqrt();
                    let x3 = bx + f64::from(0.5f32) + 1.0 / dist * (e.position[0] - bx);
                    let z3 = bz + f64::from(0.5f32) + 1.0 / dist * (e.position[2] - bz);
                    for _ in 0..per {
                        color += 5;
                        particles.push(([x3, f64::from(pos[1]) + 0.5, z3], color));
                    }
                }
                out.ended = Some((particles, glow));
                self.resonating = false;
            }
        }
        out
    }
}

/// The conduit's `destroyTarget` as the client sees it.
#[derive(Clone, Copy, Debug)]
pub struct ConduitTarget {
    pub position: [f64; 3],
    pub eye_y: f64,
    pub width: f32,
    pub height: f32,
}

/// `ConduitBlockEntity` (client side).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conduit {
    pub tick_count: i32,
    pub active_rotation: f32,
    pub is_active: bool,
    pub is_hunting: bool,
    pub effect_blocks: Vec<[i32; 3]>,
}

/// One nautilus particle: position and velocity.
pub type Particle = ([f64; 3], [f64; 3]);

impl Conduit {
    /// `updateShape`: water all around, then the frame blocks
    /// (`BlockTags.CONDUIT_EFFECT_BLOCK`) in the three 5x5 rings.
    fn update_shape(
        &mut self,
        pos: [i32; 3],
        is_water: &impl Fn([i32; 3]) -> bool,
        is_frame: &impl Fn([i32; 3]) -> bool,
    ) -> bool {
        self.effect_blocks.clear();
        for ox in -1..=1 {
            for oy in -1..=1 {
                for oz in -1..=1 {
                    if !is_water([pos[0] + ox, pos[1] + oy, pos[2] + oz]) {
                        return false;
                    }
                }
            }
        }
        for ox in -2i32..=2 {
            for oy in -2i32..=2 {
                for oz in -2i32..=2 {
                    let (ax, ay, az) = (ox.abs(), oy.abs(), oz.abs());
                    if (ax > 1 || ay > 1 || az > 1)
                        && (ox == 0 && (ay == 2 || az == 2)
                            || oy == 0 && (ax == 2 || az == 2)
                            || oz == 0 && (ax == 2 || ay == 2))
                    {
                        let p = [pos[0] + ox, pos[1] + oy, pos[2] + oz];
                        if is_frame(p) {
                            self.effect_blocks.push(p);
                        }
                    }
                }
            }
        }
        self.effect_blocks.len() >= 16
    }
    /// `ConduitBlockEntity.clientTick`: returns the nautilus particles of
    /// `animationTick`, drawn from the client level's random.
    pub fn client_tick(
        &mut self,
        pos: [i32; 3],
        game_time: i64,
        is_water: impl Fn([i32; 3]) -> bool,
        is_frame: impl Fn([i32; 3]) -> bool,
        target: Option<ConduitTarget>,
        random: &mut impl RandomSource,
    ) -> Vec<Particle> {
        self.tick_count += 1;
        if game_time % 40 == 0 {
            self.is_active = self.update_shape(pos, &is_water, &is_frame);
            self.is_hunting = self.effect_blocks.len() >= 42;
        }
        let particles = self.animation_tick(pos, target, random);
        if self.is_active {
            self.active_rotation += 1.0;
        }
        particles
    }
    fn animation_tick(
        &self,
        pos: [i32; 3],
        target: Option<ConduitTarget>,
        random: &mut impl RandomSource,
    ) -> Vec<Particle> {
        let mut out = Vec::new();
        let hh = f64::from(((self.tick_count + 35) as f32 * 0.1).msin() / 2.0 + 0.5);
        let hh = (hh * hh + hh) * f64::from(0.3f32);
        let end = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.5 + hh,
            f64::from(pos[2]) + 0.5,
        ];
        for p in &self.effect_blocks {
            if random.next_int_bound(50).unwrap_or(1) == 0 {
                let dx = -0.5 + random.next_float() + (p[0] - pos[0]) as f32;
                let dy = -2.0 + random.next_float() + (p[1] - pos[1]) as f32;
                let dz = -0.5 + random.next_float() + (p[2] - pos[2]) as f32;
                out.push((end, [f64::from(dx), f64::from(dy), f64::from(dz)]));
            }
        }
        if let Some(t) = target {
            let randx = (-0.5 + random.next_float()) * (3.0 + t.width);
            let randy = -1.0 + random.next_float() * t.height;
            let randz = (-0.5 + random.next_float()) * (3.0 + t.width);
            out.push((
                [t.position[0], t.eye_y, t.position[2]],
                [f64::from(randx), f64::from(randy), f64::from(randz)],
            ));
        }
        out
    }
}

/// `SkullBlockEntity` animation (client ticker of dragon and piglin heads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Skull {
    pub animating: bool,
    pub tick_count: i32,
}
impl Skull {
    /// `SkullBlockEntity.animation`.
    pub fn tick(&mut self, powered: bool) {
        if powered {
            self.animating = true;
            self.tick_count += 1;
        } else {
            self.animating = false;
        }
    }
    /// `getAnimation(a)`.
    pub fn get_animation(&self, a: f32) -> f32 {
        if self.animating {
            self.tick_count as f32 + a
        } else {
            self.tick_count as f32
        }
    }
}

/// `DecoratedPotBlockEntity.WobbleStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WobbleStyle {
    Positive,
    Negative,
}
impl WobbleStyle {
    pub fn duration(self) -> i32 {
        match self {
            Self::Positive => 7,
            Self::Negative => 10,
        }
    }
}

/// `DecoratedPotBlockEntity` wobble fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PotWobble {
    pub started_at: i64,
    pub style: Option<WobbleStyle>,
}
impl PotWobble {
    /// `triggerEvent(1, style)`.
    pub fn trigger_event(&mut self, event: i32, data: i32, game_time: i64) -> bool {
        if event == 1 && (0..2).contains(&data) {
            self.started_at = game_time;
            self.style = Some(if data == 0 {
                WobbleStyle::Positive
            } else {
                WobbleStyle::Negative
            });
            true
        } else {
            false
        }
    }
    /// `DecoratedPotRenderer.extractRenderState`'s `wobbleProgress`.
    pub fn progress(&self, game_time: i64, partial: f32) -> f32 {
        match self.style {
            Some(style) => {
                ((game_time - self.started_at) as f32 + partial) / style.duration() as f32
            }
            None => 0.0,
        }
    }
}

/// The animated block entities of a level, by position.
#[derive(Clone, Debug)]
pub struct BlockEntityAnims {
    pub shulkers: HashMap<[i32; 3], Shulker>,
    pub bells: HashMap<[i32; 3], Bell>,
    pub conduits: HashMap<[i32; 3], Conduit>,
    pub skulls: HashMap<[i32; 3], Skull>,
    pub pots: HashMap<[i32; 3], PotWobble>,
    /// The client level's random (conduit particles).
    pub random: LegacyRandom,
}
impl Default for BlockEntityAnims {
    fn default() -> Self {
        Self {
            shulkers: HashMap::new(),
            bells: HashMap::new(),
            conduits: HashMap::new(),
            skulls: HashMap::new(),
            pots: HashMap::new(),
            random: LegacyRandom::new(0x6c6576656c),
        }
    }
}
impl BlockEntityAnims {
    /// Drops the state of positions whose block entity is gone.
    pub fn retain(&mut self, keep: impl Fn([i32; 3]) -> bool) {
        self.shulkers.retain(|p, _| keep(*p));
        self.bells.retain(|p, _| keep(*p));
        self.conduits.retain(|p, _| keep(*p));
        self.skulls.retain(|p, _| keep(*p));
        self.pots.retain(|p, _| keep(*p));
    }
    /// The renderer's view of the block entity at `pos`; `sherds` are the
    /// pot's decorations (back, left, right, front), when it is a pot.
    pub fn block_anim(
        &self,
        pos: [i32; 3],
        game_time: i64,
        partial: f32,
        sherds: [Option<String>; 4],
    ) -> BlockAnim {
        let mut anim = BlockAnim {
            sherds,
            ..BlockAnim::default()
        };
        if let Some(s) = self.shulkers.get(&pos) {
            anim.shulker = (s.progress, s.progress_old);
        }
        if let Some(b) = self.bells.get(&pos) {
            anim.bell_ticks = b.ticks;
            anim.bell_shake = b
                .shaking
                .then_some(b.click_direction)
                .flatten()
                .map(|d| DIRECTIONS[usize::from(d)]);
        }
        if let Some(c) = self.conduits.get(&pos) {
            anim.conduit = (c.tick_count, c.is_active, c.is_hunting, c.active_rotation);
        }
        if let Some(s) = self.skulls.get(&pos) {
            anim.skull = (s.animating, s.tick_count);
        }
        if let Some(w) = self.pots.get(&pos)
            && w.style.is_some()
        {
            // extractRenderState never copies lastWobbleStyle into the render
            // state, so DecoratedPotRenderer.submit always takes the
            // non-POSITIVE branch (with the style's own duration).
            anim.wobble = Some((false, w.progress(game_time, partial)));
        }
        anim
    }
}
