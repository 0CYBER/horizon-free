//! Which chunks tick, from the chunk tickets of `DistanceManager`.
//!
//! Every player holds two kinds of ticket:
//! - `PLAYER_LOADING` at level 31 (`ENTITY_TICKING`) on every chunk within
//!   the server's view distance (Chebyshev), from `PlayerTicketTracker`;
//! - `PLAYER_SIMULATION` at level `31 - simulationDistance` on its own chunk.
//!
//! A `/forceload`ed chunk holds a `FORCED` ticket at level 31 of both kinds.
//! An entity that came through a portal leaves a `PORTAL` ticket of radius 3
//! (`Entity.placePortalTicket`: level 30 of both kinds, for 300 ticks) on the
//! chunk it arrived in; it keeps that level running with no player in it.
//! A saved one comes back with the level and the ticks it had left.
//! The dragon fight holds a `DRAGON` ticket of radius 9 for as long as a player
//! watches it (`EnderDragonFight.tick`: level 24 of both kinds on the arena,
//! so its island ticks out to 7 chunks whatever the watcher's simulation
//! distance), and drops it when the last watcher goes.
//! A viewed area (arismoko/horizon#3716, `lan::viewed_levels`) is a ticket of its own, never a
//! player's: a `Loaded` one is `PLAYER_LOADING` alone, `FULL` (33) over its
//! square and no simulation; a `Ticking` one is `PLAYER_SIMULATION` too,
//! entity ticking (31) over its square, rising by one a ring outward.
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
/// `ChunkLevel.FULL_CHUNK_LEVEL`: loaded, and no longer ticking.
pub const FULL: u8 = BLOCK_TICKING + 1;
/// The level of a chunk no ticket keeps `FULL` or better: unloaded as far as
/// the tickets go. Horizon keeps no level for the statuses below `FULL`, so
/// every level past it is this one.
pub const UNLOADED: u8 = FULL + 1;
/// What a chunk with no ticket near it has.
const NONE: (u8, u8) = (UNLOADED, UNLOADED);

/// `TicketType.PORTAL`'s timeout, in ticks.
const PORTAL_TIMEOUT: i64 = 300;
/// `Entity.placePortalTicket`'s radius: the ticket's level is `FULL` (33)
/// minus it.
const PORTAL_RADIUS: i32 = 3;
const PORTAL_LEVEL: i32 = FULL as i32 - PORTAL_RADIUS;
/// `EnderDragonFight.tick`'s radius on the arena it holds a ticket on:
/// `addTicketWithRadius(TicketType.DRAGON, new ChunkPos(0, 0), 9)`, whose level
/// is `FULL` (33) minus the radius.
const ARENA_RADIUS: i32 = 9;
const ARENA_LEVEL: i32 = FULL as i32 - ARENA_RADIUS;

/// The tickets of one level: where its players are, its forced chunks, its
/// portal tickets, the dragon fight's arena while a player watches it, and the
/// server's view and simulation distances.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub players: Vec<(i32, i32)>,
    pub forced: Vec<(i32, i32)>,
    /// Each `PORTAL` ticket's chunk and level.
    pub portal: Vec<((i32, i32), i32)>,
    /// The arena the dragon fight holds a `TicketType.DRAGON` ticket on while
    /// its boss bar has a watcher (`EnderDragonFight.tick`), `None` once none
    /// has (its `removeTicketWithRadius`).
    pub dragon: Option<(i32, i32)>,
    /// The viewed areas' tickets (arismoko/horizon#3716): never in `players` or `forced`.
    pub viewed: Vec<ViewedTicket>,
    pub view_distance: i32,
    pub simulation_distance: i32,
}

/// A viewed area's ticket: the chunks within `radius` of `center`, loaded only, or ticking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ViewedTicket {
    pub center: (i32, i32),
    pub radius: i32,
    pub ticking: bool,
}

impl ViewedTicket {
    /// How far from its centre it lowers a level: a `Ticking` area's block-ticking ring, and its
    /// `FULL` ring while the changes are recorded, past its square.
    fn reach(&self, recording: bool) -> i32 {
        if self.ticking {
            self.radius + 1 + i32::from(recording)
        } else {
            self.radius
        }
    }
    /// The (loading, simulation) level it gives a chunk `d` chunks (Chebyshev) from its centre.
    fn level(&self, d: i32) -> (i32, i32) {
        if self.ticking {
            let level = i32::from(ENTITY_TICKING) + (d - self.radius).max(0);
            (level, level)
        } else {
            (i32::from(FULL), i32::from(UNLOADED))
        }
    }
}

/// Each chunk's (loading, simulation) level, for the chunks where either is
/// block ticking or lower; `None` ticks everywhere (a game with no server and
/// no player: the headless fixtures). With them sits the box
/// `(min_x, min_z, max_x, max_z)` that holds every chunk that can random tick,
/// so a scan of the residents need not ask each one of them.
#[derive(Clone, Default)]
pub struct Levels(Option<Arc<Map>>, Option<(i32, i32, i32, i32)>);

/// The levels by chunk: a grid over the tickets' bounding box while it is
/// small (players near each other, the usual case), sparse 16×16 tiles otherwise.
/// Every block entity, mob and ticking chunk asks each tick. Tiling also avoids
/// hashing every chunk when a player crossing a chunk boundary rebuilds the map.
enum Map {
    Grid {
        x0: i32,
        z0: i32,
        width: u32,
        depth: u32,
        cells: Box<[(u8, u8)]>,
    },
    #[cfg(test)]
    Sparse(FxHashMap<(i32, i32), (u8, u8)>),
    Tiled(Tiled),
}

/// The largest grid, in chunks: 1 MiB of levels, a box 724 chunks wide.
const GRID_CELLS: u64 = 1 << 19;

