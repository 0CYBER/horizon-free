use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A dying mob or player `entity` of `kind` is about to drop `original`
    /// points of experience; the vanilla conditions have been checked.
    /// `attacking_player` is the player who last hurt it, if any. Change
    /// `amount` (0 or less drops nothing), or set `cancel` to drop none. A
    /// sculk catalyst that takes the experience does not call this.
    /// Server-side.
    pub(in crate::game) fn native_living_experience_drop(
        &mut self,
        entity: u32,
        kind: &str,
        attacking_player: Option<u128>,
        original: i32,
        amount: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_living_experience_drop(entity, kind, attacking_player, original, amount, cancel);
    }
}
