//! The `player_move` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's movement report is about to be applied: they would move from
    /// `from` to `to`, or turn from `from_rotation` to `to_rotation` (`[yaw,
    /// pitch]`). Heard once for each validated report whose position or
    /// rotation differs from the player's last reported location, after the
    /// server's own validation and before the report is committed. That
    /// location is kept on the player's connection, apart from the accepted
    /// position: it is the position given to the listener before it runs, and
    /// the destination of every server-side teleport (a join, a teleport in the
    /// level, a level change, a respawn, the resend of an unconfirmed teleport,
    /// and the correction after a veto). `from` is that location. Set `cancel`
    /// for a real veto and not a rollback: the report is not applied and its
    /// bookkeeping is skipped, and the client is corrected to the accepted
    /// position and rotation through the existing teleport correction. The veto
    /// adds no anticheat violation of its own, but the report was already seen
    /// by the server's own checks and by the anticheat's record of the client's
    /// ticks before the hook, and what a violation of those would have done
    /// still happens. The values are the exact ones: there is no once-per-tick
    /// accumulation and no threshold in the engine, so the mod filters
    /// (`if (to - from).length() < 1.0 / 16.0 { return; }` skips anything under
    /// 1/16 of a block). Their state is swapped in
    /// (`host.players().acting_player()`, `PlayerRef::tell`).
    ///
    /// A held report is ruled on in the same tick, and the player's later
    /// packets (further reports, the end of the client tick, pongs) wait in
    /// order behind it, then go on in that tick, so each later report is heard
    /// from the one before. A report is out of
    /// time at its 21st tick, counting the tick it was held at as the first: it
    /// is dropped with the correction and never committed, even when a ruling
    /// arrives after that. A player's disconnect, teleport or level change
    /// drops a pending report cleanly. Not heard: the first tick in a level (the
    /// tick after a join or a level change; the second is heard), a player riding
    /// something (`vehicle_moved` hears the vehicle), any teleport (its
    /// confirmation is validated and committed without the hook, at the teleport's
    /// target, and so is the veto's own correction; a report that arrives while a teleport is
    /// unconfirmed is not heard either), and a passage crossing (its caller
    /// reads the position at once, so the report is committed in place). The
    /// first report after a teleport that passed validation is heard, with the
    /// teleport's destination as its `from`. With no listener the report is
    /// committed in place with no extra work: nothing is held, copied or
    /// allocated. With one, the cost is one held report for each moving player
    /// a tick.
    ///
    /// A fault or an epoch timeout in a wasm listener answers nothing: it is
    /// not a veto and the report is committed, like every cancellable hook; the
    /// fault is logged, counted against the mod, and stops the mod's group for
    /// the session. A native listener that never returns is out of scope: it
    /// blocks the game thread like any native code, and the 20-tick deadline
    /// covers only a report held pending across ticks. Server-side.
    #[notes = r#"## Player move

- **`player_move(host, event)`**: Bukkit's `PlayerMoveEvent` for a guest's own movement, fired from `ServerGamePacketListenerImpl.handlePlayerPositionChange`'s place: after the server validated the report, before it is committed. Cancelling vetoes it; the client is sent back to the accepted position and rotation. The engine applies no threshold: a mod that wants Paper's 1/256 square block or ten degrees compares `from` and `to` itself."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no player move event; PlayerTickEvent covers the tick)",
        paper = "PlayerMoveEvent",
        cancellable = true
    )]
    fn player_move();
}
