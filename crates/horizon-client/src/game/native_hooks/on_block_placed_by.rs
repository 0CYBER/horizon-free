use super::*;

impl Game<'_> {
    /// `BlockItemExtension.onBlockPlacedBy`: once the block's final state,
    /// stack data and `Block.setPlacedBy` companions exist, before consumption.
    pub(in crate::game) fn native_on_block_placed_by(
        &mut self,
        call: super::fire::OnBlockPlacedByCall<'_>,
    ) {
        self.fire_on_block_placed_by(call);
    }

    /// Dispatch only for this stack's subscribers, without converting an
    /// unobserved stack or constructing a placement event.
    pub(in crate::game) fn block_item_placed(
        &mut self,
        stack: &ItemStack,
        placer: Option<mod_api::PlayerRef>,
        pos: BlockPos,
        ctx: &PlaceContext,
    ) {
        if !mod_api::__private::item_listens(mod_api::HookId::OnBlockPlacedBy, &stack.item) {
            return;
        }
        // setPlacedBy (for example a jukebox) can change the state again.
        let state = self.world.block(pos.x, pos.y, pos.z);
        let face = match ctx.face {
            Direction::Down => mod_api::BlockFace::Down,
            Direction::Up => mod_api::BlockFace::Up,
            Direction::North => mod_api::BlockFace::North,
            Direction::South => mod_api::BlockFace::South,
            Direction::West => mod_api::BlockFace::West,
            Direction::East => mod_api::BlockFace::East,
        };
        self.native_on_block_placed_by(super::fire::OnBlockPlacedByCall {
            stack,
            placer,
            pos: [pos.x, pos.y, pos.z],
            state,
            face,
            click: [ctx.hit.x, ctx.hit.y, ctx.hit.z],
        });
    }
}
