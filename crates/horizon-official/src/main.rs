//! The official game executable (docs/layers.md, "How a Horizon crate plugs in").
#![forbid(unsafe_code)]

/// The game identity marker for launcher imports, retained by `main`.
static GAME_IDENTITY: [u8; horizon_game::GAME_IDENTITY.len()] = horizon_game::GAME_IDENTITY;

fn main() {
    horizon_game::prepare_main_stack();
    std::hint::black_box(&GAME_IDENTITY);
    horizon_official::register();
    horizon_game::main();
}
