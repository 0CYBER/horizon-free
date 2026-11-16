//! The `can_be_connected_to` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, whether the block `state` at `pos` lets the fence, wall, or
    /// iron bars or pane `connector` beside it connect to it
    /// (`IBlockExtension.canBeConnectedTo`): the block being connected to answers, not the
    /// connecting one. `kind` names the connector's vanilla rule, `direction` points from the
    /// block toward the connector and `face_solid` is whether the block's face toward it is
    /// sturdy. Asked once the connector's vanilla rule answered, each time the connector is
    /// placed or settles its shape toward the block. `Some(true)` connects, `Some(false)`
    /// refuses and `None` keeps vanilla's rule. The connector's own `connects_to` is asked
    /// after, starting from this answer, and has the last word. A panicking part is contained
    /// and vanilla's answer is used. The level is read-only. With no part attached to the
    /// state, the cost is one table lookup. Diverges from upstream: new hook.
    #[notes = r#"## Blocks fences, walls and panes connect to

- **`BlockPart::can_be_connected_to(level, event)`** (`CanBeConnectedToEvent { dimension, pos, state, connector, kind, direction, face_solid }`): the block beside a fence, a wall, or iron bars or a glass pane answers whether that connector connects to it (NeoForge's `IBlockExtension.canBeConnectedTo`). `event.state` at `event.pos` is your block, the target; `event.connector` is the connecting block's state, `event.kind` its vanilla rule (`ConnectionKind::Fence`, `Wall` or `Bars`), `event.direction` the way from your block to the connector and `event.face_solid` whether your block's face toward it is sturdy. `Some(true)` connects, even where vanilla would not (a lantern beside a fence); `Some(false)` refuses, even beside a sturdy face; `None` leaves it to another part and then vanilla's rule: a fence takes a sturdy face that is not an exception, a fence of the same kind or a gate across its arm; a wall a wall, a sturdy face, bars or a pane, or a gate across its side; bars and panes a sturdy face, bars or a pane, or a wall. A wall's connected side is still low or tall from the block above it. The connector's own part is asked after, in `connects_to`, with `event.connected` starting at this answer: the connecting block has the last word (Horizon's order). Asked when the connector is placed and each time it settles its shape toward your block. A panicking part is contained with a warning and vanilla's answer is used. Authoritative servers only; the level is read-only. With no part attached to the state, one table lookup per side asked. Diverges from upstream: new hook."#]
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::can_be_connected_to::GameLevel<'_>::native_can_be_connected_to",
        neoforge = "IBlockExtension.canBeConnectedTo"
    )]
    fn can_be_connected_to(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        connector: BlockState,
        kind: ConnectionKind,
        direction: Direction,
        face_solid: bool,
        answer: &mut Option<bool>,
    );
}
