use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player tries to sleep in the bed whose head is at `pos`
    /// (NeoForge's `CanPlayerSleepEvent`, vanilla's `ServerPlayer.startSleepInBed`).
    /// `problem` is what vanilla found, first problem only: `too_far_away`,
    /// `obstructed`, `not_possible_now` (not night or thunder, the bed rule),
    /// `not_safe` (monsters near), or `None`. A mod may set `None` to let the
    /// player sleep anyway, or one of those names (`other_problem` refuses
    /// without a message) to refuse. The spawn point is set before it fires,
    /// as in vanilla, only when vanilla found no problem. After it,
    /// `native_player_bed_enter` still fires and may cancel. Server-side.
    /// API 393.
    pub(in crate::game) fn native_can_player_sleep(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        problem: &mut Option<String>,
    ) {
        self.fire_can_player_sleep(player, name, pos, problem);
    }
}
