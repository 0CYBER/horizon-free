//! `ClientNarratorClientHost::narrate`: a client mod's line for the player's
//! narrator, through the game's own `GameNarrator` (vanilla's
//! `Minecraft.getNarrator()`).
use super::*;
use horizon_narrator::GameNarrator;
use mod_api::{NarrateError, Narration};

impl Game<'_> {
    pub(in crate::game) fn narrate(
        &mut self,
        text: &horizon_client::text::Text,
        when: Narration,
    ) -> Result<(), NarrateError> {
        // A server thread's `Game` shares the process's narrator but has no
        // `Minecraft` to narrate for, as `Game::narrates` says.
        if !self.options.execution_role.is_client() {
            return Err(NarrateError::Unavailable);
        }
        let line = text.plain();
        horizon_narrator::with_narrator(|narrator| say_for_mod(narrator, &line, when))
    }
}

/// `saySystemNow` or `saySystemQueued` for a mod, refusing by name what
/// vanilla would drop silently: an empty line, an option that does not
/// narrate system messages, no speech. A refused line is not logged.
pub(super) fn say_for_mod(
    narrator: &mut GameNarrator,
    line: &str,
    when: Narration,
) -> Result<(), NarrateError> {
    if line.is_empty() {
        return Err(NarrateError::Empty);
    }
    if !narrator.status().should_narrate_system() {
        return Err(NarrateError::Off);
    }
    if !narrator.is_active() {
        return Err(NarrateError::Unavailable);
    }
    match when {
        Narration::Now => narrator.say_system_now(line),
        // `Queued`, and any later mode: never interrupts.
        _ => narrator.say_system_queued(line),
    }
    Ok(())
}
