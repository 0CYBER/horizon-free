//! The `player_bed_leave` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0279

pub trait Hooks {
    /// `PlayerWakeUpEvent`: a server player is getting out of the bed at
    /// `pos` (`Player.stopSleepInBed`), heard first, while they still read
    /// as sleeping there. `wake_immediately` is vanilla's flag for skipping
    /// the wake-up fade: generic stop-sleeping, death and bed-missing wakes
    /// pass true; morning, explicit leave and bed-rule refusal pass false.
    /// `update_level` is false for morning, true for those individual wakes.
    #[notes = r#"## Player bed leave callers

The notice precedes clearing the sleeping position and pose. The pairs below
are `(wake_immediately, update_level)`; an immediate wake resets the sleep
counter to 0, otherwise it becomes 100 for the fade-out.

| Existing server caller | Pair |
| --- | --- |
| Morning, acting player or stored guest | `(false, false)` |
| Leave-bed packet, button, close chat, or Horizon's leave/sneak path | `(false, true)` |
| Player bed-rule refusal | `(false, true)` |
| Missing bed / living-entity continuation failure | `(true, true)` |
| Death or missing acting body | `(true, true)` |
| Mod `LivingRef::stop_sleeping`, acting player or guest | `(true, true)` |

A nested wake of the same player does not emit a second notice. Plain position
teleports and transition-based dimension changes do not acquire a new blanket
wake. A distinct level-and-position wake, if added, uses `(true, true)`; a
disconnect wake, if added, uses `(true, false)`. Direct metadata clearing is not
stop-sleeping, and client metadata resets do not emit this server hook.
"#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerWakeUpEvent",
        cancellable = false,
        version = 2
    )]
    fn player_bed_leave();
}
