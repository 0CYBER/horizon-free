use super::*;
use wasmtime::{Instance, Store};

fn wasm(answer: i32) -> Vec<u8> {
    wat::parse_str(format!(
        "(module (func (export \"answer\") (result i32) i32.const {answer}))"
    ))
    .unwrap()
}
fn fixture() -> (tempfile::TempDir, ModuleCache, Engine, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let cache = ModuleCache::new(dir.path().join(".horizon-cache/wasm"));
    (dir, cache, crate::runtime::engine().unwrap(), wasm(42))
}
fn entry(cache: &ModuleCache, engine: &Engine, bytes: &[u8]) -> PathBuf {
    cache
        .root
        .join(format!("{}.cwasm", ModuleCache::key(engine, bytes)))
}
fn answer(engine: &Engine, module: &Module) -> i32 {
    crate::test_stack::on_guest_stack(|| {
        let mut store = Store::new(engine, ());
        // Production engines use epoch interruption.
        store.set_epoch_deadline(u64::MAX / 2);
        let instance = Instance::new(&mut store, module, &[]).unwrap();
        instance
            .get_typed_func::<(), i32>(&mut store, "answer")
            .unwrap()
            .call(&mut store, ())
            .unwrap()
    })
}
fn entries(cache: &ModuleCache) -> Vec<PathBuf> {
    fs::read_dir(&cache.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}
fn replace_artifact(path: &Path, bytes: &[u8]) {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    temporary.write_all(bytes).unwrap();
    temporary.as_file().sync_all().unwrap();
    temporary.persist(path).unwrap();
}
fn assert_compiled(cache: &ModuleCache, engine: &Engine, bytes: &[u8]) {
    let (module, source) = cache.load_or_compile(engine, bytes).unwrap();
    assert_eq!(source, Source::Compiled);
    assert_eq!(answer(engine, &module), 42);
}
fn assert_cached(cache: &ModuleCache, engine: &Engine, bytes: &[u8]) {
    let (module, source) = cache.load_or_compile(engine, bytes).unwrap();
    assert_eq!(source, Source::Cached);
    assert_eq!(answer(engine, &module), 42);
}

#[test]
fn first_compile_then_cached_across_launches_preserves_execution() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&cache.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(entry(&cache, &engine, &bytes))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let paths = entries(&cache);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].extension().unwrap(), "cwasm");
    let next_launch = ModuleCache::new(cache.root.clone());
    assert_cached(&next_launch, &crate::runtime::engine().unwrap(), &bytes);
}

#[test]
fn one_changed_wasm_byte_keeps_both_entries() {
    let (_dir, cache, engine, bytes) = fixture();
    let different = wasm(43);
    assert_eq!(
        bytes.iter().zip(&different).filter(|(a, b)| a != b).count(),
        1
    );
    assert_ne!(
        ModuleCache::key(&engine, &bytes),
        ModuleCache::key(&engine, &different)
    );
    assert_compiled(&cache, &engine, &bytes);
    let (module, source) = cache.load_or_compile(&engine, &different).unwrap();
    assert_eq!(source, Source::Compiled);
    assert_eq!(answer(&engine, &module), 43);
    assert_eq!(entries(&cache).len(), 2);
}

#[test]
fn configurations_coexist_without_eviction() {
    let (_dir, cache, _, bytes) = fixture();
    let production = crate::runtime::engine().unwrap();
    let mut config = wasmtime::Config::new();
    crate::runtime::without_threads(&mut config);
    config.epoch_interruption(crate::runtime::ENGINE_CONFIG.1);
    config.max_wasm_stack(horizon_guest_stack::GUEST_STACK_BYTES);
    // Wasmtime validates this bound even for synchronous engines. No async
    // support or fiber execution is enabled by setting its size.
    config.async_stack_size(
        horizon_guest_stack::GUEST_STACK_BYTES + horizon_guest_stack::HOST_HEADROOM,
    );
    config.cranelift_opt_level(wasmtime::OptLevel::None);
    let other = Engine::new(&config).unwrap();
    assert_ne!(
        ModuleCache::key(&production, &bytes),
        ModuleCache::key(&other, &bytes)
    );
    for engine in [&production, &other] {
        assert_compiled(&cache, engine, &bytes);
    }
    assert_eq!(entries(&cache).len(), 2);
    for engine in [&production, &other] {
        assert_cached(&cache, engine, &bytes);
    }
}

