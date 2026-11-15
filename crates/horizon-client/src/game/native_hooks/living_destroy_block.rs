use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mob is about to destroy a block (NeoForge's `LivingDestroyBlockEvent`):
    /// a wither breaking blocks around it, a ravager breaking leaves, the ender
    /// dragon's wall check, an enderman picking a block up, a silverfish waking
    /// its friends (the infested blocks break), a zombie breaking a door or a
    /// turtle egg. `entity` is its entity id, `kind` its type (`minecraft:wither`),
    /// `pos` the block and `state` its state. Canceling keeps the block (an
    /// enderman then holds nothing). Asked once per block, only while a mod is
    /// loaded. Server-side.
    pub(in crate::game) fn native_living_destroy_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        state: BlockStateId,
        cancel: &mut bool,
    ) {
        self.fire_living_destroy_block(entity, kind, pos, state, cancel);
    }
}
