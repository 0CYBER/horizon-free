use super::{Chunk, Hub};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    lan::{
        chunk_watch,
        terrain_view::{self, Column, EncodeContext, Pos, Ready, Rules},
        viewed_levels::send::Runs,
    },
    world::World,
};
use horizon_protocol::packets::play::{self, Clientbound as C};
use mod_api::{BlockState, TerrainView, TerrainViewError};
use std::sync::Arc;

#[derive(Default)]
pub(in crate::lan) struct Bodies {
    entries: FxHashMap<(u64, Column, bool), Arc<[u8]>>,
    bytes: usize,
}
const BODY_CACHE_BYTES: usize = 8 << 20;

impl Hub {
    pub fn define_terrain_view(
        &mut self,
        owner: &'static str,
        id: u32,
        spec: &TerrainView,
        resolve: impl Fn(BlockState) -> Option<u32>,
    ) -> Result<(), TerrainViewError> {
        self.terrain
            .get_or_insert_with(Default::default)
            .define((owner, id), spec, resolve)
    }

    pub fn set_terrain_view(
        &mut self,
        conn: u32,
        dimension: &str,
        owner: &'static str,
        id: Option<u32>,
    ) -> Result<(), TerrainViewError> {
        if let Some(views) = self.terrain.as_mut() {
            views.select(conn, dimension, owner, id)
        } else if id.is_some() {
            Err(TerrainViewError::UnknownView)
        } else {
            Ok(())
        }
    }

    pub fn terrain_protected(&self, conn: u32, dimension: &str) -> bool {
        self.terrain
            .as_ref()
            .is_some_and(|views| views.view(conn, dimension).is_some())
    }

    pub(in crate::lan) fn terrain_rules(&self, conn: u32) -> Option<Arc<Rules>> {
        self.terrain
            .as_ref()?
            .view(conn, &self.config.dimension)
            .map(|(_, r)| r.clone())
    }

    pub(in crate::lan) fn prepare_terrain_pass(&mut self) {
        let Some(views) = self.terrain.as_mut() else {
            return;
        };
        views.resident.poll();
        let dimension = &self.config.dimension;
        let mut refresh = Vec::new();
        for (i, guest) in self.guests.iter().enumerate() {
            let Some(levels) = views.players.get_mut(&guest.conn) else {
                continue;
            };
            let Some(view) = levels.get_mut(dimension) else {
                continue;
            };
            if std::mem::take(&mut view.refresh) {
                refresh.push(i);
            }
            if view.id.is_none() {
                levels.remove(dimension);
            }
        }
        for i in refresh {
            self.forget_terrain(i, |_| true);
        }
    }

