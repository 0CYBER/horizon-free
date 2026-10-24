//! The game's app bundle on macOS. Started as a bare executable, the game shows in the Dock and
//! in Cmd+Tab as "horizon-client" with the generic executable icon (winit's window icon does
//! nothing there). So the launcher starts it from a small Horizon.app inside its version folder:
//! the Info.plist names it and the icon is the launcher's own. The programs inside are hard links
//! to the version's, so the bundle takes no disk space and the release's signature is untouched.
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The bundle's folder in a version folder.
pub const APP: &str = "Horizon.app";

/// The game, then horizon-ctl, which the game looks for beside itself (Copy Connection Details).
const PROGRAMS: [&str; 2] = ["horizon-client", "horizon-ctl"];

const ICON: &[u8] = include_bytes!("../../packaging/icons/horizon.icns");

/// Its own identifier, so macOS keeps it apart from the launcher (dev.horizon.launcher); the
/// games category lets macOS turn Game Mode on when it plays full screen.
const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDisplayName</key>
	<string>Horizon</string>
	<key>CFBundleExecutable</key>
	<string>horizon-client</string>
	<key>CFBundleIconFile</key>
	<string>horizon</string>
	<key>CFBundleIdentifier</key>
	<string>dev.horizon.game</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Horizon</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.games</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
"#;

/// Brings `version`'s Horizon.app up to date and returns the game's program inside it. A program
/// replaced in the version folder (a reinstall, a build copied over) is linked again.
pub fn ensure(version: &Path) -> io::Result<PathBuf> {
    let contents = version.join(APP).join("Contents");
    let programs = contents.join("MacOS");
    fs::create_dir_all(&programs)?;
    fs::create_dir_all(contents.join("Resources"))?;
    write_if_changed(&contents.join("Info.plist"), INFO_PLIST.as_bytes())?;
    write_if_changed(&contents.join("Resources/horizon.icns"), ICON)?;
    for (i, name) in PROGRAMS.into_iter().enumerate() {
        let (from, to) = (version.join(name), programs.join(name));
        // Releases before horizon-ctl shipped have only the game.
        if i > 0 && !from.exists() {
            continue;
        }
        if !same_file(&from, &to) {
            let _ = fs::remove_file(&to);
            fs::hard_link(&from, &to)?;
        }
    }
    Ok(programs.join(PROGRAMS[0]))
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).ok().as_deref() != Some(bytes) {
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_links_the_version_programs_and_follows_a_replaced_one() {
        let version = std::env::temp_dir().join(format!("horizon-game-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&version);
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("horizon-client"), "game 1").unwrap();
        fs::write(version.join("horizon-ctl"), "ctl").unwrap();

        let program = ensure(&version).unwrap();
        assert_eq!(
            program,
            version.join("Horizon.app/Contents/MacOS/horizon-client")
        );
        assert!(same_file(&program, &version.join("horizon-client")));
        assert!(same_file(
            &version.join("Horizon.app/Contents/MacOS/horizon-ctl"),
            &version.join("horizon-ctl")
        ));
        let plist = fs::read_to_string(version.join("Horizon.app/Contents/Info.plist")).unwrap();
        assert!(plist.contains("<string>Horizon</string>"));
        assert_eq!(
            fs::read(version.join("Horizon.app/Contents/Resources/horizon.icns")).unwrap(),
            ICON
        );

        // A new build copied over the version's game: the bundle runs it, not the old one.
        fs::remove_file(version.join("horizon-client")).unwrap();
        fs::write(version.join("horizon-client"), "game 2").unwrap();
        assert_eq!(
            fs::read_to_string(ensure(&version).unwrap()).unwrap(),
            "game 2"
        );
        fs::remove_dir_all(&version).unwrap();
    }

    #[test]
    fn a_version_without_horizon_ctl_still_gets_its_bundle() {
        let version =
            std::env::temp_dir().join(format!("horizon-game-app-old-{}", std::process::id()));
        let _ = fs::remove_dir_all(&version);
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("horizon-client"), "game").unwrap();
        assert!(ensure(&version).unwrap().exists());
        assert!(!version
            .join("Horizon.app/Contents/MacOS/horizon-ctl")
            .exists());
        fs::remove_dir_all(&version).unwrap();
    }

    #[test]
    fn a_version_without_the_game_has_no_bundle_program() {
        let version =
            std::env::temp_dir().join(format!("horizon-game-app-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&version);
        fs::create_dir_all(&version).unwrap();
        assert!(ensure(&version).is_err());
        fs::remove_dir_all(&version).unwrap();
    }
}
