//! Repository-selected identity. An upstream build retains its original namespaces.
macro_rules! identity {
    ($($name:ident: $upstream:literal => $fork:literal),* $(,)?) => {$ (
        pub const $name: &str = if cfg!(horizon_fork_launcher) { $fork } else { $upstream };
    )*};
}
identity! {
    VERSION_PREFIX: "horizon-launcher " => "horizon-fork-launcher ",
    DESKTOP_ENTRY: "horizon-launcher.desktop" => "io.github.arismoko.horizonfork.launcher.desktop",
    WINDOW_TITLE: "Horizon" => "Horizon Fork Launcher (arismoko)",
    QUIT_LABEL: "Quit Horizon Launcher" => "Quit Horizon Fork Launcher (arismoko)",
    DISPLAY_NAME: "Horizon Launcher" => "Horizon Fork Launcher (arismoko)",
    EXECUTABLE: "horizon-launcher" => "horizon-fork-launcher",
    WINDOWS_EXECUTABLE: "horizon-launcher.exe" => "horizon-fork-launcher.exe",
    WINDOWS_EXECUTABLE_PREFIX: "horizon-launcher.exe." => "horizon-fork-launcher.exe.",
    APP_ID: "horizon-launcher" => "io.github.arismoko.horizonfork.launcher",
    // The fork's launcher is an update of Horizon's own: it opens the same data, game and
    // location folders and the same saved accounts, so a player's instances, worlds and sign-in
    // carry over. Only how it is installed (above and below) keeps the fork's own names.
    KEYCHAIN_SERVICE: "dev.playhorizon.launcher" => "dev.playhorizon.launcher",
    LOCATION_FOLDER: "horizon-launcher-location" => "horizon-launcher-location",
    LAUNCHER_FOLDER: "horizon-launcher" => "horizon-launcher",
    GAME_FOLDER: "horizon" => "horizon",
    PUBLISHER: "Horizon" => "arismoko",
    UNINSTALL_KEY: "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\HorizonLauncher" => "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ArismokoHorizonForkLauncher",
    INSTALL_KEY: "Software\\Horizon\\Launcher" => "Software\\ArismokoHorizonFork\\Launcher",
    REGISTRY_PARENT: "Software\\Horizon" => "Software\\ArismokoHorizonFork",
    UNINSTALL_PREFIX: "horizon-launcher-uninstall-" => "horizon-fork-launcher-uninstall-",
    SETUP_PREFIX: "horizon-launcher-setup" => "horizon-fork-launcher-setup",
    PROCESS_FILTER: "IMAGENAME eq horizon-launcher.exe" => "IMAGENAME eq horizon-fork-launcher.exe",
}

/// Where fork builds before this one kept their data, game and location folders: opened when
/// Horizon's own folder is missing, so a player of an older fork build keeps their instances.
pub const LEGACY_LAUNCHER_FOLDER: Option<&str> = if cfg!(horizon_fork_launcher) {
    Some("horizon-fork-arismoko/launcher")
} else {
    None
};
pub const LEGACY_GAME_FOLDER: Option<&str> = if cfg!(horizon_fork_launcher) {
    Some("horizon-fork-arismoko/game")
} else {
    None
};
pub const LEGACY_LOCATION_FOLDER: Option<&str> = if cfg!(horizon_fork_launcher) {
    Some("horizon-fork-arismoko-location")
} else {
    None
};

/// The launcher's data folder under `base` ([`folder_in`]).
pub fn launcher_folder_in(base: &std::path::Path) -> std::path::PathBuf {
    folder_in(base, LAUNCHER_FOLDER, LEGACY_LAUNCHER_FOLDER)
}

/// The game's folder under `base` ([`folder_in`]).
pub fn game_folder_in(base: &std::path::Path) -> std::path::PathBuf {
    folder_in(base, GAME_FOLDER, LEGACY_GAME_FOLDER)
}

