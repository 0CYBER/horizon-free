use super::*;

fn root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "snapshot-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(root.join("reader")).unwrap();
    root
}

#[test]
fn missing_and_empty_settings_do_not_report_omissions() {
    let root = root("empty");
    let captured = capture_settings_with_diagnostics(&root, ["reader", "absent"], |reason| {
        panic!("empty settings reported an omission: {reason}");
    });
    assert!(captured["reader"].is_empty());
    assert!(!captured.contains_key("absent"));
    fs::remove_dir_all(&root).unwrap();
    assert!(
        capture_settings_with_diagnostics(&root, ["reader"], |reason| {
            panic!("missing settings root reported an omission: {reason}");
        })
        .is_empty()
    );
    assert!(!root.exists());
}

#[test]
fn failed_enumeration_retains_known_omitted_keys() {
    let names = [
        Ok(OsString::from("known")),
        Err(std::io::ErrorKind::PermissionDenied.into()),
    ];
    let error = sorted_names(names.into_iter()).unwrap_err();
    let SnapshotError::Enumeration { names, error } = error else {
        panic!("expected an enumeration failure");
    };
    assert_eq!(names, ["known"]);
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let error =
        sorted_names((0..=MAX_VISITED_NAMES).map(|i| Ok(format!("key{i:04}").into()))).unwrap_err();
    let SnapshotError::EnumerationLimit(names) = error else {
        panic!("expected an enumeration limit");
    };
    assert_eq!(names.len(), MAX_VISITED_NAMES + 1);
    assert_eq!(names.first().unwrap(), "key0000");
    assert_eq!(names.last().unwrap(), "key4096");
}

