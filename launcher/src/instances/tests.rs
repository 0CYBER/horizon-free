use super::*;
use std::collections::BTreeMap;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-launcher-instances-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Every file under `dir`, relative, with its bytes.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().into(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A game home as a player has it before instances: worlds, options, packs, a session, an
/// imported modpack and a registry modpack with their own worlds.
fn player_home(home: &Path) {
    put(&home.join("saves/World/level.dat"), "world");
    put(&home.join("saves/World/region/r.0.0.mca"), "region");
    put(&home.join("saves/Other/level.dat"), "other");
    put(&home.join("options.txt"), "fov:90\n");
    put(&home.join("horizon-player.txt"), "name:Player123\n");
    put(&home.join("resourcepacks/faithful.zip"), "pack");
    put(&home.join("logs/latest.log"), "log");
    put(&home.join("sessions/a.json"), "token");
    put(&home.join("telemetry/consent.json"), "{}");
    let pack = horizon_port_pack::import::Pack {
        name: "Small Pack".into(),
        ..Default::default()
    };
    put(
        &home.join("ports/small-pack/pack.json"),
        &serde_json::to_string(&pack).unwrap(),
    );
    put(
        &home.join("ports/small-pack/home/saves/Modded/level.dat"),
        "modded",
    );
    put(
        &home.join("horizon-packs/cozy/pack.json"),
        r#"{"id":"cozy","name":"Cozy","version":"1.0.0","sha256":"00","mods":[]}"#,
    );
    put(
        &home.join("horizon-packs/cozy/home/saves/Cozy/level.dat"),
        "cozy",
    );
}

#[test]
fn build_choices_start_balanced_and_are_remembered_per_instance() {
    use crate::native_mods::Profile;
    let root = scratch("native-build-choices");
    let game = root.join("game");
    let data = root.join("launcher");
    for id in ["first", "second"] {
        fs::create_dir_all(game.join("instances").join(id)).unwrap();
    }
    for instance in list_in(&game, &data) {
        assert_eq!(instance.native_profile, Profile::Optimized);
    }
    let choose = |id, profile| {
        edit_in(&game, &data, id, |meta| meta.native_profile = Some(profile)).unwrap()
    };
    choose("first", Profile::Optimized);
    choose("second", Profile::Optimized);
    let before = fs::read(data.join("instances.json")).unwrap();
    for unknown in [serde_json::json!("future"), serde_json::json!(42)] {
        let mut stored: serde_json::Value = serde_json::from_slice(&before).unwrap();
        stored["instances"]["first"]["native_profile"] = unknown;
        stored["instances"]["first"]["version"] = serde_json::json!("kept-version");
        fs::write(
            data.join("instances.json"),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();
        edit_in(&game, &data, "second", |meta| {
            meta.name = Some("Still here".into())
        })
        .unwrap();
        let loaded = list_in(&game, &data);
        assert_eq!(
            find_in(&loaded, "first").unwrap().native_profile,
            Profile::Optimized
        );
        assert_eq!(
            find_in(&loaded, "first").unwrap().version.as_deref(),
            Some("kept-version")
        );
        let neighbour = find_in(&loaded, "second").unwrap();
        assert_eq!(neighbour.native_profile, Profile::Optimized);
        assert_eq!(neighbour.name, "Still here");
    }
    fs::write(data.join("instances.json"), before).unwrap();
    choose("second", Profile::Optimized);
    let again = list_in(&game, &data);
    assert_eq!(
        find_in(&again, "first").unwrap().native_profile,
        Profile::Optimized
    );
    assert_eq!(
        find_in(&again, "second").unwrap().native_profile,
        Profile::Optimized
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_worlds_stay_where_they_are_and_every_folder_becomes_an_instance() {
    let root = scratch("migrate");
    let (home, data) = (root.join("horizon"), root.join("launcher"));
    player_home(&home);
    let before = files(&home);

    // A launcher with instances, on a player's existing folders: no instances.json yet.
    let all = list_in(&home, &data);
    let ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, [DEFAULT, "pack:cozy", "port:small-pack"]);
    assert_eq!(all[0].name, "Horizon");
    assert_eq!(all[0].home, home);
    assert_eq!(all[1].home, home.join("horizon-packs/cozy/home"));
    assert_eq!(all[2].name, "Small Pack");
    assert_eq!(all[2].home, home.join("ports/small-pack/home"));

    // Everything the player can do with instances, then not one byte of theirs has moved.
    let empty = create_in(&home, &data, "Survival", None, None).unwrap();
    let copy = create_in(&home, &data, "My copy", Some(&all[0]), None).unwrap();
    edit_in(&home, &data, DEFAULT, |m| m.name = Some("Main".into())).unwrap();
    edit_in(&home, &data, "Small Pack", |m| {
        m.icon = Some(Icon::Texture("diamond".into()));
        m.group = Some("Modded".into());
    })
    .unwrap();
    let after = files(&home);
    for (path, bytes) in &before {
        assert_eq!(after.get(path), Some(bytes), "{} changed", path.display());
    }
    assert!(!data.join("settings.json").exists());

    // The empty one has only the player's options and name; the copy has the worlds and
    // packs, never the sessions, logs, data collection or other instances.
    assert_eq!(empty.home, home.join("instances/survival"));
    let empty_files: Vec<PathBuf> = files(&empty.home).into_keys().collect();
    assert_eq!(
        empty_files,
        [PathBuf::from("horizon-player.txt"), "options.txt".into()]
    );
    let copied = files(&copy.home);
    for kept in [
        "saves/World/region/r.0.0.mca",
        "saves/Other/level.dat",
        "resourcepacks/faithful.zip",
    ] {
        assert_eq!(copied[Path::new(kept)], before[Path::new(kept)]);
    }
    assert!(copied.keys().all(|p| {
        let top = p.components().next().unwrap().as_os_str().to_str().unwrap();
        !NOT_COPIED.contains(&top)
    }));

    let all = list_in(&home, &data);
    assert_eq!(all[0].name, "Main");
    let port = find_in(&all, "port:small-pack").unwrap();
    assert_eq!(port.group.as_deref(), Some("Modded"));
    assert_eq!(port.home, home.join("ports/small-pack/home"));
    fs::remove_dir_all(&root).unwrap();
}

/// An imported pack is listed as an instance from its `pack.json` alone: neither its reports nor
/// its carried mods are read, however many files they hold (#18074). The Modpacks page reads them
/// whole, on a thread.
#[test]
fn imported_packs_are_listed_without_reading_their_mods() {
    let root = scratch("listed-ports");
    let (home, data) = (root.join("horizon"), root.join("launcher"));
    player_home(&home);
    let carry = home.join("ports/small-pack/mods/chipped/carry");
    put(
        &carry.join("horizon-mod.toml"),
        "id = \"chipped\"
",
    );
    for i in 0..50 {
        put(
            &carry.join(format!("assets/chipped/models/block/b{i}.json")),
            "{}",
        );
    }
    // Not a pack: no `pack.json`.
    put(&home.join("ports/half-imported/coverage.json"), "[]");
    let loads = || crate::modpacks::tests::LOADS.with(std::cell::Cell::get);

    let before = loads();
    let all = list_in(&home, &data);
    let ports: Vec<_> = all.iter().filter(|i| i.kind == Kind::Port).collect();
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].id, "port:small-pack");
    assert_eq!(ports[0].name, "Small Pack");
    assert_eq!(ports[0].home, home.join("ports/small-pack/home"));
    assert_eq!(loads(), before, "listing the instances read a pack whole");

    // The page's read does read it whole, and lists the same packs.
    let read = crate::modpacks::list_in(&home.join("ports"));
    assert!(loads() > before);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].slug, "small-pack");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn names_stay_unique_and_packs_are_not_copied() {
    let root = scratch("names");
    let (home, data) = (root.join("horizon"), root.join("launcher"));
    player_home(&home);
    create_in(&home, &data, "Survival", None, None).unwrap();
    assert_eq!(
        create_in(&home, &data, "survival", None, None).unwrap_err(),
        "There is already an instance called survival. Choose another name."
    );
    assert_eq!(
        name_taken_with_flag("survival"),
        "There is already an instance called survival. Choose another name. (`--as NAME`)"
    );
    assert!(create_in(&home, &data, "  ", None, None).is_err());
    // Another name with the same folder name gets the next free folder.
    let second = create_in(&home, &data, "Survival!", None, None).unwrap();
    assert_eq!(second.id, "survival-2");
    assert_eq!(second.name, "Survival!");
    assert!(edit_in(&home, &data, "survival-2", |m| m.name =
        Some("SURVIVAL".into()))
    .is_err());
    let port = find_in(&list_in(&home, &data), "small-pack").unwrap();
    assert!(create_in(&home, &data, "Pack copy", Some(&port), None).is_err());
    assert!(!home.join("instances/pack-copy").exists());
    // An empty group is no group.
    let grouped = edit_in(&home, &data, "Survival", |m| m.group = Some(" ".into())).unwrap();
    assert_eq!(grouped.group, None);
    // An import named like an instance: both can still change everything but their names.
    let mut store = Store::load(&data).unwrap();
    store.instances.entry(port.id.clone()).or_default().name = Some("Survival".into());
    store.save(&data).unwrap();
    let launched = edit_in(&home, &data, "survival", |m| {
        m.launch.wrapper = "nice".into()
    })
    .unwrap();
    assert_eq!(launched.launch.wrapper, "nice");
    assert!(edit_in(&home, &data, "survival-2", |m| m.name =
        Some("survival".into()))
    .is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_instance_follows_the_players_answers() {
    let root = scratch("follow");
    let (main, own) = (root.join("main"), root.join("own"));
    put(&main.join("options.txt"), "fov:90\n");
    put(&main.join("horizon-player.txt"), "name:Player123\n");
    put(&main.join("telemetry/consent.json"), r#"{"version":1}"#);
    horizon_platform::no_ai::set(&main, true).unwrap();
    put(&own.join("options.txt"), "fov:70\n");
    follow_player(&main, &own);
    assert_eq!(
        fs::read_to_string(own.join("options.txt")).unwrap(),
        "fov:70\n"
    );
    assert_eq!(
        fs::read_to_string(own.join("horizon-player.txt")).unwrap(),
        "name:Player123\n"
    );
    assert_eq!(
        fs::read_to_string(own.join("telemetry/consent.json")).unwrap(),
        r#"{"version":1}"#
    );
    assert!(horizon_platform::no_ai::is_on(&own));
    horizon_platform::no_ai::set(&main, false).unwrap();
    follow_player(&main, &own);
    assert!(!horizon_platform::no_ai::is_on(&own));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn icons_are_a_texture_name_or_a_colour() {
    assert_eq!(Icon::parse("#3FA7D6"), Ok(Icon::Colour("#3fa7d6".into())));
    assert_eq!(
        Icon::parse("grass_block"),
        Ok(Icon::Texture("grass_block".into()))
    );
    for bad in ["#12345", "../x", "Grass", "", "#zzzzzz"] {
        assert!(Icon::parse(bad).is_err(), "{bad}");
    }
    let root = scratch("icon");
    put(
        &root.join("assets/minecraft/textures/item/diamond.png"),
        "png",
    );
    let diamond = Icon::Texture("diamond".into()).texture(&root).unwrap();
    assert!(diamond.ends_with("item/diamond.png"));
    assert_eq!(Icon::Texture("stone".into()).texture(&root), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_instance_keeps_its_own_version_until_it_follows_updates_again() {
    let root = scratch("version");
    let (home, data) = (root.join("horizon"), root.join("launcher"));
    player_home(&home);
    create_in(&home, &data, "Old world", None, None).unwrap();
    let pinned = edit_in(&home, &data, "Old world", |m| {
        m.version = Some("v0.1.0".into())
    })
    .unwrap();
    assert_eq!(pinned.version.as_deref(), Some("v0.1.0"));
    // Renaming keeps it; the other instances stay on Latest.
    let renamed = edit_in(&home, &data, "old-world", |m| m.name = Some("Older".into())).unwrap();
    assert_eq!(renamed.version.as_deref(), Some("v0.1.0"));
    assert!(list_in(&home, &data)
        .iter()
        .filter(|i| i.id != "old-world")
        .all(|i| i.version.is_none()));
    let follows = edit_in(&home, &data, "Older", |m| m.version = None).unwrap();
    assert_eq!(follows.version, None);
    fs::remove_dir_all(&root).unwrap();
}

const NOV_21: &str = "nightly-2026-11-21-aaaaaaa";
const NOV_22: &str = "nightly-2026-11-22-bbbbbbb";

#[test]
fn use_version_pins_only_the_selected_horizon_instance_and_newest_clears_it() {
    let Some(root) = std::env::var_os("HORIZON_TEST_USE_VERSION") else {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "instances::tests::use_version_pins_only_the_selected_horizon_instance_and_newest_clears_it"])
            .env("HORIZON_TEST_USE_VERSION", root.path())
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env("HORIZON_LAUNCHER_LOCATION_FILE", root.path().join("location.json"))
            .status()
            .unwrap()
            .success());
        return;
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(root.join("game").join(OWN).join("selected")).unwrap();
    for tag in [NOV_21, NOV_22] {
        put(&backend::version_dir(tag).join(backend::COMPLETE), "");
    }
    let settings = backend::Settings {
        instance: Some("selected".into()),
        selected: Some(NOV_21.into()),
        ..backend::Settings::default()
    };
    settings.save().unwrap();
    let settings_file = backend::data_dir().join("settings.json");
    let before = fs::read(&settings_file).unwrap();

    assert_eq!(selected().resolve_version(None).unwrap(), NOV_22);
    assert!(backend::remove_version(NOV_22)
        .unwrap_err()
        .contains("Horizon"));
    assert!(backend::version_dir(NOV_22)
        .join(backend::COMPLETE)
        .is_file());
    use_version(NOV_21).unwrap();
    assert_eq!(selected().version.as_deref(), Some(NOV_21));
    assert_eq!(selected().resolve_version(None).unwrap(), NOV_21);
    assert!(backend::remove_version(NOV_21)
        .unwrap_err()
        .contains("selected"));
    fs::create_dir_all(root.join("game").join(OWN).join(nightly::INSTANCE)).unwrap();
    edit(nightly::INSTANCE, |meta| meta.version = Some(NOV_21.into())).unwrap();
    backend::mark_auto_installed(NOV_21).unwrap();
    nightly::after_update(NOV_22);
    assert_eq!(
        find(nightly::INSTANCE).unwrap().version.as_deref(),
        Some(NOV_21)
    );
    assert!(backend::installed().iter().any(|tag| tag == NOV_21));
    assert_eq!(find(DEFAULT).unwrap().version, None);
    use_version(NOV_22).unwrap();
    assert_eq!(selected().version, None);

    edit("selected", |meta| meta.java = Some(JavaGame::default())).unwrap();
    use_version(NOV_21).unwrap();
    assert_eq!(selected().version, None);

    // The window still shows Horizon while another launcher selected the Java instance.
    use_version_for(DEFAULT, NOV_21).unwrap();
    assert_eq!(find(DEFAULT).unwrap().version.as_deref(), Some(NOV_21));
    assert_eq!(selected().id, "selected");
    assert_eq!(selected().version, None);
    assert_eq!(fs::read(settings_file).unwrap(), before);
}

#[test]
fn migration_leaves_both_files_untouched_when_version_or_instance_data_cannot_be_read() {
    if std::env::var_os("HORIZON_TEST_MIGRATION_READ").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "instances::tests::migration_leaves_both_files_untouched_when_version_or_instance_data_cannot_be_read"])
            .env("HORIZON_TEST_MIGRATION_READ", root.path())
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env("HORIZON_LAUNCHER_LOCATION_FILE", root.path().join("location.json"))
            .status().unwrap().success());
        return;
    }
    let data = backend::data_dir();
    let settings = data.join("settings.json");
    let instances = data.join(FILE);
    let old_settings = format!(r#"{{"selected":"{NOV_21}","following":null}}"#);
    put(&settings, &old_settings);
    for tag in [NOV_21, NOV_22] {
        put(&backend::version_dir(tag).join(backend::COMPLETE), "");
    }

    put(&instances, r#"{"instances":{"horizon":"incomplete"}"#);
    let corrupt = fs::read(&instances).unwrap();
    assert!(absorb_held_selection().unwrap_err().contains(FILE));
    assert_eq!(fs::read(&instances).unwrap(), corrupt);
    assert_eq!(fs::read_to_string(&settings).unwrap(), old_settings);

    fs::remove_file(&instances).unwrap();
    fs::create_dir(&instances).unwrap();
    assert!(absorb_held_selection().unwrap_err().contains(FILE));
    assert!(instances.is_dir());
    assert_eq!(fs::read_to_string(&settings).unwrap(), old_settings);

    fs::remove_dir(&instances).unwrap();
    let saved = r#"{"instances":{"horizon":{"name":"Keep this name"}}}"#;
    put(&instances, saved);
    let versions = data.join("versions");
    fs::rename(&versions, data.join("versions-kept")).unwrap();
    put(&versions, "not a readable directory");
    assert!(absorb_held_selection().unwrap_err().contains("versions"));
    assert_eq!(fs::read_to_string(&instances).unwrap(), saved);
    assert_eq!(fs::read_to_string(&settings).unwrap(), old_settings);

    fs::remove_file(&versions).unwrap();
    fs::rename(data.join("versions-kept"), &versions).unwrap();
    let game = backend::game_home();
    // Joined part by part: the errors name the path with the platform's separators.
    let port = game.join("ports").join("family-pack").join("pack.json");
    let registry = game.join("horizon-packs").join("quiet").join("pack.json");
    put(&port, "broken Java pack");
    assert!(absorb_held_selection()
        .unwrap_err()
        .contains(&port.display().to_string()));
    assert_eq!(fs::read_to_string(&instances).unwrap(), saved);
    assert_eq!(fs::read_to_string(&settings).unwrap(), old_settings);

    let manifest = horizon_port_pack::import::Pack {
        name: "Family pack".into(),
        ..Default::default()
    };
    put(&port, &serde_json::to_string(&manifest).unwrap());
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    assert!(absorb_held_selection()
        .unwrap_err()
        .contains(&registry.display().to_string()));
    assert_eq!(fs::read_to_string(&instances).unwrap(), saved);
    assert_eq!(fs::read_to_string(&settings).unwrap(), old_settings);

    put(
        &registry,
        r#"{"id":"quiet","name":"Quiet","version":"1.0","sha256":"00","mods":[]}"#,
    );
    absorb_held_selection().unwrap();
    assert!(backend::Settings::load().instance_versions_migrated);
    for id in ["port:family-pack", "pack:quiet"] {
        assert_eq!(find(id).unwrap().version.as_deref(), Some(NOV_21));
    }
}

#[test]
fn migration_preserves_held_builds_and_is_safe_to_retry_before_settings_save() {
    let root = tempfile::tempdir().unwrap();
    let (game, data) = (root.path().join("game"), root.path().join("launcher"));
    for id in ["latest", "pinned", "java", "stable"] {
        fs::create_dir_all(game.join(OWN).join(id)).unwrap();
    }
    edit_in(&game, &data, "pinned", |meta| {
        meta.version = Some("v0.2.97".into())
    })
    .unwrap();
    edit_in(&game, &data, "java", |meta| {
        meta.java = Some(JavaGame::default())
    })
    .unwrap();
    edit_in(&game, &data, "stable", |meta| {
        meta.channel = Some(Channel::Stable)
    })
    .unwrap();

    let installed = [NOV_22, NOV_21, "v0.2.97"].map(String::from);
    let legacy = format!(r#"{{"selected":"{NOV_21}","following":null}}"#);
    let mut settings = backend::Settings::parse(&legacy);
    absorb_held_selection_in(&game, &data, &installed, &mut settings).unwrap();
    let instance = |id| find_in(&list_in(&game, &data), id).unwrap();
    for id in [DEFAULT, "latest", "stable"] {
        assert_eq!(instance(id).version.as_deref(), Some(NOV_21));
    }
    assert_eq!(instance("pinned").version.as_deref(), Some("v0.2.97"));
    assert_eq!(instance("java").version, None);
    let migrated = serde_json::to_string(&settings).unwrap();
    assert_eq!(settings.selected.as_deref(), Some(NOV_22));
    assert_eq!(settings.following.as_deref(), Some(NOV_22));

    let pins = fs::read(data.join(FILE)).unwrap();
    // An interruption after writing pins, with the old settings still on disk.
    let mut retried = backend::Settings::parse(&legacy);
    absorb_held_selection_in(&game, &data, &installed, &mut retried).unwrap();
    assert_eq!(serde_json::to_string(&retried).unwrap(), migrated);

    let mut saved = backend::Settings::parse(&migrated);
    absorb_held_selection_in(&game, &data, &installed, &mut saved).unwrap();
    assert_eq!(serde_json::to_string(&saved).unwrap(), migrated);
    assert_eq!(fs::read(data.join(FILE)).unwrap(), pins);
}

#[test]
fn migration_keeps_a_legacy_follower_on_latest_as_new_builds_arrive() {
    let root = tempfile::tempdir().unwrap();
    let (game, data) = (root.path().join("game"), root.path().join("launcher"));
    let installed = [NOV_21.to_owned()];
    let mut settings = backend::Settings::parse(&format!(r#"{{"selected":"{NOV_21}"}}"#));
    absorb_held_selection_in(&game, &data, &installed, &mut settings).unwrap();
    assert_eq!(settings.following.as_deref(), Some(NOV_21));
    assert!(!data.join(FILE).exists());

    let installed = [NOV_22, NOV_21].map(String::from);
    absorb_held_selection_in(&game, &data, &installed, &mut settings).unwrap();
    assert!(!data.join(FILE).exists());
    let default = &list_in(&game, &data)[0];
    assert_eq!(
        default
            .installed_version(&installed, settings.selected.as_deref())
            .as_deref(),
        Some(NOV_22)
    );
}

#[test]
fn a_manual_install_for_java_does_not_create_an_instance_identity() {
    if std::env::var_os("HORIZON_TEST_JAVA_INSTALL").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "instances::tests::a_manual_install_for_java_does_not_create_an_instance_identity"
            ])
            .env("HORIZON_TEST_JAVA_INSTALL", "1")
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env(
                "HORIZON_LAUNCHER_LOCATION_FILE",
                root.path().join("location.json")
            )
            .status()
            .unwrap()
            .success());
        return;
    }
    let java = create_java("Java", "26.3").unwrap();
    let target = manual_install_target(java.id).unwrap();
    assert!(target.identity.is_none());
    assert!(!java.work.join(IDENTITY).exists());
}

#[test]
fn pack_play_uses_its_pin_and_refuses_an_unreadable_manifest() {
    if std::env::var_os("HORIZON_TEST_PACK_PLAY").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "instances::tests::pack_play_uses_its_pin_and_refuses_an_unreadable_manifest"
            ])
            .env("HORIZON_TEST_PACK_PLAY", "1")
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env(
                "HORIZON_LAUNCHER_LOCATION_FILE",
                root.path().join("location.json")
            )
            .status()
            .unwrap()
            .success());
        return;
    }
    let game = backend::game_home();
    let port = game.join("ports/family");
    let registry = game.join("horizon-packs/quiet");
    let java_pack = horizon_port_pack::import::Pack {
        name: "Family".into(),
        ..Default::default()
    };
    let manifests = [
        (
            &port,
            "port:family",
            serde_json::to_string(&java_pack).unwrap(),
        ),
        (
            &registry,
            "pack:quiet",
            r#"{"id":"quiet","name":"Quiet","version":"1.0","sha256":"00","mods":[]}"#.into(),
        ),
    ];
    for tag in [NOV_21, NOV_22] {
        put(&backend::version_dir(tag).join(backend::COMPLETE), "");
    }
    for (work, id, manifest) in &manifests {
        put(&work.join("pack.json"), manifest);
        use_version_for(id, NOV_21).unwrap();
    }
    assert_eq!(
        for_launch_id(DEFAULT)
            .unwrap()
            .resolve_version(None)
            .unwrap(),
        NOV_22
    );
    for (work, _, text) in manifests {
        assert_eq!(
            pack_for_launch(work)
                .unwrap()
                .resolve_version(None)
                .unwrap(),
            NOV_21
        );
        let manifest = work.join("pack.json");
        put(&manifest, "{broken");
        assert!(pack_for_launch(work)
            .unwrap_err()
            .contains(&manifest.display().to_string()));
        let mut cmd = Command::new("fixture");
        assert!(modpacks::apply_profile(&mut cmd, work)
            .unwrap_err()
            .contains(&manifest.display().to_string()));
        assert!(cmd.get_current_dir().is_none());
        fs::remove_file(&manifest).unwrap();
        assert!(pack_for_launch(work)
            .unwrap_err()
            .contains(&manifest.display().to_string()));
        put(&manifest, &text);
    }
}

