//! Player hit-source adapters.
use super::Game;

impl Game<'_> {
    /// Damage of a given type to the player from a point.
    pub(super) fn mob_hits_player_kind(&mut self, amount: f32, from: [f64; 3], kind: &str) {
        self.mob_hits_player(amount, from, kind);
    }
    /// Damage whose causing mob is known (`cause`, or none): not guessed
    /// from the mob nearest to `from`.
    pub(super) fn mob_hits_player_caused(
        &mut self,
        amount: f32,
        from: [f64; 3],
        kind: &str,
        cause: Option<u32>,
    ) {
        self.hurt_player_from(amount, Some(from), kind, Some(cause), false);
    }
    pub(super) fn mob_hits_player(
        &mut self,
        amount: f32,
        from: [f64; 3],
        damage_type: &str,
    ) -> bool {
        self.hurt_player_from(amount, Some(from), damage_type, None, false)
    }
    /// [`mob_hits_player`](Self::mob_hits_player) whose hit was a creative
    /// player's, which passes the victim's invulnerability
    /// (`DamageSource.isCreativePlayer`, arismoko/horizon#916).
    pub(in crate::game) fn mob_hits_player_from(
        &mut self,
        amount: f32,
        from: [f64; 3],
        damage_type: &str,
        by_creative_player: bool,
    ) -> bool {
        self.hurt_player_from(amount, Some(from), damage_type, None, by_creative_player)
    }
}
