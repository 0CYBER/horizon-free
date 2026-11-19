//! The tracker's sweep met by cell: a client that moves checks every entity in its reach again
//! (`ChunkMap.move`), though almost none of them changed. A cell (a chunk) a client tracked whole,
//! whose entities are the same ones, and which is now entirely within the range of its
//! shortest-ranged entity, is met without looking at them: each is seen and tracked as it was.
//!
//! Exact, not approximate. What `sees` reads of a pair is the distance against the entity's range
//! (capped at the view distance), `player_shown`, and whether the client has the chunk. The
//! farthest corner of the cell is no nearer than any entity in it, so a cell whose corner is in
//! range has every entity in range; an entity shown to every client (not a spectator, not
//! watching through another, not hidden) is shown to this one; the chunk is looked up each time.
//! That the entities are tracked comes from the tick the client last met the cell in full and
//! found them so (`Verified`); it holds until the cell's entities change (`Stat::epoch`), and
//! every other way the tracker's set changes drops it (`drop_cell`, `Tracker::forget` and `clear`).

use super::{Probe, SEES, TRACKS, Viewer};
use crate::fast_hash::FxHashMap;

/// `ServerPlayer.broadcastToPlayer` (`Entity`'s is `true`): to a spectator,
/// a player is shown only while its camera is itself; to anyone else, a
/// spectator is hidden. A player a native mod hid is shown only to players
/// of at least its level.
fn broadcast_to_player(v: &Viewer, p: &Probe) -> bool {
    super::super::player_shown::player_shown(
        v.spectator,
        v.permission,
        p.spectator,
        p.watching,
        p.hidden,
    )
}

/// TrackedEntity.updatePlayer: within the type's range (capped by the view
/// distance) horizontally, shown to the client (`Entity.broadcastToPlayer`:
/// `ServerPlayer`'s hides a spectator from players who are not spectators;
/// a guest's camera is always its own player, so a spectating client sees
/// every player) and in a chunk the client has. `held` remembers for
/// each of the tick's chunks whether the client has it (0: not looked up
/// yet, 1: no, 2: yes), so each chunk is looked up once per client.
pub(super) fn sees(v: &Viewer, p: &Probe, chunks: &[(i32, i32)], held: &mut [u8]) -> bool {
    let near = match v.sources {
        None => {
            let range = f64::from(p.range.min(v.view_distance * 16));
            let (dx, dz) = (v.position[0] - p.x, v.position[2] - p.z);
            dx * dx + dz * dz <= range * range
        }
        Some(areas) => super::super::viewed_levels::entities::in_reach(
            areas,
            chunks[p.chunk as usize],
            p.range,
        ),
    };
    near && broadcast_to_player(v, p) && {
        let held = &mut held[p.chunk as usize];
        if *held == 0 {
            *held = 1 + u8::from(v.chunks.contains(&chunks[p.chunk as usize]));
        }
        *held == 2
    }
}

/// A cell as the sweep reads it this tick.
#[derive(Clone, Copy)]
pub(super) struct Stat {
    /// The shortest tracking range of its entities, in blocks.
    rmin: i32,
    /// No entity of it is a spectator, watches through another entity, or is hidden.
    plain: bool,
    /// The tick its entities were last other ones, or one appeared or changed section.
    epoch: u32,
}

/// A cell a client tracked whole: the entities it held (`Stat::epoch`), the client's own entity
/// (never tracked) and whether the cell held it.
#[derive(Clone, Copy)]
pub(super) struct Verified {
    epoch: u32,
    entity_id: i32,
    own: u32,
}

/// What each cell holds, kept between ticks for the epochs.
#[derive(Default)]
pub(super) struct Cells {
    stats: Vec<Stat>,
    /// Each cell's entities as of the tick that last changed them: (their hash, their number,
    /// that tick).
    held: FxHashMap<(i32, i32), (u64, u32, u32)>,
    /// Tests: sweep every cell in full.
    #[cfg(test)]
    pub(super) off: bool,
}

