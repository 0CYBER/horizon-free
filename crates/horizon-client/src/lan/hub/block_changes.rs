//! `ChunkHolder.broadcastChanges`: the light and blocks that changed in the level in place, to
//! the players sent those chunks, and, wrapped with the level, to the players whose client views
//! it (`viewed_levels`).
use super::{Guest, Hub};
use crate::{
    chunk::Chunk,
    lan::{terrain_view, viewed_levels::send::Runs},
    world::World,
};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    viewed_levels::Level,
};
use std::{borrow::Cow, sync::Arc};

impl Hub {
    /// The players sent chunk `key` of the level in place: those in it (None), and those whose
    /// client views it, with the level to wrap with. The chunks each was sent come with it.
    fn chunk_holders(
        &self,
        key: (i32, i32),
    ) -> Vec<(
        usize,
        Option<Level>,
        &crate::fast_hash::FxHashSet<(i32, i32)>,
    )> {
        let viewed = self.viewed.active();
        (0..self.guests.len())
            .filter_map(|i| {
                let g = &self.guests[i];
                if self.watches(g) {
                    g.sent_chunks
                        .contains(&key)
                        .then_some((i, None, &g.sent_chunks))
                } else if viewed {
                    self.view_here(g)
                        .filter(|v| v.sent.contains(&key))
                        .map(|v| (i, Some(v.level), &v.sent))
                } else {
                    None
                }
            })
            .collect()
    }

    /// `body`, a play clientbound packet, to each of `holders`.
    fn send_to_holders(&self, holders: &[(usize, Option<Level>)], body: &[u8]) {
        for &(i, level) in holders {
            let g = &self.guests[i];
            match level {
                None => g.sender.send(body.to_vec()),
                Some(level) => {
                    let mut runs = Runs::new(level);
                    runs.push(body);
                    g.sender.send_many(runs.finish());
                }
            }
        }
    }

    /// `ChunkHolder.broadcastChanges`' light part: the light sections that
    /// changed in a column go to the players for whom it is on the border of
    /// what they track (`ChunkMap.isChunkOnTrackedBorder`); the others light
    /// the blocks they are sent themselves.
    pub(in crate::lan) fn broadcast_light_changes(&mut self, world: &mut World) {
        let Some(log) = world.light_log.as_mut() else {
            return;
        };
        if log.is_empty() {
            return;
        }
        let mut changes: Vec<_> = log.drain().collect();
        // One entry per column, so the order is the columns'.
        changes.sort_unstable_by_key(|&(column, _)| column);
        for ((x, z), sections) in changes {
            if let Some(far) = self.far.as_mut() {
                far.changed(x, z);
            }
            let viewers: Vec<(usize, Option<Level>)> = self
                .chunk_holders((x, z))
                .into_iter()
                .filter(|(_, _, sent)| {
                    (-1..=1).any(|dz| (-1..=1).any(|dx| !sent.contains(&(x + dx, z + dz))))
                })
                .map(|(i, level, _)| (i, level))
                .collect();
            if viewers.is_empty() {
                continue;
            }
            let Some(chunk) = world.chunks.get(&(x, z)) else {
                continue;
            };
            let body = self.encoder.light_update(chunk, sections);
            self.send_to_holders(&viewers, &body);
        }
    }

