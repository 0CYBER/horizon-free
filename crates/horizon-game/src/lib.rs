//! The game as the platform runs it, for an executable to start (docs/layers.md, "How a Horizon
//! crate plugs in"). The official executable (`horizon-official`, Horizon layer) registers its
//! hooks (`horizon_platform::hooks`) and then calls [`main`]; it never names an engine crate.
//!
//! An executable built on it:
//!
//! ```ignore
//! static GAME_IDENTITY: [u8; horizon_game::GAME_IDENTITY.len()] = horizon_game::GAME_IDENTITY;
//!
//! fn main() {
//!     horizon_game::run_main(|| {
//!         std::hint::black_box(&GAME_IDENTITY);
//!         my_crate::register();
//!         horizon_game::main();
//!     });
//! }
//! ```
//!
//! The platform's own parts register here, in [`main`], as they leave the engine (layer plan M6).
#![forbid(unsafe_code)]

mod look;

/// Identifies an executable as the game for launcher imports, without an API version or build id.
pub const GAME_IDENTITY: [u8; horizon_mod_package::game_identity::MARK.len()] =
    horizon_mod_package::game_identity::MARK;

/// Gives the windows an icon (a PNG). Call it before [`main`]; without it the windows keep the
/// system's default icon. The icon is branding, so the executable gives it, not the platform.
pub fn set_icon_png(png: &'static [u8]) {
    horizon_client::window::set_icon_png(png);
}

/// Gives the menus the platform's look (tokens and fonts of `horizon-theme`). [`main`] calls it;
/// an executable that starts the game another way (the launcher's native-mod build) calls it
/// before. A second call keeps the first look.
pub fn give_menu_look() {
    look::give_menus();
}

/// Runs the game with the hooks registered so far, and returns when it ends. Register hooks
/// (`horizon_platform::hooks::register`) before calling it. The menus get the platform's look
/// first ([`give_menu_look`]); the engine alone draws vanilla's neutral one.
pub fn main() {
    give_menu_look();
    horizon_guest_stack::prepare_main();
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    std::hint::black_box(&horizon_mod_package::holder::CAPABILITY);
    horizon_mod_host::main_with(Vec::new(), &[], &[], startup);
}

/// Establish the executable main thread stack before initialization.
pub use horizon_guest_stack::prepare_main as prepare_main_stack;
/// Run an executable's whole `main` on a main thread stack the guest fits on.
pub use horizon_guest_stack::run_main;
