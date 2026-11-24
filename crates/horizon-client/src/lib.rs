//! Gameplay and startup, above the reusable client library.
// No `#![recursion_limit]` or other inner attribute horizon-mixins doesn't know here: the
// launcher's horizon-mixins then rejects every native mod targeting this crate (#8169).
#![forbid(unsafe_code)]
extern crate self as horizon_client;
pub use horizon_client_lib::carrier_store;
pub use horizon_client_lib::*;
#[doc(hidden)]
pub mod entry;

// The game's allocator lives in the library, not the executable: the prebuilt game (arismoko/horizon#589) is this crate as a
// dylib, and the allocator must be the one its code and the mods' executable share. The Windows and macOS archives
// build with `system-allocator`: their loaders bind each library to std's own allocator, which everything then
// shares (arismoko/horizon#2394, arismoko/horizon#3877).
#[cfg(not(feature = "system-allocator"))]
#[global_allocator]
static ALLOCATOR: horizon_threads::GameAllocator = horizon_threads::GAME_ALLOCATOR;
pub mod game;
pub mod orb_limits;
pub mod server_impulse;

#[cfg(test)]
mod test_cwd_isolation {
    #[test]
    fn test_binary_starts_in_isolated_game_dir() {
        horizon_threads::test_game_dir::ensure_linked();
        let isolated = horizon_threads::test_game_dir::isolated_dir()
            .expect("test binary should start in a per-binary scratch game directory");
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_ne!(
            std::fs::canonicalize(isolated).unwrap_or_else(|_| isolated.to_path_buf()),
            std::fs::canonicalize(manifest).unwrap_or_else(|_| manifest.to_path_buf()),
        );
    }
}