    pub(in crate::lan) fn broadcast_block_changes(&mut self, world: &mut World) {
        self.prepare_terrain_pass();
        let has_views = self.has_terrain_views();
        // The states this tick's writes replaced: what a protected view's neighbours showed before.
        // The writes held back for the next pass keep theirs for it.
        let previous = if self.terrain.is_some() {
            let taken = world.block_previous.take().unwrap_or_default();
            world.block_previous = Some(
                world
                    .block_log_held
                    .iter()
                    .filter_map(|p| Some((*p, *taken.get(p)?)))
                    .collect(),
            );
            taken
        } else {
            world.block_previous = None;
            Default::default()
        };
        // Chunks changed in bulk (`World::edit_runs`) go whole to those who see them: one packet each, where a
        // change of each block would be thousands (the client lights and meshes them once, on its workers).
        let bulk: Vec<(i32, i32)> = if world.bulk_log.is_empty() {
            Vec::new()
        } else {
            let mut keys: Vec<_> = world.bulk_log.drain().collect();
            keys.sort_unstable();
            keys
        };
        if !bulk.is_empty() {
            // The single-player window's far terrain draws a column it isn't sent as the window last saw it: it is
            // handed the column to summarize again (`handoff::changed`).
            let unsent = |g: &Guest, key: &(i32, i32)| {
                !self.terrain_protected(g.conn, &g.dimension)
                    && self.here(g)
                    && g.address == crate::lan::conn::MEMORY_ADDRESS
                    && !g.sent_chunks.contains(key)
            };
            for key in &bulk {
                if let Some(chunk) = world.chunks.get(key)
                    && self.guests.iter().any(|g| unsent(g, key))
                {
                    crate::lan::handoff::changed(chunk);
                }
            }
            let recording = self.recording();
            let columns: Vec<&Chunk> = bulk
                .iter()
                .filter_map(|k| world.chunks.get(k).map(|c| &**c))
                .collect();
            self.encoder.warm(&columns);
            for chunk in columns {
                let key = (chunk.x, chunk.z);
                if let Some(far) = self.far.as_mut() {
                    far.changed(key.0, key.1);
                }
                let holders = self.holders(key);
                if holders.is_empty() {
                    continue;
                }
                let ordinary = self.ordinary_terrain_holders(&holders, has_views);
                if !ordinary.is_empty() {
                    let body = self.encoder.body_in(chunk, world);
                    self.send_to_holders(&ordinary, &body);
                }
                if recording {
                    // The prediction's replay excuses a client near the column: any of its blocks may have
                    // changed, so the one nearest the player stands for them.
                    for i in in_level(&holders) {
                        self.guests.update(i, |g| {
                            let Some(record) = g.record.as_deref_mut() else {
                                return;
                            };
                            let slack = crate::lan::prediction::slack(std::array::from_fn(|i| {
                                g.position[i] - g.previous_position[i]
                            }));
                            let column =
                                |c: f64, at: i32| (c.floor() as i32).clamp(at * 16, at * 16 + 15);
                            let at = [
                                column(g.position[0], key.0),
                                g.position[1].floor() as i32,
                                column(g.position[2], key.1),
                            ];
                            if crate::lan::prediction::near(g.position, at, slack) {
                                record.changed_near(self.ticks, at);
                            }
                        });
                    }
                }
            }
        }
        let mut refresh: crate::fast_hash::FxHashSet<_> = if has_views {
            bulk.iter()
                .copied()
                .flat_map(|key| std::iter::once(key).chain(terrain_view::adjacent(key)))
                .collect()
        } else {
            Default::default()
        };
        let Some(log) = world.block_log.as_mut() else {
            self.finish_terrain_refresh(&refresh);
            return;
        };
        if !bulk.is_empty() {
            // (what changed in them one at a time went with them, but for one-shot fields: a chunk has none)
            log.retain(|p| {
                bulk.binary_search(&(p.0 >> 4, p.2 >> 4)).is_err()
                    || matches!(
                        world.block_entities.get(p),
                        Some(crate::containers::BlockEntity::Mod(m)) if m.has_client_update_fields()
                    )
            });
        }
        if log.is_empty() {
            self.finish_terrain_refresh(&refresh);
            return;
        }
        let mut changes = std::mem::take(log);
        changes.sort_unstable();
        changes.dedup();
        // The prediction's replay excuses a client that hadn't seen a change near it.
        let recording = self.recording();
        // Each changed block can change the exposure of its six neighbours, including across a
        // chunk border. The ordinary view receives only the original log.
        let mut extra = std::collections::BTreeMap::<_, Vec<_>>::new();
        if let Some(views) = self.terrain.as_ref().filter(|_| has_views) {
            for &p in &changes {
                for face in terrain_view::FACES {
                    let pos @ (x, y, z) = terrain_view::offset(p, face);
                    let column = terrain_view::column(pos);
                    if let Some(chunk) = world.chunks.get(&column) {
                        if terrain_view::known(chunk, x, y, z)
                            .is_some_and(|s| views.targets_state(y, s))
                        {
                            extra.entry((x >> 4, y >> 4, z >> 4)).or_default().push(pos);
                        }
                    } else if column != terrain_view::column(p) {
                        refresh.insert(column);
                    }
                }
            }
        }
        // Group by section.
        let mut sections: std::collections::BTreeMap<(i32, i32, i32), Vec<(i32, i32, i32)>> =
            Default::default();
        for p in changes {
            sections
                .entry((p.0 >> 4, p.1 >> 4, p.2 >> 4))
                .or_default()
                .push(p);
        }
        let protected_sections = extra.keys().copied().collect::<Vec<_>>();
        for key in protected_sections {
            sections.entry(key).or_default();
        }
        for ((sx, sy, sz), positions) in sections {
            if let Some(far) = self.far.as_mut() {
                far.changed(sx, sz);
            }
            let holders = self.holders((sx, sz));
            if holders.is_empty() {
                continue;
            }
            let section = [sx, sy, sz];
            let state_at = |(x, y, z)| world.block(x, y, z).0;
            let ordinary = self.ordinary_terrain_holders(&holders, has_views);
            if !ordinary.is_empty() && !positions.is_empty() {
                let packet = block_packet(section, &positions, state_at);
                self.send_to_holders(&ordinary, &packet.encode());
            }
            let mut candidates = extra.remove(&(sx, sy, sz)).unwrap_or_default();
            let mut sent_to = crate::fast_hash::FxHashMap::<usize, Arc<Vec<_>>>::default();
            if has_views {
                candidates.extend(positions.iter().copied());
                candidates.sort_unstable();
                candidates.dedup();
                let mut encoded = crate::fast_hash::FxHashMap::default();
                for &(i, level) in &holders {
                    let conn = self.guests[i].conn;
                    let Some((view, rules)) = self
                        .terrain
                        .as_ref()
                        .unwrap()
                        .view(conn, &self.config.dimension)
                    else {
                        continue;
                    };
                    let personal = (!view.reveals.is_empty()).then_some(conn);
                    let (sent, body) = encoded
                        .entry((rules.generation, personal))
                        .or_insert_with(|| {
                            // A block that only sits beside a change is sent to a player whose
                            // picture of it changes, so its position says nothing more than the
                            // picture does.
                            let sent: Arc<Vec<_>> = Arc::new(
                                candidates
                                    .iter()
                                    .copied()
                                    .filter(|p| {
                                        positions.binary_search(p).is_ok() || {
                                            let state = state_at(*p);
                                            view.state(world, rules, *p, state)
                                                != view.state_before(
                                                    world, rules, *p, state, &previous,
                                                )
                                        }
                                    })
                                    .collect(),
                            );
                            let body = (!sent.is_empty()).then(|| {
                                block_packet(section, &sent, |p| {
                                    view.state(world, rules, p, state_at(p))
                                })
                                .encode()
                            });
                            (sent, body)
                        })
                        .clone();
                    if let Some(body) = body {
                        self.send_to_holders(&[(i, level)], &body);
                    }
                    sent_to.insert(i, sent);
                }
            }
            if recording {
                let bounds = crate::lan::prediction::bounds(&positions);
                for i in in_level(&holders) {
                    self.guests.update(i, |g| {
                        crate::lan::prediction::note_changes(g, self.ticks, bounds, &positions);
                    });
                }
            }
            // Block entities with data the client shows.
            let metadata_positions = if has_views { &candidates } else { &positions };
            for &pos @ (x, y, z) in metadata_positions {
                let state = world.block(x, y, z).0;
                let entity = world.block_entities.get(&pos);
                if !self.encoder.entity_matches(state, entity) {
                    continue;
                }
                let mut entitled = if has_views {
                    Cow::Owned(
                        holders
                            .iter()
                            .copied()
                            .filter(|&(i, _)| {
                                let conn = self.guests[i].conn;
                                let sent = match sent_to.get(&i) {
                                    Some(sent) => sent.binary_search(&pos).is_ok(),
                                    None => {
                                        positions.binary_search(&pos).is_ok()
                                            && !self.terrain_protected(conn, &self.config.dimension)
                                    }
                                };
                                sent && self.terrain_state(conn, world, pos, state) == state
                            })
                            .collect(),
                    )
                } else {
                    Cow::Borrowed(holders.as_slice())
                };
                if entitled.is_empty() {
                    continue;
                }
                // A decorated pot is no block entity: its tag is the chunk's saved one.
                let tag = match entity {
                    Some(entity) => self.encoder.change_tag(pos, state, entity),
                    None => world
                        .chunks
                        .get(&(x >> 4, z >> 4))
                        .and_then(|c| crate::lan::chunks::pot_tag(c, pos)),
                };
                let Some(tag) = tag.or_else(|| {
                    let terrain = self.terrain.as_ref().filter(|_| has_views)?;
                    let tag = self.encoder.chest_snapshot_tag(state, entity?)?;
                    // A revealed chest needs the snapshot omitted from its projected chunk.
                    entitled.to_mut().retain(|&(i, _)| {
                        terrain
                            .view(self.guests[i].conn, &self.config.dimension)
                            .is_some_and(|(_, rules)| rules.targets_state(y, state))
                    });
                    (!entitled.is_empty()).then_some(tag)
                }) else {
                    continue;
                };
                let Some(kind) = self.encoder.entity_type(state) else {
                    continue;
                };
                let body = C::BlockEntityData(play::BlockEntityData {
                    pos: [x, y, z],
                    kind,
                    tag,
                })
                .encode();
                self.send_to_holders(&entitled, &body);
            }
        }
        self.finish_terrain_refresh(&refresh);
    }

