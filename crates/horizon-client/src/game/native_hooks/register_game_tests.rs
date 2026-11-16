use super::*;
use mod_api::GameTests;
impl Game<'_> {
    /// Register functions before resolving the world's test definitions.
    pub(in crate::game) fn native_register_game_tests(&self, functions: &mut dyn GameTests) {
        self.fire_register_game_tests(functions);
    }
}