impl Map {
    fn get(&self, x: i32, z: i32) -> (u8, u8) {
        match self {
            Self::Grid {
                x0,
                z0,
                width,
                depth,
                cells,
            } => {
                let (dx, dz) = (x.wrapping_sub(*x0) as u32, z.wrapping_sub(*z0) as u32);
                if dx < *width && dz < *depth {
                    cells[(dz * width + dx) as usize]
                } else {
                    NONE
                }
            }
            #[cfg(test)]
            Self::Sparse(map) => map.get(&(x, z)).copied().unwrap_or(NONE),
            Self::Tiled(map) => map.get(x, z),
        }
    }
    /// Each chunk whose loading level is `FULL` or better, with that level.
    fn for_each_loaded(&self, mut visit: impl FnMut(i32, i32, u8)) {
        match self {
            Self::Grid {
                x0,
                z0,
                width,
                cells,
                ..
            } => {
                for (i, &(loading, _)) in cells.iter().enumerate() {
                    if loading < UNLOADED {
                        let (dx, dz) = (i as u32 % width, i as u32 / width);
                        visit(
                            x0.wrapping_add(dx as i32),
                            z0.wrapping_add(dz as i32),
                            loading,
                        );
                    }
                }
            }
            #[cfg(test)]
            Self::Sparse(map) => {
                for (&(x, z), &(loading, _)) in map {
                    if loading < UNLOADED {
                        visit(x, z, loading);
                    }
                }
            }
            Self::Tiled(Tiled(tiles)) => {
                for (&(tx, tz), tile) in tiles {
                    for (i, &(loading, _)) in tile.iter().enumerate() {
                        if loading < UNLOADED {
                            visit(
                                tx * 16 + (i as i32 & 15),
                                tz * 16 + (i as i32 >> 4),
                                loading,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// A chunk's ticket level changing (`ChunkTicketLevelUpdatedEvent`), on
/// `ChunkLevel`'s scale: `ENTITY_TICKING` and lower tick, `BLOCK_TICKING`
/// runs blocks only, `FULL` is loaded and still, and `UNLOADED` is where no
/// ticket reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelChange {
    /// Chunk coordinates.
    pub chunk: (i32, i32),
    pub old: u8,
    pub new: u8,
}

/// The chunks whose loading level differs between two maps (`None`: no
/// tickets, every chunk unloaded), by chunk.
fn level_changes(old: Option<&Map>, new: Option<&Map>) -> Vec<LevelChange> {
    let level = |map: Option<&Map>, x, z| map.map_or(UNLOADED, |m| m.get(x, z).0);
    let mut changes = Vec::new();
    if let Some(old) = old {
        old.for_each_loaded(|x, z, was| {
            let now = level(new, x, z);
            if now != was {
                changes.push(LevelChange {
                    chunk: (x, z),
                    old: was,
                    new: now,
                });
            }
        });
    }
    if let Some(new) = new {
        new.for_each_loaded(|x, z, now| {
            // A chunk the old map held was compared above.
            if level(old, x, z) == UNLOADED {
                changes.push(LevelChange {
                    chunk: (x, z),
                    old: UNLOADED,
                    new: now,
                });
            }
        });
    }
    changes.sort_unstable_by_key(|change| change.chunk);
    changes
}

type Tile = Box<[(u8, u8); 256]>;

/// Sparse tiles keep each nearby run of levels contiguous. Construction hashes
/// once per intersecting tile rather than once per chunk of every ticket.
struct Tiled(FxHashMap<(i32, i32), Tile>);
impl Tiled {
    /// The same levels as the grid, for tickets too far apart for one.
    fn new(s: &Sources, recording: bool) -> Self {
        let mut map = Self(FxHashMap::default());
        let (view, sim, reach) = reach(s, recording);
        for &p in &s.players {
            map.add(p, reach, i32::from(ENTITY_TICKING), view, sim);
        }
        for &p in &s.forced {
            map.add(
                p,
                1 + i32::from(recording),
                i32::from(ENTITY_TICKING),
                0,
                i32::from(ENTITY_TICKING),
            );
        }
        for &(p, level) in &s.portal {
            map.add(p, ticket_radius(level, recording), level, 0, level);
        }
        if let Some(arena) = s.dragon {
            map.add(
                arena,
                ARENA_RADIUS - i32::from(!recording),
                ARENA_LEVEL,
                0,
                ARENA_LEVEL,
            );
        }
        for t in &s.viewed {
            map.add_with(t.center, t.reach(recording), |d| t.level(d));
        }
        map
    }
    fn get(&self, x: i32, z: i32) -> (u8, u8) {
        self.0
            .get(&(x >> 4, z >> 4))
            .map_or(NONE, |tile| tile[((z & 15) * 16 + (x & 15)) as usize])
    }
    /// Lower each cell to the minimum of its level and the source plus distance;
    /// loading stays flat within the view distance.
    fn add(&mut self, p: (i32, i32), r: i32, loading: i32, flat: i32, sim: i32) {
        self.add_with(p, r, |d| (loading + (d - flat).max(0), sim + d));
    }
    /// Lower each cell within `r` of `(px, pz)` to the minimum of its levels and `level` at its
    /// distance.
    fn add_with(&mut self, (px, pz): (i32, i32), r: i32, level: impl Fn(i32) -> (i32, i32)) {
        let (x0, z0, x1, z1) = (px - r, pz - r, px + r, pz + r);
        for tz in z0 >> 4..=z1 >> 4 {
            for tx in x0 >> 4..=x1 >> 4 {
                let tile = self
                    .0
                    .entry((tx, tz))
                    .or_insert_with(|| Box::new([NONE; 256]));
                let (bx, bz) = (tx * 16, tz * 16);
                for z in z0.max(bz)..=z1.min(bz + 15) {
                    for x in x0.max(bx)..=x1.min(bx + 15) {
                        let (loading, sim) = level((x - px).abs().max((z - pz).abs()));
                        let e = &mut tile[((z - bz) * 16 + (x - bx)) as usize];
                        e.0 = e.0.min(loading.min(i32::from(NONE.0)) as u8);
                        e.1 = e.1.min(sim.min(i32::from(NONE.1)) as u8);
                    }
                }
            }
        }
    }
}

impl Levels {
    /// Chunk coordinates.
    fn get(&self, x: i32, z: i32) -> Option<(u8, u8)> {
        Some(self.0.as_ref()?.get(x, z))
    }
    /// `DistanceManager.inEntityTickingRange`, in an entity-ticking chunk:
    /// where entities tick.
    pub fn entity_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= ENTITY_TICKING && s <= ENTITY_TICKING)
    }
    /// `DistanceManager.inEntityTickingRange` alone: the simulation level,
    /// whatever the chunk's own status. `ChunkMap.tick` sends an entity's
    /// changes there only (or when it changed section or needs sync).
    pub fn in_entity_ticking_range(&self, x: i32, z: i32) -> bool {
        self.get(x, z).is_none_or(|(_, s)| s <= ENTITY_TICKING)
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
    /// The box that holds every chunk `random_ticking` can answer for, or
    /// `None` with no tickets at all, where it ticks everywhere. A box whose
    /// minimum is past its maximum holds nothing: the tickets hold no player,
    /// no forced chunk and no `PORTAL` one, so nothing random ticks.
    pub fn random_tick_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.1
    }
    /// The loading track has `FullChunkStatus.ENTITY_TICKING`, independently
    /// of whether the simulation tracker also permits entities to tick.
    pub fn loaded_entity_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(loading, _)| loading <= ENTITY_TICKING)
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

/// A `TicketType.PORTAL` ticket, as `chunk_tickets.dat` saves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalTicket {
    pub chunk: (i32, i32),
    pub level: i32,
    pub ticks_left: i64,
}

/// The levels, rebuilt only when their sources change (a player crossing
/// into another chunk, a distance or a forced chunk changing).
#[derive(Default)]
pub struct ChunkTickets {
    sources: Option<Sources>,
    levels: Levels,
    portal: Vec<PortalTicket>,
    /// Whether [`Self::update`] keeps the level changes it makes.
    recording: bool,
    /// The level changes since the game last took them, in the order the
    /// updates made them.
    changes: Vec<LevelChange>,
}

impl ChunkTickets {
    /// `None`: tick everywhere.
    pub fn update(&mut self, mut sources: Option<Sources>) {
        // The game submits a fresh `Sources` value every tick even when no
        // player or ticket moved.  The normalized vectors are already held
        // in `self.sources`; when the caller preserves their order, avoid
        // sorting/deduplicating them and rebuilding the portal list just to
        // discover that the levels are unchanged.  If a caller supplies the
        // same set in another order, the existing normalization path below
        // remains authoritative.
        if let (Some(next), Some(old)) = (&sources, &self.sources)
            && next.view_distance == old.view_distance
            && next.simulation_distance == old.simulation_distance
            && next.players == old.players
            && next.forced == old.forced
            && next.dragon == old.dragon
            && next.viewed == old.viewed
            && self.portal_matches(old)
        {
            return;
        }
        if let Some(s) = sources.as_mut() {
            s.players.sort_unstable();
            s.players.dedup();
            s.forced.sort_unstable();
            s.viewed.sort_unstable();
            s.portal = self.portal.iter().map(|t| (t.chunk, t.level)).collect();
            s.portal.sort_unstable();
        }
        if sources == self.sources {
            return;
        }
        let levels = match sources.as_ref() {
            Some(s) => {
                let map = Some(Arc::new(compute(s, self.recording)));
                // Tickets with no random tick of their own still get a box, an
                // empty one: `None` is for a game with no tickets at all.
                Levels(map, Some(random_bounds(s).unwrap_or((1, 1, 0, 0))))
            }
            None => Levels(None, None),
        };
        if self.recording {
            self.changes
                .extend(level_changes(self.levels.0.as_deref(), levels.0.as_deref()));
        }
        self.levels = levels;
        self.sources = sources;
    }

    fn portal_matches(&self, sources: &Sources) -> bool {
        self.portal.len() == sources.portal.len()
            && sources.portal.iter().all(|chunk| {
                self.portal
                    .iter()
                    .any(|ticket| (ticket.chunk, ticket.level) == *chunk)
            })
    }
    /// Keep (or stop keeping) each chunk's level changes, for a
    /// `chunk_ticket_level_updated` listener. With `false`, updates omit the
    /// extra FULL ring and the change scan. Switching modes rebuilds the
    /// baseline without emitting changes; unchanged modes do no work.
    pub fn record_changes(&mut self, on: bool) {
        if self.recording != on {
            if let Some(s) = &self.sources {
                self.levels.0 = Some(Arc::new(compute(s, on)));
            }
            self.recording = on;
        }
        if !on {
            self.changes.clear();
        }
    }
    /// The level changes kept since the last call, in order: each update's by
    /// chunk, one entry for each update that changed the chunk, so that one
    /// chunk's entries chain from the old level of the first to the new of
    /// the last. A game with no tickets (`update(None)`) holds every chunk
    /// unloaded.
    pub fn take_changes(&mut self) -> Vec<LevelChange> {
        std::mem::take(&mut self.changes)
    }
    pub fn levels(&self) -> Levels {
        self.levels.clone()
    }
    /// A level's tickets as `TicketStorage.activateAllDeactivatedTickets`
    /// leaves its saved `PORTAL` ones: each keeps its level and the ticks it
    /// had left, as `addTicket` adds them in turn.
    pub fn with_saved_portal(saved: Vec<PortalTicket>) -> Self {
        let mut tickets = Self::default();
        for ticket in saved {
            tickets.add(ticket);
        }
        tickets
    }
    /// `TicketStorage.addTicket`: one of the same level already on the chunk
    /// only starts its timeout again.
    fn add(&mut self, ticket: PortalTicket) {
        match self
            .portal
            .iter_mut()
            .find(|t| t.chunk == ticket.chunk && t.level == ticket.level)
        {
            Some(t) => t.ticks_left = PORTAL_TIMEOUT,
            None => self.portal.push(ticket),
        }
    }
    /// `Entity.placePortalTicket`: a `PORTAL` ticket of radius 3.
    pub fn add_portal(&mut self, chunk: (i32, i32)) {
        self.add(PortalTicket {
            chunk,
            level: PORTAL_LEVEL,
            ticks_left: PORTAL_TIMEOUT,
        });
    }
    /// `TicketStorage.purgeStaleTickets` for the `PORTAL` tickets, once a
    /// tick: a ticket counts down only where `can_expire` (`canTicketExpire`:
    /// `PORTAL` waits for its chunk to finish loading) and goes below zero,
    /// `PORTAL_TIMEOUT + 1` counted ticks after it was placed.
    pub fn tick_portal(&mut self, mut can_expire: impl FnMut(&PortalTicket) -> bool) {
        self.portal.retain_mut(|t| {
            if !can_expire(t) {
                return true;
            }
            // `Ticket.decreaseTicksLeft`: a long, which wraps.
            t.ticks_left = t.ticks_left.wrapping_sub(1);
            t.ticks_left >= 0
        });
    }
    /// While one is held, it keeps its level active (`shouldKeepDimensionActive`).
    pub fn portal_tickets(&self) -> &[PortalTicket] {
        &self.portal
    }
}

/// How far a ticket of `level` reaches with levels of `FULL` or
/// lower: one more a chunk away; levels above `FULL` count as unloaded.
fn ticket_radius(level: i32, recording: bool) -> i32 {
    (i32::from(BLOCK_TICKING) + i32::from(recording) - level).max(0)
}

/// The box that holds every chunk `random_ticking` can answer for: a player's
/// simulation ticket reaches its simulation distance, a forced chunk only ever
/// random ticks in the chunk itself, and a `PORTAL` ticket inside its whole
/// area. Each is a superset of that ticket's random-ticking chunks, so the box
/// never rejects a chunk that random ticks.
fn random_bounds(s: &Sources) -> Option<(i32, i32, i32, i32)> {
    let sim = s.simulation_distance.clamp(0, 32);
    let boxes = s
        .players
        .iter()
        .map(|&p| (p, sim))
        .chain(s.forced.iter().map(|&f| (f, 0)))
        .chain(
            s.portal
                .iter()
                .filter(|&&(_, level)| level <= i32::from(ENTITY_TICKING))
                .map(|&(p, level)| (p, i32::from(ENTITY_TICKING) - level)),
        )
        .chain(s.dragon.map(|arena| (arena, ARENA_RADIUS)))
        .chain(
            s.viewed
                .iter()
                .filter(|t| t.ticking)
                .map(|t| (t.center, t.radius)),
        );
    boxes.fold(None, |b, ((x, z), r)| {
        let (lo_x, lo_z, hi_x, hi_z) = (x - r, z - r, x + r, z + r);
        Some(match b {
            None => (lo_x, lo_z, hi_x, hi_z),
            Some((a, b, c, d)) => (lo_x.min(a), lo_z.min(b), hi_x.max(c), hi_z.max(d)),
        })
    })
}

/// How far a player's tickets reach, and the simulation ticket's level.
fn reach(s: &Sources, recording: bool) -> (i32, i32, i32) {
    let view = s.view_distance.clamp(0, 32);
    let simulation = s.simulation_distance.clamp(0, 32);
    // PLAYER_LOADING within the view distance, PLAYER_SIMULATION at the
    // player's chunk (`getPlayerTicketLevel`: never below 0).
    let player_sim = (i32::from(ENTITY_TICKING) - simulation).max(0);
    let reach = (view + 1 + i32::from(recording))
        .max(i32::from(BLOCK_TICKING) + i32::from(recording) - player_sim);
    (view, player_sim, reach)
}

fn compute(s: &Sources, recording: bool) -> Map {
    let (view, player_sim, reach) = reach(s, recording);
    let boxes = s
        .players
        .iter()
        .map(|&p| (p, reach))
        .chain(s.forced.iter().map(|&f| (f, 1 + i32::from(recording))))
        .chain(
            s.portal
                .iter()
                .map(|&(p, level)| (p, ticket_radius(level, recording))),
        )
        .chain(s.dragon.map(|arena| (arena, ARENA_RADIUS)))
        .chain(s.viewed.iter().map(|t| (t.center, t.reach(recording))));
    let Some((x0, z0, x1, z1)) = boxes.fold(None, |b, ((x, z), r)| {
        let (lo_x, lo_z, hi_x, hi_z) = (
            i64::from(x) - i64::from(r),
            i64::from(z) - i64::from(r),
            i64::from(x) + i64::from(r),
            i64::from(z) + i64::from(r),
        );
        Some(match b {
            None => (lo_x, lo_z, hi_x, hi_z),
            Some((a, b, c, d)) => (lo_x.min(a), lo_z.min(b), hi_x.max(c), hi_z.max(d)),
        })
    }) else {
        return Map::Tiled(Tiled(FxHashMap::default()));
    };
    let (width, depth) = ((x1 - x0 + 1) as u64, (z1 - z0 + 1) as u64);
    if width * depth > GRID_CELLS {
        return Map::Tiled(Tiled::new(s, recording));
    }
    let (x0, z0, width, depth) = (x0 as i32, z0 as i32, width as u32, depth as u32);
    let mut cells = vec![NONE; (width * depth) as usize].into_boxed_slice();
    let mut lower = |x: i32, z: i32, loading: i32, sim: i32| {
        let i = ((z - z0) as u32 * width + (x - x0) as u32) as usize;
        let e = &mut cells[i];
        e.0 = e.0.min(loading.min(i32::from(NONE.0)) as u8);
        e.1 = e.1.min(sim.min(i32::from(NONE.1)) as u8);
    };
    for &(px, pz) in &s.players {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let d = dx.abs().max(dz.abs());
                lower(
                    px + dx,
                    pz + dz,
                    i32::from(ENTITY_TICKING) + (d - view).max(0),
                    player_sim + d,
                );
            }
        }
    }
    for &(fx, fz) in &s.forced {
        for dz in -(1 + i32::from(recording))..=1 + i32::from(recording) {
            for dx in -(1 + i32::from(recording))..=1 + i32::from(recording) {
                let level = i32::from(ENTITY_TICKING) + dx.abs().max(dz.abs());
                lower(fx + dx, fz + dz, level, level);
            }
        }
    }
    for &((cx, cz), base) in &s.portal {
        let r = ticket_radius(base, recording);
        for dz in -r..=r {
            for dx in -r..=r {
                let level = base + dx.abs().max(dz.abs());
                lower(cx + dx, cz + dz, level, level);
            }
        }
    }
    if let Some((ax, az)) = s.dragon {
        for dz in -ARENA_RADIUS..=ARENA_RADIUS {
            for dx in -ARENA_RADIUS..=ARENA_RADIUS {
                let level = ARENA_LEVEL + dx.abs().max(dz.abs());
                lower(ax + dx, az + dz, level, level);
            }
        }
    }
    for t in &s.viewed {
        let ((cx, cz), r) = (t.center, t.reach(recording));
        for dz in -r..=r {
            for dx in -r..=r {
                let (loading, sim) = t.level(dx.abs().max(dz.abs()));
                lower(cx + dx, cz + dz, loading, sim);
            }
        }
    }
    Map::Grid {
        x0,
        z0,
        width,
        depth,
        cells,
    }
}

/// The old per-chunk map: the reference the grid and tiles are tested against.
#[cfg(test)]
fn compute_sparse(s: &Sources) -> FxHashMap<(i32, i32), (u8, u8)> {
    let recording = true;
    let (view, player_sim, reach) = reach(s, recording);
    let mut levels: FxHashMap<(i32, i32), (u8, u8)> = FxHashMap::default();
    let mut lower = |key, loading: i32, sim: i32| {
        let e = levels.entry(key).or_insert(NONE);
        e.0 = e.0.min(loading.min(i32::from(NONE.0)) as u8);
        e.1 = e.1.min(sim.min(i32::from(NONE.1)) as u8);
    };
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
        for dz in -(1 + i32::from(recording))..=1 + i32::from(recording) {
            for dx in -(1 + i32::from(recording))..=1 + i32::from(recording) {
                let level = i32::from(ENTITY_TICKING) + dx.abs().max(dz.abs());
                lower((fx + dx, fz + dz), level, level);
            }
        }
    }
    for &((cx, cz), base) in &s.portal {
        let r = ticket_radius(base, recording);
        for dz in -r..=r {
            for dx in -r..=r {
                let level = base + dx.abs().max(dz.abs());
                lower((cx + dx, cz + dz), level, level);
            }
        }
    }
    if let Some((ax, az)) = s.dragon {
        for dz in -ARENA_RADIUS..=ARENA_RADIUS {
            for dx in -ARENA_RADIUS..=ARENA_RADIUS {
                let level = ARENA_LEVEL + dx.abs().max(dz.abs());
                lower((ax + dx, az + dz), level, level);
            }
        }
    }
    for t in &s.viewed {
        let ((cx, cz), r) = (t.center, t.reach(recording));
        for dz in -r..=r {
            for dx in -r..=r {
                let (loading, sim) = t.level(dx.abs().max(dz.abs()));
                lower((cx + dx, cz + dz), loading, sim);
            }
        }
    }
    levels.retain(|_, l| *l != NONE);
    levels
}

#[cfg(test)]
mod tests {
    #[test]
    fn recording_controls_reach_and_baseline() {
        for (sources, off_cells, on_cells, edge) in [
            (
                Sources {
                    players: vec![(0, 0)],
                    view_distance: 32,
                    ..Sources::default()
                },
                4489,
                4761,
                34,
            ),
            (
                Sources {
                    forced: vec![(0, 0)],
                    ..Sources::default()
                },
                9,
                25,
                2,
            ),
            (
                Sources {
                    portal: vec![((0, 0), 30)],
                    ..Sources::default()
                },
                25,
                49,
                3,
            ),
        ] {
            for recording in [false, true] {
                let map = compute(&sources, recording);
                let Map::Grid { ref cells, .. } = map else {
                    panic!("expected grid")
                };
                assert_eq!(cells.len(), if recording { on_cells } else { off_cells });
                assert_eq!(map.get(edge, 0).0, if recording { FULL } else { UNLOADED });
                if !recording {
                    assert!(cells.iter().all(|&(loading, _)| loading <= BLOCK_TICKING));
                }
                let tiled = Tiled::new(&sources, recording);
                for z in -36..=36 {
                    for x in -36..=36 {
                        assert_eq!(map.get(x, z), tiled.get(x, z));
                    }
                }
            }
        }
        let boundary = Sources {
            forced: vec![(14, 14)],
            ..Sources::default()
        };
        assert_eq!(Tiled::new(&boundary, false).0.len(), 1);
        assert_eq!(Tiled::new(&boundary, true).0.len(), 4);
        let mut tickets = ChunkTickets::default();
        let sources = Some(Sources {
            forced: vec![(15, 15)],
            ..Sources::default()
        });
        tickets.update(sources.clone());
        assert_eq!(tickets.levels.0.as_ref().unwrap().get(17, 15).0, UNLOADED);
        tickets.record_changes(true);
        assert_eq!(tickets.levels.0.as_ref().unwrap().get(17, 15).0, FULL);
        tickets.update(sources.clone());
        assert!(tickets.take_changes().is_empty());
        tickets.record_changes(false);
        assert_eq!(tickets.levels.0.as_ref().unwrap().get(17, 15).0, UNLOADED);
        tickets.update(sources);
        assert!(tickets.take_changes().is_empty());
        tickets.record_changes(true);
        tickets.update(Some(Sources {
            forced: vec![(16, 15)],
            ..Sources::default()
        }));
        assert_eq!(tickets.levels.0.as_ref().unwrap().get(18, 15).0, FULL);
        assert!(
            tickets
                .take_changes()
                .iter()
                .any(|c| c.chunk == (18, 15) && c.new == FULL)
        );
    }

