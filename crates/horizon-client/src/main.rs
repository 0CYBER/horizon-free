//! Horizon: native Rust client for Minecraft Java Edition 26.3 gameplay.
#![forbid(unsafe_code)]
mod game;

fn main() {
    if let Err(error) = game::run() {
        eprintln!("Horizon: {error}");
        std::process::exit(1);
    }
}
