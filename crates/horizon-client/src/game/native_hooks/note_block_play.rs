use super::*;

impl GameLevel<'_> {
    /// The note block at `pos` is about to play (NeoForge's
    /// `NoteBlockEvent.Play`, in `NoteBlock.playNote`), once a block above
    /// no longer mutes it. `player` and `player_entity_id` name the player
    /// who tuned or punched it, and are `None` for redstone.
    /// `instrument` is its instrument and `note` its note, 0 to 24; change
    /// either to play another (a note out of range keeps the block's own).
    /// Set `cancel` and it doesn't play: no sound, no particle, no
    /// `note_block_play` game event. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_note_block_play(
        &self,
        call: mod_api::__private::NoteBlockPlayCall<'_>,
    ) {
        self.fire_note_block_play(call);
    }
}
