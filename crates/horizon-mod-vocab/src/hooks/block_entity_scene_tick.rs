//! The `block_entity_scene_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Tick a screen scene's copy, never a level's block entity. Implement this on a Part
    /// attached to a block entity type to opt that type into ticking in any mod's clocked
    /// scene. `pos` and `state` are the scene block's local position and state, `kind` is
    /// the block entity type, and `tick` is the scene tick being run. Mutate `fields` to
    /// animate the copy the type's existing `block_entity_submit` draws. There is no
    /// world handle or host, and `SceneClock::animate_blocks` does not gate this hook.
    ///
    /// Not on wasm: the ticker borrows native scene-owned fields directly, without a guest call.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "kind"
    )]
    fn block_entity_scene_tick();
}