#[test]
fn truncated_entry_is_replaced() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let (mapped, source) = cache.load_or_compile(&engine, &bytes).unwrap();
    assert_eq!(source, Source::Cached);
    let path = entry(&cache, &engine, &bytes);
    let artifact = fs::read(&path).unwrap();
    replace_artifact(&path, &artifact[..10]);
    assert_eq!(answer(&engine, &mapped), 42);
    assert_compiled(&cache, &engine, &bytes);
    assert_cached(&cache, &engine, &bytes);
    assert_eq!(entries(&cache).len(), 1);
    assert_eq!(answer(&engine, &mapped), 42);
}

#[test]
fn damaged_header_is_replaced() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    let mut artifact = fs::read(&path).unwrap();
    artifact[0] ^= 0xff;
    replace_artifact(&path, &artifact);
    assert_compiled(&cache, &engine, &bytes);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn regular_file_root_always_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("file");
    fs::write(&root, b"not a directory").unwrap();
    let cache = ModuleCache::new(root.clone());
    assert!(cache.disabled.as_ref().unwrap().contains("folder"));
    for _ in 0..2 {
        assert_compiled(&cache, &crate::runtime::engine().unwrap(), &wasm(42));
    }
    assert_eq!(fs::read(root).unwrap(), b"not a directory");
}

#[test]
fn stale_temporaries_are_removed_and_fresh_ones_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".horizon-cache/wasm");
    drop(ModuleCache::new(root.clone()));
    let stale = root.join(".1-0.tmp");
    let fresh = root.join(".2-0.tmp");
    let entry = root.join(format!(
        "{}.cwasm",
        ModuleCache::key(&crate::runtime::engine().unwrap(), &wasm(42))
    ));
    for path in [&stale, &fresh, &entry] {
        fs::write(path, b"x").unwrap();
    }
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    for path in [&stale, &entry] {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(two_hours_ago)
            .unwrap();
    }
    drop(ModuleCache::new(root.clone()));
    assert!(!stale.exists());
    assert!(fresh.exists());
    assert!(entry.exists());
}

#[test]
fn the_key_is_one_sha256_per_input() {
    let engine = crate::runtime::test_engine();
    let a = ModuleCache::key(&engine, &wasm(42));
    assert_eq!(a.len(), 129);
    assert_eq!(a, ModuleCache::key(&engine, &wasm(42)));
    assert_ne!(a, ModuleCache::key(&engine, &wasm(43)));
}

#[cfg(unix)]
#[test]
fn writable_folder_disables_cache_with_folder_reason() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, cache, engine, bytes) = fixture();
    fs::set_permissions(&cache.root, fs::Permissions::from_mode(0o777)).unwrap();
    let disabled = ModuleCache::new(cache.root.clone());
    let reason = disabled.disabled.as_ref().unwrap();
    assert!(reason.contains("folder"));
    assert!(reason.contains(cache.root.to_str().unwrap()));
    assert_compiled(&disabled, &engine, &bytes);
    assert!(entries(&disabled).is_empty());
}

#[cfg(unix)]
#[test]
fn writable_entry_is_replaced_with_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_cached(&cache, &engine, &bytes);
}

#[cfg(unix)]
#[test]
fn symlink_entry_is_not_followed() {
    let (dir, cache, engine, bytes) = fixture();
    let target = dir.path().join("target");
    fs::write(
        &target,
        Module::new(&engine, &bytes).unwrap().serialize().unwrap(),
    )
    .unwrap();
    let before = fs::read(&target).unwrap();
    std::os::unix::fs::symlink(&target, entry(&cache, &engine, &bytes)).unwrap();
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(fs::read(target).unwrap(), before);
    assert_cached(&cache, &engine, &bytes);
}