#[test]
fn creating_with_a_stale_pin_does_not_publish_an_instance_or_copy_its_worlds() {
    if std::env::var_os("HORIZON_TEST_CREATE_PIN").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "instances::tests::creating_with_a_stale_pin_does_not_publish_an_instance_or_copy_its_worlds"])
            .env("HORIZON_TEST_CREATE_PIN", "1")
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env("HORIZON_LAUNCHER_LOCATION_FILE", root.path().join("location.json"))
            .status().unwrap().success());
        return;
    }
    let game = backend::game_home();
    let world = Path::new("saves/Family/level.dat");
    put(&game.join(world), "existing world");
    let older = backend::version_dir(NOV_21);
    put(&older.join(backend::COMPLETE), "");
    put(&older.join(backend::CLIENT), "pre-holder game");
    put(&backend::version_dir(NOV_22).join(backend::COMPLETE), "");
    let source = find(DEFAULT).unwrap();
    assert!(backend::installed().contains(&NOV_21.into()));
    backend::remove_version(NOV_21).unwrap();

    let create = |downloading| {
        create_with_installation(
            "Family copy",
            Some(&source.id),
            Some(NOV_21.into()),
            Channel::Nightly,
            downloading,
        )
    };
    assert!(create(None).unwrap_err().contains(NOV_21));
    assert!(!game.join(OWN).exists());
    assert!(!backend::data_dir().join(FILE).exists());
    assert_eq!(
        fs::read_to_string(game.join(world)).unwrap(),
        "existing world"
    );

    let made = create(Some(NOV_21)).unwrap();
    assert_eq!(made.version.as_deref(), Some(NOV_21));
    assert!(made.home.join(world).is_file());
    assert_eq!(find(&made.id).unwrap().version.as_deref(), Some(NOV_21));
}

