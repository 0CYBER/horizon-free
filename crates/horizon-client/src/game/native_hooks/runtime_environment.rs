//! Metadata reads use the bootstrap snapshot, never the world's logical side.
use super::Game;

impl Game<'_> {
    pub(in crate::game) fn dist(&self) -> mod_api::Dist {
        crate::entry::runtime_environment::get().dist()
    }
    pub(in crate::game) fn game_version(&self) -> &'static str {
        crate::entry::runtime_environment::get().game_version()
    }
    pub(in crate::game) fn server_data_version(&self) -> i32 {
        horizon_core::version::DATA_VERSION
    }
    pub(in crate::game) fn horizon_version(&self) -> &'static str {
        crate::entry::runtime_environment::get().horizon_version()
    }
    pub(in crate::game) fn game_tests_enabled(&self) -> bool {
        crate::entry::runtime_environment::get().game_tests_enabled()
    }
}

#[cfg(test)]
#[path = "runtime_environment/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_environment/server_data_version_tests.rs"]
mod server_data_version_tests;
