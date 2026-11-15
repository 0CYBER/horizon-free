//! The `can_player_sleep` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player tries to sleep in the bed whose head is at `pos`
    /// (NeoForge's `CanPlayerSleepEvent`, vanilla's `ServerPlayer.startSleepInBed`).
    /// `problem` is what vanilla found, first problem only: `too_far_away`,
    /// `obstructed`, `not_possible_now` (not night or thunder, the bed rule),
    /// `not_safe` (monsters near), or `None`. A mod may set `None` to let the
    /// player sleep anyway, or one of those names (`other_problem` refuses
    /// without a message) to refuse. The spawn point is set before it fires,
    /// as in vanilla, only when vanilla found no problem. After it,
    /// `player_bed_enter` still fires and may cancel. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "CanPlayerSleepEvent",
        cancellable = false
    )]
    fn can_player_sleep();
}
