//! The client side of animated block entities (`horizon_client::block_entity_anim`):
//! their client tickers (`Level.tickBlockEntities` on the client level) and
//! the block events that drive them (`ClientboundBlockEventPacket` reaching
//! `BlockEntity.triggerEvent`). The renderer reads the resulting state through
//! [`Game::block_anim`] and [`Game::chest_lid`].
use super::Game;
use horizon_client::{
    block_entity_anim::{BellEntity, Ticker},
    block_entity_render::BlockAnim,
    particles::{ParticleOptions, ParticleType},
};
use std::collections::HashMap;

/// The handle of the local player among a bell's gathered entities.
const PLAYER: u64 = u64::MAX;

impl Game<'_> {
    /// `BlockEntity.triggerEvent(a, b)` on the client copy of the block entity at `pos`.
    pub(super) fn block_entity_event(&mut self, pos: [i32; 3], a: i32, b: i32) {
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        let time = self.world.game_time;
        match self.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                self.be_anims
                    .chests
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b);
            }
            Some(Ticker::Shulker { .. }) => {
                self.be_anims
                    .shulkers
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b);
            }
            Some(Ticker::Bell) => {
                let gathered = self.bell_gather(pos);
                self.be_anims
                    .bells
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b, time, || gathered);
            }
            Some(Ticker::Pot) => {
                self.be_anims
                    .pots
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b, time);
            }
            _ => {}
        }
    }
    /// A player opened (`open`) or closed the container at `key`: the opener
    /// count block event (`ContainerOpenersCounter` / `ShulkerBoxBlockEntity`
    /// `startOpen`/`stopOpen`) for chests, ender chests and shulker boxes,
    /// on both halves of a double chest (`CompoundContainer`).
    pub(super) fn container_opener_event(&mut self, key: (i32, i32, i32), open: bool) {
        // Spectators never count as openers.
        if self.spectator() {
            return;
        }
        let pos = [key.0, key.1, key.2];
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        match self.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                let count = i32::from(open);
                self.block_entity_event(pos, 1, count);
                if let Some(other) = self.chest_other_half(key) {
                    self.block_entity_event([other.0, other.1, other.2], 1, count);
                }
            }
            Some(Ticker::Shulker { .. }) => {
                let shulker = self.be_anims.shulkers.entry(pos).or_default();
                let (count, _) = if open {
                    shulker.start_open()
                } else {
                    shulker.stop_open()
                };
                self.block_entity_event(pos, 1, count);
            }
            _ => {}
        }
    }
    /// `BellBlockEntity.updateEntities`: the living entities whose bounding
    /// box meets the bell's block box inflated by 48.
    fn bell_gather(&self, pos: [i32; 3]) -> Vec<u64> {
        let lo = [
            f64::from(pos[0]) - 48.0,
            f64::from(pos[1]) - 48.0,
            f64::from(pos[2]) - 48.0,
        ];
        let hi = [
            lo[0] + 49.0 + 48.0,
            lo[1] + 49.0 + 48.0,
            lo[2] + 49.0 + 48.0,
        ];
        let meets = |p: [f64; 3], width: f64, height: f64| {
            let (bl, bh) = (
                [p[0] - width / 2.0, p[1], p[2] - width / 2.0],
                [p[0] + width / 2.0, p[1] + height, p[2] + width / 2.0],
            );
            (0..3).all(|a| bl[a] < hi[a] && bh[a] > lo[a])
        };
        let mut out = Vec::new();
        if let Some(p) = &self.player
            && meets([p.position.x, p.position.y, p.position.z], 0.6, 1.8)
        {
            out.push(PLAYER);
        }
        for m in &self.mobs {
            let p = [m.position.x, m.position.y, m.position.z];
            if meets(p, m.kind.width, m.kind.height) {
                out.push(u64::from(m.id));
            }
        }
        out
    }
    /// The client tickers of the loaded animated block entities, once per
    /// client tick (`ClientLevel.tickBlockEntities`).
    pub(super) fn tick_block_entity_anims(&mut self) {
        let Some(r) = &self.renderer else {
            return;
        };
        let present: HashMap<[i32; 3], Ticker> = r
            .block_entities()
            .filter_map(|(p, s)| Some((*p, self.be_anims.ticker(*s)?)))
            .collect();
        self.be_anims.retain_present(&present);
        let players: Vec<[f64; 3]> = match &self.player {
            Some(p) if !self.spectator() => vec![[p.position.x, p.position.y, p.position.z]],
            _ => Vec::new(),
        };
        let raiders = self
            .res
            .stations
            .enchantments
            .entity_tags
            .get("minecraft:raiders")
            .cloned()
            .unwrap_or_default();
        let frames = self
            .res
            .items
            .block_tags
            .get("minecraft:conduit_effect_block")
            .cloned()
            .unwrap_or_default();
        let mut positions: Vec<([i32; 3], Ticker)> = present.into_iter().collect();
        positions.sort_unstable_by_key(|(p, _)| *p);
        for (pos, ticker) in positions {
            match ticker {
                Ticker::Lid => self.be_anims.chests.entry(pos).or_default().tick(),
                Ticker::Book => {
                    let anims = &mut self.be_anims;
                    anims
                        .books
                        .entry(pos)
                        .or_default()
                        .tick(pos, &players, &mut anims.book_random);
                }
                Ticker::Shulker { facing } => {
                    // The lid's push of entities in its way (moveCollidedEntities)
                    // and the neighbour updates are left to the level.
                    self.be_anims
                        .shulkers
                        .entry(pos)
                        .or_default()
                        .tick(pos, facing);
                }
                Ticker::Skull { powered } => {
                    self.be_anims.skulls.entry(pos).or_default().tick(powered);
                }
                Ticker::Bell => {
                    let player = self.player.as_ref().map(|p| BellEntity {
                        position: [p.position.x, p.position.y, p.position.z],
                        alive: true,
                        raider: false,
                    });
                    let mobs = &self.mobs;
                    let entity = |h: u64| {
                        if h == PLAYER {
                            return player;
                        }
                        let m = mobs.iter().find(|m| u64::from(m.id) == h)?;
                        Some(BellEntity {
                            position: [m.position.x, m.position.y, m.position.z],
                            alive: m.health > 0.0,
                            raider: raiders.contains(&m.kind.id),
                        })
                    };
                    let tick = self
                        .be_anims
                        .bells
                        .entry(pos)
                        .or_default()
                        .tick(pos, entity);
                    let centre = [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]) + 0.5,
                        f64::from(pos[2]) + 0.5,
                    ];
                    if tick.resonate {
                        // The server's copy plays the resonance for everyone.
                        self.audio
                            .play("minecraft:block.bell.resonate", Some(centre), 1.0, 1.0);
                    }
                    if let Some((particles, glow)) = tick.ended {
                        for (p, color) in particles {
                            // ColorParticleOption.create(ENTITY_EFFECT, color): ARGB.
                            let c = color as u32;
                            let channel = |shift: u32| ((c >> shift) & 255) as f32 / 255.0;
                            let options = ParticleOptions {
                                color: [channel(16), channel(8), channel(0), channel(24)],
                                ..ParticleOptions::simple(ParticleType::EntityEffect)
                            };
                            self.particles.add(&options, p, [0.0; 3]);
                        }
                        // makeRaidersGlow.
                        for m in self.mobs.iter_mut() {
                            if glow.contains(&u64::from(m.id)) {
                                m.add_effect("minecraft:glowing", 60, 0);
                            }
                        }
                    }
                }
                Ticker::Conduit => {
                    let world = &self.world;
                    let facts = &self.res.facts;
                    let registry = self.res.assets.registry;
                    let is_water = |p: [i32; 3]| {
                        facts.get(world.block(p[0], p[1], p[2])).fluid
                            == horizon_client::blocks::Fluid::Water
                    };
                    let is_frame = |p: [i32; 3]| {
                        registry
                            .state(world.block(p[0], p[1], p[2]))
                            .is_some_and(|s| frames.contains(&s.name))
                    };
                    let time = world.game_time;
                    let anims = &mut self.be_anims;
                    let particles = anims.conduits.entry(pos).or_default().client_tick(
                        pos,
                        time,
                        is_water,
                        is_frame,
                        None,
                        self.particles.random(),
                    );
                    let nautilus = ParticleOptions::simple(ParticleType::Nautilus);
                    for (p, v) in particles {
                        self.particles.add(&nautilus, p, v);
                    }
                }
                Ticker::Pot => {}
            }
        }
    }
    /// The renderer's view of the block entity at `pos`.
    pub(super) fn block_anim(&self, pos: [i32; 3], partial: f32) -> BlockAnim {
        self.be_anims
            .block_anim(pos, self.world.game_time, partial, Default::default())
    }
    /// `ChestLidController.getOpenness(partial)` at `pos`.
    pub(super) fn chest_lid(&self, pos: [i32; 3], partial: f32) -> f32 {
        self.be_anims.lid(pos, partial)
    }
}
