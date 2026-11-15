/// The release assets of each system Horizon ships for: (OS, architecture, the game, the launcher).
/// game-build.yml builds the game's, nightly-build.yml signs and publishes them, and launcher.yml
/// builds the launcher's (`platform_assets_are_published`).
pub const PLATFORMS: [(&str, &str, &str, &str); 5] = [
    (
        "linux",
        "x86_64",
        "horizon-linux-x86_64.tar.gz",
        "horizon-launcher-linux-x86_64.tar.gz",
    ),
    (
        "macos",
        "aarch64",
        "horizon-macos-arm64.zip",
        "horizon-launcher-macos-arm64.zip",
    ),
    // Intel Macs (arismoko/horizon#1161).
    (
        "macos",
        "x86_64",
        "horizon-macos-x64.zip",
        "horizon-launcher-macos-x64.zip",
    ),
    (
        "windows",
        "x86_64",
        "horizon-windows-x64.zip",
        "horizon-launcher-windows-x64.zip",
    ),
    (
        "windows",
        "aarch64",
        "horizon-windows-arm64.zip",
        "horizon-launcher-windows-arm64.zip",
    ),
];
