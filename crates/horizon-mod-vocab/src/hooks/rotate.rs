//! The `rotate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A structure template is about to place the block `state` at `pos` turned by `rotation`
    /// (`IBlockExtension.rotate`, where vanilla runs `state.mirror(mirror).rotate(rotation)`):
    /// `state` is already mirrored, and `rotated` starts at the game's own answer (vanilla's
    /// table, or a mod block's template's). Set it to another state of the same block to place
    /// that instead. A state of another block, or one the game does not have, is refused and
    /// the game's answer kept. Asked for every block a template places: world generation's
    /// structures, `/place template`, the structure block and the jigsaw pieces that join
    /// them, and the template snapshots `structure_template` reads. Mirroring stays the
    /// game's. Server-side, on the worldgen worker threads and on the server thread, with the
    /// server's worldgen `context`: no `Host`, no writes; `context.state_with_property` builds
    /// states. A panicking part is contained and the game's answer kept. Diverges from
    /// upstream: new hook.
    #[notes = r#"## Rotated block states

- **`BlockPart::rotate(context, event)`** (`RotateEvent { state, pos, rotation }`, `event.rotated`): NeoForge's `IBlockExtension.rotate(state, level, pos, rotation)`, asked as a structure template places a block, where vanilla runs `state.mirror(mirror).rotate(rotation)`. `event.state` is already mirrored (mirroring has no NeoForge extension and stays the game's: vanilla's table, or the template's for a mod block), `event.pos` is where the block goes and `event.rotation` the template's turn. `event.rotated` starts at the game's own turned state; set it to another state of the same block, typically built with `context.state_with_property(event.state, "facing", "north")`, to place that instead. A state of another block, or one this game does not have, is refused with a warning and the game's answer kept.
- **Where it is asked.** Every Horizon path that places a template's blocks: world generation's structures (`StructureTemplate.placeInWorld`, the legacy pieces' `placeBlock` and their template filters), `/place template` and the structure block, the jigsaw blocks a jigsaw structure joins its pieces by, and the snapshots the host's `structure_template` reads. A block entity's data on rotation is a separate step after this one.
- **What it reads.** Like every worldgen Part method, it runs on the worldgen worker threads and on the server thread, with the server's worldgen `context` (a `WorldgenHost`, no `Host` and nothing to write), so it is a pure function of its inputs: the same seed gives the same world.
- **Panics.** A panicking part is contained with a warning and the game's answer kept.
- **Cost.** With no part subscribed, one subscription check per placed block; with one, a table lookup and the state conversions per placed block."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "rotated",
        context = "worldgen",
        scope = "block",
        key = "state",
        target = "mod_api::world::rotation::native_rotate",
        neoforge = "IBlockExtension.rotate"
    )]
    fn rotate();
}
