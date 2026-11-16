//! Immutable physical-process information shared by native and wasm mods.

/// The physical process, independent of a hook's logical side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum Dist {
    /// A client process, including its integrated server.
    #[default]
    Client,
    /// A dedicated server process.
    DedicatedServer,
}

/// Process information captured by the engine before registration.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: fields are private; only the engine constructor supplies a snapshot.
pub struct RuntimeEnvironment {
    dist: Dist,
    game_version: std::sync::Arc<str>,
    horizon_version: std::sync::Arc<str>,
    game_tests_enabled: bool,
}

impl RuntimeEnvironment {
    /// Capture the engine's fixed startup values.
    pub fn new(
        dist: Dist,
        game_version: String,
        horizon_version: String,
        game_tests_enabled: bool,
    ) -> Self {
        Self {
            dist,
            game_version: game_version.into(),
            horizon_version: horizon_version.into(),
            game_tests_enabled,
        }
    }
    /// The physical process; an integrated server remains a client.
    pub fn dist(&self) -> Dist {
        self.dist
    }
    /// The exact supported Minecraft version.
    pub fn game_version(&self) -> &str {
        &self.game_version
    }
    /// The version printed in Horizon crash reports.
    pub fn horizon_version(&self) -> &str {
        &self.horizon_version
    }
    /// Whether test-only content was enabled at process startup.
    pub fn game_tests_enabled(&self) -> bool {
        self.game_tests_enabled
    }
}