    fn ordinary_terrain_holders<'a>(
        &self,
        holders: &'a [(usize, Option<Level>)],
        has_views: bool,
    ) -> Cow<'a, [(usize, Option<Level>)]> {
        if !has_views {
            return Cow::Borrowed(holders);
        }
        Cow::Owned(
            holders
                .iter()
                .copied()
                .filter(|&(i, _)| {
                    !self.terrain_protected(self.guests[i].conn, &self.config.dimension)
                })
                .collect(),
        )
    }

    /// [`Self::chunk_holders`] without the chunks they were sent.
    fn holders(&self, key: (i32, i32)) -> Vec<(usize, Option<Level>)> {
        self.chunk_holders(key)
            .into_iter()
            .map(|(i, level, _)| (i, level))
            .collect()
    }
}

/// The holders in the level in place: the prediction's replay is of them only.
fn block_packet(
    section: [i32; 3],
    positions: &[terrain_view::Pos],
    state: impl Fn(terrain_view::Pos) -> u32,
) -> C {
    if let [p @ (x, y, z)] = *positions {
        C::BlockUpdate(play::BlockUpdate {
            pos: [x, y, z],
            state: state(p) as i32,
        })
    } else {
        C::SectionBlocksUpdate(play::SectionBlocksUpdate {
            section,
            changes: positions
                .iter()
                .map(|&p @ (x, y, z)| {
                    i64::from(state(p)) << 12
                        | i64::from(x & 15) << 8
                        | i64::from(z & 15) << 4
                        | i64::from(y & 15)
                })
                .collect(),
        })
    }
}

fn in_level(holders: &[(usize, Option<Level>)]) -> impl Iterator<Item = usize> + '_ {
    holders
        .iter()
        .filter(|(_, level)| level.is_none())
        .map(|&(i, _)| i)
}
