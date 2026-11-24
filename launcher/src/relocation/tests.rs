use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "horizon-relocation-paths-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn roots(&self, name: &str) -> Roots {
        Roots {
            launcher: self.0.join(name).join("launcher"),
            game: self.0.join(name).join("game"),
            build: self.0.join(name).join("build"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn settings_servers_and_java_homes_move_only_under_managed_roots() {
    let fixture = Fixture::new();
    let source = fixture.roots("source");
    let stage = fixture.roots("staging");
    let target = fixture.roots("destination");
    fs::create_dir_all(&stage.launcher).unwrap();
    let external = fixture.0.join("external/minecraft");
    // A sibling with the same text prefix is not inside the game root.
    let sibling = source.game.with_file_name("game-other").join("server");
    let command = format!("run --home {}", source.game.display());
    let settings = json!({
        "minecraft_dir": source.launcher.join("minecraft"),
        "server_dir": source.game.join("servers/main"),
        "background": external,
        "game_args": command,
        "unknown": {"path": source.game.join("leave-alone")}
    });
    let servers = json!([
        {"dir": source.game.join("servers/main"), "name": source.game},
        {"dir": external},
        {"dir": sibling},
        {"dir": "relative/server"}
    ]);
    let instances = json!({"instances": {
        "managed": {"java": {"home": source.build.join("jdk")},
            "launch": {"env": {"CUSTOM_HOME": source.game}, "wrapper": command}},
        "external": {"java": {"home": external}},
        "automatic": {"java": {"home": null}},
        "future": {"unknown": source.launcher}
    }});
    for (name, value) in [
        ("settings.json", &settings),
        ("servers.json", &servers),
        ("instances.json", &instances),
    ] {
        write_json(&stage.launcher.join(name), value);
    }

    prepare_paths(&stage, &source, &target).unwrap();

    let mut expected_settings = settings;
    expected_settings["minecraft_dir"] = json!(target.launcher.join("minecraft"));
    expected_settings["server_dir"] = json!(target.game.join("servers").join("main"));
    assert_eq!(
        read_json(&stage.launcher.join("settings.json")),
        expected_settings
    );
    let mut expected_servers = servers;
    expected_servers[0]["dir"] = json!(target.game.join("servers").join("main"));
    assert_eq!(
        read_json(&stage.launcher.join("servers.json")),
        expected_servers
    );
    let mut expected_instances = instances;
    expected_instances["instances"]["managed"]["java"]["home"] = json!(target.build.join("jdk"));
    assert_eq!(
        read_json(&stage.launcher.join("instances.json")),
        expected_instances
    );
}

#[test]
fn nested_build_path_uses_the_most_specific_root() {
    let fixture = Fixture::new();
    let mut source = fixture.roots("source");
    source.build = source.launcher.join("native");
    let target = fixture.roots("destination");
    let stage = fixture.roots("staging");
    fs::create_dir_all(&stage.launcher).unwrap();
    write_json(
        &stage.launcher.join("settings.json"),
        &json!({
            "minecraft_dir": source.build.join("jdk"),
            "server_dir": source.game,
            "background": source.launcher.join("background.png")
        }),
    );

    prepare_paths(&stage, &source, &target).unwrap();

    let mut actual = read_json(&stage.launcher.join("settings.json"));
    let server_dir = actual
        .as_object_mut()
        .unwrap()
        .remove("server_dir")
        .unwrap();
    assert_eq!(Path::new(server_dir.as_str().unwrap()), target.game);
    assert_eq!(
        actual,
        json!({
            "minecraft_dir": target.build.join("jdk"),
            "background": target.launcher.join("background.png")
        })
    );
}

#[test]
fn missing_optional_files_are_allowed_but_invalid_json_is_not_replaced() {
    let fixture = Fixture::new();
    let source = fixture.roots("source");
    let stage = fixture.roots("staging");
    let target = fixture.roots("destination");
    fs::create_dir_all(&stage.launcher).unwrap();
    prepare_paths(&stage, &source, &target).unwrap();
    let path = stage.launcher.join("settings.json");
    fs::write(&path, b"{invalid settings").unwrap();
    assert!(prepare_paths(&stage, &source, &target).is_err());
    assert_eq!(fs::read(path).unwrap(), b"{invalid settings");
}

#[cfg(windows)]
#[test]
fn windows_managed_paths_are_matched_without_case_sensitive_prefixes() {
    let fixture = Fixture::new();
    let source = fixture.roots("source");
    let stage = fixture.roots("staging");
    let target = fixture.roots("destination");
    fs::create_dir_all(&stage.launcher).unwrap();
    let differently_cased = source.game.to_string_lossy().to_uppercase();
    write_json(
        &stage.launcher.join("servers.json"),
        &json!([
            {"dir": PathBuf::from(differently_cased).join("world")}
        ]),
    );
    prepare_paths(&stage, &source, &target).unwrap();
    assert_eq!(
        read_json(&stage.launcher.join("servers.json")),
        json!([
            {"dir": target.game.join("world")}
        ])
    );
}

/// #16685: one running process with non-ASCII text in its command line made the process list unreadable, and every
/// move stopped. The text holds a character of each common console code page (Western, Cyrillic, Chinese), so the old
/// list isn't UTF-8 on any of them.
#[cfg(windows)]
#[test]
fn a_non_ascii_command_line_does_not_stop_the_process_check() {
    let mut other = process::command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep 60 # é тест 微信",
        ])
        .spawn()
        .unwrap();
    let fixture = Fixture::new();
    let result = ensure_idle(&fixture.roots("source"));
    let _ = other.kill();
    // A game or build running on this machine is a fair answer; a list it couldn't read is not.
    if let Err(error) = result {
        assert!(error.starts_with("Close games"), "{error}");
    }
}

#[test]
fn progress_follows_a_move_part_by_part() {
    let fixture = Fixture::new();
    let source = fixture.roots("source");
    for (root, name) in [
        (&source.launcher, "settings.json"),
        (&source.game, "options.txt"),
        (&source.build, "compiled.bin"),
    ] {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join(name), b"{}").unwrap();
    }
    let bootstrap = fixture.0.join("bootstrap/location.json");
    fs::create_dir_all(bootstrap.parent().unwrap()).unwrap();
    let mut progress = Progress::default();
    progress.apply(Update::Step(Step::Waiting));
    let mut steps = vec![Step::Waiting];
    data_location::migrate(
        &source,
        &fixture.0.join("destination"),
        &bootstrap,
        |message| {
            if let Some(update) = Update::from_progress(message) {
                progress.apply(update);
            }
            let step = progress.step.unwrap();
            if steps.last() != Some(&step) {
                steps.push(step);
            }
        },
    )
    .unwrap();
    assert_eq!(
        steps,
        [
            Step::Waiting,
            Step::Launcher,
            Step::Game,
            Step::Build,
            Step::Committing
        ]
    );
    assert_eq!(progress.files, 3);
}

#[test]
fn other_messages_are_no_update() {
    assert_eq!(
        Update::from_progress("Data location changed while waiting"),
        None
    );
}

/// The whole job, failure report included, ends in a code without a window.
#[test]
fn a_worker_without_a_target_fails_with_a_code() {
    let worker = Worker {
        target: None,
        restart: false,
    };
    let mut updates = Vec::new();
    assert_eq!(
        worker.run_and_finish(|update| updates.push(update), &AtomicBool::new(false)),
        1
    );
    assert!(updates.is_empty());
}
