//! The chunk-unload sweep of `entity_chunks` (`remove`): every family's
//! rows of the unloaded chunks leave — each was saved above — and their
//! ids are forgotten with them.
use super::*;

impl Game<'_> {
    /// Remove the entities of chunks that unloaded: their records were
    /// written into `out` above, so the rows themselves now go, and what
    /// each family keeps (a ridden boat, an unsaved projectile) stays.
    /// `keys` are the chunks leaving.
    pub(super) fn unload_removed_entities(&mut self, keys: &HashSet<(i32, i32)>) {
        let gone = |p: Vec3| keys.contains(&chunk_of(p));
        let mut removed: Vec<u32> = Vec::new();
        let riding = self.riding;
        self.boats.retain(|b| {
            let keep = !gone(b.position) || Some(b.id) == riding;
            if !keep {
                removed.push(b.id);
            }
            keep
        });
        self.crystals.retain(|c| !gone(c.position));
        self.ominous_spawners.retain(|s| {
            let keep = !gone(Vec3::new(s.position[0], s.position[1], s.position[2]));
            if !keep {
                removed.push(s.id);
            }
            keep
        });
        if self.dragon.as_ref().is_some_and(|d| gone(d.position)) {
            self.discard_resident_dragon();
        }
        self.carts.retain(|c| {
            let keep = !gone(c.position) || Some(c.id) == riding;
            if !keep {
                removed.push(c.id);
            }
            keep
        });
        self.fangs.list.retain(|f| {
            let keep = !gone(Vec3::new(f.pos[0], f.pos[1], f.pos[2]));
            if !keep {
                removed.push(f.id);
            }
            keep
        });
        self.others.retain(|o| {
            let keep = !gone(o.position);
            if !keep {
                removed.push(o.id);
            }
            keep
        });
        self.armor_stands.retain(|a| {
            let keep = !gone(a.position);
            if !keep {
                removed.push(a.id);
            }
            keep
        });
        self.item_frames.retain(|f| {
            let (lo, hi) = f.bounds();
            let keep = !gone(Vec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ));
            if !keep {
                removed.push(f.id);
            }
            keep
        });
        self.paintings.retain(|p| {
            let keep = !gone(painting_center(p));
            if !keep {
                removed.push(p.id);
            }
            keep
        });
        self.cushions.retain(|c| {
            let keep = !gone(c.position);
            if !keep {
                removed.push(c.id);
            }
            keep
        });
        // The potion clouds written above leave with their chunk; a
        // dragon's breath (no potion) was not written and stays.
        self.breath_clouds.retain(|c| {
            let keep = c.potion.is_none() || !gone(Vec3::new(c.at[0], c.at[1], c.at[2]));
            if !keep {
                removed.push(c.id);
            }
            keep
        });
        // Riders leave with their root vehicle, a boat's with the boat.
        let leaving: HashSet<u32> = self
            .mob_object_slots()
            .filter(|&object| {
                let root = self.mob_at(self.mob_root(object));
                match root.extra.mount {
                    Some(v) if removed.contains(&v) => true,
                    Some(v) if self.boats.iter().any(|b| b.id == v) => false,
                    _ => gone(root.position),
                }
            })
            .map(|object| self.mob_at(object).id)
            .collect();
        self.mobs.retain(|m| {
            let keep = !leaving.contains(&m.id);
            if !keep {
                removed.push(m.id);
            }
            keep
        });
        self.persist.off_level.retain(|id, _| {
            let keep = !leaving.contains(id);
            if !keep {
                removed.push(*id);
            }
            keep
        });
        if !self.persist.off_level.is_empty() {
            // A retained boat or cart root is in no section and never saved:
            // it lives only while a passenger or player still rides it.
            let ridden: HashSet<u32> = self
                .mob_objects()
                .filter_map(|m| m.extra.mount)
                .chain(riding)
                .collect();
            self.persist.off_level.retain(|id, object| {
                let keep = object.as_mob().is_some() || ridden.contains(id);
                if !keep {
                    removed.push(*id);
                }
                keep
            });
        }
        self.dropped.retain(|e| {
            let keep = !gone(e.position);
            if !keep {
                removed.push(e.id);
            }
            keep
        });
        self.orbs.retain(|o| !gone(o.position));
        // Unloaded with their chunk: PersistentEntitySectionManager.stopTracking.
        self.native_leaves_flush();
        // the ones saved with the chunk go with it (a trident too, unsaved).
        self.creatures.projectiles.retain(|p| {
            let leaves = entity_nbt::saved_with_chunk(p) || p.arrow.is_some();
            let keep = !leaves || !gone(p.position);
            if !keep {
                removed.push(p.id);
            }
            keep
        });
        self.falling.retain(|f| !gone(f.position));
        self.rockets.retain(|r| !gone(r.position));
        self.tridents.retain(|t| !gone(t.position));
        self.tnts.retain(|t| !gone(t.position));
        // A general row leaves with its chunk, its riders with it: a rider's
        // record saved inside its root's, a rider goes when its root's chunk
        // goes and stays while the root stays, whatever its own position.
        let mut gone_general_roots: HashSet<u32> = self
            .resident_entities
            .general
            .rows
            .iter()
            .filter(|row| row.mount.is_none() && gone(row.position))
            .map(|row| row.id)
            .collect();
        loop {
            let before = gone_general_roots.len();
            for row in &self.resident_entities.general.rows {
                if row
                    .mount
                    .is_some_and(|mount| gone_general_roots.contains(&mount))
                {
                    gone_general_roots.insert(row.id);
                }
            }
            if gone_general_roots.len() == before {
                break;
            }
        }
        self.resident_entities.general.rows.retain(|row| {
            let keep = match row.mount {
                None => !gone(row.position),
                Some(root) => !gone_general_roots.contains(&root),
            };
            if !keep {
                removed.push(row.id);
            }
            keep
        });
        self.persist
            .anon
            .retain(|_, (at, _)| !keys.contains(&chunk_of(*at)));
        for id in removed {
            self.forget_entity(id);
        }
    }
}
