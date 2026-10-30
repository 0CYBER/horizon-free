//! Test blocks on the server: `handleSetTestBlock` and the mode a placed
//! test block starts with.
use super::{Game, lan::LanHost};
use horizon_client::{
    containers::BlockEntity,
    test_block::{TestBlock, TestBlockMode},
};
use horizon_protocol::packets::play;

type Pos = (i32, i32, i32);

impl Game<'_> {
    fn test_block_mut(&mut self, pos: Pos) -> Option<&mut TestBlock> {
        match self.world.block_entities.get_mut(&pos)? {
            BlockEntity::TestBlock(b) => Some(b),
            _ => None,
        }
    }

    /// The `mode` of the block state at `pos`.
    pub(super) fn test_block_state_mode(&self, pos: Pos) -> Option<TestBlockMode> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let mode = self
            .pack
            .assets
            .registry
            .state(state)?
            .properties
            .get("mode")?;
        TestBlockMode::of_serialized_name(mode)
    }

    /// `new TestBlockEntity(pos, state)`: the mode is the state's.
    pub(super) fn test_block_placed(&mut self, pos: Pos) {
        let Some(mode) = self.test_block_state_mode(pos) else {
            return;
        };
        if let Some(b) = self.test_block_mut(pos) {
            b.mode = mode;
        }
    }

    /// `handleSetTestBlock` for a guest: only a creative player with game
    /// master commands edits the block.
    pub(super) fn lan_set_test_block(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::SetTestBlock,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest(host, conn, |game| {
            if allowed {
                game.apply_set_test_block(&p);
            }
        });
    }

    fn apply_set_test_block(&mut self, p: &play::SetTestBlock) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        if self.test_block_mut(pos).is_none() {
            return;
        }
        let mode = TestBlockMode::by_id(p.mode);
        // TestBlockEntity.setMode: the entity's and the block state's.
        if let Some(b) = self.test_block_mut(pos) {
            b.mode = mode;
        }
        self.set_mode_property(pos, "minecraft:test_block", mode.serialized_name());
        if let Some(b) = self.test_block_mut(pos) {
            b.message = p.message.clone();
        }
        self.mark_block_entity_updated(pos);
    }
}