#[cfg(unix)]
#[test]
fn hard_linked_entry_is_not_read() {
    use std::os::unix::fs::MetadataExt;
    let (dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    let alias = dir.path().join("alias");
    fs::hard_link(&path, &alias).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().nlink(), 2);
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(fs::metadata(path).unwrap().nlink(), 1);
    assert_eq!(fs::metadata(alias).unwrap().nlink(), 1);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn artifact_shipped_beside_package_is_ignored() {
    let (dir, cache, engine, _) = fixture();
    let bytes = wat::parse_str(
        r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (func (export "horizon_layout"))
        (func (export "horizon_in"))
        (func (export "horizon_hook"))
        (func (export "answer") (result i32) i32.const 42))"#,
    )
    .unwrap();
    let package = dir.path().join("mods/planted");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("mod.wasm"), &bytes).unwrap();
    fs::write(package.join("horizon-mod.toml"), format!(
        "id='planted'\nname='Planted'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[wasm]\nsha512='{}'\ntoolchain='test'\nbuilt_with='fixture'\n",
        horizon_mod_package::manifest::sha512_of(&bytes)
    )).unwrap();
    fs::write(
        package.join(format!("{}.cwasm", ModuleCache::key(&engine, &bytes))),
        Module::new(&engine, &bytes).unwrap().serialize().unwrap(),
    )
    .unwrap();
    let loaded = horizon_mod_package::Package::read_wasm(&package).unwrap();
    assert_compiled(&cache, &engine, &loaded);
    assert_cached(&cache, &engine, &loaded);
}

#[test]
#[ignore = "local timing measurement; set HORIZON_CACHE_BENCH_WASM"]
fn measure_compile_and_cached_load() {
    let path = std::env::var("HORIZON_CACHE_BENCH_WASM").unwrap();
    let bytes = fs::read(&path).unwrap();
    let (_dir, cache, _, _) = fixture();
    let engine = crate::runtime::engine().unwrap();
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let module = Module::new(&engine, &bytes).unwrap();
        cold.push(start.elapsed().as_secs_f64() * 1000.0);
        drop(module);
    }
    assert_eq!(
        cache.load_or_compile(&engine, &bytes).unwrap().1,
        Source::Compiled
    );
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let (module, source) = cache.load_or_compile(&engine, &bytes).unwrap();
        warm.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(source, Source::Cached);
        drop(module);
    }
    cold.sort_by(f64::total_cmp);
    warm.sort_by(f64::total_cmp);
    eprintln!(
        "wasm cache measurement: {path}, {} bytes; compile median {:.3} ms, cached median {:.3} ms; compile {cold:?}, cached {warm:?}",
        bytes.len(),
        cold[2],
        warm[2]
    );
}

#[test]
fn group_linker_uses_cache_and_programmatic_linking_leaves_it_empty() {
    crate::test_stack::on_guest_stack(|| {
        use crate::link::{Group, Member};
        let (_dir, cache, _, _) = fixture();
        // Programmatic linking does not install a production epoch deadline.
        let engine = crate::runtime::test_engine();
        let bytes = wat::parse_str(
            r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (func (export "answer") (result i32) i32.const 42))"#,
        )
        .unwrap();
        let members = [Member {
            name: "fixture",
            module: &bytes,
        }];
        let mut plain = Group::link(&engine, &members).unwrap();
        let mut result = [wasmtime::Val::I32(0)];
        plain.call("fixture", "answer", &[], &mut result).unwrap();
        assert_eq!(result[0].unwrap_i32(), 42);
        assert!(entries(&cache).is_empty());
        for _ in 0..2 {
            let mut group = Group::link_cached_with(
                Store::new(&engine, ()),
                &members,
                Some(&cache),
                |_, _, _| None,
            )
            .unwrap();
            group.call("fixture", "answer", &[], &mut result).unwrap();
            assert_eq!(result[0].unwrap_i32(), 42);
            assert_eq!(entries(&cache).len(), 1);
        }
        assert_cached(&cache, &engine, &bytes);
    })
}

