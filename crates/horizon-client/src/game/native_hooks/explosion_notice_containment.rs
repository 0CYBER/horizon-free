use super::*;

impl Game<'_> {
    // A notice cannot veto removal. Recover from a failed native callback just
    // as crash::contain recovers from other mod-controlled work, and let the
    // caller finish removal even if the callback already changed the world.
    pub(in crate::game) fn contained_block_exploded(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        block_entity: Option<BlockEntityRef>,
        explosion: &ExplosionStart<'_>,
    ) {
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_on_block_exploded(pos, state, block_entity, explosion);
        }) {
            eprintln!("on_block_exploded callback failed: {error}");
        }
    }
}
