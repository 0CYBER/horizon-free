use super::*;
use horizon_mod_package::manifest::collect_jar_requirements;
use sha2::Digest;
use std::{cell::Cell, io::Write, path::PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("launcher-jars-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source(seed: &[u8]) -> (JarRef, Vec<u8>) {
    let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    zip.start_file(
        "assets/example/test.txt",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(seed).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let jar = JarRef::new(
        "example".into(),
        "1".into(),
        format!("{:x}", sha2::Sha512::digest(&bytes)),
    )
    .unwrap();
    (jar, bytes)
}

fn requirements(jars: &[JarRef]) -> Vec<JarRequirement> {
    collect_jar_requirements([("example", jars)]).unwrap()
}

fn package(root: &Path, id: &str, jars: Vec<JarRef>, native: bool) -> PathBuf {
    let path = root.join(id);
    fs::create_dir_all(&path).unwrap();
    let mut text =
        format!("id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n");
    if native {
        text.push_str(&format!(
            "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        ));
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            "[package]\nname='not_the_mod_identity'\nversion='9.9.9'\n",
        )
        .unwrap();
        fs::write(path.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
    }
    for jar in jars {
        text.push_str(&format!(
            "[[jar]]\nid='{}'\nversion='{}'\nsha512='{}'\n",
            jar.id(),
            jar.version(),
            jar.sha512()
        ));
    }
    fs::write(path.join("horizon-mod.toml"), text).unwrap();
    path
}

#[test]
fn conflicting_metadata_for_one_exact_hash_is_refused_across_mods() {
    let (jar, _) = source(b"shared");
    for (id, version) in [("other", "1"), ("example", "2")] {
        let conflicting = JarRef::new(id.into(), version.into(), jar.sha512().into()).unwrap();
        let error = collect_jar_requirements([
            ("alpha", std::slice::from_ref(&jar)),
            ("beta", std::slice::from_ref(&conflicting)),
        ])
        .unwrap_err();
        assert_eq!(*error.first, jar);
        assert_eq!(*error.second, conflicting);
    }
}

#[test]
fn cache_hits_are_offline_and_corrupt_files_are_not_silently_replaced() {
    let home = Scratch::new();
    let (jar, bytes) = source(b"cache");
    let cache = home.0.join("jars");
    fs::create_dir_all(&cache).unwrap();
    let path = cache.join(format!("{}.jar", jar.sha512()));
    fs::write(&path, bytes).unwrap();
    prepare_with(
        &home.0,
        &requirements(std::slice::from_ref(&jar)),
        Permission::Refuse,
        |_| panic!("cache hit must not use the network"),
        |_, _, _| panic!("download"),
    )
    .unwrap();
    fs::write(&path, b"corrupt").unwrap();
    let error = prepare_with(
        &home.0,
        &requirements(&[jar]),
        Permission::Allow,
        |_| panic!("corrupt cache must fail"),
        |_, _, _| panic!("download"),
    )
    .unwrap_err();
    assert!(matches!(error, PrepareError::HashMismatch { .. }));
    assert_eq!(fs::read(path).unwrap(), b"corrupt");
}

#[test]
fn all_missing_hashes_share_one_consent_and_each_download_once() {
    let home = Scratch::new();
    let (a, first) = source(b"first");
    let (b, second) = source(b"second");
    let prompts = Cell::new(0);
    let ask = |jars: &[&JarRef]| {
        prompts.set(prompts.get() + 1);
        assert_eq!(jars.len(), 2);
        true
    };
    let lookups = Cell::new(0);
    let mut downloads = 0;
    let jars = collect_jar_requirements([
        ("alpha", [a.clone()].as_slice()),
        ("beta", [b, a.clone()].as_slice()),
    ])
    .unwrap();
    assert_eq!(
        jars.iter().find(|r| r.jar == a).unwrap().owners,
        ["alpha", "beta"]
    );
    prepare_with(
        &home.0,
        &jars,
        Permission::Ask(&ask),
        |missing| {
            assert_eq!(prompts.get(), 1, "consent precedes metadata requests");
            lookups.set(lookups.get() + 1);
            assert_eq!(missing.len(), 2);
            Ok(missing
                .iter()
                .map(|jar| (jar.sha512().to_owned(), jar.sha512().to_owned()))
                .collect())
        },
        |url, jar, dest| {
            assert_eq!(url, jar.sha512());
            downloads += 1;
            fs::write(
                dest,
                if jar.sha512() == a.sha512() {
                    &first
                } else {
                    &second
                },
            )
            .map_err(|e| e.to_string())
        },
    )
    .unwrap();
    assert_eq!(prompts.get(), 1);
    assert_eq!(lookups.get(), 1);
    assert_eq!(downloads, 2);
    assert_eq!(fs::read_dir(home.0.join("jars")).unwrap().count(), 2);
    fs::remove_file(a.cache_path(&home.0.join("jars"))).unwrap();
    let refusal = prepare_with(
        &home.0,
        &jars,
        Permission::Refuse,
        |_| panic!("a shared missing jar must refuse before lookup"),
        |_, _, _| panic!("a shared missing jar must refuse before downloads"),
    )
    .unwrap_err();
    let PrepareError::Refused { missing } = &refusal else {
        panic!("expected the shared typed missing-file payload");
    };
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].owners, ["alpha", "beta"]);
    assert_eq!(missing[0].jar, a);
    assert_eq!(missing[0].path, a.cache_path(&home.0.join("jars")));
    assert!(refusal.to_string().contains("alpha, beta: example 1:"));
}

#[test]
fn one_unresolved_hash_refuses_the_whole_batch_before_any_download() {
    let home = Scratch::new();
    let (first, _) = source(b"available");
    let (second, _) = source(b"unavailable");
    let error = prepare_with(
        &home.0,
        &requirements(&[first.clone(), second.clone()]),
        Permission::Allow,
        |missing| {
            assert_eq!(missing.len(), 2);
            Ok(BTreeMap::from([(
                first.sha512().into(),
                "exact.jar".into(),
            )]))
        },
        |_, _, _| panic!("an incomplete lookup must not download any file"),
    )
    .unwrap_err();
    let PrepareError::Download { jar, path, .. } = error else {
        panic!("the unresolved file must be named");
    };
    assert_eq!(*jar, second);
    assert_eq!(
        path,
        home.0.join("jars").join(format!("{}.jar", second.sha512()))
    );
    assert!(!home.0.join("jars").exists());
}

#[test]
fn headless_refusal_is_actionable_and_never_contacts_modrinth() {
    let home = Scratch::new();
    let (jar, _) = source(b"missing");
    let expected = format!("{}.jar", jar.sha512());
    let error = prepare_with(
        &home.0,
        &requirements(&[jar]),
        Permission::Refuse,
        |_| panic!("headless refusal must be offline"),
        |_, _, _| panic!("download"),
    )
    .unwrap_err();
    assert!(matches!(&error, PrepareError::Refused { missing } if missing.len() == 1));
    assert!(error.to_string().contains("--allow-jar-downloads"));
    assert!(error.to_string().contains(&expected));
    assert!(!home.0.join("jars").exists());
}

#[test]
fn declined_consent_and_missing_or_failed_lookup_never_download() {
    let home = Scratch::new();
    let (jar, _) = source(b"missing");
    let ask = |_: &[&JarRef]| false;
    let error = prepare_with(
        &home.0,
        &requirements(std::slice::from_ref(&jar)),
        Permission::Ask(&ask),
        |_| panic!("declined consent must not contact Modrinth"),
        |_, _, _| panic!("declined"),
    )
    .unwrap_err();
    assert!(matches!(error, PrepareError::Refused { .. }));
    for result in [Ok(BTreeMap::new()), Err("transport failed".to_owned())] {
        let consented = Cell::new(false);
        let ask = |_: &[&JarRef]| {
            consented.set(true);
            true
        };
        let expected = if result.is_err() {
            "transport failed"
        } else {
            "no exact file"
        };
        let error = prepare_with(
            &home.0,
            &requirements(std::slice::from_ref(&jar)),
            Permission::Ask(&ask),
            |_| {
                assert!(consented.get());
                result.clone()
            },
            |_, _, _| panic!("lookup failed"),
        )
        .unwrap_err();
        let reason = match (&result, error) {
            (Ok(_), PrepareError::Download { reason, .. })
            | (Err(_), PrepareError::Lookup { reason, .. }) => reason,
            _ => panic!("metadata and per-file lookup failures must remain distinct"),
        };
        assert!(reason.contains(expected));
    }
    assert!(!home.0.join("jars").exists());
}

#[test]
fn failed_metadata_lookup_names_every_missing_jar_but_not_cache_hits() {
    let first = JarRef::new("first".into(), "2".into(), "a".repeat(128)).unwrap();
    let second = JarRef::new("second".into(), "3".into(), "b".repeat(128)).unwrap();
    for with_cache_hit in [false, true] {
        let home = Scratch::new();
        let cache = home.0.join("jars");
        let (cached, bytes) = source(b"already cached");
        let cached_path = cache.join(format!("{}.jar", cached.sha512()));
        let mut jars = vec![first.clone(), second.clone()];
        if with_cache_hit {
            fs::create_dir_all(&cache).unwrap();
            fs::write(&cached_path, &bytes).unwrap();
            jars.insert(0, cached);
        }
        let jars = requirements(&jars);
        let prompts = Cell::new(0);
        let ask = |missing: &[&JarRef]| {
            prompts.set(prompts.get() + 1);
            assert_eq!(missing, &[&first, &second]);
            true
        };
        let lookups = Cell::new(0);
        let error = prepare_with(
            &home.0,
            &jars,
            Permission::Ask(&ask),
            |missing| {
                assert_eq!(prompts.get(), 1);
                assert_eq!(missing, &[&first, &second]);
                lookups.set(lookups.get() + 1);
                Err("transport failed".into())
            },
            |_, _, _| panic!("failed metadata must not download any file"),
        )
        .unwrap_err();
        let display = error.to_string();
        let PrepareError::Lookup { missing, reason } = error else {
            panic!("a failed batch must not be attributed to a single jar");
        };
        assert_eq!(prompts.get(), 1);
        assert_eq!(lookups.get(), 1);
        assert_eq!(reason, "transport failed");
        assert!(display.starts_with(
            "Jar metadata lookup failed: transport failed. Put the original files at these exact cache paths, then retry:\n"
        ));
        assert_eq!(missing.len(), 2);
        for (entry, jar) in missing.iter().zip([&first, &second]) {
            let path = cache.join(format!("{}.jar", jar.sha512()));
            assert_eq!(&entry.jar, jar);
            assert_eq!(entry.owners, ["example"]);
            assert_eq!(entry.path, path);
            let line = format!(
                "example: {} {}: {}",
                jar.id(),
                jar.version(),
                path.display()
            );
            assert!(display.lines().any(|actual| actual == line));
        }
        assert!(!display.contains(&cached_path.display().to_string()));
        if with_cache_hit {
            assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
            assert_eq!(fs::read(cached_path).unwrap(), bytes);
        } else {
            assert!(!cache.exists());
        }
    }
}

#[test]
fn final_launch_combines_only_enabled_content_dev_and_adjacent_native_candidates() {
    use horizon_mod_package::mod_list::{DIR, FILE, NATIVE_DIR};
    use std::process::Command;
    let root = Scratch::new();
    let game = root.0.join("game");
    let bin = root.0.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join("horizon-client");
    fs::write(&program, b"synthetic executable placeholder: never run").unwrap();
    let (shared, shared_bytes) = source(b"shared");
    let (dev, dev_bytes) = source(b"dev");
    let (excluded, _) = source(b"must not fetch");
    package(&game.join(DIR), "plain", vec![shared.clone()], false);
    package(&game.join(NATIVE_DIR), "other", vec![shared.clone()], false);
    package(&game.join(DIR), "disabled", vec![excluded.clone()], false);
    package(&game.join(NATIVE_DIR), "next_build", vec![excluded], true);
    package(&bin.join(NATIVE_DIR), "built", vec![shared.clone()], true);
    let dev_path = package(&root.0.join("external"), "dev", vec![dev.clone()], false);
    fs::write(
        game.join(FILE),
        "mod.plain:true\nmod.other:true\nmod.disabled:false\nmod.next_build:true\n",
    )
    .unwrap();
    fs::write(
        bin.join("native-mods.json"),
        r#"{"mods":[{"id":"next_build"}]}"#,
    )
    .unwrap();
    let mut command = Command::new(&program);
    command
        .current_dir(&root.0)
        .env("HORIZON_HOME", root.0.join("wrong-cache"))
        .args(["--game-dir", "game", "--home", "cache", "--dev-mod"])
        .arg(dev_path);
    let error = prepare_launch(
        &mut command,
        &Default::default(),
        Permission::headless(false),
    )
    .unwrap_err();
    let LaunchError::Prepare(PrepareError::Refused { missing }) = error else {
        panic!("expected typed refusal for just the active data candidates");
    };
    let hashes: std::collections::BTreeSet<_> = missing.iter().map(|m| m.jar.sha512()).collect();
    assert_eq!(
        hashes,
        [shared.sha512(), dev.sha512()].into_iter().collect()
    );
    for jar in &missing {
        assert_eq!(
            jar.path,
            root.0
                .join("cache/jars")
                .join(format!("{}.jar", jar.jar.sha512()))
        );
    }
    fs::create_dir_all(root.0.join("cache/jars")).unwrap();
    for (jar, bytes) in [(&shared, shared_bytes), (&dev, dev_bytes)] {
        fs::write(
            root.0
                .join("cache/jars")
                .join(format!("{}.jar", jar.sha512())),
            bytes,
        )
        .unwrap();
    }
    prepare_launch(
        &mut command,
        &Default::default(),
        Permission::headless(false),
    )
    .unwrap();
    assert!(!root.0.join("wrong-cache").exists());
    assert_eq!(fs::read_dir(root.0.join("cache/jars")).unwrap().count(), 2);

    // Dedicated remainder and the wrapper's final environment use the very same cache policy.
    let mut server = Command::new(&program);
    server
        .current_dir(&game)
        .env("HORIZON_HOME", root.0.join("wrong-cache"))
        .args(["--dedicated", "--nogui"]);
    let mut launch = crate::launch::Launch::default();
    launch.env.insert(
        "HORIZON_HOME".into(),
        root.0.join("cache").to_str().unwrap().into(),
    );
    prepare_launch(&mut server, &launch, Permission::headless(false)).unwrap();
}

#[test]
fn information_requests_do_not_read_packages_or_create_a_cache() {
    let root = Scratch::new();
    for flag in ["--help", "-h", "--launch-api"] {
        let mut command = std::process::Command::new(root.0.join("not-installed"));
        command
            .current_dir(&root.0)
            .env("HORIZON_HOME", root.0.join("cache"))
            .args([flag, "--missing-value"]);
        prepare_launch(
            &mut command,
            &Default::default(),
            Permission::headless(false),
        )
        .unwrap();
        assert!(!root.0.join("cache").exists());
    }
}