#[test]
fn failed_replacement_keeps_compiled_module_and_cleans_temp_files() {
    let (_dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    fs::create_dir(&path).unwrap();
    for _ in 0..2 {
        assert_compiled(&cache, &engine, &bytes);
    }
    assert_eq!(entries(&cache), vec![path]);
}

#[test]
fn compiler_actor() {
    let Some(input) = std::env::var_os("HORIZON_COMPILER_INPUT") else {
        return;
    };
    let output = std::env::var_os("HORIZON_COMPILER_OUTPUT").unwrap();
    match std::env::var("HORIZON_COMPILER_MODE").as_deref() {
        Ok("blocked") => {
            fs::write(
                Path::new(&input)
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("compile-blocked"),
                b"ready",
            )
            .unwrap();
            loop {
                std::thread::park();
            }
        }
        Ok("crash") => std::process::abort(),
        Ok("nonzero") => std::process::exit(7),
        Ok("wrong") => {
            fs::write(&input, wasm(43)).unwrap();
            let engine = crate::runtime::engine().unwrap();
            let key = ModuleCache::key(&engine, &wasm(43));
            crate::compile_child::compile(Path::new(&input), &key).unwrap();
        }
        _ => crate::compile_child::compile(Path::new(&input), &output.to_string_lossy()).unwrap(),
    }
}

#[test]
#[ignore = "local child measurement; set HORIZON_CACHE_BENCH_WASM"]
fn measure_child_compile() {
    let bytes = fs::read(std::env::var_os("HORIZON_CACHE_BENCH_WASM").unwrap()).unwrap();
    let engine = crate::runtime::engine().unwrap();
    let mut times = Vec::new();
    for _ in 0..5 {
        let (_dir, cache, _, _) = fixture();
        let input = crate::compile_child::Input::new(&cache.root, &bytes).unwrap();
        let start = std::time::Instant::now();
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "module_cache::tests::compiler_actor"
                ])
                .env_clear()
                .env("HORIZON_COMPILER_INPUT", &input.0)
                .env("HORIZON_COMPILER_OUTPUT", ModuleCache::key(&engine, &bytes))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        times.push(start.elapsed().as_secs_f64());
    }
    eprintln!("child compile {} bytes: {times:?}", bytes.len());
}

#[test]
fn compiler_key_mismatch_writes_nothing() {
    let (_dir, cache, _, bytes) = fixture();
    let input = crate::compile_child::Input::new(&cache.root, &bytes).unwrap();
    assert!(crate::compile_child::compile(&input.0, &"0".repeat(64)).is_err());
    drop(input);
    assert!(entries(&cache).is_empty());
}

fn actor_command(input: &Path, key: &str, mode: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args([
            "--include-ignored",
            "--exact",
            "module_cache::tests::compiler_actor",
        ])
        .env("HORIZON_COMPILER_INPUT", input)
        .env("HORIZON_COMPILER_OUTPUT", key)
        .env("HORIZON_COMPILER_MODE", mode);
    command
}
fn happy(input: &Path, key: &str) -> std::process::Command {
    actor_command(input, key, "happy")
}
fn killed(input: &Path, key: &str) -> std::process::Command {
    actor_command(input, key, "midwrite")
}
fn blocked(input: &Path, key: &str) -> std::process::Command {
    actor_command(input, key, "blocked")
}
fn wrong(input: &Path, key: &str) -> std::process::Command {
    actor_command(input, key, "wrong")
}
fn crash(input: &Path, key: &str) -> std::process::Command {
    actor_command(input, key, "crash")
}
fn nonzero(input: &Path, key: &str) -> std::process::Command {
    actor_command(input, key, "nonzero")
}
fn missing(_: &Path, _: &str) -> std::process::Command {
    std::process::Command::new("/no-such-horizon-compiler-2878")
}