/// The cells met without their entities, for the test to know the path was taken.
#[cfg(test)]
pub(super) static SKIPPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A hash that tells sets of ids apart, added up so the order does not matter.
fn mix(id: i32) -> u64 {
    let mut z = u64::from(id as u32).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Cells {
    pub(super) fn on(&self) -> bool {
        #[cfg(test)]
        {
            !self.off
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    pub(super) fn stats(&self) -> &[Stat] {
        &self.stats
    }

    /// This tick's cells: `chunks` and the indices of their entities in `probes`, `is_moved` for
    /// the entities that appeared or changed section, `now` the level tick, `index` the cells
    /// of the tick by position.
    pub(super) fn update(
        &mut self,
        now: u32,
        chunks: &[(i32, i32)],
        members: &[Vec<u32>],
        probes: &[Probe],
        is_moved: &[bool],
        index: &FxHashMap<(i32, i32), u32>,
    ) {
        self.stats.clear();
        for (cell, members) in chunks.iter().zip(members) {
            let (mut rmin, mut plain, mut changed, mut sum) = (i32::MAX, true, false, 0u64);
            for &i in members {
                let p = &probes[i as usize];
                rmin = rmin.min(p.range);
                plain &= !p.spectator && !p.watching && p.hidden.is_none();
                changed |= is_moved[i as usize];
                sum = sum.wrapping_add(mix(p.id));
            }
            let len = members.len() as u32;
            let epoch = match self.held.get(cell) {
                Some(&(s, l, e)) if s == sum && l == len && !changed => e,
                _ => now,
            };
            self.held.insert(*cell, (sum, len, epoch));
            self.stats.push(Stat { rmin, plain, epoch });
        }
        if self.held.len() > 4 * chunks.len() + 64 {
            self.held.retain(|cell, _| index.contains_key(cell));
        }
    }
}

/// Whether every point of the chunk `cell` is within `range` blocks (capped at the client's view
/// distance) of the client, horizontally: the farthest corner is.
fn within(v: &Viewer, (cx, cz): (i32, i32), range: i32) -> bool {
    let range = f64::from(range.min(v.view_distance * 16));
    let far = |at: f64, cell: i32| {
        let (lo, hi) = (f64::from(cell * 16), f64::from(cell * 16 + 16));
        (at - lo).abs().max((at - hi).abs())
    };
    let (dx, dz) = (far(v.position[0], cx), far(v.position[2], cz));
    dx * dx + dz * dz <= range * range
}

/// Meets `cell` without its entities when the client tracked it whole and nothing has changed:
/// their flags are set, and how many of them it tracks is returned. None: look at each.
pub(super) fn skip(
    v: &Viewer,
    cell: (i32, i32),
    stat: &Stat,
    members: &[u32],
    probes: &[Probe],
    flags: &mut [u8],
) -> Option<usize> {
    if v.sources.is_some() || !stat.plain {
        return None;
    }
    let ver = *v.tracker.verified.get(&cell)?;
    if ver.epoch != stat.epoch
        || ver.entity_id != v.entity_id
        || !within(v, cell, stat.rmin)
        || !v.chunks.contains(&cell)
    {
        return None;
    }
    for &index in members {
        let index = index as usize;
        flags[index] = if probes[index].id == v.entity_id {
            0
        } else {
            SEES | TRACKS
        };
    }
    #[cfg(test)]
    SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Some(members.len() - ver.own as usize)
}

/// What the client made of `cell` met in full: whether it tracks every entity of it (but its
/// own), for the next time. `tracked` is how many it tracked, `started` whether it started
/// tracking any.
pub(super) fn record(
    v: &mut Viewer,
    cell: (i32, i32),
    stat: &Stat,
    members: &[u32],
    probes: &[Probe],
    (tracked, started): (usize, bool),
) {
    if v.sources.is_none() && stat.plain && !started {
        let own = members
            .iter()
            .filter(|&&m| probes[m as usize].id == v.entity_id)
            .count();
        if tracked == members.len() - own {
            let ver = Verified {
                epoch: stat.epoch,
                entity_id: v.entity_id,
                own: own as u32,
            };
            v.tracker.verified.insert(cell, ver);
            return;
        }
    }
    drop_cell(v, cell);
}

/// The client no longer tracks `cell` whole, as far as the next sweep can tell.
pub(super) fn drop_cell(v: &mut Viewer, cell: (i32, i32)) {
    if !v.tracker.verified.is_empty() {
        v.tracker.verified.remove(&cell);
    }
}
