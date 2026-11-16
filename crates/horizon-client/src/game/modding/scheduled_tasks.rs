//! The delayed tasks mods scheduled in the running world (`schedule_task`): a queue counted in
//! server ticks, owned by the world's lifecycle and dropped with it.

use mod_api::{MAX_PENDING_TASKS, ScheduleError, TaskId};
use std::collections::{BTreeMap, HashMap};

/// What a due task hands back: its id, the owning mod's `[mod] id` and the mod's token.
pub(super) type DueTask = (TaskId, &'static str, u64);

/// Runs every item `next` yields through `run`, even when one run panics: the other items still
/// run, and the first panic comes back for the caller to put back in flight once it has restored
/// its own state. `state` is passed to both so each may borrow the whole of it in turn.
pub(super) fn run_all<S, T>(
    state: &mut S,
    next: fn(&mut S) -> Option<T>,
    run: fn(&mut S, T),
) -> Option<Box<dyn std::any::Any + Send>> {
    let mut panic = None;
    while let Some(item) = next(state) {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut *state, item)));
        if let Err(payload) = result {
            panic.get_or_insert(payload);
        }
    }
    panic
}

#[derive(Default)]
pub(super) struct ScheduledTasks {
    /// Ticks advanced since the world opened.
    now: u64,
    next_id: u64,
    /// Waiting tasks by due tick, then id (the order they were scheduled).
    queue: BTreeMap<(u64, u64), (&'static str, u64)>,
    /// Each waiting task's due tick, to find it again on cancel.
    due: HashMap<u64, u64>,
    /// How many tasks each mod has waiting.
    pending: HashMap<&'static str, usize>,
}

impl ScheduledTasks {
    /// A task for `owner` due `delay_ticks` ticks from now (0 counts as 1: never this tick).
    pub(super) fn schedule(
        &mut self,
        owner: Option<&'static str>,
        delay_ticks: u32,
        token: u64,
    ) -> Result<TaskId, ScheduleError> {
        let owner = owner.ok_or(ScheduleError::NotInMod)?;
        let waiting = self.pending.entry(owner).or_insert(0);
        if *waiting >= MAX_PENDING_TASKS {
            return Err(ScheduleError::TooMany);
        }
        *waiting += 1;
        self.next_id += 1;
        let id = self.next_id;
        let due = self.now.saturating_add(u64::from(delay_ticks.max(1)));
        self.queue.insert((due, id), (owner, token));
        self.due.insert(id, due);
        Ok(TaskId(id))
    }

    /// Withdraw `id` if `owner` scheduled it and it is still waiting.
    pub(super) fn cancel(&mut self, owner: Option<&'static str>, id: TaskId) -> bool {
        let Some(&due) = self.due.get(&id.0) else {
            return false;
        };
        if self.queue.get(&(due, id.0)).map(|task| task.0) != owner || owner.is_none() {
            return false;
        }
        self.remove(due, id.0);
        true
    }

    /// A server tick starts: "now" moves to it, before any hook of the tick runs, so a task
    /// scheduled by one is measured from this tick and never fires in it.
    pub(super) fn begin_tick(&mut self) {
        self.now += 1;
    }

    /// The next task due by now, in the order they were scheduled, taken off the queue. One at a
    /// time, so a task whose hook panics leaves the rest queued. A task scheduled while others
    /// run is due on a later tick.
    pub(super) fn pop_due(&mut self) -> Option<DueTask> {
        let (&(due, id), _) = self.queue.first_key_value()?;
        if due > self.now {
            return None;
        }
        let (owner, token) = self.queue.remove(&(due, id))?;
        self.due.remove(&id);
        if let Some(waiting) = self.pending.get_mut(owner) {
            *waiting -= 1;
        }
        Some((TaskId(id), owner, token))
    }

    /// The world closed or opened: nothing carries over.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    fn remove(&mut self, due: u64, id: u64) {
        if let Some((owner, _)) = self.queue.remove(&(due, id))
            && let Some(waiting) = self.pending.get_mut(owner)
        {
            *waiting -= 1;
        }
        self.due.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Option<&str> = Some("a");
    const B: Option<&str> = Some("b");

    /// One tick: "now" moves, then every due task comes out.
    fn advance(tasks: &mut ScheduledTasks) -> Vec<DueTask> {
        tasks.begin_tick();
        std::iter::from_fn(|| tasks.pop_due()).collect()
    }

    fn ids(due: &[DueTask]) -> Vec<(u64, u64)> {
        due.iter().map(|d| (d.0.0, d.2)).collect()
    }

