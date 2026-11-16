//! The `note_block_play` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The note block at `pos` is about to play (NeoForge's
    /// `NoteBlockEvent.Play`, in `NoteBlock.playNote`), once a block above
    /// no longer mutes it. `player` and `player_entity_id` name the player
    /// who tuned or punched it, and are `None` for redstone.
    /// `instrument` is its instrument and `note` its note, 0 to 24; change
    /// either to play another (a note out of range keeps the block's own).
    /// Set `cancel` and it doesn't play: no sound, no particle, no
    /// `note_block_play` game event. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "NoteBlockEvent.Play",
        cancellable = true,
        target = "horizon_client::game::native_hooks::note_block_play::GameLevel<'_>::native_note_block_play"
    )]
    fn note_block_play(
        dimension: LevelDimension,
        player: Option<u128>,
        player_entity_id: Option<i32>,
        pos: BlockPos,
        instrument: &mut NoteInstrument,
        note: &mut i32,
        cancel: &mut bool,
    );
}