    /// Start the protected packets for newly ready columns while the level is still ticking.
    /// At most half the resident queue is reserved by this look-ahead.
    pub fn warm_admitted_terrain(
        &mut self,
        world: &World,
        streaming: &crate::streaming::Streaming,
        columns: impl IntoIterator<Item = (i32, i32)>,
    ) {
        let Some(terrain) = self.terrain.as_mut() else {
            return;
        };
        if !terrain.resident.can_warm() {
            return;
        }
        let dimension = &self.config.dimension;
        let has_view = self
            .guests
            .iter()
            .any(|g| g.watches_level(dimension) && terrain.view(g.conn, dimension).is_some());
        if !has_view {
            return;
        }
        let protos = |x, z| streaming.proto_sections(x, z);
        let ctx = EncodeContext {
            dimension,
            encoder: &self.encoder,
            world,
            protos: &protos,
            include_far: self.far.is_some(),
        };
        terrain.resident.poll();
        for at @ (x, z) in columns.into_iter().take(64) {
            let Some(chunk) = world
                .chunks
                .get(&at)
                .filter(|c| c.lit && c.postprocess.is_empty())
            else {
                continue;
            };
            if !super::neighbours_ready(&world.chunks, |x, z| protos(x, z).is_some(), chunk) {
                continue;
            }
            for guest in self.guests.iter().filter(|g| g.watches_level(dimension)) {
                let (cx, cz) = guest.chunk();
                if !super::tracked(cx, cz, guest.view_distance, x, z)
                    || guest.sent_chunks.contains(&at)
                {
                    continue;
                }
                if let Some(rules) = terrain
                    .view(guest.conn, dimension)
                    .map(|(_, rules)| rules.clone())
                {
                    terrain.resident.get(&ctx, &rules, at);
                    if !terrain.resident.can_warm() {
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn warm_first_terrain(
        &mut self,
        i: usize,
        world: &World,
        streaming: Option<&crate::streaming::Streaming>,
    ) {
        let guest = &self.guests[i];
        let Some(rules) = self.terrain_rules(guest.conn) else {
            return;
        };
        let protos = |x, z| streaming.and_then(|s| s.proto_sections(x, z));
        let ctx = EncodeContext {
            dimension: &self.config.dimension,
            encoder: &self.encoder,
            world,
            protos: &protos,
            include_far: self.far.is_some(),
        };
        let resident = &mut self.terrain.as_mut().unwrap().resident;
        for &at in guest.chunk_queue.iter().take(32) {
            if !resident.can_warm() {
                break;
            }
            if let Some(chunk) = world
                .chunks
                .get(&at)
                .filter(|c| c.lit && c.postprocess.is_empty())
                && (streaming.is_none()
                    || super::neighbours_ready(&world.chunks, |x, z| protos(x, z).is_some(), chunk))
            {
                resident.get(&ctx, &rules, at);
            }
        }
    }

    pub(in crate::lan) fn protected_ready(
        &mut self,
        rules: &Arc<Rules>,
        world: &World,
        protos: &dyn Fn(i32, i32) -> Option<crate::section_mask::SectionMask>,
        at: Column,
    ) -> Option<Ready> {
        self.terrain.as_mut()?.resident.get(
            &EncodeContext {
                dimension: &self.config.dimension,
                encoder: &self.encoder,
                world,
                protos,
                include_far: self.far.is_some(),
            },
            rules,
            at,
        )
    }

    pub(in crate::lan) fn protected_body(
        &mut self,
        bodies: &mut Bodies,
        rules: &Arc<Rules>,
        chunk: &Chunk,
        world: &World,
        protos: &dyn Fn(i32, i32) -> Option<crate::section_mask::SectionMask>,
        far: bool,
    ) -> Option<Arc<[u8]>> {
        let at = (chunk.x, chunk.z);
        let key = (rules.generation, at, far);
        if let Some(body) = bodies.entries.get(&key) {
            return Some(body.clone());
        }
        let ready = self.protected_ready(rules, world, protos, at)?;
        self.finish_protected_body(bodies, key, chunk, world, protos, ready)
    }

    pub(super) fn finish_protected_body(
        &self,
        bodies: &mut Bodies,
        key: (u64, Column, bool),
        chunk: &Chunk,
        world: &World,
        protos: &dyn Fn(i32, i32) -> Option<crate::section_mask::SectionMask>,
        ready: Ready,
    ) -> Option<Arc<[u8]>> {
        let far = key.2;
        if let Ready::Shared(body) = &ready
            && !body.has_entities()
        {
            return Some(body.static_body(far).clone());
        }
        if let Some(body) = bodies.entries.get(&key) {
            return Some(body.clone());
        }
        if bodies.entries.len() == 32 || bodies.bytes >= BODY_CACHE_BYTES {
            return None;
        }
        let body = match ready {
            Ready::Shared(body) => self.encoder.terrain_body(chunk, world, &body, far),
            Ready::Ordinary if far => self.encoder.body_far(chunk, world).into(),
            Ready::Ordinary => self.encoder.body_with(chunk, world, protos).into(),
        };
        // Bodies with live tags are only shared within this pass.
        if bodies.bytes + body.len() > BODY_CACHE_BYTES {
            return None;
        }
        bodies.bytes += body.len();
        bodies.entries.insert(key, body.clone());
        Some(body)
    }

    fn forget_terrain(&mut self, i: usize, matches: impl Fn(Column) -> bool) {
        let dimension = &self.config.dimension;
        self.guests.update(i, |g| {
            if &g.dimension == dimension {
                let forget: Vec<_> = g
                    .sent_chunks
                    .iter()
                    .copied()
                    .filter(|&k| matches(k))
                    .collect();
                for (x, z) in forget {
                    g.sent_chunks.remove(&(x, z));
                    g.far_sent_chunks.remove(&(x, z));
                    chunk_watch::unwatch(&mut self.chunk_watches, g, (x, z));
                    g.send(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }));
                    g.center = None;
                }
            } else if let Some(view) = g.viewed.views.get_mut(dimension) {
                let mut runs = Runs::new(view.level);
                view.sent.retain(|&(x, z)| {
                    if matches((x, z)) {
                        view.linger.remove(&(x, z));
                        view.dirty = true;
                        runs.push(
                            &C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }).encode(),
                        );
                        false
                    } else {
                        true
                    }
                });
                g.sender.send_many(runs.finish());
            }
        });
    }

    pub(in crate::lan) fn has_terrain_views(&self) -> bool {
        self.terrain.as_ref().is_some_and(|v| {
            v.players.values().any(|levels| {
                levels
                    .get(&self.config.dimension)
                    .is_some_and(|s| s.id.is_some())
            })
        })
    }

    pub(super) fn finish_terrain_refresh(&mut self, affected: &FxHashSet<Column>) {
        if let Some(far) = self.far.as_mut() {
            for &(x, z) in affected {
                far.changed(x, z);
            }
        }
        self.refresh_protected_columns(affected);
    }

    fn refresh_protected_columns(&mut self, columns: &FxHashSet<Column>) {
        if self.terrain.is_none() || columns.is_empty() {
            return;
        }
        for i in 0..self.guests.len() {
            if self.terrain_protected(self.guests[i].conn, &self.config.dimension) {
                self.forget_terrain(i, |key| columns.contains(&key));
            }
        }
    }

    /// Chunks entering or leaving the resident set can change the known faces of adjacent columns.
    pub fn terrain_residency_changed(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        if !self.has_terrain_views() {
            return;
        }
        let affected: FxHashSet<_> = columns
            .into_iter()
            .flat_map(|at| std::iter::once(at).chain(terrain_view::adjacent(at)))
            .collect();
        self.finish_terrain_refresh(&affected);
    }

    pub(in crate::lan) fn terrain_state(
        &self,
        conn: u32,
        world: &World,
        pos: Pos,
        state: u32,
    ) -> u32 {
        self.terrain
            .as_ref()
            .and_then(|v| v.view(conn, &self.config.dimension))
            .map_or(state, |(selection, rule)| {
                selection.state(world, rule, pos, state)
            })
    }

    pub fn send_block_correction(&self, conn: u32, world: &World, [x, y, z]: [i32; 3]) {
        let state = self.terrain_state(conn, world, (x, y, z), world.block(x, y, z).0);
        self.send_to(
            conn,
            &C::BlockUpdate(play::BlockUpdate {
                pos: [x, y, z],
                state: state as i32,
            }),
        );
    }

    pub fn terrain_metadata_visible(&self, conn: u32, world: &World, [x, y, z]: [i32; 3]) -> bool {
        let state = world.block(x, y, z).0;
        self.terrain_state(conn, world, (x, y, z), state) == state
    }

    fn terrain_reveal_packets(
        &self,
        world: &World,
        pos @ (x, y, z): Pos,
    ) -> impl Iterator<Item = C> {
        let state = world.block(x, y, z).0;
        let kind = self.encoder.entity_type(state);
        let tag = world
            .block_entities
            .get(&pos)
            .and_then(|entity| {
                self.encoder
                    .update_tag(pos, state, entity)
                    .or_else(|| self.encoder.chest_snapshot_tag(state, entity))
            })
            .or_else(|| {
                crate::lan::chunks::pot_tag(world.chunks.get(&terrain_view::column(pos))?, pos)
            });
        std::iter::once(C::BlockUpdate(play::BlockUpdate {
            pos: [x, y, z],
            state: state as i32,
        }))
        .chain(kind.zip(tag).map(|(kind, tag)| {
            C::BlockEntityData(play::BlockEntityData {
                pos: [x, y, z],
                kind,
                tag,
            })
        }))
    }

    /// Called after an authoritative successful break. Keeps a bounded set of revealed positions.
    pub fn reveal_terrain_after_break(&mut self, conn: u32, world: &mut World, at: [i32; 3]) {
        let Some(rules) = self.terrain_rules(conn) else {
            return;
        };
        let radius = i32::from(rules.spec.reveal_radius);
        if radius == 0 {
            return;
        }
        let Some(guest) = self.guest(conn) else {
            return;
        };
        if guest.dimension != self.config.dimension {
            return;
        }
        let [x, y, z] = at;
        let selection = self
            .terrain
            .as_mut()
            .unwrap()
            .players
            .get_mut(&conn)
            .unwrap()
            .get_mut(&self.config.dimension)
            .unwrap();
        // An entry can be kept while the player stays near its chunk. When full, only already
        // revealed positions stay revealed, until the next tracking or view change frees room.
        let mut revealed = Vec::new();
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    if dx.abs() + dy.abs() + dz.abs() > radius || selection.reveals.len() == 1024 {
                        continue;
                    }
                    let pos = (x + dx, y + dy, z + dz);
                    let state = world.block(pos.0, pos.1, pos.2).0;
                    if rules.in_world(world, pos, state) != state && selection.reveals.insert(pos) {
                        revealed.push(pos);
                    }
                }
            }
        }
        let Some(guest) = self.guest(conn) else {
            return;
        };
        for pos in revealed
            .into_iter()
            .filter(|&p| guest.sent_chunks.contains(&terrain_view::column(p)))
        {
            guest
                .sender
                .send_many(self.terrain_reveal_packets(world, pos).map(|p| p.encode()));
            if matches!(world.block_entities.get(&pos), Some(crate::containers::BlockEntity::Mod(entity))
                if entity.has_client_update_fields())
                && let Some(log) = world.block_log.as_mut()
            {
                log.push(pos);
            }
        }
    }

    pub(in crate::lan) fn terrain_overlays(
        &self,
        conn: u32,
        world: &World,
        columns: impl Iterator<Item = Column>,
    ) -> Vec<Vec<u8>> {
        let Some((selection, rules)) = self
            .terrain
            .as_ref()
            .and_then(|v| v.view(conn, &self.config.dimension))
        else {
            return Vec::new();
        };
        if selection.reveals.is_empty() {
            return Vec::new();
        }
        let mut packets = Vec::new();
        for key in columns {
            let reveals = selection
                .reveals
                .iter()
                .copied()
                .filter(|&p| terrain_view::column(p) == key);
            if world.chunks.contains_key(&key) {
                for pos in reveals {
                    packets.extend(self.terrain_reveal_packets(world, pos).map(|p| p.encode()));
                }
                continue;
            }
            // A saved far column: its snapshot is what the body kept of it.
            let Some(far) = self.far.as_ref() else {
                continue;
            };
            let reveals: Vec<_> = reveals.collect();
            if reveals.is_empty() {
                continue;
            }
            let concealed = far.concealed(key.0, key.1, rules.generation);
            for pos in reveals {
                let Some(hidden) = concealed.iter().find(|c| c.pos == pos) else {
                    continue;
                };
                packets.push(
                    C::BlockUpdate(play::BlockUpdate {
                        pos: [pos.0, pos.1, pos.2],
                        state: hidden.state as i32,
                    })
                    .encode(),
                );
                if let Some((kind, tag)) = &hidden.tag {
                    packets.push(
                        C::BlockEntityData(play::BlockEntityData {
                            pos: [pos.0, pos.1, pos.2],
                            kind: *kind,
                            tag: tag.clone(),
                        })
                        .encode(),
                    );
                }
            }
        }
        packets
    }
}
