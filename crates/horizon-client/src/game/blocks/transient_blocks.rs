//! `ClientboundAddTransientBlockPacket`: a block the server places that the
//! client shows for a moment, drawn as a moving block until the section mesh
//! that holds it has been rebuilt (a landed falling block would otherwise
//! leave a gap between the entity going and the mesh catching up).
use super::Game;
pub(super) use horizon_client_gameplay::transient_blocks::TransientBlocks;
use horizon_client_gameplay::transient_blocks::shows_block;
use horizon_client_render::render_geometry::Vertex;
use horizon_world::registry::BlockStateId;
use std::time::Instant;

impl Game<'_> {
    /// `ClientPacketListener.handleAddTransientBlockPacket`.
    pub(super) fn add_transient_block(&mut self, pos: [i32; 3], state: i32) {
        self.transient_blocks.queue(pos, BlockStateId(state as u32));
    }

    /// `LevelExtractor.drainTransientBlockQueue`, once a frame.
    pub(super) fn drain_transient_block_queue(&mut self) {
        self.transient_blocks
            .drain(Instant::now(), self.streaming.sequence());
    }

    /// `LevelRenderer.submitTransientBlocks`: each block whose time has not
    /// run out, drawn as a moving block at its position, camera-relative.
    /// One whose section mesh has been rebuilt since it came is dropped
    /// (`performTransientBlockRemovals`).
    pub(super) fn transient_block_geometry(&mut self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        if self.transient_blocks.shown.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut shown = std::mem::take(&mut self.transient_blocks.shown);
        shown.retain(|pos, block| {
            if now >= block.live_until {
                return false;
            }
            // performTransientBlockRemovals: the section's uploaded mesh began compiling
            // at or after the block was created, so it shows the block.
            !self.section_of(*pos).is_some_and(|section| {
                shows_block(
                    block.created,
                    self.streaming.compile_start(section),
                    self.section_upload_pending(section),
                )
            })
        });
        let start = out.len();
        for (pos, block) in &shown {
            let at = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            self.moving_block_vertices(
                block.state,
                horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                at - camera,
                out,
            );
        }
        // Lit as the blocks a piston moves, which are drawn the same way.
        self.lit_block_entity(start..out.len());
        self.transient_blocks.shown = shown;
    }
}
