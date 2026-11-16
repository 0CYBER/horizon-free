//! Delayed server tasks (`ModStateServerHost::schedule_task`), the stand-in for vanilla's
//! `MinecraftServer.tell(new TickTask(tick, runnable))`.

/// A task a mod scheduled: unique in the running world, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: an opaque id the game builds and mods only compare and hand back
#[derive(horizon_mod_api_macros::Wire)]
pub struct TaskId(pub u64);

/// Why a task was not scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two refusals, matched without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum ScheduleError {
    /// The caller is not inside a mod's hook, so no mod owns the task.
    NotInMod,
    /// The mod already has `MAX_PENDING_TASKS` tasks waiting.
    TooMany,
}

/// How many tasks one mod may have waiting at once.
pub const MAX_PENDING_TASKS: usize = 4096;
