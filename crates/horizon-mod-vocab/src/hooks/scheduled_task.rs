//! The `scheduled_task` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A task a mod scheduled with `schedule_task` is due, at the start of the server tick it
    /// was due on, after `server_tick`'s start phase, in the order the tasks were scheduled.
    /// `id` is the id `schedule_task` returned, `owner` the `[mod] id` of the mod that scheduled
    /// it and `token` the value it passed. Every mod is asked about every task, so a mod ignores
    /// tasks it does not own. Server-side.
    #[notes = r#"## Delayed tasks

`host.schedule_task(delay_ticks, token)` asks for a call back after `delay_ticks` server ticks
and `host.cancel_task(id)` withdraws it, the stand-in for vanilla's
`MinecraftServer.tell(new TickTask(getTickCount() + delay, runnable))`. A closure cannot cross
into a wasm mod or outlive the borrowed host, so the callback is this hook: `Hooks::scheduled_task`
gets the `TaskId`, the `owner` (the scheduling mod's `[mod] id`) and the `token` the mod passed,
on the server thread, at the start of the due tick right after `server_tick`'s start phase. Every
mod is asked about every task, so a mod ignores tasks whose `owner` is not its own.

A delay counts ticks the server runs: a paused integrated server does not advance it, and `0`
and `1` both mean the next tick. Tasks due on the same tick run in the order they were
scheduled. A task scheduled from `scheduled_task` itself runs no earlier than the next tick.
`schedule_task` must be called from inside a mod's hook (`ScheduleError::NotInMod` otherwise),
and a mod may have `MAX_PENDING_TASKS` waiting at once (`ScheduleError::TooMany`). Ids are
never reused in a world. `cancel_task` answers true if the task was still waiting and the
caller's own; a fired, unknown or another mod's id answers false.

The queue belongs to the running world: it is dropped when the world closes and is never saved,
so a task never leaks into the next world and a mod that must outlive a restart writes its
deadline to its own storage and schedules again when the world opens."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla MinecraftServer.tell(TickTask); NeoForge adds no event for it)",
        cancellable = false
    )]
    fn scheduled_task();
}
