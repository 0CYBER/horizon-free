//! The client side of animated block entities (`horizon_client::block_entity_anim`):
//! their client tickers (`Level.tickBlockEntities` on the client level) and
//! the block events that drive them (`ClientboundBlockEventPacket` reaching
//! `BlockEntity.triggerEvent`). The renderer reads the resulting state through
//! [`Game::block_anim`] and [`Game::chest_lid`].
use super::Game;
use horizon_client::{
    block_entity_anim::{BellEntity, EndGateway, Shulker, ShulkerStatus, Ticker},
    block_entity_render::BlockAnim,
    collisions::{SceneCollisions, no_collision_empty},
    containers::BlockEntity,
    particles::{ParticleOptions, ParticleType},
    save::number,
    skins,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::nbt::Tag;

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
            Some(Ticker::Gateway) => {
                let initial = self.gateway_loaded(pos);
                self.be_anims
                    .gateways
                    .entry(pos)
                    .or_insert(initial)
                    .trigger_event(a, b);
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
        if !self.spectator() {
            self.container_openers_changed(key, open);
        }
    }
    /// `container_opener_event` for a player who is not a spectator (a guest).
    pub(super) fn container_openers_changed(&mut self, key: (i32, i32, i32), open: bool) {
        let pos = [key.0, key.1, key.2];
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        match self.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                self.lid_openers_changed(key, open);
                if let Some(other) = self.chest_other_half(key) {
                    self.lid_openers_changed(other, open);
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
                self.lan_block_event(pos, 1, count);
            }
            _ => {}
        }
    }
    /// `ContainerOpenersCounter.incrementOpeners` / `decrementOpeners` of the
    /// chest at `key` (the host and the guests are counted), then
    /// `ChestBlockEntity.signalOpenCount`: block event 1 with the count, here
    /// and for the guests.
    fn lid_openers_changed(&mut self, key: (i32, i32, i32), open: bool) {
        let count = self.lid_openers.entry(key).or_default();
        *count = (*count + if open { 1 } else { -1 }).max(0);
        let count = *count;
        if count == 0 {
            self.lid_openers.remove(&key);
        }
        let pos = [key.0, key.1, key.2];
        self.block_entity_event(pos, 1, count);
        self.lan_block_event(pos, 1, count);
    }
    /// `BellBlockEntity.updateEntities`: the living entities whose bounding
    /// box meets the bell's block box inflated by 48.
    fn bell_gather(&self, pos: [i32; 3]) -> Vec<u64> {
        let area = Aabb::new(
            Vec3::new(
                f64::from(pos[0]) - 48.0,
                f64::from(pos[1]) - 48.0,
                f64::from(pos[2]) - 48.0,
            ),
            Vec3::new(
                f64::from(pos[0]) + 49.0,
                f64::from(pos[1]) + 49.0,
                f64::from(pos[2]) + 49.0,
            ),
        );
        let mut out = Vec::new();
        if self
            .player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            out.push(PLAYER);
        }
        for m in &self.mobs {
            if !m.removed && m.bounds().intersects(area) {
                out.push(u64::from(m.id));
            }
        }
        out
    }
    /// `ShulkerBoxBlock.canOpen`: a closed box opens only when its lid has
    /// room to rise (`level.noCollision` of the half-open lid box).
    pub(super) fn shulker_can_open(&self, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]).0;
        let Some(Ticker::Shulker { facing }) = self.be_anims.ticker(state) else {
            return true;
        };
        let closed = self
            .be_anims
            .shulkers
            .get(&pos)
            .is_none_or(|s| s.status == ShulkerStatus::Closed);
        if !closed {
            return true;
        }
        let b = Shulker::lid_box(pos, facing);
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        no_collision_empty(
            &mut scene,
            Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])),
        )
    }
    /// The client tickers of the loaded animated block entities, once per
    /// client tick (`ClientLevel.tickBlockEntities`).
    pub(super) fn tick_block_entity_anims(&mut self) {
        let Some(r) = &self.renderer else {
            return;
        };
        // Sorted by position: the order they tick in, and a quick lookup.
        // Read again only when the drawn block entities changed.
        let revision = r.block_entity_revision();
        if self.be_anims.present_revision != Some(revision) {
            let mut present: Vec<([i32; 3], Ticker)> = r
                .block_entities()
                .filter_map(|(p, s)| Some((*p, self.be_anims.ticker(*s)?)))
                .collect();
            present.sort_unstable_by_key(|(p, _)| *p);
            self.be_anims.retain_present(&present);
            self.be_anims.present = present;
            self.be_anims.present_revision = Some(revision);
        }
        let positions = std::mem::take(&mut self.be_anims.present);
        let players: Vec<[f64; 3]> = match &self.player {
            Some(p) if !self.spectator() => vec![[p.position.x, p.position.y, p.position.z]],
            _ => Vec::new(),
        };
        let raiders = self
            .res
            .stations()
            .enchantments
            .entity_tags
            .get("minecraft:raiders")
            .cloned()
            .unwrap_or_default();
        let frames = self
            .res
            .items()
            .block_tags
            .get("minecraft:conduit_effect_block")
            .cloned()
            .unwrap_or_default();
        for &(pos, ticker) in &positions {
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
                    let player = self.player_view().map(|p| BellEntity {
                        position: [p.position.x, p.position.y, p.position.z],
                        alive: p.alive,
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
                            alive: m.alive() && !m.removed,
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
                    // On a server its own copy does this and sends the sound.
                    if tick.resonate && self.remote.is_none() {
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
                        // makeRaidersGlow (the server's effect).
                        for m in self.mobs.iter_mut().filter(|_| self.remote.is_none()) {
                            if glow.contains(&u64::from(m.id)) {
                                m.add_effect("minecraft:glowing", 60, 0);
                            }
                        }
                    }
                }
                Ticker::Conduit => {
                    let world = &self.world;
                    let facts = self.res.facts();
                    let registry = self.pack.assets.registry;
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
                Ticker::Gateway => {
                    // The server copy's portalTick runs first; its block event
                    // reaches the client copy before the client ticks it.
                    let initial = self.gateway_loaded(pos);
                    let anims = &mut self.be_anims;
                    let server = anims.gateway_servers.entry(pos).or_insert(initial);
                    // A server sends its copy's block event instead.
                    let triggered = self.remote.is_none() && server.portal_tick();
                    let client = anims.gateways.entry(pos).or_insert(initial);
                    if triggered {
                        client.trigger_event(1, 0);
                    }
                    client.tick();
                }
            }
        }
        self.be_anims.present = positions;
    }
    /// `EndGatewayBlock.entityInside` on the server: whether the gateway at
    /// `pos` let the entity in (`triggerCooldown`, with its block event).
    pub(super) fn gateway_enter(&mut self, pos: [i32; 3]) -> bool {
        let initial = self.gateway_loaded(pos);
        let entered = self
            .be_anims
            .gateway_servers
            .entry(pos)
            .or_insert(initial)
            .enter();
        if entered {
            self.block_entity_event(pos, 1, 0);
        }
        entered
    }
    /// The End gateway at `pos` as its chunk was loaded: `Age` from the saved
    /// block entity (`getUpdateTag`), or a new gateway.
    fn gateway_loaded(&self, pos: [i32; 3]) -> EndGateway {
        let age = self.saved_tag(pos).and_then(|t| number(t.get("Age")));
        EndGateway {
            age: age.unwrap_or(0),
            teleport_cooldown: 0,
        }
    }
    /// The block entity at `pos` Horizon keeps as loaded, if any.
    fn saved_tag(&self, pos: [i32; 3]) -> Option<&Tag> {
        self.world
            .chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .and_then(|c| c.saved.as_deref())
            .and_then(|saved| {
                saved.block_entities.iter().find(|t| {
                    [t.get("x"), t.get("y"), t.get("z")]
                        .map(number)
                        .eq(&pos.map(|v| Some(i64::from(v))))
                })
            })
    }
    /// The renderer's view of the block entity at `pos`, with a decorated
    /// pot's saved `sherds` (back, left, right, front).
    pub(super) fn block_anim(&self, pos: [i32; 3], partial: f32) -> BlockAnim {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let sherds = if &*self.res.facts().get(state).name == "minecraft:decorated_pot" {
            let sherds = self.saved_tag(pos).and_then(|t| t.get("sherds"));
            ["back", "left", "right", "front"].map(|side| match sherds.and_then(|s| s.get(side)) {
                // An item id, or the item stack the processed templates hold.
                Some(Tag::String(id)) => Some(id.to_string_lossy()),
                Some(stack) => {
                    horizon_client::save::item_from_nbt(stack).map(|s| s.item.to_string())
                }
                None => None,
            })
        } else {
            Default::default()
        };
        self.be_anims
            .block_anim(pos, self.world.game_time, partial, sherds)
    }
    /// `ChestLidController.getOpenness(partial)` at `pos`.
    pub(super) fn chest_lid(&self, pos: [i32; 3], partial: f32) -> f32 {
        self.be_anims.lid(pos, partial)
    }
    /// `SkullBlockRenderer.getPlayerSkin`: the skin sprite of the profile of
    /// the player head at `pos`. The skin download is started here; while it
    /// is in flight (or when it cannot be got) the profile's default skin
    /// stands in. A head with no profile answers None: `SKIN_BY_TYPE`.
    pub(super) fn skull_skin_at(&self, pos: [i32; 3]) -> Option<&str> {
        let Some(BlockEntity::Skull(skull)) =
            self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return None;
        };
        let skin = skull.skin.as_ref()?;
        if let Some((url, location)) = &skin.download {
            self.skin_downloads
                .borrow_mut()
                .state(skins::Kind::Skin, url);
            if self.gui.atlas.sprites.contains_key(location) {
                return Some(location);
            }
        }
        Some(skin.fallback)
    }
}