/// `base/folder`, or `base/legacy` when only that one exists (an older fork build's data).
pub fn folder_in(base: &std::path::Path, folder: &str, legacy: Option<&str>) -> std::path::PathBuf {
    let current = base.join(folder);
    match legacy.map(|old| base.join(old)) {
        Some(old) if !current.exists() && old.exists() => old,
        _ => current,
    }
}

/// Product home used by the Windows installer's About link.
pub const ABOUT_URL: &str = if cfg!(horizon_fork_launcher) {
    horizon_addresses::FORK_LAUNCHER_ABOUT
} else {
    horizon_addresses::LAUNCHER_ABOUT
};

/// The fork's builds use the Horizon launcher's own data folders (they update it), so nothing in
/// them is this launcher's alone: the uninstaller keeps them.
pub const SHARES_DATA: bool = cfg!(horizon_fork_launcher);

/// Whether the uninstaller may remove `path` as this launcher's own data: the complete owned
/// suffix, not just a generic leaf selected by an override. A fork build owns only the fork's
/// older folder ([`LEGACY_LAUNCHER_FOLDER`]), never the folder it shares ([`SHARES_DATA`]).
/// Uninstallation keeps the existing named-folder guard for relocated data as well.
pub fn owns_launcher_data_dir(path: &std::path::Path) -> bool {
    let owned = if SHARES_DATA {
        LEGACY_LAUNCHER_FOLDER
    } else {
        Some(LAUNCHER_FOLDER)
    };
    owned.is_some_and(|owned| ends_with(path, owned))
}

/// Whether `path` ends with every component of `suffix`, case aside.
fn ends_with(path: &std::path::Path, suffix: &str) -> bool {
    let mut actual = path.components().rev();
    std::path::Path::new(suffix)
        .components()
        .rev()
        .all(|expected| {
            actual.next().is_some_and(|component| {
                component
                    .as_os_str()
                    .eq_ignore_ascii_case(expected.as_os_str())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn horizon_folders_win_and_an_older_fork_folder_is_kept_when_alone() {
        let base = std::env::temp_dir().join(format!("hz-folder-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let old = "old-fork/launcher";
        // Nothing yet: Horizon's folder.
        assert_eq!(
            folder_in(&base, "horizon-launcher", Some(old)),
            base.join("horizon-launcher")
        );
        // Only the older fork's folder: it is kept.
        std::fs::create_dir_all(base.join(old)).unwrap();
        assert_eq!(
            folder_in(&base, "horizon-launcher", Some(old)),
            base.join(old)
        );
        // Both: Horizon's.
        std::fs::create_dir_all(base.join("horizon-launcher")).unwrap();
        assert_eq!(
            folder_in(&base, "horizon-launcher", Some(old)),
            base.join("horizon-launcher")
        );
        assert_eq!(
            folder_in(&base, "horizon-launcher", None),
            base.join("horizon-launcher")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn data_removal_requires_the_complete_owned_namespace() {
        let profile = Path::new("profile");
        // The folder shared with the Horizon launcher is never the fork's to remove; the
        // Horizon launcher's own build owns it.
        let shared = profile.join(LAUNCHER_FOLDER);
        assert_eq!(owns_launcher_data_dir(&shared), !SHARES_DATA);
        assert_eq!(
            owns_launcher_data_dir(&profile.join(LAUNCHER_FOLDER.to_ascii_uppercase())),
            !SHARES_DATA
        );
        // The fork's own older folder is the fork build's alone, and never the Horizon one's.
        let fork = profile.join("horizon-fork-arismoko/launcher");
        assert_eq!(owns_launcher_data_dir(&fork), SHARES_DATA);
        for unowned in [
            "launcher",
            "unrelated/launcher",
            "not-horizon-fork-arismoko/launcher",
            "horizon-launcher-backup",
        ] {
            assert!(!owns_launcher_data_dir(&profile.join(unowned)), "{unowned}");
        }
    }
}
