use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// An experience orb looks for a player to follow (NeoForge's
    /// `XpOrbTargetingEvent`, `ExperienceOrb.followNearbyPlayer`), on a
    /// server. `following` starts as the default: the nearest living player
    /// within `scan_distance` (8) blocks who isn't a spectator, or `None`.
    /// Set another player, or `None` to follow no one. The orb only goes
    /// for a living player who isn't a spectator, in its level, and only
    /// moves once they are within 8 blocks. Horizon looks again on every
    /// orb tick (vanilla keeps the player it follows while they stay
    /// within 8 blocks), so this is asked every tick for each orb: answer
    /// fast.
    pub(in crate::game) fn native_xp_orb_targeting(
        &mut self,
        orb: EntityRef,
        scan_distance: f64,
        following: &mut Option<PlayerRef>,
    ) {
        self.fire_xp_orb_targeting(orb, scan_distance, following);
    }
}
