//! Set-once platform services used by the engine during load and profiler commands.
use mod_vocab::{
    __private::{RegistrationData, RuleSchema},
    DimensionSettings, ModEntry, PackageInfo,
};
use std::sync::OnceLock;

/// Platform callbacks. All boundary values belong to the engine vocabulary.
pub struct Platform {
    pub install_packages: fn(&[PackageInfo], &[PackageInfo]),
    pub install_registration_data: fn(RegistrationData),
    pub install_parts:
        fn(&[ModEntry], &RuleSchema<'_>, fn(&DimensionSettings) -> bool) -> Result<(), Vec<String>>,
    pub profiler_is_enabled: fn() -> bool,
    pub profiler_start: fn() -> bool,
    pub profiler_stop: fn(),
    pub profiler_summary: fn() -> String,
    pub profiler_firefox_profile: fn() -> Result<Vec<u8>, String>,
}
/// A platform socket access or installation refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformRefusal {
    NotInstalled,
    AlreadyInstalled,
}
impl std::fmt::Display for PlatformRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotInstalled => "engine platform socket is not installed",
            Self::AlreadyInstalled => "engine platform socket is already installed",
        })
    }
}
impl std::error::Error for PlatformRefusal {}
static PLATFORM: OnceLock<Platform> = OnceLock::new();
/// Install the process provider once, before engine startup.
pub fn install_platform(provider: Platform) -> Result<(), PlatformRefusal> {
    install(&PLATFORM, provider)
}
fn install(socket: &OnceLock<Platform>, provider: Platform) -> Result<(), PlatformRefusal> {
    socket
        .set(provider)
        .map_err(|_| PlatformRefusal::AlreadyInstalled)
}
/// Obtain the provider, refusing calls before host startup without panicking.
pub fn platform() -> Result<&'static Platform, PlatformRefusal> {
    get(&PLATFORM)
}
fn get(socket: &OnceLock<Platform>) -> Result<&Platform, PlatformRefusal> {
    socket.get().ok_or(PlatformRefusal::NotInstalled)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn provider() -> Platform {
        Platform {
            install_packages: |_, _| {},
            install_registration_data: |_| {},
            install_parts: |_, _, _| Ok(()),
            profiler_is_enabled: || false,
            profiler_start: || true,
            profiler_stop: || {},
            profiler_summary: String::new,
            profiler_firefox_profile: || Ok(Vec::new()),
        }
    }
    #[test]
    fn second_fill_refuses_by_name() {
        let socket = OnceLock::new();
        assert_eq!(install(&socket, provider()), Ok(()));
        let error = install(&socket, provider()).unwrap_err();
        assert_eq!(error, PlatformRefusal::AlreadyInstalled);
        assert_eq!(
            error.to_string(),
            "engine platform socket is already installed"
        );
    }
    #[test]
    fn call_before_fill_is_refused() {
        assert!(matches!(
            get(&OnceLock::new()),
            Err(PlatformRefusal::NotInstalled)
        ));
    }
}
