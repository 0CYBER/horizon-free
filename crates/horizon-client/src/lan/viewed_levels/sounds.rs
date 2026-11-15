//! A viewed level's sounds (arismoko/horizon#3983, gap 12). Besides vanilla's broadcast in their own level, the
//! sounds at a position go wrapped to the players whose clients view that level and hold its
//! chunk, within the broadcast distance plus 16 blocks of the view's centre chunk. The client
//! asks its mods where in its own level to play
//! each one (`viewed_level_sound`). Only a client that answered revision 3 is sent them
//! (`Answer::sounds`); one of revision 1 or 2 still views the level and hears none of it. A sound
//! bound to an entity goes, by the same rule, only to a view that tracks the entity (9d), as its
//! client can place it only then.
use super::super::hub::Hub;
use super::send::Runs;

/// Blocks past a sound's broadcast distance an area's centre chunk may be: one chunk.
const PAST_RANGE: f64 = 16.0;

impl Hub {
    /// A sound at `at` in level `dimension`, broadcast there within `range` blocks, wrapped for
    /// each player whose client hears that level's sounds, holds the sound's chunk, and has an
    /// area in the level whose centre chunk is within `range` + 16 blocks of it: the Chebyshev
    /// distance between the two chunks, times 16. A sound bound to entity `emitter` goes only to
    /// a view that tracks that entity. `body` encodes the sound packet, once, for the first such
    /// player: with none, nothing is built.
    pub fn send_viewed_sound(
        &self,
        dimension: &str,
        at: [f64; 3],
        range: f64,
        emitter: Option<i32>,
        body: impl Fn() -> Vec<u8>,
    ) {
        if !self.viewed.active() {
            return;
        }
        let chunk = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        let mut encoded = None;
        for g in self.guests.iter() {
            if !g.sender.viewed_levels.sounds() || !g.views_level(dimension) {
                continue;
            }
            let Some(view) = g.viewed.view(dimension) else {
                continue;
            };
            let near = g.viewed.areas_in(dimension).any(|a| {
                let d = (i64::from(a.center.0) - i64::from(chunk.0))
                    .abs()
                    .max((i64::from(a.center.1) - i64::from(chunk.1)).abs());
                d as f64 * 16.0 <= range + PAST_RANGE
            });
            let placed = emitter.is_none_or(|id| view.tracker.is_tracking(id));
            if !near || !placed || !view.sent.contains(&chunk) {
                continue;
            }
            let body = encoded.get_or_insert_with(&body);
            let mut runs = Runs::new(view.level);
            runs.push(body);
            g.sender.send_many(runs.finish());
        }
    }
}
