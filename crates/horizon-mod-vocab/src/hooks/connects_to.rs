//! The `connects_to` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a fence, wall, or iron bars or pane of yours connects a side:
    /// set `connected` to say whether the block `state` at `pos` connects toward
    /// `neighbour`, the state beside it, `direction` pointing from the block to
    /// that neighbour and `face_solid` whether the neighbour's face toward the
    /// block is sturdy. `kind` names vanilla's rule, and `connected` holds its
    /// answer: `ConnectionKind::Fence` is `FenceBlock.connectsTo` (a solid face, a
    /// fence of the same kind or a gate across the arm), `Wall` is
    /// `WallBlock.connectsTo` (a wall, a solid face, iron bars or a pane, or a
    /// gate across the side; a connected side is still low or tall from the
    /// block above) and `Bars` is `IronBarsBlock.attachsTo` for iron bars and
    /// glass panes (a solid face, bars or a pane, or a wall; vanilla's rule
    /// ignores `direction`). Change it, to false as well as to true, when the
    /// block is your own. The neighbour's part answers first, in
    /// `can_be_connected_to`, and `connected` starts from its answer: this
    /// hook has the last word. Asked whenever the block settles its shape
    /// toward a side or is placed, inside the level's update: answer from the
    /// arguments without touching the world. Only the block's own part is asked, so
    /// vanilla blocks cost nothing. Authoritative servers only. Contract version
    /// `2`: version `1` was `fence_connects_to`, for fences alone and without
    /// `kind`. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "connected",
        scope = "block",
        key = "state",
        version = 2,
        target = "horizon_client::game::native_hooks::connects_to::GameLevel<'_>::native_connects_to",
        neoforge = "none (vanilla FenceBlock.connectsTo / WallBlock.connectsTo / IronBarsBlock.attachsTo)"
    )]
    fn connects_to(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        kind: ConnectionKind,
        neighbour: BlockState,
        direction: Direction,
        face_solid: bool,
        connected: &mut bool,
    );
}
