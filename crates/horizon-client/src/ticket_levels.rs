//! Which chunks tick, from the chunk tickets of `DistanceManager`.
//!
//! Every player holds two kinds of ticket:
//! - `PLAYER_LOADING` at level 31 (`ENTITY_TICKING`) on every chunk within
//!   the server's view distance (Chebyshev), from `PlayerTicketTracker`;
//! - `PLAYER_SIMULATION` at level `31 - simulationDistance` on its own chunk.
//!
//! A `/forceload`ed chunk holds a `FORCED` ticket at level 31 of both kinds.
//! Levels rise by one per chunk away from a ticket (`ChunkTracker`) and each
//! chunk takes the lowest. The loading level gives the chunk's
//! `FullChunkStatus` (`ChunkLevel.fullStatus`: 31 entity ticking, 32 block
//! ticking, 33 full) and the simulation level gives what
//! `inEntityTickingRange` (31 or less) and `inBlockTickingRange` (32 or less)
//! answer. Vanilla needs both:
//! - entities tick where both levels are entity ticking (`ServerLevel.tick`,
//!   and `EntityTickList` holds only the entities of entity-ticking chunks);
//! - random ticks run in the simulation tracker's entity-ticking chunks that
//!   have a ticking chunk (`ChunkMap.forEachBlockTickingChunk`);
//! - scheduled ticks, block events and block entities run where both are
//!   block ticking (`isPositionTickingWithEntitiesLoaded`,
//!   `shouldTickBlocksAt`, `LevelChunk.isTicking`).
//!
//! Whoever the players are (the local player, LAN guests, a dedicated
//! server's players), they count the same.
use crate::fast_hash::FxHashMap;
use std::sync::Arc;

/// `ChunkLevel`'s levels.
pub const ENTITY_TICKING: u8 = 31;
pub const BLOCK_TICKING: u8 = 32;
/// What a chunk with no ticket near it has (`FULL` and above count as none).
const NONE: (u8, u8) = (BLOCK_TICKING + 1, BLOCK_TICKING + 1);

/// The tickets of one level: where its players are, its forced chunks, and
/// the server's view and simulation distances.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub players: Vec<(i32, i32)>,
    pub forced: Vec<(i32, i32)>,
    pub view_distance: i32,
    pub simulation_distance: i32,
}

/// Each chunk's (loading, simulation) level, for the chunks where either is
/// block ticking or lower; `None` ticks everywhere (a game with no server and
/// no player: the headless fixtures).
#[derive(Clone, Default)]
pub struct Levels(Option<Arc<FxHashMap<(i32, i32), (u8, u8)>>>);

impl Levels {
    /// Chunk coordinates.
    fn get(&self, x: i32, z: i32) -> Option<(u8, u8)> {
        let map = self.0.as_ref()?;
        Some(map.get(&(x, z)).copied().unwrap_or(NONE))
    }
    /// `DistanceManager.inEntityTickingRange`, in an entity-ticking chunk:
    /// where entities tick.
    pub fn entity_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= ENTITY_TICKING && s <= ENTITY_TICKING)
    }
    /// `inBlockTickingRange` in a block-ticking chunk: where scheduled ticks,
    /// block events and block entities run.
    pub fn block_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= BLOCK_TICKING && s <= BLOCK_TICKING)
    }
    /// `inEntityTickingRange` in a block-ticking chunk: where random ticks run.
    pub fn random_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= BLOCK_TICKING && s <= ENTITY_TICKING)
    }
    /// `ChunkHolder.getFullStatus` is block ticking or more: the chunk's
    /// ticking future can complete (its saved ticks unpack then).
    pub fn loaded_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z).is_none_or(|(l, _)| l <= BLOCK_TICKING)
    }
    /// The same as [`Self::entity_ticking`] at a block position.
    pub fn entity_ticking_at(&self, x: i32, z: i32) -> bool {
        self.entity_ticking(x >> 4, z >> 4)
    }
    /// The same as [`Self::block_ticking`] at a block position.
    pub fn block_ticking_at(&self, x: i32, z: i32) -> bool {
        self.block_ticking(x >> 4, z >> 4)
    }
}

/// The levels, rebuilt only when their sources change (a player crossing
/// into another chunk, a distance or a forced chunk changing).
#[derive(Default)]
pub struct ChunkTickets {
    sources: Option<Sources>,
    levels: Levels,
}

impl ChunkTickets {
    /// `None`: tick everywhere.
    pub fn update(&mut self, mut sources: Option<Sources>) {
        if let Some(s) = sources.as_mut() {
            s.players.sort_unstable();
            s.players.dedup();
            s.forced.sort_unstable();
        }
        if sources == self.sources {
            return;
        }
        self.levels = Levels(sources.as_ref().map(|s| Arc::new(compute(s))));
        self.sources = sources;
    }
    pub fn levels(&self) -> Levels {
        self.levels.clone()
    }
}

