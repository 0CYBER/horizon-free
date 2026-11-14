//! The engine's input queues: the key presses and clicks that wait between the event that adds
//! them (a key, a button, a pad, the agent socket, a mod's `click_mapping`) and the client tick
//! that acts on them. A tick takes a bounded number of entries, oldest first, and leaves the rest
//! queued for the next tick; a full queue refuses a new entry and says so in the log once. Human
//! input reaches neither bound, so it acts in the same order and on the same tick as it would on an
//! unbounded queue; only input added in a loop is spread over several ticks.

/// The bounds of one queue.
#[derive(Clone, Copy, Debug)]
pub struct QueueLimits {
    /// The queue's name in the log line of a refusal.
    pub name: &'static str,
    /// The most entries it holds: one more is refused.
    pub max_len: usize,
    /// The most entries one tick takes.
    pub per_tick: usize,
}

/// A full queue refused an entry: the first refusal of each queue is logged, the others are not.
pub(crate) fn refuse(logged: &mut bool, limits: QueueLimits) {
    if !std::mem::replace(logged, true) {
        log::warn!(
            "input queue {} is full at {} entries: further entries are refused",
            limits.name,
            limits.max_len
        );
    }
}

/// A first-in, first-out queue of entries, bounded by its [`QueueLimits`].
#[derive(Debug)]
pub struct InputQueue<T> {
    entries: Vec<T>,
    limits: QueueLimits,
    refused: bool,
}

impl<T> InputQueue<T> {
    pub const fn new(limits: QueueLimits) -> Self {
        Self {
            entries: Vec::new(),
            limits,
            refused: false,
        }
    }
    /// Queue `entry` behind the others; at the maximum it is refused, as a key pressed with a full
    /// keyboard buffer is lost.
    pub fn push(&mut self, entry: T) {
        if self.entries.len() >= self.limits.max_len {
            refuse(&mut self.refused, self.limits);
        } else {
            self.entries.push(entry);
        }
    }
    /// The entries this tick acts on, oldest first, at most `per_tick` of them. The rest stay
    /// queued, and an entry pushed while the caller works through these waits behind them.
    pub fn take_tick(&mut self) -> Vec<T> {
        let rest = if self.entries.len() > self.limits.per_tick {
            self.entries.split_off(self.limits.per_tick)
        } else {
            Vec::new()
        };
        std::mem::replace(&mut self.entries, rest)
    }
    /// Drop every entry (a screen opened: `KeyMapping.releaseAll`).
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// The queued entries, oldest first.
    pub fn entries(&self) -> &[T] {
        &self.entries
    }
}

/// The clicks of one button waiting for the tick: a count, bounded as a queue is.
#[derive(Clone, Copy, Debug)]
pub struct ClickQueue {
    count: u32,
    limits: QueueLimits,
    refused: bool,
}

impl ClickQueue {
    pub const fn new(limits: QueueLimits) -> Self {
        Self {
            count: 0,
            limits,
            refused: false,
        }
    }
    fn max(&self) -> u32 {
        u32::try_from(self.limits.max_len).unwrap_or(u32::MAX)
    }
    /// One more click; at the maximum it is refused.
    pub fn press(&mut self) {
        self.add(1);
    }
    fn add(&mut self, clicks: u32) {
        let count = self.count.saturating_add(clicks).min(self.max());
        if count - self.count < clicks {
            refuse(&mut self.refused, self.limits);
        }
        self.count = count;
    }
    /// The clicks this tick acts on, at most `per_tick`; the rest stay queued.
    pub fn take_tick(&mut self) -> u32 {
        let taken = self
            .count
            .min(u32::try_from(self.limits.per_tick).unwrap_or(u32::MAX));
        self.count -= taken;
        taken
    }
    /// Add the clicks of `other`, queued after these.
    pub fn merge(&mut self, other: Self) {
        self.refused |= other.refused;
        self.add(other.count);
    }
    /// Drop every click.
    pub fn clear(&mut self) {
        self.count = 0;
    }
    /// The queued clicks.
    pub fn count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: QueueLimits = QueueLimits {
        name: "test",
        max_len: 8,
        per_tick: 3,
    };

    /// Each tick takes three, oldest first; the rest wait, and an entry queued while a tick works
    /// waits behind them. The ninth entry of a full queue is refused.
    #[test]
    fn a_tick_takes_its_bound_oldest_first_and_a_full_queue_refuses() {
        let mut queue = InputQueue::new(SMALL);
        for entry in 0..9 {
            queue.push(entry);
        }
        assert_eq!(queue.entries(), [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(queue.take_tick(), [0, 1, 2]);
        queue.push(100);
        assert_eq!(queue.take_tick(), [3, 4, 5]);
        assert_eq!(queue.take_tick(), [6, 7, 100]);
        assert_eq!(queue.take_tick(), Vec::<i32>::new());
    }

    /// A click count stops at its maximum, pressed or merged, and a tick takes three at most.
    #[test]
    fn a_click_count_stops_at_its_maximum() {
        let mut clicks = ClickQueue::new(SMALL);
        for _ in 0..9 {
            clicks.press();
        }
        assert_eq!(clicks.count(), 8);
        assert_eq!(clicks.take_tick(), 3);
        let mut more = ClickQueue::new(SMALL);
        more.count = 7;
        clicks.merge(more);
        assert_eq!(clicks.count(), 8);
        let mut huge = ClickQueue::new(QueueLimits {
            max_len: usize::MAX,
            ..SMALL
        });
        huge.count = u32::MAX - 1;
        huge.press();
        huge.press();
        assert_eq!(huge.count(), u32::MAX);
    }
}
