//! The `is_fire_source` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, whether the block `state` at `pos`, under a
    /// fire, keeps that fire burning (`FireBlock.tick`'s check of the
    /// dimension's `infiniburn` tag), asked on every tick of the fire above it:
    /// `Some(true)` makes the fire endless, as netherrack does in the
    /// overworld (rain doesn't put it out and it never burns out), and
    /// `Some(false)` refuses a block the tag lists. Left `None`, the
    /// dimension's tag answers. Asked only for the block under a fire, so its
    /// side is always the top. Answer only for your own blocks. The level is
    /// read-only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "source",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::is_fire_source::GameLevel<'_>::native_is_fire_source",
        neoforge = "IBlockExtension.isFireSource"
    )]
    fn is_fire_source();
}
