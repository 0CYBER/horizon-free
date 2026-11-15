// game-module-cfg: test
//! The zero-argument server fixture that the tests carried over from Horizon main call
//! (`native_machines::tests::server_game()`). The fork's fixture
//! ([`game_tests::tests::server_game`]) borrows its resources; this one leaks them for the life
//! of the test process, as main's did, so the game is `'static`.
pub(in crate::game) mod tests {
    use crate::game::Game;

    /// A server's game on the prepared resources: nobody is playing, and the chunk at the origin
    /// is loaded.
    pub(in crate::game) fn server_game() -> Game<'static> {
        let resources: &'static mut Option<std::sync::Arc<crate::game::Resources>> =
            Box::leak(Box::new(None));
        crate::game::game_tests::tests::server_game(resources)
    }
}
