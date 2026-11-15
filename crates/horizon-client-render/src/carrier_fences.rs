//! Carrier transfer fences: the world cells and carriers locked while
//! a carrier ownership operation runs between prepared and committed
//! (issue arismoko/horizon#2413, slice 2). The set lives on the World so every refusal
//! point — the player's break, the piston hook, the container gate and
//! the mod write gate — reads one place. With no operation in flight
//! every check is one load of an empty set.

use std::collections::HashSet;

/// The locked world cells and carriers of the operations in flight.
#[derive(Default)]
pub struct Fences {
    cells: HashSet<[i32; 3]>,
    carriers: HashSet<u128>,
    /// Set when a transfer broke after `prepared`: the fences hold, the
    /// operation stays for recovery at world open, and every further
    /// transfer refuses until the world reopens.
    broken: bool,
    /// Test seam: the next flush after `prepared` fails, as a crashed
    /// writer would. Set by the carrier transfer tests only.
    pub fail_flush: bool,
    /// Test seam: the store's index capacity for transfers opened from
    /// this game (None: the default 4,096). Lets the retention tests
    /// exercise the overflow paths in seconds.
    pub capacity_override: Option<usize>,
}
impl Fences {
    /// Whether a world cell is locked against edits, interactions,
    /// pistons and container opening.
    pub fn locks_cell(&self, pos: &[i32; 3]) -> bool {
        !self.cells.is_empty() && self.cells.contains(pos)
    }

    /// Whether a carrier is locked against motion and inventory.
    pub fn locks_carrier(&self, uuid: u128) -> bool {
        !self.carriers.is_empty() && self.carriers.contains(&uuid)
    }

    /// Locks one world cell.
    pub fn lock_cell(&mut self, pos: [i32; 3]) {
        self.cells.insert(pos);
    }

    /// Unlocks one world cell.
    pub fn unlock_cell(&mut self, pos: [i32; 3]) {
        self.cells.remove(&pos);
    }

    /// Locks one carrier.
    pub fn lock_carrier(&mut self, uuid: u128) {
        self.carriers.insert(uuid);
    }

    /// Unlocks one carrier.
    pub fn unlock_carrier(&mut self, uuid: u128) {
        self.carriers.remove(&uuid);
    }

    /// Whether a transfer broke after `prepared` and the world must be
    /// reopened before another runs.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Whether any cell a piston move would push or destroy is locked
    /// by a carrier transfer: the whole move refuses. The piston gate
    /// seeds its cancellation with this, so a held cell also skips the
    /// piston hooks (`GameLevel::allow_piston_move`).
    pub fn holds_move(
        &self,
        push: &[horizon_core::position::BlockPos],
        destroy: &[horizon_core::position::BlockPos],
    ) -> bool {
        push.iter().chain(destroy.iter()).any(|pos| {
            let [x, y, z] = [pos.x, pos.y, pos.z];
            self.locks_cell(&[x, y, z])
        })
    }

    /// Marks the transfer broken: fences hold, further transfers refuse.
    pub fn mark_broken(&mut self) {
        self.broken = true;
    }
}
