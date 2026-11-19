//! The controller settings through `Settings::load` and `Settings::save`, with their store
//! registered as the game's entry does. Its own test binary: the stores' list seals on the first
//! load, so the store has to be in it before any test of this process loads settings.
use std::fs;
use std::path::PathBuf;

use horizon_client_render::{gamepad, settings::Settings};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-pad-settings-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_players_controller_file_survives_load_and_save_and_stays_theirs() {
    horizon_hooks::settings::register(&gamepad::store::STORE).unwrap();
    let alex = scratch("alex");
    fs::write(
        alex.join(gamepad::FILE),
        "bindLayout:2\nbind_key.jump:east\nlookSensitivity:0.7\nlegacyViews:inventory=console\n",
    )
    .unwrap();
    let options = alex.join("options.txt");
    let mut settings = Settings::load(&options);
    assert_eq!(settings.controller.look_sensitivity, 0.7);
    assert_eq!(
        settings.controller.binding("key.jump"),
        Some(gamepad::Button::East)
    );
    settings.controller.invert_y = true;
    settings.save(&options);
    let written = fs::read_to_string(alex.join(gamepad::FILE)).unwrap();
    assert!(written.contains("invertY:true\n"), "{written}");
    assert!(
        written.contains("legacyViews:inventory=console\n"),
        "a line this build doesn't read stays: {written}"
    );
    assert_eq!(Settings::load(&options).controller, settings.controller);
    // The next profile in the same seat has no file: the defaults, nothing of Alex's.
    let sam = scratch("sam");
    let theirs = Settings::load(&sam.join("options.txt"));
    assert_eq!(theirs.controller, gamepad::PadSettings::default());
    theirs.save(&sam.join("options.txt"));
    assert!(!sam.join(gamepad::FILE).exists(), "defaults make no file");
    for dir in [alex, sam] {
        let _ = fs::remove_dir_all(dir);
    }
}
