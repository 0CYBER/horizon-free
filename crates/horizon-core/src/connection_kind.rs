/// Which vanilla rule a connection query comes from: the block asking is a fence,
/// a wall, or iron bars or a pane, and vanilla's own answer is that rule's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectionKind {
    /// `FenceBlock.connectsTo`: an arm of a fence.
    Fence,
    /// `WallBlock.connectsTo`: a side of a wall, low or tall from the block above.
    Wall,
    /// `IronBarsBlock.attachsTo`: an arm of iron bars or a glass pane, which are
    /// the same block class.
    Bars,
}
