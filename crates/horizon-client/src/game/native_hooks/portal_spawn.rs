use super::*;

impl GameLevel<'_> {
    /// A fire placed at `pos` (`state`) found an empty obsidian frame, `shape`,
    /// and is about to fill it with a nether portal (`BlockEvent.PortalSpawnEvent`,
    /// in `BaseFireBlock.onPlace`), in the Overworld or the Nether. Set
    /// `cancel` and no portal forms: the fire stays where it can survive, as
    /// without a frame. Server-side.
    /// A player is tuning a note block on the authoritative server, not in
    /// client prediction. `context` gives its dimension, position, instrument
    /// and old note; `new_note` starts as vanilla's cycled note. Set it to 0
    /// through 24; an out-of-range value keeps vanilla's. Cancel to leave the
    /// note unchanged and prevent playback. API 387.
    #[inline(always)]
    pub(in crate::game) fn native_portal_spawn(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        shape: PortalShape,
        cancel: &mut bool,
    ) {
        self.fire_portal_spawn(dimension, pos, state, shape, cancel);
    }
}