fn compute(s: &Sources) -> FxHashMap<(i32, i32), (u8, u8)> {
    let view = s.view_distance.clamp(0, 32);
    let simulation = s.simulation_distance.clamp(0, 32);
    let mut levels: FxHashMap<(i32, i32), (u8, u8)> = FxHashMap::default();
    let mut lower = |key, loading: i32, sim: i32| {
        let e = levels.entry(key).or_insert(NONE);
        e.0 = e.0.min(loading.min(i32::from(NONE.0)) as u8);
        e.1 = e.1.min(sim.min(i32::from(NONE.1)) as u8);
    };
    // PLAYER_LOADING within the view distance, PLAYER_SIMULATION at the
    // player's chunk (`getPlayerTicketLevel`: never below 0).
    let player_sim = (i32::from(ENTITY_TICKING) - simulation).max(0);
    let reach = (view + 1).max(i32::from(BLOCK_TICKING) - player_sim);
    for &(px, pz) in &s.players {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let d = dx.abs().max(dz.abs());
                lower(
                    (px + dx, pz + dz),
                    i32::from(ENTITY_TICKING) + (d - view).max(0),
                    player_sim + d,
                );
            }
        }
    }
    for &(fx, fz) in &s.forced {
        for dz in -1i32..=1 {
            for dx in -1i32..=1 {
                let level = i32::from(ENTITY_TICKING) + dx.abs().max(dz.abs());
                lower((fx + dx, fz + dz), level, level);
            }
        }
    }
    levels.retain(|_, l| *l != NONE);
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(players: &[(i32, i32)], forced: &[(i32, i32)], view: i32, sim: i32) -> Levels {
        let mut t = ChunkTickets::default();
        t.update(Some(Sources {
            players: players.to_vec(),
            forced: forced.to_vec(),
            view_distance: view,
            simulation_distance: sim,
        }));
        t.levels()
    }

    #[test]
    fn simulation_distance_inside_the_view() {
        // view 8, simulation 4: entities and random ticks out to 4 chunks,
        // scheduled ticks out to 5.
        let l = levels(&[(0, 0)], &[], 8, 4);
        assert!(l.entity_ticking(4, -4) && !l.entity_ticking(5, 0));
        assert!(l.random_ticking(-4, 4) && !l.random_ticking(0, 5));
        assert!(l.block_ticking(5, 5) && !l.block_ticking(6, 0));
        assert!(l.loaded_ticking(9, 9) && !l.loaded_ticking(10, 0));
    }

    #[test]
    fn view_distance_inside_the_simulation() {
        // view 4, simulation 10: what isn't loaded at entity ticking (beyond
        // the view distance) runs no entities, and block ticks stop where the
        // chunk is only full (beyond the view distance + 1).
        let l = levels(&[(0, 0)], &[], 4, 10);
        assert!(l.entity_ticking(4, 0) && !l.entity_ticking(5, 0));
        assert!(l.random_ticking(5, 0) && !l.random_ticking(6, 0));
        assert!(l.block_ticking(5, 5) && !l.block_ticking(6, 0));
    }

    #[test]
    fn every_player_and_forced_chunk() {
        let l = levels(&[(0, 0), (40, 0)], &[(-100, 7)], 4, 4);
        assert!(l.entity_ticking(44, 0) && l.entity_ticking(0, -4));
        assert!(!l.entity_ticking(20, 0));
        // A forced chunk entity ticks, its neighbours only block tick.
        assert!(l.entity_ticking(-100, 7) && !l.entity_ticking(-101, 7));
        assert!(l.block_ticking(-101, 8) && !l.block_ticking(-102, 7));
        assert!(!l.random_ticking(-101, 7));
        assert!(l.entity_ticking_at(-1600, 112) && !l.block_ticking_at(-1617, 112));
    }

    #[test]
    fn no_tickets_tick_nothing_and_no_sources_everything() {
        let none = levels(&[], &[], 10, 10);
        assert!(!none.block_ticking(0, 0) && !none.entity_ticking(0, 0));
        let everywhere = ChunkTickets::default().levels();
        assert!(everywhere.random_ticking(1000, -1000));
    }

    #[test]
    fn rebuilt_only_when_sources_change() {
        let mut t = ChunkTickets::default();
        let s = |players: Vec<(i32, i32)>| {
            Some(Sources {
                players,
                forced: Vec::new(),
                view_distance: 2,
                simulation_distance: 2,
            })
        };
        t.update(s(vec![(0, 0), (5, 5)]));
        let first = t.levels();
        t.update(s(vec![(5, 5), (0, 0)]));
        assert!(Arc::ptr_eq(
            first.0.as_ref().unwrap(),
            t.levels().0.as_ref().unwrap()
        ));
        t.update(s(vec![(1, 0), (5, 5)]));
        assert!(t.levels().entity_ticking(3, 0) && !first.entity_ticking(3, 0));
        t.update(None);
        assert!(t.levels().0.is_none());
    }
}
