//! The `player_arm_swing` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0276

pub trait Hooks {
    /// A criterion of a server player's advancement was granted or revoked, after its progress changed
    /// and before any completion rewards. A grant or revoke that changes nothing is not heard.
    /// `progress` is an owned snapshot of every criterion (including its stored grant date),
    /// the requirement groups and their completion at this exact mutation. It belongs to
    /// the mutated progress, including first-tick and reload grants queued by the server hub.
    /// During a nested player turn, a suspended player's hub record may contain displaced
    /// progress. A listener may retain a clone; later mutations do not change the snapshot.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "AdvancementEvent.AdvancementProgressEvent",
        cancellable = false
    )]
    fn player_advancement_progress(
        player: u128,
        player_entity_id: i32,
        name: &str,
        advancement: &str,
        criterion: &str,
        progress_type: ProgressType,
        progress: &AdvancementProgress,
    );
    /// A guest's arm swing was handled by the server. `hand` is `mainhand`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerAnimationEvent)"
    )]
    fn player_arm_swing();
}