    #[test]
    fn recording_preserves_ticking_predicates() {
        for view in [2, 4, 8, 16, 32] {
            for sim in [2, 4, 8, 16, 32] {
                for combo in 0..5 {
                    let s = Sources {
                        players: vec![(0, 0)],
                        forced: if combo >= 1 { vec![(50, 0)] } else { vec![] },
                        portal: if combo >= 2 {
                            vec![((0, 50), 30)]
                        } else {
                            vec![]
                        },
                        dragon: (combo >= 3).then_some((50, 50)),
                        viewed: Vec::new(),
                        view_distance: view,
                        simulation_distance: sim,
                    };
                    let a = Levels(Some(Arc::new(compute(&s, false))), random_bounds(&s));
                    let b = Levels(Some(Arc::new(compute(&s, true))), random_bounds(&s));
                    assert_eq!(a.random_tick_bounds(), b.random_tick_bounds());
                    for z in -36..=60 {
                        for x in -36..=60 {
                            for predicate in [
                                Levels::entity_ticking,
                                Levels::in_entity_ticking_range,
                                Levels::block_ticking,
                                Levels::random_ticking,
                                Levels::loaded_entity_ticking,
                                Levels::loaded_ticking,
                            ] {
                                assert_eq!(predicate(&a, x, z), predicate(&b, x, z));
                            }
                        }
                    }
                    assert!(a.entity_ticking(0, 0));
                    assert!(!a.entity_ticking(100, 100));
                }
            }
        }
    }

