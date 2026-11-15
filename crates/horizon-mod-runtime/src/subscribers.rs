//! Subscriber sets for generated native-hook dispatch.

/// One mod's subscription to one hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subscription {
    /// The mod's index in installation (dependency) order.
    pub index: usize,
    /// The hook's index.
    pub hook: usize,
    /// Higher runs first; equal priorities keep installation order.
    pub priority: i32,
    /// Whether the mod is still called once an earlier listener cancelled the event.
    pub receives_cancelled: bool,
}

/// One subscriber of a hook, as dispatch visits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subscriber {
    /// The mod's index in installation order.
    pub index: usize,
    /// Whether it is still called once the event is cancelled.
    pub receives_cancelled: bool,
}

/// Immutable per-hook subscriber lists, resolved and ordered once at installation.
pub struct Subscribers {
    /// Each hook's subscribers, in dispatch order: `order[starts[hook]..starts[hook + 1]]`.
    order: Box<[Subscriber]>,
    starts: Box<[usize]>,
    words_per_hook: usize,
    words: Box<[u64]>,
    receivers: Box<[bool]>,
}

impl Subscribers {
    /// Build the lists from subscriptions; a mod subscribing to a hook twice is listed once.
    pub fn new(
        hooks: usize,
        mods: usize,
        subscriptions: impl IntoIterator<Item = Subscription>,
    ) -> Self {
        let words_per_hook = mods.div_ceil(64);
        let mut words = vec![0; hooks * words_per_hook];
        let mut listed = Vec::new();
        for subscription in subscriptions {
            let Subscription { index, hook, .. } = subscription;
            assert!(index < mods && hook < hooks);
            let word = &mut words[hook * words_per_hook + index / 64];
            if *word & (1 << (index % 64)) == 0 {
                *word |= 1 << (index % 64);
                listed.push(subscription);
            }
        }
        listed.sort_by_key(|s| (s.hook, std::cmp::Reverse(s.priority), s.index));
        let mut starts = vec![0; hooks + 1];
        let mut receivers = vec![false; hooks];
        for subscription in &listed {
            starts[subscription.hook + 1] += 1;
            receivers[subscription.hook] |= subscription.receives_cancelled;
        }
        for hook in 0..hooks {
            starts[hook + 1] += starts[hook];
        }
        let order = listed
            .iter()
            .map(|s| Subscriber {
                index: s.index,
                receives_cancelled: s.receives_cancelled,
            })
            .collect();
        Self {
            order,
            starts: starts.into_boxed_slice(),
            words_per_hook,
            words: words.into_boxed_slice(),
            receivers: receivers.into_boxed_slice(),
        }
    }

    /// Whether a hook has a subscriber.
    pub fn contains_any(&self, hook: usize) -> bool {
        self.starts[hook] != self.starts[hook + 1]
    }

    /// Whether a particular mod subscribes to a hook.
    pub fn contains(&self, hook: usize, index: usize) -> bool {
        self.words[hook * self.words_per_hook + index / 64] & (1 << (index % 64)) != 0
    }

    /// Whether a subscriber of a hook is still called once the event is cancelled.
    pub fn receives_cancelled(&self, hook: usize) -> bool {
        self.receivers[hook]
    }

    /// A hook's subscribers in dispatch order: descending priority, then installation order.
    pub fn subscribers(&self, hook: usize) -> impl Iterator<Item = Subscriber> + '_ {
        self.order[self.starts[hook]..self.starts[hook + 1]]
            .iter()
            .copied()
    }

    /// A hook's subscribers' indices in dispatch order.
    pub fn indices(&self, hook: usize) -> impl Iterator<Item = usize> + '_ {
        self.subscribers(hook).map(|s| s.index)
    }
}

#[cfg(test)]
mod tests {
    use super::{Subscribers, Subscription};

    fn at(index: usize, hook: usize, priority: i32) -> Subscription {
        Subscription {
            index,
            hook,
            priority,
            receives_cancelled: false,
        }
    }

    #[test]
    fn builds_sparse_sets_in_mod_order() {
        let set = Subscribers::new(2, 70, [at(69, 0, 0), at(1, 0, 0), at(1, 0, 0)]);
        assert!(set.contains_any(0));
        assert!(!set.contains_any(1));
        assert!(set.contains(0, 69));
        assert!(!set.contains(0, 68));
        assert_eq!(set.indices(0).collect::<Vec<_>>(), [1, 69]);
    }

    #[test]
    fn higher_priority_runs_first_and_ties_keep_mod_order() {
        let set = Subscribers::new(
            2,
            4,
            [
                at(0, 0, 0),
                at(1, 0, 5),
                at(2, 0, -1),
                at(3, 0, 5),
                at(3, 1, 0),
            ],
        );
        assert_eq!(set.indices(0).collect::<Vec<_>>(), [1, 3, 0, 2]);
        assert_eq!(set.indices(1).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn a_hook_knows_whether_any_subscriber_receives_cancelled_events() {
        let receiving = Subscription {
            receives_cancelled: true,
            ..at(1, 1, 0)
        };
        let set = Subscribers::new(2, 2, [at(0, 0, 0), receiving]);
        assert!(!set.receives_cancelled(0));
        assert!(set.receives_cancelled(1));
        assert!(set.subscribers(1).all(|s| s.receives_cancelled));
    }
}
