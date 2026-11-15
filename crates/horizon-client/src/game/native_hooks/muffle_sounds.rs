use super::*;

impl Game<'_> {
    /// Muffles the game's sounds for a while, as ears ringing after a close
    /// blast hear them: `amount` (0 to 1) of every sound's volume is taken
    /// away and `lowpass` (0 to 1) dulls it, from untouched to a 300 Hz
    /// low-pass. Both start in full and fade back in a straight line over
    /// `ticks` client ticks, held while the game is paused. The sound events in
    /// `exempt` (`mymod:ear.ringing`) play clearly through it, those already
    /// playing too, so the mod's own ringing stays sharp. Everything else is
    /// muffled, music and the interface's clicks included. A new call replaces
    /// the muffle in place; `ticks` 0 ends it at once. Nothing changes while no
    /// mod calls it. False, with nothing changed, on a server, for an amount or
    /// strength outside 0 to 1, or for an exempt event that isn't
    /// `namespace:path` or more than 64 of them. API @API:muffle_sounds@.
    pub(in crate::game) fn muffle_sounds(
        &mut self,
        amount: f32,
        lowpass: f32,
        ticks: u32,
        exempt: &[&str],
    ) -> bool {
        if !self.native_window() || exempt.len() > 64 || !exempt.iter().all(|e| sound_id(e)) {
            return false;
        }
        self.audio.muffle(amount, lowpass, ticks, exempt)
    }
}