    use super::*;

    fn levels(players: &[(i32, i32)], forced: &[(i32, i32)], view: i32, sim: i32) -> Levels {
        let mut t = ChunkTickets::default();
        t.update(Some(Sources {
            players: players.to_vec(),
            forced: forced.to_vec(),
            portal: Vec::new(),
            dragon: None,
            viewed: Vec::new(),
            view_distance: view,
            simulation_distance: sim,
        }));
        t.levels()
    }

    /// A viewed `Ticking` area entity-ticks its square and block-ticks the ring past it; a
    /// `Loaded` one is loaded and ticks nothing. Neither is a player's or a forced chunk's
    /// ticket, and the grid and the tiles agree.
    #[test]
    fn viewed_areas_tick_by_their_mode_only() {
        let s = Sources {
            viewed: vec![
                ViewedTicket {
                    center: (0, 0),
                    radius: 2,
                    ticking: true,
                },
                ViewedTicket {
                    center: (100, 0),
                    radius: 2,
                    ticking: false,
                },
            ],
            ..Sources::default()
        };
        let mut t = ChunkTickets::default();
        t.update(Some(s.clone()));
        let l = t.levels();
        assert!(l.entity_ticking(2, -2) && l.random_ticking(-2, 2));
        assert!(!l.entity_ticking(3, 0) && l.block_ticking(3, 3) && !l.random_ticking(3, 0));
        assert!(!l.block_ticking(4, 0));
        assert!(l.loaded_ticking(3, 3) && !l.loaded_ticking(4, 4));
        for (x, z) in [(100, 0), (102, 2), (98, -2)] {
            assert!(!l.block_ticking(x, z) && !l.entity_ticking(x, z), "{x} {z}");
            assert_eq!(l.get(x, z), Some((FULL, UNLOADED)));
        }
        assert_eq!(l.get(103, 0), Some(NONE));
        assert!(s.players.is_empty() && s.forced.is_empty());
        for recording in [false, true] {
            let tiled = Map::Tiled(Tiled::new(&s, recording));
            let grid = compute(&s, recording);
            for z in -6..=6 {
                for x in -6..=106 {
                    assert_eq!(tiled.get(x, z), grid.get(x, z), "{x} {z} {recording}");
                }
            }
        }
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
        // A command asks the loading status, not the smaller simulation area.
        assert!(l.loaded_entity_ticking(8, 8));
        assert!(!l.loaded_entity_ticking(9, 0));
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
        assert!(l.loaded_entity_ticking(4, 0));
        assert!(!l.loaded_entity_ticking(5, 0));
        // ChunkMap.tick's sendChanges reads the simulation level alone.
        assert!(l.in_entity_ticking_range(10, 0) && !l.in_entity_ticking_range(11, 0));
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

    /// `EnderDragonFight.tick` holds the arena's `TicketType.DRAGON` ticket
    /// (radius 9, level 24) for as long as a player watches the fight, so the
    /// island ticks with its watcher far outside their simulation distance,
    /// and drops it when the last watcher goes.
    #[test]
    fn a_watched_dragon_arena_ticks_beyond_the_watchers_simulation_distance() {
        let mut t = ChunkTickets::default();
        // A watcher 12 chunks out whose simulation distance is 2: nothing
        // around (0, 0) ticks for it.
        let sources = |dragon| {
            Some(Sources {
                players: vec![(12, 0)],
                forced: Vec::new(),
                portal: Vec::new(),
                dragon,
                viewed: Vec::new(),
                view_distance: 12,
                simulation_distance: 2,
            })
        };
        t.update(sources(None));
        let lonely = t.levels();
        assert!(!lonely.entity_ticking(0, 0) && !lonely.block_ticking(0, 0));
        assert!(!lonely.random_ticking(3, 3));
        t.update(sources(Some((0, 0))));
        let watched = t.levels();
        // Level 24 on the arena rising one a chunk away: entities out to 7
        // chunks, block ticks and scheduled ticks out to 8.
        assert!(watched.entity_ticking(0, 0) && watched.entity_ticking(-7, 7));
        assert!(!watched.entity_ticking(8, 0) && watched.block_ticking(8, -8));
        assert!(watched.random_ticking(-7, 7) && !watched.random_ticking(8, 0));
        assert!(watched.loaded_ticking(8, 0) && !watched.block_ticking(9, 9));
        // The watcher's own tickets are what they were.
        assert!(watched.entity_ticking(12, 0) && !watched.entity_ticking(15, 0));
        // No watcher: the arena goes back to nobody's ticket.
        t.update(sources(None));
        assert!(!t.levels().block_ticking(0, 0));
    }

    #[test]
    fn rebuilt_only_when_sources_change() {
        let mut t = ChunkTickets::default();
        let s = |players: Vec<(i32, i32)>| {
            Some(Sources {
                players,
                forced: Vec::new(),
                portal: Vec::new(),
                dragon: None,
                viewed: Vec::new(),
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

    /// The grid answers as the map does, everywhere around the tickets.
    #[test]
    #[ignore = "daily"]
    fn grid_matches_the_map() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * n as u64 + 1)) as i32 - n
        };
        for case in 0..200 {
            let spread = [3, 20, 80][case % 3];
            let players: Vec<_> = (0..1 + case % 12)
                .map(|_| (next(spread), next(spread)))
                .collect();
            let forced: Vec<_> = (0..case % 4)
                .map(|_| (next(spread), next(spread)))
                .collect();
            let s = Sources {
                players,
                forced,
                portal: Vec::new(),
                // Tiled maps hold the arena too.
                dragon: (case % 3 == 0).then(|| (next(spread), next(spread))),
                viewed: Vec::new(),
                view_distance: 2 + next(12).abs(),
                simulation_distance: 2 + next(12).abs(),
            };
            let Map::Grid { .. } = compute(&s, true) else {
                panic!("case {case} fell back to the map");
            };
            let (grid, map) = (compute(&s, true), Map::Sparse(compute_sparse(&s)));
            let r = spread + 50;
            for z in -r..=r {
                for x in -r..=r {
                    assert_eq!(grid.get(x, z), map.get(x, z), "case {case} at ({x}, {z})");
                }
            }
        }
    }

    #[test]
    fn far_apart_tickets_tick_only_their_neighbourhoods() {
        let l = levels(
            &[(0, 0), (100_000, -100_000)],
            &[(i32::MAX - 2, i32::MIN + 2)],
            10,
            10,
        );
        assert!(matches!(**l.0.as_ref().unwrap(), Map::Tiled(_)));
        assert!(l.entity_ticking(100_010, -100_010) && !l.block_ticking(50_000, 0));
        assert!(l.entity_ticking(i32::MAX - 2, i32::MIN + 2));
    }

    /// `tick_blocks` filters its candidates with the bounds box before it asks
    /// `random_ticking`: whatever random ticks is inside it, every ticket's
    /// own area, a `PORTAL` ticket's with no player near it included.
    #[test]
    fn random_tick_bounds_cover_every_random_ticking_chunk() {
        // A saved ticket of a lower level random ticks further out.
        let mut t = ChunkTickets::with_saved_portal(vec![PortalTicket {
            chunk: (-60, 200),
            level: 20,
            ticks_left: 1,
        }]);
        // A view distance shorter than the simulation one, two players far
        // apart, a forced chunk, `PORTAL` tickets that outlive them, and the
        // dragon fight's arena, which random ticks with nobody near it.
        let sources = || {
            Some(Sources {
                players: vec![(60, 40), (80, 80)],
                forced: vec![(-30, 17)],
                portal: Vec::new(),
                dragon: Some((0, 0)),
                viewed: Vec::new(),
                view_distance: 4,
                simulation_distance: 12,
            })
        };
        t.update(sources());
        for portal in [(0, 0), (25, -25), (300, 300), (-80, 95)] {
            t.add_portal(portal);
            t.update(sources());
            let levels = t.levels();
            let (min_x, min_z, max_x, max_z) = levels.random_tick_bounds().unwrap();
            for z in -100..=310 {
                for x in -100..=310 {
                    if levels.random_ticking(x, z) {
                        assert!(
                            x >= min_x && x <= max_x && z >= min_z && z <= max_z,
                            "({x}, {z}) random ticks outside the box"
                        );
                    }
                }
            }
        }
    }

    /// A saved `PORTAL` ticket keeps its own level, one more a chunk away
    /// (`ChunkTracker`): 28 entity ticks out to three chunks (31) and block
    /// ticks at four (32); 32 block ticks its own chunk only.
    #[test]
    fn saved_portal_tickets_keep_their_level() {
        let ticket = |chunk, level| PortalTicket {
            chunk,
            level,
            ticks_left: 5,
        };
        let mut t = ChunkTickets::with_saved_portal(vec![ticket((0, 0), 28), ticket((40, 0), 32)]);
        t.update(Some(Sources::default()));
        let l = t.levels();
        assert!(l.entity_ticking(3, -3) && !l.entity_ticking(4, 0));
        assert!(l.block_ticking(4, 4) && !l.block_ticking(5, 0));
        assert!(l.block_ticking(40, 0) && !l.entity_ticking(40, 0));
        assert!(!l.block_ticking(41, 0));
    }

    fn one_player(chunk: (i32, i32), view: i32, simulation: i32) -> Option<Sources> {
        Some(Sources {
            players: vec![chunk],
            forced: Vec::new(),
            portal: Vec::new(),
            dragon: None,
            viewed: Vec::new(),
            view_distance: view,
            simulation_distance: simulation,
        })
    }

    fn change(chunk: (i32, i32), old: u8, new: u8) -> LevelChange {
        LevelChange { chunk, old, new }
    }

    /// A player's loading level is 31 (entity ticking) out to the view
    /// distance, then one more a chunk: 32 block ticking, 33 full, and
    /// nothing past it. Chunk (5, 0) of a player with view 4 changes level
    /// four times as the player moves in one tick (several updates of the
    /// tickets): the entries chain, each `old` the last `new`, and a level the
    /// chunk keeps through an update has no entry.
    #[test]
    fn several_changes_to_one_chunk_chain_old_to_new() {
        let mut t = ChunkTickets::default();
        t.record_changes(true);
        let chunk = (5, 0);
        t.update(one_player((0, 0), 4, 4));
        assert!(t.take_changes().contains(&change(chunk, UNLOADED, 32)));
        // 2 away, 4 away (still 31), 6 away, 5 away, 10 away.
        for at in [(3, 0), (9, 0), (-1, 0), (0, 0), (-5, 0)] {
            t.update(one_player(at, 4, 4));
        }
        let chain: Vec<_> = t
            .take_changes()
            .into_iter()
            .filter(|c| c.chunk == chunk)
            .collect();
        assert_eq!(
            chain,
            [
                change(chunk, 32, 31),
                change(chunk, 31, 33),
                change(chunk, 33, 32),
                change(chunk, 32, UNLOADED),
            ]
        );
    }

    #[test]
    fn full_and_unloaded_stay_apart_from_the_ticking_levels() {
        let mut t = ChunkTickets::default();
        t.record_changes(true);
        t.update(one_player((0, 0), 4, 4));
        let _ = t.take_changes();
        // View 4: 6 away is level 33 (full: loaded, not ticking), 7 away is
        // past it. A player a chunk nearer turns 7 into 33 and 6 into 32.
        t.update(one_player((1, 0), 4, 4));
        let nearer = t.take_changes();
        assert!(nearer.contains(&change((7, 0), UNLOADED, 33)));
        assert!(nearer.contains(&change((6, 0), 33, 32)));
        // Two chunks away from there, a full chunk is unloaded in one step
        // from 33, and a block-ticking one from 32: nothing is clamped to
        // the ticking range.
        t.update(one_player((-1, 0), 4, 4));
        let away = t.take_changes();
        assert!(away.contains(&change((7, 0), 33, UNLOADED)));
        assert!(away.contains(&change((6, 0), 32, UNLOADED)));
        assert!(away.contains(&change((5, 0), 31, 33)));
    }

    #[test]
    fn unchanged_levels_report_nothing() {
        let mut t = ChunkTickets::default();
        t.record_changes(true);
        t.update(one_player((0, 0), 4, 4));
        let _ = t.take_changes();
        t.update(one_player((0, 0), 4, 4));
        assert!(t.take_changes().is_empty());
        // A simulation distance changes the simulation track, not a loading
        // level.
        t.update(one_player((0, 0), 4, 2));
        assert!(t.take_changes().is_empty());
        // One chunk over: the chunks both positions keep at 31 are not told.
        t.update(one_player((1, 0), 4, 2));
        let moved = t.take_changes();
        assert!(moved.iter().all(|c| c.old != c.new));
        assert!(
            !moved
                .iter()
                .any(|c| (-3..=4).contains(&c.chunk.0) && (-4..=4).contains(&c.chunk.1))
        );
        assert!(moved.contains(&change((-5, 0), 32, 33)));
        assert!(moved.contains(&change((-6, 0), 33, UNLOADED)));
    }

    #[test]
    fn nothing_is_kept_unless_asked() {
        let mut t = ChunkTickets::default();
        t.update(one_player((0, 0), 4, 4));
        t.update(one_player((40, 0), 4, 4));
        assert!(t.take_changes().is_empty());
        assert_eq!(
            t.changes.capacity(),
            0,
            "an update with no listener allocated"
        );
        t.record_changes(true);
        t.update(one_player((0, 0), 4, 4));
        t.record_changes(false);
        assert!(t.take_changes().is_empty());
    }

    /// An update tells its chunks by position; tickets that were not there
    /// load every chunk they keep, from unloaded, and no tickets (`None`)
    /// unload them all.
    #[test]
    fn a_first_update_loads_every_chunk_and_none_unloads_them() {
        let mut t = ChunkTickets::default();
        t.record_changes(true);
        t.update(Some(Sources {
            players: Vec::new(),
            forced: vec![(10, 10)],
            portal: Vec::new(),
            dragon: None,
            viewed: Vec::new(),
            view_distance: 2,
            simulation_distance: 2,
        }));
        let loaded = t.take_changes();
        // A forced chunk is 31, 32 a ring out and 33 two rings out: 5x5.
        assert_eq!(loaded.len(), 25);
        assert_eq!(loaded[0], change((8, 8), UNLOADED, 33));
        assert_eq!(loaded[12], change((10, 10), UNLOADED, 31));
        assert_eq!(loaded[24], change((12, 12), UNLOADED, 33));
        assert!(loaded.windows(2).all(|w| w[0].chunk < w[1].chunk));
        t.update(None);
        let unloaded = t.take_changes();
        assert_eq!(unloaded.len(), 25);
        assert!(unloaded.iter().all(|c| c.new == UNLOADED));
        assert_eq!(unloaded[12], change((10, 10), 31, UNLOADED));
    }

    /// The changes between two sets of tickets are the chunks whose loading
    /// level differs in the per-chunk reference map, however the levels are
    /// stored (grid, map, tiles) and wherever the tickets are.
    #[test]
    fn changes_are_the_chunks_whose_reference_level_differs() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * n as u64 + 1)) as i32 - n
        };
        for case in 0..80 {
            let spread = [4, 25, 3000][case % 3];
            let mut sources = || Sources {
                players: (0..1 + case % 6)
                    .map(|_| (next(spread), next(spread)))
                    .collect(),
                forced: (0..case % 3)
                    .map(|_| (next(spread), next(spread)))
                    .collect(),
                portal: (0..case % 2)
                    .map(|_| ((next(spread), next(spread)), PORTAL_LEVEL))
                    .collect(),
                dragon: (case % 4 == 0).then(|| (next(spread), next(spread))),
                viewed: Vec::new(),
                view_distance: 2 + next(8).abs(),
                simulation_distance: 2 + next(8).abs(),
            };
            let (a, b) = (sources(), sources());
            let (old, new) = (compute_sparse(&a), compute_sparse(&b));
            let mut expected = Vec::new();
            for &key in old.keys().chain(new.keys()) {
                let was = old.get(&key).map_or(UNLOADED, |l| l.0);
                let now = new.get(&key).map_or(UNLOADED, |l| l.0);
                if was != now {
                    expected.push(change(key, was, now));
                }
            }
            expected.sort_unstable_by_key(|c| c.chunk);
            expected.dedup();
            let stored: [(Map, Map); 3] = [
                (compute(&a, true), compute(&b, true)),
                (Map::Sparse(old), Map::Sparse(new)),
                (
                    Map::Tiled(Tiled::new(&a, true)),
                    Map::Tiled(Tiled::new(&b, true)),
                ),
            ];
            for (old, new) in &stored {
                assert_eq!(level_changes(Some(old), Some(new)), expected, "case {case}");
            }
        }
    }

    /// cargo test --release -p horizon-client-lib --lib ticket_levels::tests::levels_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn levels_cost() {
        for (players, spacing, distance) in [
            (1usize, 0, 10),
            (10, 0, 10),
            (25, 0, 10),
            (50, 0, 10),
            (100, 150, 10),
            (100, 150, 32),
        ] {
            // The benchmark's bots: most near spawn, a fifth flying outwards.
            let at: Vec<_> = (0..players)
                .map(|i| {
                    if spacing != 0 {
                        return ((i % 10) as i32 * spacing, (i / 10) as i32 * spacing);
                    }
                    let a = i as f64 * 2.399;
                    let d = if i % 5 == 0 { 30.0 } else { 3.0 };
                    ((a.cos() * d) as i32, (a.sin() * d) as i32)
                })
                .collect();
            let s = Sources {
                players: at.clone(),
                forced: Vec::new(),
                portal: Vec::new(),
                dragon: None,
                viewed: Vec::new(),
                view_distance: distance,
                simulation_distance: distance,
            };
            let rounds = 20;
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(compute_sparse(std::hint::black_box(&s)));
            }
            let map_build = t.elapsed().as_secs_f64() * 1e3 / rounds as f64;
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(compute(std::hint::black_box(&s), true));
            }
            let selected_build = t.elapsed().as_secs_f64() * 1e3 / rounds as f64;
            let mut tickets = ChunkTickets::default();
            tickets.update(Some(s.clone()));
            let mut moved = s.clone();
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                moved.players[0].0 += 1;
                tickets.update(Some(moved.clone()));
                std::hint::black_box(tickets.levels());
            }
            let moving = t.elapsed().as_secs_f64() * 1e3 / rounds as f64;
            println!("one player moving: {moving:.3} ms/update");
            // A tick's questions: every chunk within 12 of every player.
            let (map, selected) = (Map::Sparse(compute_sparse(&s)), compute(&s, true));
            let tiled = Map::Tiled(Tiled::new(&s, true));
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(Tiled::new(std::hint::black_box(&s), true));
            }
            let tiled_build = t.elapsed().as_secs_f64() * 1e3 / rounds as f64;
            let ask = |m: &Map| {
                let t = std::time::Instant::now();
                let mut n = 0u32;
                for _ in 0..rounds {
                    for &(px, pz) in &at {
                        for z in pz - 12..=pz + 12 {
                            for x in px - 12..=px + 12 {
                                n +=
                                    u32::from(std::hint::black_box(m.get(x, z)).0 <= BLOCK_TICKING);
                            }
                        }
                    }
                }
                std::hint::black_box(n);
                t.elapsed().as_secs_f64() * 1e3 / rounds as f64
            };
            println!(
                "{players} players: spacing={spacing} distance={distance}: scalar/selected/tiled build {map_build:.3}/{selected_build:.3}/{tiled_build:.3} ms, {} lookups {:.3}/{:.3}/{:.3} ms",
                players * 625,
                ask(&map),
                ask(&selected),
                ask(&tiled)
            );
        }
    }
}

