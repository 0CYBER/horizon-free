//! The `note_block_change` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player is tuning a note block on the authoritative server, not in
    /// client prediction. `context` gives its dimension, position, instrument
    /// and old note; `new_note` starts as vanilla's cycled note. Set it to 0
    /// through 24; an out-of-range value keeps vanilla's. Cancel to leave the
    /// note unchanged and prevent playback.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "NoteBlockEvent.Change",
        cancellable = true,
        target = "horizon_client::game::native_hooks::native_note_block_change::GameLevel<'_>::native_note_block_change"
    )]
    fn note_block_change();
}
