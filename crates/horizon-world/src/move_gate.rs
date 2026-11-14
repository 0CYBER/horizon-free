//! The gate one physical entity move goes through (#2414): what the move's
//! cause is and what the `entity_move` decision turned out to be, so the game
//! layer decides once and the kind's physics runs with the answer.
//!
//! The gate is plain data: it takes no borrow, allocates nothing and scans no
//! mod, so a move with no listener costs one branch. The client's
//! `move_gate` module arms it (and, from PR 1b on, fires the `entity_move`
//! hook there, in that one function) and is the only caller allowed to reach
//! the collide entries.
use crate::entity_move::MoverType;
use horizon_core::geometry::Vec3;

/// What a gate decided for one `Entity.move` invocation.
#[derive(Clone, Copy, Debug)]
pub enum MoveDecision {
    /// Run the kind's physics with this movement.
    Run(Vec3),
    /// Skip the invocation: no position, velocity, collision-flag or fall
    /// change, and no displacement notice.
    Skip,
}

/// One physical `Entity.move` invocation (#2414): the mover's cause, the
/// movement it asked for, and the decision taken. Created pending, armed at
/// the move moment by the client's `move_gate::arm`, and then handed to the
/// kind's physics, which resolves its collision through the same client
/// module.
#[derive(Clone, Copy, Debug)]
pub struct MoveGate {
    cause: MoverType,
    requested: Vec3,
    decision: MoveDecision,
}

impl MoveGate {
    /// A pending gate: no cause, no decision. The mover that receives it arms
    /// it at its move moment.
    pub const fn new() -> Self {
        Self {
            cause: MoverType::SelfMovement,
            requested: Vec3::ZERO,
            decision: MoveDecision::Run(Vec3::ZERO),
        }
    }

    /// Arm the gate for one move invocation: this cause asked for this
    /// movement. The client's `move_gate::arm` is the one door that calls
    /// this; from PR 1b it fires the `entity_move` hook first.
    pub fn arm(&mut self, cause: MoverType, movement: Vec3) {
        self.cause = cause;
        self.requested = movement;
        self.decision = MoveDecision::Run(movement);
    }

    /// Decide the invocation runs with this movement instead of the armed
    /// one: a listener's replacement (PR 1b). The request it overrides stays
    /// what [`Self::overridden`] measures against.
    pub fn decide(&mut self, movement: Vec3) {
        self.decision = MoveDecision::Run(movement);
    }

    /// Whether a listener's decision is not what the move asked for: a
    /// cancel, or a replacement with a different movement. The game layer
    /// reads this flag instead of comparing deltas — the ruling's «a cancel
    /// or a replacement sends a correction» (#2414) holds however small the
    /// move, so the sites that send corrections key on it, not on distance.
    pub fn overridden(&self) -> bool {
        match self.decision {
            MoveDecision::Run(delta) => delta != self.requested,
            MoveDecision::Skip => true,
        }
    }

    /// The cause the gate was armed with.
    pub fn cause(&self) -> MoverType {
        self.cause
    }

    /// The movement the decision runs with: the armed movement, or what a
    /// listener replaced it with (PR 1b).
    pub fn delta(&self) -> Vec3 {
        match self.decision {
            MoveDecision::Run(delta) => delta,
            MoveDecision::Skip => Vec3::ZERO,
        }
    }

    /// Whether the invocation is skipped: the kind's physics must not run.
    pub fn skipped(&self) -> bool {
        matches!(self.decision, MoveDecision::Skip)
    }

    /// Skip the invocation (PR 1b: a listener's cancel, or a stale decision).
    pub fn skip(&mut self) {
        self.decision = MoveDecision::Skip;
    }
}

impl Default for MoveGate {
    fn default() -> Self {
        Self::new()
    }
}
