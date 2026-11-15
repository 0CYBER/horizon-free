//! What one viewed level's client world holds: the chunks sent, those to send, and those no area
//! covers any more, kept [`LINGER`] ticks before they are forgotten (Immersive Portals' delayed
//! unload).
use super::areas::ChunkArea;
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    lan::entity::Tracker,
};

/// Ticks a sent chunk no area covers stays on the client: Immersive Portals' 4 generations of 13
/// ticks (`defaultDelayUnloadGenerations`, `updateInterval`). An engine default.
pub const LINGER: u64 = 52;

/// A level other than its own that a player's client views.
pub struct View {
    /// What its client names the level by: its Login index, or its slot and epoch
    /// (`horizon_protocol::viewed_levels`).
    pub level: horizon_protocol::viewed_levels::Level,
    pub sent: FxHashSet<(i32, i32)>,
    /// Covered and not sent, nearest to an area's centre first.
    pub queue: Vec<(i32, i32)>,
    /// Sent chunks no area covers, with the hub tick they stopped being covered.
    pub linger: FxHashMap<(i32, i32), u64>,
    /// The areas changed since the queue was made.
    pub dirty: bool,
    /// The weather last sent: raining, rain level, thunder level.
    pub weather: (bool, f32, f32),
    /// The clocks last sent (`send::Clock`), so that a jump or a new rate reaches the client.
    pub(super) clock: super::send::Clock,
    /// The areas' centres and radii, in chunks, as of the last requeue: where the view's client
    /// tracks the level's entities from.
    pub sources: Vec<((i32, i32), i32)>,
    /// The level's entities the client was sent (`TrackedEntity.seenBy`, for this view).
    pub tracker: Tracker,
}

impl View {
    pub fn new(level: horizon_protocol::viewed_levels::Level) -> Self {
        Self {
            level,
            sent: Default::default(),
            queue: Vec::new(),
            linger: Default::default(),
            dirty: true,
            weather: (false, 0.0, 0.0),
            clock: Default::default(),
            sources: Vec::new(),
            tracker: Tracker::default(),
        }
    }
    /// Nothing sent, queued or kept: the client's world can close.
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty() && self.queue.is_empty() && self.linger.is_empty()
    }

    /// The queue for `areas` (every mode) as of hub tick `now`: the queued chunks still covered,
    /// then the covered ones neither sent nor queued, nearest to an area's centre first (Immersive
    /// Portals sends a loader's chunks ring by ring from the inside out). The sent chunks no area
    /// covers start to linger; those covered again stop.
    pub fn requeue(&mut self, areas: &[&ChunkArea], now: u64) {
        let sent = &self.sent;
        self.queue
            .retain(|&(x, z)| covers(areas, x, z) && !sent.contains(&(x, z)));
        let mut queued: FxHashSet<(i32, i32)> = self.queue.iter().copied().collect();
        for a in areas {
            let r = a.radius as i32;
            for dz in -r..=r {
                for dx in -r..=r {
                    let key = (a.center.0 + dx, a.center.1 + dz);
                    if !sent.contains(&key) && queued.insert(key) {
                        self.queue.push(key);
                    }
                }
            }
        }
        self.queue.sort_by_key(|&(x, z)| distance(areas, x, z));
        self.sources = areas.iter().map(|a| (a.center, a.radius as i32)).collect();
        for &(x, z) in sent {
            if covers(areas, x, z) {
                self.linger.remove(&(x, z));
            } else {
                self.linger.entry((x, z)).or_insert(now);
            }
        }
        self.dirty = false;
    }

    /// The lingering chunks whose [`LINGER`] ticks are up at hub tick `now`, in order, taken out
    /// of the view: the client is to forget them.
    pub fn due(&mut self, now: u64) -> Vec<(i32, i32)> {
        let mut gone: Vec<(i32, i32)> = self
            .linger
            .iter()
            .filter(|&(_, &since)| now.saturating_sub(since) >= LINGER)
            .map(|(&key, _)| key)
            .collect();
        gone.sort_unstable();
        for key in &gone {
            self.linger.remove(key);
            self.sent.remove(key);
        }
        gone
    }
}

/// Whether one of `areas` holds the chunk.
fn covers(areas: &[&ChunkArea], x: i32, z: i32) -> bool {
    areas.iter().any(|a| {
        let r = a.radius as i32;
        (x - a.center.0).abs() <= r && (z - a.center.1).abs() <= r
    })
}

/// The Chebyshev distance from the chunk to the nearest area's centre.
fn distance(areas: &[&ChunkArea], x: i32, z: i32) -> i32 {
    areas
        .iter()
        .map(|a| (x - a.center.0).abs().max((z - a.center.1).abs()))
        .min()
        .unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::viewed_levels::areas::ChunkAreaMode;

    fn area(center: (i32, i32), radius: u32) -> ChunkArea {
        ChunkArea {
            level: "minecraft:the_nether".into(),
            center,
            radius,
            mode: ChunkAreaMode::Loaded,
        }
    }

    #[test]
    fn the_queue_goes_ring_by_ring_from_the_centre() {
        let a = area((10, -4), 2);
        let mut view = View::new(horizon_protocol::viewed_levels::Level::Index(2));
        view.sent.insert((10, -4));
        view.requeue(&[&a], 0);
        assert_eq!(view.queue.len(), 24, "a 5x5 square less the sent centre");
        let rings: Vec<i32> = view
            .queue
            .iter()
            .map(|&(x, z)| (x - 10).abs().max((z + 4).abs()))
            .collect();
        assert!(rings[..8].iter().all(|&d| d == 1), "{rings:?}");
        assert!(rings[8..].iter().all(|&d| d == 2), "{rings:?}");
    }

    #[test]
    fn an_uncovered_chunk_is_forgotten_after_the_delay_unless_covered_again() {
        let near = area((0, 0), 1);
        let mut view = View::new(horizon_protocol::viewed_levels::Level::Index(1));
        view.sent.extend([(0, 0), (5, 5), (6, 6)]);
        view.requeue(&[&near], 100);
        assert!(view.due(151).is_empty());
        // (6, 6) is covered again before its time.
        let far = area((6, 6), 0);
        view.requeue(&[&near, &far], 120);
        assert_eq!(view.due(152), vec![(5, 5)]);
        assert!(view.sent.contains(&(6, 6)));
        assert!(view.linger.is_empty());
    }
}
