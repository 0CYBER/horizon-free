use super::*;

impl GameLevel<'_> {
    /// A player is tuning a note block on the authoritative server, not in
    /// client prediction. `context` gives its dimension, position, instrument
    /// and old note; `new_note` starts as vanilla's cycled note. Set it to 0
    /// through 24; an out-of-range value keeps vanilla's. Cancel to leave the
    /// note unchanged and prevent playback.
    #[inline(always)]
    pub(in crate::game) fn native_note_block_change(
        &self,
        context: &NoteBlockChange,
        new_note: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_note_block_change(context, new_note, cancel);
    }
}