#[test]
#[ignore = "daily"]
fn child_success_avoids_in_process_compile() {
    let (_dir, cache, _, bytes) = fixture();
    let engine = crate::runtime::engine().unwrap();
    let (module, source) = cache
        .load_with_child(
            &engine,
            &bytes,
            Some((happy, std::time::Duration::from_secs(60))),
        )
        .unwrap();
    assert_eq!(source, Source::Cached);
    assert_eq!(answer(&engine, &module), 42);
    assert_eq!(cache.compiles.load(Ordering::Relaxed), 0);
    assert_eq!(entries(&cache).len(), 1);
}

#[test]
#[ignore = "daily"]
fn child_failures_refuse_without_unbounded_compilation() {
    for (command, marker) in [
        (wrong as fn(&Path, &str) -> std::process::Command, None),
        (crash, None),
        (nonzero, None),
        (missing, None),
        (killed, Some("write-blocked")),
        (blocked, Some("compile-blocked")),
    ] {
        let (_dir, cache, _, bytes) = fixture();
        let engine = crate::runtime::engine().unwrap();
        let error = cache
            .load_with_child(
                &engine,
                &bytes,
                Some((command, std::time::Duration::from_secs(10))),
            )
            .unwrap_err();
        if marker.is_some() {
            assert!(
                error.downcast_ref::<CompileTimeout>().is_some(),
                "{error:#}"
            );
        } else {
            assert!(
                error.downcast_ref::<CompileUnavailable>().is_some(),
                "{error:#}"
            );
            let diagnostic = error.to_string();
            assert!(diagnostic.contains("cannot enforce compilation budget"));
            assert!(diagnostic.contains("private writable .horizon-cache/wasm"));
        }
        if let Some(marker) = marker {
            assert!(cache.root.parent().unwrap().join(marker).exists());
        }
        assert_eq!(cache.compiles.load(Ordering::Relaxed), 0);
        assert!(
            !cache
                .entry_path(&ModuleCache::key(&engine, &bytes))
                .exists()
        );
        assert!(
            entries(&cache)
                .iter()
                .all(|path| path.extension().unwrap() == "cwasm")
        );
    }
}

#[test]
fn internal_compiler_flag_refuses_malformed_arguments() {
    use std::ffi::OsString;
    for args in [
        vec!["--compile-wasm-module"],
        vec!["--compile-wasm-module", "input"],
        vec!["--compile-wasm-module", "input", "key", "extra"],
    ] {
        assert_eq!(
            crate::compile_child(args.into_iter().map(OsString::from)),
            Some(1)
        );
    }
    assert_eq!(crate::compile_child([OsString::from("--help")]), None);
}

#[test]
fn compiler_refuses_correctly_keyed_input_outside_cache_folder() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = wasm(42);
    let engine = crate::runtime::engine().unwrap();
    let input = crate::compile_child::Input::new(dir.path(), &bytes).unwrap();
    let key = ModuleCache::key(&engine, &bytes);
    assert_eq!(
        crate::compile_child([
            std::ffi::OsString::from("--compile-wasm-module"),
            input.0.clone().into_os_string(),
            key.into(),
        ]),
        Some(1)
    );
    assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_none_or(|ext| ext != "cwasm")
    }));
}

