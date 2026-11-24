//! Installed mod code loading above the engine (docs/layers.md, arismoko/horizon#2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod native_libraries;
pub use native_libraries::{GAME_BUILD_ID, RUSTC_VERSION, main};

mod loading;
pub use loading::main_with;

/// Establish main-thread stack capacity before compiler-child handling.
pub use horizon_guest_stack::prepare_main as prepare_main_stack;
pub use horizon_mod_wasm::compile_child;

/// Handles the private compiler invocation before any game startup work.
pub fn exit_if_compile_child() {
    horizon_guest_stack::prepare_main();
    if let Some(code) = compile_child(std::env::args_os().skip(1)) {
        std::process::exit(code);
    }
}

// Keeps the empty `mod_points` in the SDK build (Cargo.toml says why).
use mod_points as _;

/// A point listener of generated native startup; none exists since #19576.
pub use mod_api::__private::native::SlotListener;

#[cfg(test)]
#[path = "../build_identity.rs"]
#[allow(dead_code)] // `resolve` runs `git`; only build.rs calls it, the tests stay off processes
mod build_identity;