    #[test]
    fn tasks_run_after_their_delay_in_the_order_they_were_scheduled() {
        let mut tasks = ScheduledTasks::default();
        tasks.schedule(A, 2, 20).unwrap();
        tasks.schedule(A, 1, 10).unwrap();
        tasks.schedule(B, 2, 21).unwrap();
        tasks.schedule(B, 0, 0).unwrap();
        assert_eq!(ids(&advance(&mut tasks)), [(2, 10), (4, 0)]);
        assert_eq!(ids(&advance(&mut tasks)), [(1, 20), (3, 21)]);
        assert!(advance(&mut tasks).is_empty());
    }

    #[test]
    fn a_task_scheduled_while_others_run_is_due_on_a_later_tick() {
        let mut tasks = ScheduledTasks::default();
        tasks.schedule(A, 1, 1).unwrap();
        tasks.begin_tick();
        assert!(tasks.pop_due().is_some());
        tasks.schedule(A, 0, 2).unwrap();
        assert!(tasks.pop_due().is_none());
        assert_eq!(ids(&advance(&mut tasks)), [(2, 2)]);
    }

    #[test]
    fn a_task_scheduled_from_the_tick_start_hook_waits_for_a_later_tick() {
        // The lifecycle moves "now" before the start hook runs, so the hook schedules from the new
        // tick: delay 0 and 1 fire on the next tick, delay N on the Nth after the current one.
        let mut tasks = ScheduledTasks::default();
        tasks.begin_tick();
        tasks.schedule(A, 0, 100).unwrap();
        tasks.schedule(A, 1, 101).unwrap();
        tasks.schedule(A, 3, 103).unwrap();
        assert!(tasks.pop_due().is_none());
        assert_eq!(ids(&advance(&mut tasks)), [(1, 100), (2, 101)]);
        assert!(advance(&mut tasks).is_empty());
        assert_eq!(ids(&advance(&mut tasks)), [(3, 103)]);
    }

    #[test]
    fn only_the_owner_cancels_a_waiting_task() {
        let mut tasks = ScheduledTasks::default();
        let id = tasks.schedule(A, 3, 1).unwrap();
        assert!(!tasks.cancel(B, id));
        assert!(!tasks.cancel(None, id));
        assert!(tasks.cancel(A, id));
        assert!(!tasks.cancel(A, id));
        assert!(!tasks.cancel(A, TaskId(99)));
        for _ in 0..3 {
            assert!(advance(&mut tasks).is_empty());
        }
        let fired = tasks.schedule(A, 1, 2).unwrap();
        assert_eq!(advance(&mut tasks).len(), 1);
        assert!(!tasks.cancel(A, fired));
    }

    #[test]
    fn a_mod_outside_its_hooks_cannot_schedule_and_one_mod_is_capped() {
        let mut tasks = ScheduledTasks::default();
        assert_eq!(tasks.schedule(None, 1, 0), Err(ScheduleError::NotInMod));
        for token in 0..MAX_PENDING_TASKS as u64 {
            tasks.schedule(A, 5, token).unwrap();
        }
        assert_eq!(tasks.schedule(A, 5, 0), Err(ScheduleError::TooMany));
        assert!(tasks.schedule(B, 5, 0).is_ok());
        // A fired or cancelled task frees its place.
        for _ in 0..5 {
            advance(&mut tasks);
        }
        assert!(tasks.schedule(A, 1, 0).is_ok());
    }

    #[test]
    fn the_largest_delay_does_not_wrap() {
        let mut tasks = ScheduledTasks::default();
        tasks.schedule(A, u32::MAX, 1).unwrap();
        assert!(advance(&mut tasks).is_empty());
    }

    #[test]
    fn closing_the_world_drops_every_task() {
        let mut tasks = ScheduledTasks::default();
        tasks.schedule(A, 1, 1).unwrap();
        tasks.clear();
        assert!(advance(&mut tasks).is_empty());
        assert_eq!(tasks.schedule(A, 1, 2).unwrap(), TaskId(1));
    }

    #[test]
    fn a_panicking_task_does_not_drop_the_others() {
        let mut state = (ScheduledTasks::default(), Vec::<u64>::new());
        state.0.schedule(A, 1, 1).unwrap();
        state.0.schedule(A, 1, 2).unwrap();
        state.0.schedule(B, 1, 3).unwrap();
        state.0.begin_tick();
        let panic = run_all(
            &mut state,
            |s| s.0.pop_due(),
            |s, task| {
                if task.2 == 1 {
                    panic!("the first task fails");
                }
                s.1.push(task.2);
            },
        );
        assert!(panic.is_some(), "the panic comes back to the caller");
        assert_eq!(state.1, [2, 3]);
        assert!(state.0.pop_due().is_none());
    }
}
