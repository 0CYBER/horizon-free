//! Sections whose mesh the arena refused (`Renderer::upload`): the mesh is gone once
//! `Streaming::free` takes it, so the only way back is to have the game mesh the section
//! again. They wait here, each longer than the last, so a real shortage of memory does not
//! spin the mesher, and the first refusal of a section is logged with its cause.
use super::SectionKey;
use crate::{fast_hash::FxHashMap, world::DirtySections};
use std::time::{Duration, Instant};

/// How long a section refused once waits for its mesh back: twice that for each further
/// refusal, up to `MAX`.
const FIRST: Duration = Duration::from_secs(1);
/// The longest a refused section waits: video memory that is gone for good must not
/// take the mesher with it.
const MAX: Duration = Duration::from_secs(30);

/// How long a section refused `attempts` times (at least one) waits before the game
/// is asked to mesh it again.
fn delay(attempts: u32) -> Duration {
    // 32 times `FIRST` is past `MAX`, and capping the doubling keeps the shift small.
    (FIRST * (1 << attempts.saturating_sub(1).min(5))).min(MAX)
}

/// A refused section, and when its mesh is asked for again.
struct Refused {
    /// How many times its upload was refused, this one included.
    attempts: u32,
    /// When the game may be asked to mesh it again.
    due: Instant,
    /// Already handed back to the game: it is not again until the section fails once more.
    queued: bool,
}

/// The sections whose mesh the arena refused, waiting for their retry.
#[derive(Default)]
pub struct UploadRetries {
    refused: FxHashMap<SectionKey, Refused>,
}

impl UploadRetries {
    /// Section `key`'s upload was refused: the arena had no room for its `quads`, neither
    /// video nor system memory. The game is asked to mesh it again after a delay that
    /// doubles with each refusal, and the first refusal is logged.
    pub fn refused(&mut self, key: SectionKey, now: Instant, quads: u32) {
        let attempts = self.refused.get(&key).map_or(1, |r| r.attempts + 1);
        let wait = delay(attempts);
        self.refused.insert(
            key,
            Refused {
                attempts,
                due: now + wait,
                queued: false,
            },
        );
        if attempts == 1 {
            log::warn!(
                "Horizon: no room in the terrain arena for a section of {quads} quads: \
                 video and system memory are both full, so it is meshed again in {:.1} s",
                wait.as_secs_f64()
            );
        }
    }
    /// Section `key`'s mesh went in (or it has none): nothing to retry for it.
    pub fn uploaded(&mut self, key: SectionKey) {
        self.refused.remove(&key);
    }
    /// Puts the sections whose retry is due back in the mesh queue `dirty`
    /// (`World::dirty_sections`), as an edit puts a changed section there: the mesher
    /// makes their mesh again and the next upload finds out whether the arena takes it.
    /// How many went back.
    pub fn queue_due(&mut self, dirty: &mut DirtySections, now: Instant) -> usize {
        let mut queued = 0;
        for (&key, refused) in &mut self.refused {
            if !refused.queued && now >= refused.due {
                dirty.insert(key);
                refused.queued = true;
                queued += 1;
            }
        }
        queued
    }
    /// Drops the retries of sections in columns `keep` refuses: their meshes are not made
    /// again, so asking for them would only grow the queue.
    pub fn retain_columns(&mut self, mut keep: impl FnMut(i32, i32) -> bool) {
        if !self.refused.is_empty() {
            self.refused.retain(|&(x, _, z), _| keep(x, z));
        }
    }
    pub fn clear(&mut self) {
        self.refused.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.refused.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::SectionKey;
    use super::{FIRST, MAX, UploadRetries, delay};
    use crate::world::DirtySections;
    use std::time::{Duration, Instant};

    const KEY: SectionKey = (3, 5, -2);

    /// When `key`'s retry comes due, found by asking in 1 ms steps from `from`.
    fn due_in(retries: &mut UploadRetries, dirty: &mut DirtySections, from: Instant) -> Duration {
        (0..=MAX.as_millis() as u64)
            .find(|&ms| retries.queue_due(dirty, from + Duration::from_millis(ms)) == 1)
            .map(Duration::from_millis)
            .expect("a refused section is meshed again within the cap")
    }

    /// A section the arena refused goes back to the mesh queue when its delay is up, once,
    /// and stops being retried once a mesh of it goes in: the hole it left is meshed and
    /// drawn again, and nothing keeps asking for a section that made it.
    #[test]
    fn a_refused_section_is_meshed_again_until_one_of_its_meshes_is_uploaded() {
        let mut dirty = DirtySections::default();
        let mut retries = UploadRetries::default();
        let start = Instant::now();
        retries.refused(KEY, start, 24);
        // Not at once: a section the arena has no room for must not be meshed every frame.
        assert_eq!(retries.queue_due(&mut dirty, start), 0);
        assert!(
            !dirty.contains(&KEY),
            "meshed again in the same frame it failed"
        );
        assert_eq!(
            retries.queue_due(&mut dirty, start + delay(1)),
            1,
            "the section is not meshed again once its delay is up"
        );
        assert!(
            dirty.contains(&KEY),
            "the section is not back in the mesh queue"
        );
        // Once is enough: the mesh is in the works, and the upload decides again.
        assert_eq!(retries.queue_due(&mut dirty, start + MAX + MAX), 0);
        // The retry's mesh went in: the section is drawn again and nothing is retried.
        dirty.take_column(KEY.0, KEY.2);
        retries.uploaded(KEY);
        assert_eq!(retries.queue_due(&mut dirty, start + MAX + MAX), 0);
        assert!(!dirty.contains(&KEY));
        assert!(retries.is_empty(), "an uploaded section is still retried");
    }

    /// Each refusal of the same section waits longer than the one before, up to the cap, so
    /// a shortage of memory that does not pass does not spin the mesher at the delay's rate
    /// forever.
    #[test]
    fn each_refusal_waits_longer_than_the_last_and_no_longer_than_the_cap() {
        let mut dirty = DirtySections::default();
        let mut retries = UploadRetries::default();
        let mut waited = Vec::new();
        for _ in 0..6 {
            let start = Instant::now();
            retries.refused(KEY, start, 24);
            waited.push(due_in(&mut retries, &mut dirty, start));
            // The mesh came back and was refused again, so the next wait is longer.
            dirty.take_column(KEY.0, KEY.2);
        }
        assert_eq!(
            waited[0], FIRST,
            "a section refused once is meshed at once again"
        );
        assert!(
            waited.windows(2).all(|w| w[1] > w[0]),
            "the waits do not grow: {waited:?}"
        );
        assert_eq!(waited.last().copied(), Some(MAX), "{waited:?}");
        assert_eq!(
            delay(u32::MAX),
            MAX,
            "a section refused for good outwaits the cap"
        );
    }

    /// A section whose column unloaded is not asked for again: its mesh would have no
    /// chunk to be made from.
    #[test]
    fn an_unloaded_column_is_not_meshed_again() {
        let mut dirty = DirtySections::default();
        let mut retries = UploadRetries::default();
        let start = Instant::now();
        retries.refused(KEY, start, 24);
        retries.retain_columns(|x, z| (x, z) != (KEY.0, KEY.2));
        assert!(retries.is_empty(), "a retry outlives the column it is in");
        assert_eq!(retries.queue_due(&mut dirty, start + MAX), 0);
        assert!(!dirty.contains(&KEY));
    }
}