#[test]
#[cfg(unix)]
fn unsafe_unreadable_and_non_utf8_entries_keep_their_names() {
    use std::os::unix::ffi::OsStringExt;
    let root = root("named-refusals");
    fs::write(root.join("reader/value"), "inside").unwrap();
    fs::create_dir(root.join("reader/directory")).unwrap();
    std::os::unix::fs::symlink("missing", root.join("reader/dangling")).unwrap();
    let mut refused = BTreeMap::new();
    let captured =
        capture_settings_with_report(&root, ["reader", "reader"], |mod_id, key, reason| {
            assert_eq!(mod_id, "reader");
            assert!(
                refused
                    .insert(key.unwrap().to_owned(), reason.to_string())
                    .is_none()
            );
        });
    assert_eq!(captured["reader"].len(), 1);
    assert_eq!(refused.len(), 2);
    assert!(refused[OsStr::new("directory")].contains("not a regular file"));
    assert!(refused[OsStr::new("dangling")].starts_with("unsafe or unreadable entry:"));
    let invalid_key = OsString::from_vec(vec![0xff]);
    fs::write(root.join("reader").join(&invalid_key), "invalid key").unwrap();
    refused.clear();
    let captured = capture_settings_with_report(&root, ["reader"], |_, key, reason| {
        refused.insert(key.unwrap().to_owned(), reason.to_string());
    });
    assert!(!captured.contains_key("reader"));
    assert_eq!(refused.len(), 4);
    assert!(refused[&invalid_key].starts_with("package contains a non-UTF-8 key:"));
    assert!(refused[OsStr::new("value")].starts_with("package contains a non-UTF-8 key:"));
    std::os::unix::fs::symlink(root.join("reader"), root.join("alias")).unwrap();
    let mut scopes = Vec::new();
    let captured = capture_settings_with_report(&root, ["alias"], |mod_id, key, reason| {
        assert_eq!(mod_id, "alias");
        assert!(key.is_none());
        scopes.push(reason.to_string());
    });
    assert!(captured.is_empty());
    assert_eq!(scopes.len(), 1);
    assert!(scopes[0].starts_with("cannot inspect settings directory:"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn published_loop_readers_share_one_immutable_settings_capture() {
    let root = root("published");
    fs::write(root.join("reader/value"), "before").unwrap();
    fs::write(root.join("reader/invalid"), [0xff]).unwrap();
    fs::write(root.join("reader/large"), vec![b'x'; MAX_SETTING_BYTES + 1]).unwrap();
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
        [],
    );
    let mut diagnostics = Vec::new();
    let settings = capture_settings_with_diagnostics(&root, ["reader"], |report| {
        diagnostics.push(report.to_owned());
    });
    let game = LoopContext {
        side: LoopSide::Server,
        snapshot: Snapshot {
            inputs: load.snapshot.inputs.clone(),
            settings: shared_settings(settings),
        },
        recipe_properties: Default::default(),
    };
    fs::write(root.join("reader/value"), "after").unwrap();
    fs::write(root.join("reader/later-invalid"), [0xff]).unwrap();
    let audio = game.clone();
    let worldgen = game.worldgen_context();
    assert!(Arc::ptr_eq(
        &game.snapshot.settings,
        &audio.snapshot.settings
    ));
    assert!(Arc::ptr_eq(
        &game.snapshot.settings,
        &worldgen.snapshot.settings
    ));
    std::thread::scope(|scope| {
        scope.spawn(|| {
            assert_eq!(
                worldgen.setting("reader", "value").as_deref(),
                Some("before")
            );
            assert_eq!(worldgen.setting("reader", "invalid"), None);
        });
        scope.spawn(|| {
            assert_eq!(audio.setting("reader", "value").as_deref(), Some("before"));
            assert_eq!(audio.setting("reader", "later-invalid"), None);
        });
    });
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("\"reader\"/\"invalid\": invalid UTF-8:"));
    assert!(diagnostics[0].contains("\"reader\"/\"large\": file byte limit"));
    capture_settings_with_diagnostics(&root, ["reader"], |report| {
        diagnostics.push(report.to_owned());
    });
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics[1].contains("\"reader\"/\"later-invalid\": invalid UTF-8:"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn initial_snapshot_is_shared_scoped_and_renewed_only_by_a_new_load() {
    let root = root("initial-publication");
    fs::create_dir(root.join("inactive")).unwrap();
    fs::write(root.join("reader/mode"), "before").unwrap();
    fs::write(root.join("inactive/mode"), "not admitted").unwrap();
    let settings = shared_settings(capture_settings_with_diagnostics(
        &root,
        ["reader", "inactive"],
        |reason| panic!("valid initial settings reported a refusal: {reason}"),
    ));
    let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
    let initial = LoadContext::from_settings_snapshot(
        DataLoadOrigin::ClientPreload,
        inputs.clone(),
        ["reader", "inactive"],
        settings.clone(),
    );
    assert!(Arc::ptr_eq(&settings, &initial.snapshot.settings));
    let admitted = LoadContext::from_settings_snapshot(
        DataLoadOrigin::ClientPreload,
        inputs,
        ["reader"],
        settings.clone(),
    );
    assert_eq!(admitted.setting("inactive", "mode"), None);
    assert!(Arc::ptr_eq(
        &settings[1].1,
        &admitted.snapshot.settings[0].1
    ));
    fs::write(root.join("reader/mode"), "after").unwrap();
    fs::write(root.join("reader/late"), "later key").unwrap();
    let loop_context = admitted.captured_loop_context(LoopSide::Client);
    let worldgen = loop_context.worldgen_context();
    assert!(Arc::ptr_eq(
        &admitted.snapshot.settings,
        &loop_context.snapshot.settings
    ));
    assert_eq!(
        loop_context.setting("reader", "mode").as_deref(),
        Some("before")
    );
    assert_eq!(worldgen.setting("reader", "late"), None);
    let next = admitted.for_load(DataLoadOrigin::ClientPreload, ["reader"]);
    assert_eq!(next.setting("reader", "mode").as_deref(), Some("after"));
    assert_eq!(next.setting("reader", "late").as_deref(), Some("later key"));
    assert_eq!(
        loop_context.setting("reader", "mode").as_deref(),
        Some("before")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn reviewer_capture_follows_external_symlink() {
    let root = root("symlinks");
    let data = root.join("data");
    fs::create_dir_all(data.join("reader")).unwrap();
    fs::write(root.join("secret"), "outside-token").unwrap();
    std::os::unix::fs::symlink(root.join("secret"), data.join("reader/token")).unwrap();
    fs::write(data.join("reader/normal"), "inside").unwrap();
    std::os::unix::fs::symlink("normal", data.join("reader/internal")).unwrap();
    std::os::unix::fs::symlink("missing", data.join("reader/dangling")).unwrap();
    std::os::unix::fs::symlink(root.join("reader"), data.join("alias")).unwrap();
    fs::write(root.join("reader/token"), "escaped").unwrap();
    let captured = capture_settings(&data, ["reader", "alias", "../reader"]);
    assert_eq!(captured["reader"].get("token"), None);
    assert_eq!(captured["reader"].get("internal"), None);
    assert_eq!(captured["reader"].get("dangling"), None);
    assert_eq!(captured["reader"]["normal"], "inside");
    assert!(!captured.contains_key("alias"));
    assert!(!captured.contains_key("../reader"));
    std::os::unix::fs::symlink(&data, root.join("linked-root")).unwrap();
    assert!(capture_settings(&root.join("linked-root"), ["reader"]).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
#[cfg(unix)]
fn fifo_and_directory_do_not_block_or_become_settings() {
    let root = root("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.join("reader/pipe"))
            .status()
            .unwrap()
            .success()
    );
    fs::create_dir(root.join("reader/directory")).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let input = root.clone();
    let worker =
        std::thread::spawn(move || send.send(capture_settings(&input, ["reader"])).unwrap());
    let captured = receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("snapshot blocked on FIFO");
    assert!(captured["reader"].is_empty());
    worker.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn admission_is_sorted_and_enumeration_is_bounded() {
    let root = root("ordered");
    fs::create_dir(root.join("second")).unwrap();
    for i in 0..1025 {
        fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
        fs::write(root.join(format!("second/key{:04}", 1024 - i)), "x").unwrap();
    }
    let captured = capture_settings(&root, ["second", "reader"]);
    assert_eq!(captured["reader"], captured["second"]);
    assert!(!captured["reader"].contains_key("key1024"));
    assert_eq!(captured["reader"]["key0000"], "x");
    for i in 1025..4097 {
        fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
    }
    assert!(!capture_settings(&root, ["reader"]).contains_key("reader"));
    assert!(sorted_names((0..4097).map(|i| Ok(i.to_string().into()))).is_err());
    assert_eq!(
        sorted_names((0..4096).map(|i| Ok(i.to_string().into())))
            .unwrap()
            .len(),
        4096
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
#[cfg(unix)]
fn validates_open_handle_after_entry_replacement() {
    use std::io::Read;
    let root = root("replaced");
    let path = root.join("reader/value");
    fs::write(&path, "opened value").unwrap();
    let entries = snapshot_files_with_hook(&root, "reader", |_| {
        fs::remove_file(&path).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
    })
    .unwrap();
    let (_, file) = entries.into_iter().next().unwrap();
    let mut file = file.unwrap();
    let mut value = String::new();
    file.read_to_string(&mut value).unwrap();
    assert_eq!(value, "opened value");

    // Now open the FIFO, then replace its path with a regular file. Path
    // metadata would wrongly accept it, but descriptor metadata must refuse.
    let entries = snapshot_files_with_hook(&root, "reader", |_| {
        fs::remove_file(&path).unwrap();
        fs::write(&path, "replacement").unwrap();
    })
    .unwrap();
    assert!(entries.into_iter().next().unwrap().1.is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn byte_limits_and_invalid_utf8() {
    let root = root("bytes");
    fs::write(root.join("reader/exact"), vec![b'a'; 65536]).unwrap();
    fs::write(root.join("reader/large"), vec![b'b'; 65537]).unwrap();
    fs::write(root.join("reader/invalid"), [0xff]).unwrap();
    let mut refused = Vec::new();
    let captured = capture_settings_with_report(&root, ["reader"], |mod_id, key, reason| {
        refused.push((
            mod_id.to_owned(),
            key.unwrap().to_owned(),
            reason.to_string(),
        ));
    });
    assert_eq!(captured["reader"]["exact"].len(), 65536);
    assert_eq!(captured["reader"].get("large"), None);
    assert_eq!(captured["reader"].get("invalid"), None);
    assert_eq!(refused.len(), 2);
    assert_eq!(refused[0].0, "reader");
    assert_eq!(refused[0].1, "invalid");
    assert!(refused[0].2.starts_with("invalid UTF-8:"));
    assert_eq!(refused[1].1, "large");
    assert!(refused[1].2.starts_with("file byte limit"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn key_limit() {
    let root = root("count");
    for i in 0..1027 {
        fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
    }
    let mut refused = Vec::new();
    let captured = capture_settings_with_report(&root, ["reader"], |mod_id, key, reason| {
        assert_eq!(mod_id, "reader");
        assert!(reason.to_string().starts_with("key limit"));
        refused.push(key.unwrap().to_owned());
    });
    assert_eq!(captured["reader"].len(), 1024);
    assert_eq!(refused, ["key1024", "key1025", "key1026"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reviewer_whitespace_spends_raw_aggregate_budget() {
    let root = root("raw-budget");
    for i in 0..256 {
        fs::write(root.join(format!("reader/key{i:04}")), vec![b' '; 65536]).unwrap();
    }
    fs::write(root.join("reader/z-extra"), "x").unwrap();
    let mut refused = Vec::new();
    let captured = capture_settings_with_report(&root, ["reader"], |mod_id, key, reason| {
        assert_eq!(mod_id, "reader");
        assert!(reason.to_string().starts_with("snapshot byte limit"));
        refused.push(key.unwrap().to_owned());
    });
    assert_eq!(captured["reader"].len(), 256);
    assert!(!captured["reader"].contains_key("z-extra"));
    assert_eq!(refused, ["z-extra"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn reviewer_symlink_replacement_keeps_opened_value() {
    use std::io::Read;
    let root = root("symlink-replaced");
    let path = root.join("reader/value");
    fs::write(&path, "opened").unwrap();
    fs::write(root.join("target"), "replacement").unwrap();
    let entries = snapshot_files_with_hook(&root, "reader", |_| {
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(root.join("target"), &path).unwrap();
    })
    .unwrap();
    let (_, file) = entries.into_iter().next().unwrap();
    let mut file = file.unwrap();
    let mut value = String::new();
    file.read_to_string(&mut value).unwrap();
    assert_eq!(value, "opened");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aggregate_limit() {
    let root = root("aggregate");
    fs::create_dir(root.join("second")).unwrap();
    for i in 0..129 {
        for package in ["reader", "second"] {
            fs::write(root.join(format!("{package}/key{i}")), vec![b'x'; 65536]).unwrap();
        }
    }
    let captured = capture_settings(&root, ["second", "reader"]);
    assert_eq!(captured["reader"].len(), 129);
    assert_eq!(captured["second"].len(), 127);
    assert_eq!(
        captured
            .values()
            .flat_map(|values| values.values())
            .map(String::len)
            .sum::<usize>(),
        16 * 1024 * 1024
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_are_scoped_and_frozen_until_a_new_load() {
    let root = std::env::temp_dir().join(format!(
        "horizon-load-settings-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let active = root.join("active");
    fs::create_dir_all(&active).unwrap();
    fs::create_dir_all(root.join("inactive")).unwrap();
    fs::write(active.join("mode"), "  before\n").unwrap();
    fs::write(active.join("empty"), " \n").unwrap();
    fs::write(active.join("invalid"), [0xff]).unwrap();
    fs::write(root.join("inactive/mode"), "not loaded").unwrap();
    let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
    let before = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs.clone(), ["active"]);

    // Change the files before the first lookup, not just between reads.
    fs::write(active.join("mode"), "  after\n").unwrap();
    fs::write(active.join("late"), "new key").unwrap();
    assert_eq!(before.setting("active", "mode").as_deref(), Some("before"));
    assert_eq!(before.setting("active", "empty").as_deref(), Some(""));
    assert_eq!(before.setting("active", "invalid"), None);
    assert_eq!(before.setting("active", "late"), None);
    assert_eq!(before.setting("inactive", "mode"), None);
    assert_eq!(read_setting(&active, "mode").as_deref(), Some("after"));

    let after = LoadContext::new(DataLoadOrigin::ClientPreload, inputs.clone(), ["active"]);
    assert_eq!(after.setting("active", "mode").as_deref(), Some("after"));
    assert_eq!(after.setting("active", "late").as_deref(), Some("new key"));
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(before.setting("active", "mode").as_deref(), Some("before"));
    assert_eq!(after.setting("active", "mode").as_deref(), Some("after"));
    let missing = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
    assert_eq!(missing.setting("active", "mode"), None);
    assert!(!root.exists());
}

#[test]
fn a_loop_context_shares_the_registries_and_captures_its_own_settings() {
    let root = std::env::temp_dir().join(format!(
        "horizon-loop-settings-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let mod_dir = root.join("active");
    fs::create_dir_all(&mod_dir).unwrap();
    fs::write(mod_dir.join("mode"), "one").unwrap();
    let registries = BTreeMap::from([("item".to_owned(), vec!["a:b".to_owned()])]);
    let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
    let load = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, ["active"]);

    let client = load.loop_context(LoopSide::Client, ["active"]);
    fs::write(mod_dir.join("mode"), "two").unwrap();
    let server = load.loop_context(LoopSide::Server, ["active"]);

    assert_eq!(client.side(), LoopSide::Client);
    assert_eq!(server.side(), LoopSide::Server);
    assert_eq!(
        client.registry("item").as_deref(),
        Some(&["a:b".to_owned()][..])
    );
    assert_eq!(server.registry("item"), client.registry("item"));
    assert_eq!(client.setting("active", "mode").as_deref(), Some("one"));
    assert_eq!(server.setting("active", "mode").as_deref(), Some("two"));
    assert_eq!(load.setting("active", "mode").as_deref(), Some("one"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_worldgen_context_shares_the_registries_and_captures_its_own_settings() {
    let root = std::env::temp_dir().join(format!(
        "horizon-worldgen-settings-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let mod_dir = root.join("active");
    fs::create_dir_all(&mod_dir).unwrap();
    fs::write(mod_dir.join("ground"), "podzol").unwrap();
    let registries = BTreeMap::from([("block".to_owned(), vec!["a:b".to_owned()])]);
    let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
    let load = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
    fs::write(mod_dir.join("ground"), "mud").unwrap();

    let worldgen = load.worldgen_context(["active"]);
    fs::write(mod_dir.join("ground"), "moss").unwrap();

    assert_eq!(
        worldgen.registry("block").as_deref(),
        Some(&["a:b".to_owned()][..])
    );
    assert_eq!(worldgen.setting("active", "ground").as_deref(), Some("mud"));
    assert_eq!(worldgen.setting("inactive", "ground"), None);
    assert_eq!(load.setting("active", "ground").as_deref(), Some("podzol"));
    fs::remove_dir_all(&root).unwrap();
}
