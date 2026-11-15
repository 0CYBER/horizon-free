use super::*;

impl GameLevel<'_> {
    /// Whether a fence, wall, or iron bars or pane connects a side
    /// (`FenceBlock.connectsTo`, `WallBlock.connectsTo`,
    /// `IronBarsBlock.attachsTo`, named by `kind`): set `connected` to say
    /// whether the block `state` at `pos` connects toward `neighbour`, the
    /// state beside it, `direction` pointing from the block to that neighbour
    /// and `face_solid` whether the neighbour's face toward it is sturdy.
    /// `connected` holds vanilla's answer: change it, to false as well as to
    /// true, when the block is your own. Asked whenever the block settles its
    /// shape or is placed, inside the level's update. Only the block's own
    /// part is asked, so vanilla blocks cost nothing. Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_connects_to(&self, call: super::fire::ConnectsToCall<'_>) {
        self.fire_connects_to(call);
    }
}
