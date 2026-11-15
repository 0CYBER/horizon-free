//! Position writes captured while a lifted mob list is being ticked.
//!
//! Mob ticks cannot call back through `Game` while their world view is borrowed. Keep the
//! write-side record separate from `Mob`; the game drains it as soon as the current mob's tick
//! is complete, preserving every teleport before the ordinary movement write.

use crate::join_seq::JoinSeq;
use horizon_core::geometry::Vec3;
use std::cell::RefCell;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug)]
pub struct PositionWrite {
    pub id: u32,
    pub join: JoinSeq,
    pub from: Option<Vec3>,
    pub to: Vec3,
    /// The write was filed and delivered at the write boundary.  It remains in the
    /// per-tick record only so the ordinary movement bookkeeping knows that this
    /// mob already had a position write and must not synthesize a second one.
    pub delivered: bool,
}

impl PositionWrite {
    pub const fn new(id: u32, join: JoinSeq, from: Option<Vec3>, to: Vec3) -> Self {
        Self {
            id,
            join,
            from,
            to,
            delivered: false,
        }
    }

    pub const fn captured(id: u32, join: JoinSeq, from: Vec3, to: Vec3) -> Self {
        Self::new(id, join, Some(from), to)
    }

    pub const fn baseline(id: u32, join: JoinSeq, to: Vec3) -> Self {
        Self::new(id, join, None, to)
    }
}

/// A live section boundary for a mob write.  The game supplies this only while
/// the entity list is lifted; keeping it behind a tiny trait lets the mob code
/// report a write without borrowing `Game` through its world view.
pub trait WriteObserver {
    fn position_written(&mut self, id: u32, from: Vec3, to: Vec3);
}

/// A shared live observer used inside cloned `MobWorld` views.
pub type Observer = std::rc::Rc<RefCell<dyn WriteObserver>>;

#[derive(Default)]
struct State {
    enabled: bool,
    writes: VecDeque<PositionWrite>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Start collecting writes for one lifted creature tick.
pub fn begin(enabled: bool) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.enabled = enabled;
        state.writes.clear();
    });
}

/// Capture a committed teleport while the current mob is being ticked.
pub fn record(id: u32, join: JoinSeq, from: Vec3, to: Vec3) {
    record_with(id, join, from, to, None);
}

/// Capture a write and, when a live observer is present, deliver it before the
/// mob continues its tick.  The record is retained as a delivered marker so
/// the end-of-tick fallback does not replay it.
pub fn record_with(id: u32, join: JoinSeq, from: Vec3, to: Vec3, observer: Option<Observer>) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.enabled {
            if let Some(observer) = observer.as_ref() {
                observer.borrow_mut().position_written(id, from, to);
            }
            let mut write = PositionWrite::captured(id, join, from, to);
            write.delivered = observer.is_some();
            state.writes.push_back(write);
        }
    });
}

/// Move the current mob's writes into the game's per-tick record.
pub fn drain_for(id: u32, out: &mut Vec<PositionWrite>) -> usize {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let mut count = 0;
        while state.writes.front().is_some_and(|write| write.id == id) {
            // Writes are recorded in tick order, so the current mob owns a contiguous prefix.
            // Pop that prefix without rescanning the records already handed to the game.
            out.push(state.writes.pop_front().expect("checked above"));
            count += 1;
        }
        count
    })
}

/// Stop collecting and retain the allocation for the next tick.
pub fn finish() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.enabled = false;
        state.writes.clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_multiple_writes_for_one_mob_in_order() {
        begin(true);
        let join = JoinSeq::next();
        record(
            7,
            join,
            Vec3::new(15.5, 64.0, 0.5),
            Vec3::new(16.5, 64.0, 0.5),
        );
        record(
            7,
            join,
            Vec3::new(16.5, 64.0, 0.5),
            Vec3::new(32.5, 64.0, 0.5),
        );
        record(8, JoinSeq::next(), Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let mut out = Vec::new();
        drain_for(7, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, 7);
        assert_eq!(out[0].from.unwrap().x, 15.5);
        assert_eq!(out[1].to.x, 32.5);
        finish();
    }

    #[test]
    fn disabled_capture_does_not_record() {
        begin(false);
        record(7, JoinSeq::next(), Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let mut out = Vec::new();
        drain_for(7, &mut out);
        assert!(out.is_empty());
        finish();
    }
}
