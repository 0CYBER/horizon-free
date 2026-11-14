//! The `entity_move` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// One physical `Entity.move` invocation (#2414, design #2308, the
    /// maintainer ruling of the same issue): the entity `entity` of `kind`
    /// (`minecraft:zombie`) is about to move by `delta` (blocks this tick) for
    /// `cause`'s sake - itself, the player behind a position packet, a piston,
    /// a shulker box lid, a shulker mob's opening lid. It fires before the
    /// move's own handling, no-physics and the stuck multiplier included, in
    /// the one gate every kind's physics resolves through, so a forgotten
    /// family cannot move around it. Change `delta` to replace the movement
    /// (still through every ordinary restriction) or set `canceled` to skip
    /// the invocation: no position, velocity, collision-flag or fall change,
    /// and no displacement notice. A replacement component that is not a
    /// finite number, or lies outside ±64 blocks, is refused whole - the move
    /// stands as the requested delta would. A listener that unwinds keeps the
    /// requested delta, not a replacement an earlier listener had proposed,
    /// and is switched off for the session. It takes no Host - the maintainer
    /// ruling, like the enderman's teleport: the listener only decides the
    /// delta, and its `context` is the loop `context` of the side that ticks
    /// the body. Server-authoritative: the client does not predict a custom
    /// movement. Beyond NeoForge: there is no counterpart.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        context = "loop",
        cancellable = true,
        isolate = true,
        neoforge = "none (fork addition; see travel-and-move.md, #2308)",
        target = "horizon_client_lib::move_gate::native_entity_move"
    )]
    fn entity_move();
}
