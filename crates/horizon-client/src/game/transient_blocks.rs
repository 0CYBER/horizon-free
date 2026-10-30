//! `ClientboundAddTransientBlockPacket`: a block the server places that the
//! client shows for a moment, drawn as a moving block until the section mesh
//! that holds it has been rebuilt (a landed falling block would otherwise
//! leave a gap between the entity going and the mesh catching up).
use super::Game;
use horizon_client::mesher::Vertex;
use horizon_world::registry::BlockStateId;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

/// `TransientBlock.DEFAULT_TIME_TO_LIVE`, in seconds.
const DEFAULT_TIME_TO_LIVE: f64 = 1.0;

/// `TransientBlock`: a block queued by the packet.
struct Queued {
    pos: [i32; 3],
    state: BlockStateId,
    time_to_live: f64,
}

/// `TransientBlockRenderState`: a block on show.
struct Shown {
    state: BlockStateId,
    live_until: Instant,
    /// Whether the section mesh was seen out of date since it was added:
    /// `SectionOcclusionGraph.schedulePropagationFrom` removes the block once
    /// a mesh started after it is done, which is the mesh being pending and
    /// then not.
    seen_pending: bool,
}

/// `LevelExtractor.transientBlockQueue` and `LevelRenderer.transientBlocks`.
#[derive(Default)]
pub(super) struct TransientBlocks {
    queue: VecDeque<Queued>,
    /// By block position, so a second one at the same place replaces it.
    shown: HashMap<[i32; 3], Shown>,
}

impl TransientBlocks {
    /// `LevelExtractor.queueTransientBlock`.
    fn queue(&mut self, pos: [i32; 3], state: BlockStateId) {
        self.queue.push_back(Queued {
            pos,
            state,
            time_to_live: DEFAULT_TIME_TO_LIVE,
        });
    }

    /// `LevelExtractor.drainTransientBlockQueue`: queued blocks go on show
    /// for their time to live from `now`.
    fn drain(&mut self, now: Instant) {
        while let Some(block) = self.queue.pop_front() {
            self.shown.insert(
                block.pos,
                Shown {
                    state: block.state,
                    live_until: now + Duration::from_secs_f64(block.time_to_live),
                    seen_pending: false,
                },
            );
        }
    }

    pub(super) fn clear(&mut self) {
        self.queue.clear();
        self.shown.clear();
    }
}

impl Game<'_> {
    /// `ClientPacketListener.handleAddTransientBlockPacket`.
    pub(super) fn add_transient_block(&mut self, pos: [i32; 3], state: i32) {
        self.transient_blocks.queue(pos, BlockStateId(state as u32));
    }

    /// `LevelExtractor.drainTransientBlockQueue`, once a frame.
    pub(super) fn drain_transient_block_queue(&mut self) {
        self.transient_blocks.drain(Instant::now());
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
            let pending = self.section_mesh_pending(*pos);
            block.seen_pending |= pending;
            !(block.seen_pending && !pending)
        });
        for (pos, block) in &shown {
            let at = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            self.moving_block_vertices(
                block.state,
                horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                at - camera,
                out,
            );
        }
        self.transient_blocks.shown = shown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `drainTransientBlockQueue`: a block goes on show for a second from the
    /// frame that drains it, and a second at the same place replaces it.
    #[test]
    fn queued_blocks_go_on_show_for_a_second() {
        let mut blocks = TransientBlocks::default();
        blocks.queue([1, 2, 3], BlockStateId(5));
        assert!(blocks.shown.is_empty());
        let now = Instant::now();
        blocks.drain(now);
        assert!(blocks.queue.is_empty());
        assert_eq!(
            blocks.shown[&[1, 2, 3]].live_until,
            now + Duration::from_secs(1)
        );
        blocks.queue([1, 2, 3], BlockStateId(6));
        blocks.drain(now);
        assert_eq!(blocks.shown.len(), 1);
        assert_eq!(blocks.shown[&[1, 2, 3]].state, BlockStateId(6));
    }
}
