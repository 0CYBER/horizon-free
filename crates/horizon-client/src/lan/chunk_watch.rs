//! The chunks each player watches, for the mods' `chunk_watch`, `chunk_sent`
//! and `chunk_unwatch` (NeoForge's `ChunkWatchEvent`) and for the passages the
//! server sends with a chunk (arismoko/horizon#3775). The hub records them only while a mod
//! listens or a passage was registered; the game reads them after the hub's step.

use mod_api::HookId;

use super::guest::Guest;

/// What happened to a player's chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkWatchKind {
    /// The chunk entered the player's view (`ChunkWatchEvent.Watch`).
    Watch,
    /// Its packet went to the player (`ChunkWatchEvent.Sent`).
    Sent,
    /// The player stopped watching it (`ChunkWatchEvent.UnWatch`).
    Unwatch,
}

/// One chunk event for one player, in the order the hub made them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkWatch {
    pub kind: ChunkWatchKind,
    pub uuid: u128,
    pub entity_id: i32,
    pub chunk: [i32; 2],
}

/// The events of one level, in a row: the level is named once for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelWatches {
    /// The level the chunks are in (`minecraft:overworld`).
    pub dimension: String,
    pub watches: Vec<ChunkWatch>,
}

/// The hub's chunk events since the game last took them, in order, run by
/// run of the same level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkWatchLog {
    runs: Vec<LevelWatches>,
    /// A mod registered a passage since the world opened: the chunks sent from
    /// then on are recorded whether or not a mod listens.
    passages: bool,
}

impl ChunkWatchLog {
    /// A log of `runs`, as the hub would have made it.
    pub fn new(runs: Vec<LevelWatches>) -> Self {
        Self {
            runs,
            passages: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Every event so far, run by run; the log is left empty.
    pub fn take(&mut self) -> Vec<LevelWatches> {
        std::mem::take(&mut self.runs)
    }

    /// Whether the hub records the chunks players watch: while a mod listens
    /// ([`listened`]), and from the first live passage on while this hub lasts.
    pub fn recording(&self) -> bool {
        self.passages || listened()
    }

    /// The server has a live passage: record from now on, for the passages the
    /// game sends with each chunk sent.
    pub fn record_for_passages(&mut self) {
        self.passages = true;
    }

    /// One event for `g`'s chunk `(x, z)` in its level, which a new run
    /// names only when it isn't the last event's level.
    pub(super) fn push(&mut self, g: &Guest, kind: ChunkWatchKind, (x, z): (i32, i32)) {
        let watch = ChunkWatch {
            kind,
            uuid: g.uuid,
            entity_id: g.entity_id,
            chunk: [x, z],
        };
        match self.runs.last_mut() {
            Some(run) if run.dimension == g.dimension => run.watches.push(watch),
            _ => self.runs.push(LevelWatches {
                dimension: g.dimension.clone(),
                watches: vec![watch],
            }),
        }
    }
}

/// Whether a mod listens to any of the three.
pub fn listened() -> bool {
    [HookId::ChunkWatch, HookId::ChunkSent, HookId::ChunkUnwatch]
        .into_iter()
        .any(mod_api::__private::listened)
}

/// `g` stops watching chunk `(x, z)` (`ChunkMap.dropChunk`), if it did.
pub(super) fn unwatch(log: &mut ChunkWatchLog, g: &mut Guest, key: (i32, i32)) {
    if g.watched.remove(&key) {
        log.push(g, ChunkWatchKind::Unwatch, key);
    }
}

/// `g` stops watching every chunk (`applyChunkTrackingView(EMPTY)`): before
/// it leaves its level, respawns or disconnects. In the order vanilla walks
/// its square view (`ChunkTrackingView.Positioned.forEach`): by x, then z.
pub(super) fn unwatch_all(log: &mut ChunkWatchLog, g: &mut Guest) {
    if g.watched.is_empty() {
        return;
    }
    let mut keys: Vec<_> = g.watched.drain().collect();
    keys.sort_unstable();
    for key in keys {
        log.push(g, ChunkWatchKind::Unwatch, key);
    }
}