#[cfg(test)]
mod tiled_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn tiles_match_scalar_levels_with_overlaps_and_portals() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * n as u64 + 1)) as i32 - n
        };
        for case in 0..60 {
            let s = Sources {
                players: (0..case % 13)
                    .map(|_| (next(4000), next(4000)))
                    .chain([(15, -16), (16, -15)])
                    .collect(),
                forced: (0..case % 4).map(|_| (next(100), next(100))).collect(),
                // Saved tickets keep any level: these reach from no chunk to
                // past a tile.
                portal: (0..case % 5)
                    .map(|i| ((next(100), next(100)), [30, 12, 31, 32, 40][i]))
                    .chain([((16, -16), PORTAL_LEVEL)])
                    .collect(),
                dragon: (case % 4 == 0).then(|| (next(100), next(100))),
                viewed: Vec::new(),
                view_distance: [0, 2, 10, 32][case % 4],
                simulation_distance: [32, 10, 2, 0][(case / 4) % 4],
            };
            let expected = compute_sparse(&s);
            let tiled = Tiled::new(&s, true);
            for (&(x, z), &want) in &expected {
                assert_eq!(tiled.get(x, z), want, "case {case}, {x}, {z}");
            }
            // Query a fixed neighbourhood of every source, including holes
            // and negative coordinates, without depending on the tile layout.
            for &(cx, cz) in s
                .players
                .iter()
                .chain(&s.forced)
                .chain(s.portal.iter().map(|(p, _)| p))
            {
                for dz in -48..=48 {
                    for dx in -48..=48 {
                        let key = (cx + dx, cz + dz);
                        assert_eq!(
                            tiled.get(key.0, key.1),
                            expected.get(&key).copied().unwrap_or(NONE),
                            "case {case}, {key:?}"
                        );
                    }
                }
            }
            assert_eq!(tiled.get(50_000, -50_000), NONE);
        }
    }
}
