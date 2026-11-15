use super::*;

impl Game<'_> {
    /// A tamed mob's owner message, after eligibility and message construction.
    pub(in crate::game) fn native_living_death_message(
        &mut self,
        entity: u32,
        kind: &str,
        message: &mut Option<Text>,
    ) {
        self.fire_living_death_message(entity, kind, message);
    }
}