#[test]
fn installation_edit_refuses_a_deleted_build_unless_a_download_is_ready() {
    if std::env::var_os("HORIZON_TEST_INSTALLATION_EDIT").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "instances::tests::installation_edit_refuses_a_deleted_build_unless_a_download_is_ready"])
            .env("HORIZON_TEST_INSTALLATION_EDIT", "1")
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env("HORIZON_LAUNCHER_LOCATION_FILE", root.path().join("location.json"))
            .status().unwrap().success());
        return;
    }
    let older = backend::version_dir(NOV_21);
    put(&older.join(backend::COMPLETE), "");
    put(&older.join(backend::CLIENT), "pre-holder game");
    put(&backend::version_dir(NOV_22).join(backend::COMPLETE), "");
    let instance = create_java("Switching", "26.3").unwrap();
    let save = |meta: &mut Meta| {
        Plays::Horizon.apply_to(meta);
        meta.version = Some(NOV_21.into());
    };
    assert!(backend::installed().contains(&NOV_21.into()));
    backend::remove_version(NOV_21).unwrap();
    let store = backend::data_dir().join(FILE);
    let saved = fs::read(&store).unwrap();

    let error = edit_installation(&instance.id, None, save).unwrap_err();
    assert!(error.contains(NOV_21));
    assert_eq!(fs::read(&store).unwrap(), saved);
    assert!(for_launch_id(&instance.id).unwrap().java.is_some());

    let pending = edit_installation(&instance.id, Some(NOV_21), save).unwrap();
    assert!(pending.java.is_none());
    assert_eq!(pending.version.as_deref(), Some(NOV_21));
}

