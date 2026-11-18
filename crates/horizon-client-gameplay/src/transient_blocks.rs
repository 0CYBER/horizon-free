//! `ClientboundAddTransientBlockPacket`: a block the server places that the
//! client shows for a moment, drawn as a moving block until the section mesh
//! that holds it has been rebuilt (a landed falling block would otherwise
//! leave a gap between the entity going and the mesh catching up).
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
pub struct Shown {
    pub state: BlockStateId,
    pub live_until: Instant,
    /// `createTimeNs`, on the mesher's dispatch clock (`Streaming::sequence`):
    /// `SectionOcclusionGraph.schedulePropagationFrom` removes the block once a
    /// mesh of its section whose compile task started after this is uploaded.
    pub created: u64,
}

/// `LevelExtractor.transientBlockQueue` and `LevelRenderer.transientBlocks`.
#[derive(Default)]
pub struct TransientBlocks {
    queue: VecDeque<Queued>,
    /// By block position, so a second one at the same place replaces it.
    pub shown: HashMap<[i32; 3], Shown>,
}

impl TransientBlocks {
    /// `LevelExtractor.queueTransientBlock`.
    pub fn queue(&mut self, pos: [i32; 3], state: BlockStateId) {
        self.queue.push_back(Queued {
            pos,
            state,
            time_to_live: DEFAULT_TIME_TO_LIVE,
        });
    }

    /// `LevelExtractor.drainTransientBlockQueue`: queued blocks go on show
    /// for their time to live from `now`.
    pub fn drain(&mut self, now: Instant, created: u64) {
        while let Some(block) = self.queue.pop_front() {
            self.shown.insert(
                block.pos,
                Shown {
                    state: block.state,
                    live_until: now + Duration::from_secs_f64(block.time_to_live),
                    created,
                },
            );
        }
    }

    pub fn clear(&mut self) {
        self.queue.clear();
        self.shown.clear();
    }
}

/// `performTransientBlockRemovals`: a section's uploaded mesh shows a block created at `created`
/// when its compile task started after it (`state.createTimeNs > compileTaskStartTimeNs` keeps it).
pub fn shows_block(created: u64, compile_start: Option<u64>, upload_pending: bool) -> bool {
    !upload_pending && compile_start.is_some_and(|start| created < start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_dropped_once_a_mesh_compiled_after_it_is_uploaded() {
        // No mesh yet, or one that began before the block existed: kept.
        assert!(!shows_block(5, None, false));
        assert!(!shows_block(5, Some(5), false));
        assert!(!shows_block(5, Some(3), false));
        // A later compile start shows it, once the mesh is uploaded.
        assert!(shows_block(5, Some(6), false));
        assert!(!shows_block(5, Some(6), true));
    }

    /// `drainTransientBlockQueue`: a block goes on show for a second from the
    /// frame that drains it, and a second at the same place replaces it.
    #[test]
    fn queued_blocks_go_on_show_for_a_second() {
        let mut blocks = TransientBlocks::default();
        blocks.queue([1, 2, 3], BlockStateId(5));
        assert!(blocks.shown.is_empty());
        let now = Instant::now();
        blocks.drain(now, 7);
        assert!(blocks.queue.is_empty());
        assert_eq!(blocks.shown[&[1, 2, 3]].created, 7);
        assert_eq!(
            blocks.shown[&[1, 2, 3]].live_until,
            now + Duration::from_secs(1)
        );
        blocks.queue([1, 2, 3], BlockStateId(6));
        blocks.drain(now, 9);
        assert_eq!(blocks.shown.len(), 1);
        assert_eq!(blocks.shown[&[1, 2, 3]].state, BlockStateId(6));
    }
}
