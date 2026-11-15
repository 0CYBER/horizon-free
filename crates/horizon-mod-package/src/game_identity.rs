//! The fixed signature identifying a Horizon game executable to the launcher.
//! This is neither a mod API version nor a per-build id, and proves no authenticity.

/// Embedded by both the ordinary game and launcher-generated native game executables.
pub const MARK: [u8; 24] = *b"horizon-game-executable;";