#[test]
fn play_time_adds_up_per_instance_and_survives_a_rename() {
    let root = scratch("play-time");
    let (home, data) = (root.join("home"), root.join("data"));
    fs::create_dir_all(home.join(OWN).join("skyblock")).unwrap();
    let ended = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    record_play_in(&data, "skyblock", Duration::from_secs(25 * 60 + 10), ended).unwrap();
    edit_in(&home, &data, "skyblock", |m| m.name = Some("Sky".into())).unwrap();
    let later = ended + Duration::from_secs(3600);
    record_play_in(&data, "skyblock", Duration::from_secs(40 * 60), later).unwrap();
    let all = list_in(&home, &data);
    let sky = find_in(&all, "Sky").unwrap();
    assert_eq!(sky.played, Duration::from_secs(65 * 60 + 10));
    assert_eq!(sky.last_played, Some(later));
    let default = find_in(&all, DEFAULT).unwrap();
    assert_eq!(
        (default.played, default.last_played),
        (Duration::ZERO, None)
    );
    // Never played: nothing written for it.
    let text = fs::read_to_string(data.join(FILE)).unwrap();
    assert!(!text.contains("horizon"), "{text}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn play_time_reads_as_minutes_then_hours() {
    let t = |s| play_time(Duration::from_secs(s));
    assert_eq!(t(59), "under a minute");
    assert_eq!(t(12 * 60 + 59), "12 min");
    assert_eq!(t(3 * 3600 + 5 * 60), "3 h 05 min");
    assert_eq!(t(140 * 3600 + 7 * 60), "140 h");
}

/// A Java instance: an empty folder of its own (the real game keeps its own options), its
/// version in instances.json, kept by a copy, and refused by Play until the launcher starts
/// the real game; Horizon instances' entries stay as they were.
#[test]
fn a_java_instance_plays_one_minecraft_version_in_its_own_folder() {
    let root = scratch("java");
    let (home, data) = (root.join("horizon"), root.join("launcher"));
    player_home(&home);
    let java = create_in(&home, &data, "Vanilla 1.21.4", None, Some("1.21.4")).unwrap();
    assert_eq!(java.home, home.join(OWN).join("vanilla-1-21-4"));
    assert_eq!(fs::read_dir(&java.home).unwrap().count(), 0);
    assert_eq!(java.java.as_ref().unwrap().minecraft, "1.21.4");
    let copy = create_in(&home, &data, "Second", Some(&java), None).unwrap();
    assert_eq!(copy.java, java.java);
    let horizon = create_in(&home, &data, "Horizon two", None, None).unwrap();
    assert_eq!(horizon.java, None);
    // Home offers Horizon's updates to a Horizon instance only, never to a Java one (#18394).
    assert!(horizon.follows_horizon());
    assert!(!java.follows_horizon());
    let store = fs::read_to_string(data.join(FILE)).unwrap();
    assert_eq!(
        store.matches("\"minecraft\": \"1.21.4\"").count(),
        2,
        "{store}"
    );
    let mut cmd = Command::new("game");
    let refused = apply(&mut cmd, &java).unwrap_err();
    assert!(
        refused.contains("plays Minecraft 1.21.4 (Java Edition)"),
        "{refused}"
    );
    edit_in(&home, &data, &java.id, |m| {
        m.java.as_mut().unwrap().memory_mb = Some(6144)
    })
    .unwrap();
    let edited = find_in(&list_in(&home, &data), &java.id).unwrap();
    assert_eq!(edited.java.unwrap().memory_mb, Some(6144));
    fs::remove_dir_all(&root).unwrap();
}

/// Delete moves an instance's folder to the Trash and forgets it; one deleted outside the
/// launcher is forgotten when the list is next read (#horizon-bugs: an instance that stayed).
#[test]
fn deleted_instances_go_to_the_trash_and_are_forgotten() {
    let root = scratch("delete");
    let (home, data, trash) = (root.join("home"), root.join("data"), root.join("trash"));
    player_home(&home);
    let to_trash = |p: &Path| crate::desktop_seams::to_folder(p, &trash);
    let survival = create_in(&home, &data, "Survival", None, None).unwrap();
    put(&survival.home.join("saves/Base/level.dat"), "base");
    edit_in(&home, &data, "Survival", |m| m.group = Some("Mine".into())).unwrap();
    let gone = delete_in(&home, &data, "Survival", &to_trash).unwrap();
    assert_eq!(gone.id, "survival");
    assert_eq!(
        fs::read(trash.join("survival/saves/Base/level.dat")).unwrap(),
        b"base"
    );
    assert!(find_in(&list_in(&home, &data), "Survival").is_err());
    assert!(!Store::load(&data)
        .unwrap()
        .instances
        .contains_key("survival"));
    // The game's home stays; an imported pack goes like an instance.
    assert!(delete_in(&home, &data, DEFAULT, &to_trash)
        .unwrap_err()
        .contains("can't be deleted"));
    assert!(home.join("saves/World/level.dat").is_file());
    delete_in(&home, &data, "small-pack", &to_trash).unwrap();
    assert!(trash
        .join("small-pack/home/saves/Modded/level.dat")
        .is_file());
    // A folder removed by hand: its choices are dropped, a new one of that name starts clean.
    create_in(&home, &data, "Skyblock", None, None).unwrap();
    edit_in(&home, &data, "Skyblock", |m| m.group = Some("Old".into())).unwrap();
    fs::remove_dir_all(home.join(OWN).join("skyblock")).unwrap();
    assert!(find_in(&list_in(&home, &data), "Skyblock").is_err());
    assert!(!Store::load(&data)
        .unwrap()
        .instances
        .contains_key("skyblock"));
    let again = create_in(&home, &data, "Skyblock", None, None).unwrap();
    assert_eq!(again.group, None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_mod_browser_installs_for_the_instance_loader() {
    let vanilla = JavaGame {
        minecraft: "1.21.1".into(),
        ..JavaGame::default()
    };
    assert_eq!(vanilla.target().loader, None);
    let quilt = JavaGame {
        loader: Some(crate::loaders::Loader {
            kind: crate::loaders::Kind::Quilt,
            version: "0.30.1".into(),
        }),
        ..vanilla
    };
    let target = quilt.target();
    assert_eq!(
        (target.minecraft.as_str(), target.loader.as_deref()),
        ("1.21.1", Some("quilt"))
    );
    // A Quilt instance takes Fabric mods too.
    assert!(target.accepts("fabric") && !target.accepts("neoforge"));
}

/// A chosen picture is kept scaled to 128 pixels in the instance's folder; removing it moves it
/// to the trash; a file that isn't an image is refused.
#[test]
fn a_picture_is_kept_small_and_goes_to_the_trash() {
    let root = scratch("picture");
    let (home, trash) = (root.join("home"), root.join("trash"));
    let mut photo = Vec::new();
    image::RgbaImage::from_pixel(600, 400, image::Rgba([200, 90, 40, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut photo),
            image::ImageFormat::Png,
        )
        .unwrap();
    let png = picture(photo).unwrap();
    let to_trash = |p: &Path| crate::desktop_seams::to_folder(p, &trash);
    set_picture(&home, Some(&png), &to_trash).unwrap();
    let kept = image::open(home.join(PICTURE)).unwrap();
    assert_eq!((kept.width(), kept.height()), (128, 85));
    set_picture(&home, None, &to_trash).unwrap();
    assert!(!home.join(PICTURE).exists());
    assert!(trash.join(PICTURE).is_file());
    // Nothing to remove is fine.
    set_picture(&home, None, &to_trash).unwrap();
    assert!(picture(b"not an image".to_vec()).is_err());
    fs::remove_dir_all(&root).unwrap();
}

/// Edit switches what an instance plays (Horizon, Vanilla, a loader, another Minecraft version):
/// its folder, with its worlds, mods and options, stays byte for byte as it was, a Java
/// instance's settings come back after a turn as Horizon, and what can't keep working is named
/// first. The game's home plays what it was made with.
#[test]
fn switching_what_an_instance_plays_keeps_its_worlds_mods_and_settings() {
    use crate::loaders::{Kind as L, Loader};
    let root = scratch("switch");
    let (home, data) = (root.join("horizon"), root.join("launcher"));
    player_home(&home);
    let made = create_in(&home, &data, "Mine", None, None).unwrap();
    put(&made.home.join("saves/Base/level.dat"), "base");
    put(&made.home.join("saves/Base/region/r.0.0.mca"), "region");
    put(
        &made
            .home
            .join(crate::browser::java::MODS_DIR)
            .join("minimap.hmod"),
        "content hmod",
    );
    put(
        &made
            .home
            .join(crate::native_mods::INSTALLED)
            .join("treefall.hmod"),
        "native hmod",
    );
    put(
        &made
            .home
            .join(crate::native_mods::INSTALLED)
            .join("obsolete.hnmod"),
        "not a supported package",
    );
    put(&made.home.join("options.txt"), "fov:90");
    edit_in(&home, &data, &made.id, |m| {
        m.version = Some("nightly-2026-11-01-c9d474d".into())
    })
    .unwrap();
    let before = files(&made.home);
    let fabric = |v: &str| Plays::Java {
        minecraft: v.into(),
        loader: Some(Loader {
            kind: L::Fabric,
            version: "0.16.0".into(),
        }),
    };

    // Horizon to Fabric: both kinds of Horizon mod stop loading; its world changes version.
    let now = find_in(&list_in(&home, &data), &made.id).unwrap();
    assert_eq!(
        unkept(&now, &fabric("1.21.1"), "26.3"),
        [
            Unkept::Mods {
                count: 2,
                made_for: "Horizon".into()
            },
            Unkept::Worlds {
                count: 1,
                from: "26.3".into(),
                to: "1.21.1".into()
            },
        ]
    );
    let java = switch::switch_in(&home, &data, &made.id, fabric("1.21.1")).unwrap();
    assert_eq!(java.java.as_ref().unwrap().minecraft, "1.21.1");
    assert_eq!(java.version.as_deref(), Some("nightly-2026-11-01-c9d474d"));
    edit_in(&home, &data, &made.id, |m| {
        m.java.as_mut().unwrap().memory_mb = Some(6144);
        m.java.as_mut().unwrap().jvm_args = "-XX:+UseZGC".into();
    })
    .unwrap();
    put(&made.home.join("mods/sodium.jar"), "jar");
    let with_jar = files(&made.home);

    // Fabric to Quilt keeps Fabric's mods working; another Minecraft version may not.
    let quilt = Plays::Java {
        minecraft: "1.21.1".into(),
        loader: Some(Loader {
            kind: L::Quilt,
            version: "0.27.0".into(),
        }),
    };
    let java = find_in(&list_in(&home, &data), &made.id).unwrap();
    assert_eq!(unkept(&java, &quilt, "26.3"), []);
    assert_eq!(
        unkept(&java, &fabric("1.20.1"), "26.3")[0],
        Unkept::ModsVersion {
            count: 1,
            from: "1.21.1".into(),
            to: "1.20.1".into()
        }
    );

    // A turn as Horizon, then Vanilla: the Java settings come back, nothing on disk moved.
    switch::switch_in(&home, &data, &made.id, Plays::Horizon).unwrap();
    let horizon = find_in(&list_in(&home, &data), &made.id).unwrap();
    assert_eq!(horizon.java, None);
    let vanilla = Plays::Java {
        minecraft: "1.21.4".into(),
        loader: None,
    };
    let back = switch::switch_in(&home, &data, &made.id, vanilla).unwrap();
    let back = back.java.unwrap();
    assert_eq!(
        (
            back.minecraft.as_str(),
            back.memory_mb,
            back.jvm_args.as_str()
        ),
        ("1.21.4", Some(6144), "-XX:+UseZGC")
    );
    assert_eq!(back.loader, None);
    assert_eq!(files(&made.home), with_jar);
    assert_eq!(
        with_jar.len(),
        before.len() + 1,
        "only the jar put there is new"
    );

    // The game's home can't switch, and is left as it was.
    let home_files = files(&home.join("saves"));
    let refused = switch::switch_in(&home, &data, DEFAULT, Plays::Horizon).unwrap_err();
    assert!(refused.contains("plays what it was made with"), "{refused}");
    assert_eq!(files(&home.join("saves")), home_files);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_instance_without_a_channel_is_nightly_and_an_unknown_one_too() {
    let meta: Meta = serde_json::from_str("{}").unwrap();
    assert_eq!(meta.channel.unwrap_or_default(), Channel::Nightly);
    let meta: Meta = serde_json::from_str(r#"{"channel":"stable","group":"g"}"#).unwrap();
    assert_eq!(meta.channel, Some(Channel::Stable));
    let meta: Meta = serde_json::from_str(r#"{"channel":"beta","group":"g"}"#).unwrap();
    assert_eq!((meta.channel, meta.group.as_deref()), (None, Some("g")));
}

/// An instance pinned to a version that is gone keeps its pin: Play refuses rather than open its
/// worlds in the launcher's version. One that follows the launcher, or plays Java, has no pin.
#[test]
fn a_pinned_version_that_is_gone_stays_unresolved() {
    let instance = |version: Option<&str>, java: Option<JavaGame>| Instance {
        id: "own".into(),
        name: "Own".into(),
        kind: Kind::Own,
        work: PathBuf::new(),
        home: PathBuf::new(),
        icon: None,
        group: None,
        version: version.map(String::from),
        channel: Channel::default(),
        native_profile: crate::native_mods::Profile::Optimized,
        played: Duration::ZERO,
        last_played: None,
        launch: Launch::default(),
        java,
    };
    let installed = ["v-b".to_string()];
    assert_eq!(
        instance(Some("v-a"), None).missing_pin(&installed),
        Some("v-a")
    );
    assert_eq!(instance(Some("v-b"), None).missing_pin(&installed), None);
    assert_eq!(instance(None, None).missing_pin(&installed), None);
    let java = Some(JavaGame::default());
    assert_eq!(instance(Some("v-a"), java).missing_pin(&installed), None);
}

/// A sparse file of `len` bytes: it weighs nothing on disk, and reading it whole would allocate.
fn sparse(path: &Path, len: u64) {
    fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[test]
fn a_file_over_the_cap_is_not_read() {
    let dir = scratch("capped");
    let file = dir.join(PICTURE);
    fs::write(&file, b"abc").unwrap();
    assert_eq!(read_capped(&file, 3).unwrap(), b"abc");
    assert!(read_capped(&file, 2).is_err());
    assert!(read_capped(&dir.join("missing"), 3).is_err());
    // Just over the cap: refused from its size, without a read of the zeros.
    sparse(&file, MAX_PICTURE_FILE + 1);
    assert!(read_capped(&file, MAX_PICTURE_FILE).is_err());
    sparse(&file, MAX_TEXT_FILE + 1);
    assert!(read_text_capped(&file, MAX_TEXT_FILE).is_err());
    fs::remove_dir_all(dir).unwrap();
}