#[test]
fn compiler_keeps_old_live_input_and_other_writer_temporary() {
    let (_dir, cache, _, bytes) = fixture();
    let engine = crate::runtime::engine().unwrap();
    let input = crate::compile_child::Input::new(&cache.root, &bytes).unwrap();
    let writer = cache.root.join(".98765-0.tmp");
    fs::write(&writer, b"unfinished writer").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    for path in [&input.0, &writer] {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    crate::compile_child::compile(&input.0, &ModuleCache::key(&engine, &bytes)).unwrap();
    assert!(input.0.exists());
    assert_eq!(fs::read(writer).unwrap(), b"unfinished writer");
}

/// Local measurement only: newline-separated module paths, no installed player data.
#[test]
#[ignore = "local size measurement; set HORIZON_CACHE_SIZE_MODULES"]
fn measure_serialized_sizes() {
    let engine = crate::runtime::engine().unwrap();
    let measure = |name: &str, bytes: &[u8]| {
        let module = Module::new(&engine, bytes).unwrap();
        let artifact = module.serialize().unwrap();
        eprintln!(
            "CACHE_SIZE | {name} | {} | {} |",
            bytes.len(),
            artifact.len()
        );
    };
    for value in [42, 43, 99] {
        measure(&format!("answer-{value} fixture"), &wasm(value));
    }
    let dynamic = wat::parse_str(
        r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (func (export "answer") (result i32) i32.const 42))"#,
    )
    .unwrap();
    measure("dynamic-link fixture", &dynamic);
    let paths = std::env::var("HORIZON_CACHE_SIZE_MODULES").unwrap();
    for path in paths.lines().filter(|path| !path.is_empty()) {
        measure(path, &fs::read(path).unwrap());
    }
}

#[test]
fn startup_removes_stale_and_old_format_but_keeps_current() {
    let (_dir, cache, engine, bytes) = fixture();
    let stale = cache
        .root
        .join(format!("{}-{}.cwasm", "0".repeat(64), "1".repeat(64)));
    let old = cache.root.join(format!("{}.cwasm", "2".repeat(64)));
    let malformed = cache.root.join("malformed.cwasm");
    let current = entry(&cache, &engine, &bytes);
    for path in [&stale, &old, &malformed, &current] {
        fs::write(path, b"keep").unwrap();
    }
    drop(ModuleCache::new(cache.root.clone()));
    assert!(!stale.exists());
    assert!(!old.exists());
    assert!(!malformed.exists());
    assert_eq!(fs::read(current).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn startup_cleanup_preserves_symlink_targets_and_unrelated_files() {
    let (dir, cache, _, _) = fixture();
    let target = dir.path().join("target");
    fs::write(&target, b"target data").unwrap();
    let link = cache
        .root
        .join(format!("{}-{}.cwasm", "0".repeat(64), "1".repeat(64)));
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let unrelated = cache.root.join("notes.txt");
    fs::write(&unrelated, b"notes").unwrap();
    let directory = cache.root.join("directory.cwasm");
    fs::create_dir(&directory).unwrap();
    drop(ModuleCache::new_with_limit(cache.root.clone(), false, 0));
    assert_eq!(fs::read(target).unwrap(), b"target data");
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(unrelated).unwrap(), b"notes");
    assert!(directory.is_dir());
}

#[test]
fn startup_size_limit_removes_oldest_first() {
    let (_dir, cache, engine, _) = fixture();
    let prefix = ModuleCache::engine_part(&engine);
    let paths = ["1", "2", "3"].map(|digit| {
        cache
            .root
            .join(format!("{prefix}-{}.cwasm", digit.repeat(64)))
    });
    for (path, seconds) in paths.iter().zip([100, 200, 300]) {
        fs::write(path, b"1234").unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    }
    let next = ModuleCache::new_with_limit(cache.root.clone(), false, 10);
    assert!(!paths[0].exists());
    assert_eq!(fs::read(&paths[1]).unwrap(), b"1234");
    assert_eq!(fs::read(&paths[2]).unwrap(), b"1234");
    // Writes after startup are retained even when they exceed this launch's budget.
    assert_compiled(&next, &engine, &wasm(42));
    assert_cached(&next, &engine, &wasm(42));
}

#[test]
fn compiled_module_loads_cached_after_startup_cleanup() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let stale = cache.root.join(format!("{}.cwasm", "0".repeat(64)));
    fs::write(&stale, b"obsolete").unwrap();
    let next = ModuleCache::new(cache.root.clone());
    assert!(!stale.exists());
    assert_cached(&next, &engine, &bytes);
}
