//! One invulnerability observation across a paused player hit.
use super::{Environment, Hurting};

impl Environment<'_> {
    pub(super) fn observe_hit_invulnerability(&mut self, vanilla: bool) -> bool {
        if let Some(answer) = self.invulnerability_checked {
            return answer;
        }
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let answer = self
            .level
            .invulnerability_check(Hurting::Player(self.entity, source), vanilla);
        // Only the Entity-level decision is retained. Its inputs are captured at hit entry.
        // Player/Living/ServerPlayer predicates still evaluate their own guards on each call.
        self.invulnerability_checked = Some(answer);
        answer
    }
}
