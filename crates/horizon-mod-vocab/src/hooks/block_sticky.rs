//! The `block_sticky` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the block state this part is attached to counts as a slime block to a piston
    /// (`IBlockExtension.isSlimeBlock`, which `PistonStructureResolver` asks wherever it tells slime
    /// from honey): set `slime` to decide. `slime` starts with vanilla's answer, true for
    /// `minecraft:slime_block` only. The structure resolver reads it for the one pair that never
    /// sticks, slime beside honey, so a block answering true is held apart from honey as slime is;
    /// stickiness itself is `is_sticky_block`'s. Asked on an authoritative server while a piston
    /// resolves its structure: answer from the state, without touching the world. With no part
    /// attached to the state, the cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "slime",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::block_sticky::GameLevel<'_>::native_is_slime_block",
        neoforge = "IBlockExtension.isSlimeBlock"
    )]
    fn is_slime_block();
    /// Whether the block state this part is attached to is sticky to a piston
    /// (`IBlockExtension.isStickyBlock`): set `sticky` to decide. `sticky` starts with vanilla's
    /// answer, true for `minecraft:slime_block` and `minecraft:honey_block`. A sticky block pulls
    /// along the blocks beside it that `can_stick_to` lets it hold, so a piston moving one drags
    /// its neighbours, and a structure of them moves as one (still at most twelve blocks). A
    /// block that is not sticky still moves with a sticky neighbour that holds on to it, because
    /// sticking needs one sticky side. Asked on an authoritative server while a piston resolves its
    /// structure: answer from the state, without touching the world. With no part attached to the
    /// state, the cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "sticky",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::block_sticky::GameLevel<'_>::native_is_sticky_block",
        neoforge = "IBlockExtension.isStickyBlock"
    )]
    fn is_sticky_block();
    /// Whether the block state this part is attached to sticks to `other`, the state beside it
    /// (`IBlockExtension.canStickTo`): set `sticks` to decide. `sticks` starts with vanilla's
    /// answer: false for honey beside slime (as `is_slime_block` tells slime), otherwise true when
    /// either state is sticky (`is_sticky_block`). Two blocks stick when each says it sticks to the
    /// other, as `PistonStructureResolver` asks (NeoForge's `canStickToEachOther`), so a part that
    /// answers false for one neighbour is enough to let go of it and a part cannot make a block
    /// stick to one that refuses. The pair is asked from both sides, and the other block's part, if it
    /// has one, answers its own side. Asked on an authoritative server while a piston resolves its
    /// structure: answer from the two states, without touching the world. With no part attached to
    /// the state, the cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "sticks",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::block_sticky::GameLevel<'_>::native_can_stick_to",
        neoforge = "IBlockExtension.canStickTo"
    )]
    fn can_stick_to();
}
