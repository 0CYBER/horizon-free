//! Actual subprocess compilation, not a sleep or mocked compiler.
use crate::{
    compile_budget::CompileBudget,
    module_cache::{CompileTimeout, CompileUnavailable, ModuleCache, Source},
};
use std::{path::Path, process::Command};

#[test]
fn compiler_worker() {
    let Some(input) = std::env::var_os("HORIZON_TEST_COMPILE_INPUT") else {
        return;
    };
    crate::compile_child::compile(
        Path::new(&input),
        &std::env::var("HORIZON_TEST_COMPILE_KEY").unwrap(),
    )
    .unwrap();
}
fn command(input: &Path, key: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--include-ignored",
            "--exact",
            "compile_budget_tests::compiler_worker",
        ])
        .env("HORIZON_TEST_COMPILE_INPUT", input)
        .env("HORIZON_TEST_COMPILE_KEY", key);
    command
}
#[test]
#[ignore = "daily"]
fn actual_child_timeout_then_same_module_succeeds_and_is_cached() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = ModuleCache::new(dir.path().join(".horizon-cache/wasm"));
    let engine = crate::runtime::engine().unwrap();
    let bytes =
        wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))").unwrap();
    cache.budget = CompileBudget::parse("0.000000001").unwrap();
    let error = cache
        .load_with_child_command(&engine, &bytes, Some((command, cache.budget.limit)))
        .unwrap_err();
    assert!(error.downcast_ref::<CompileTimeout>().is_some());
    let diagnostic =
        crate::Refusal::from_link(crate::link::LinkError::compile("libexample.so", error))
            .to_string();
    assert!(diagnostic.contains("example:mod.wasm"));
    assert!(diagnostic.contains("0.000000001 seconds"));
    assert!(diagnostic.contains("wasm.compile_timeout_secs"));
    assert!(
        !cache
            .entry_path(&ModuleCache::key(&engine, &bytes))
            .exists()
    );
    cache.budget = CompileBudget::parse("60").unwrap();
    let (module, source) = cache
        .load_with_child_command(&engine, &bytes, Some((command, cache.budget.limit)))
        .unwrap();
    assert_eq!(
        source,
        Source::Cached,
        "recovery must consume the child-produced artifact"
    );
    assert!(
        cache
            .entry_path(&ModuleCache::key(&engine, &bytes))
            .is_file()
    );
    crate::test_stack::on_guest_stack(|| {
        let mut store = wasmtime::Store::new(&engine, ());
        store.set_epoch_deadline(u64::MAX / 2);
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        assert_eq!(
            instance
                .get_typed_func::<(), i32>(&mut store, "answer")
                .unwrap()
                .call(&mut store, ())
                .unwrap(),
            42
        );
    });
    let (_, source) = cache.load_or_compile(&engine, &bytes).unwrap();
    assert_eq!(source, crate::module_cache::Source::Cached);
}

#[test]
#[ignore = "daily"]
fn unavailable_or_untrusted_cache_refuses_applied_unix_budget() {
    use std::{fs, os::unix::fs::PermissionsExt};
    for untrusted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".horizon-cache/wasm");
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        if untrusted {
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        } else {
            fs::write(&root, b"not a directory").unwrap();
        }
        let mut cache = ModuleCache::new(root);
        cache.budget = CompileBudget::parse("0.000000001").unwrap();
        let engine = crate::runtime::engine().unwrap();
        let bytes = wat::parse_str("(module)").unwrap();
        let error = cache
            .load_with_child_command(&engine, &bytes, Some((command, cache.budget.limit)))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<CompileUnavailable>().unwrap().budget,
            cache.budget
        );
        let diagnostic =
            crate::Refusal::from_link(crate::link::LinkError::compile("libexample.so", error))
                .to_string();
        assert!(diagnostic.contains("example:mod.wasm"));
        assert!(diagnostic.contains("0.000000001 seconds"));
        assert!(diagnostic.contains(cache.root.to_str().unwrap()));
        assert!(
            !cache
                .entry_path(&ModuleCache::key(&engine, &bytes))
                .exists()
        );
    }
}

#[test]
fn installed_settings_refusal_worker() {
    let Some(root) = std::env::var_os("HORIZON_TEST_SETTINGS_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let dedicated = std::env::var("HORIZON_TEST_SETTINGS_SIDE").unwrap() == "server";
    let value = std::env::var("HORIZON_TEST_SETTINGS_VALUE").unwrap();
    let player = root.join("player");
    let mut args = vec!["--game-dir".to_owned(), "game".to_owned()];
    if dedicated {
        args.push("--dedicated".to_owned());
        args.push("nogui".to_owned());
    } else {
        args.push("--player-dir".to_owned());
        args.push(player.to_str().unwrap().to_owned());
    }
    let launch =
        horizon_mod_package::selection::LaunchArguments::parse(args.into_iter(), root, root, None)
            .unwrap();
    let loaded = crate::load_installed(&launch, &player);
    let expected =
        crate::Refusal::CompileSetting(crate::compile_budget::SettingError::InvalidValue(value));
    assert!(loaded.mods.is_empty());
    assert_eq!(loaded.refused, [("example".to_owned(), expected.clone())]);
    // These are the public registry consumed by the Mods page and startup content exclusion.
    assert_eq!(crate::refused(), loaded.refused);
    assert_eq!(crate::refusal("example"), Some(expected.clone()));
    assert_eq!(crate::refusal("unrelated"), None);
    crate::initialize(&[]);
    assert_eq!(crate::refusal("example"), Some(expected));
}

#[test]
#[ignore = "daily"]
fn invalid_startup_settings_publish_and_report_refusals_on_both_sides() {
    use std::fs;
    for side in ["client", "server"] {
        for value in ["0", "not-a-number"] {
            let dir = tempfile::tempdir().unwrap();
            let game = dir.path().join("game");
            let player = dir.path().join("player");
            let package = game.join("horizon-mods/example");
            fs::create_dir_all(&package).unwrap();
            fs::create_dir_all(&player).unwrap();
            let module = wat::parse_str(
                r#"(module
                (@custom "dylink.0" "\01\04\00\00\00\00")
                (func (export "horizon_layout"))
                (func (export "horizon_in"))
                (func (export "horizon_hook")))"#,
            )
            .unwrap();
            fs::write(package.join("mod.wasm"), &module).unwrap();
            fs::write(package.join("horizon-mod.toml"), format!(
                "id='example'\nname='Example'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[wasm]\nsha512='{}'\ntoolchain='test'\nbuilt_with='test'\n",
                horizon_mod_package::manifest::sha512_of(&module),
            )).unwrap();
            fs::write(game.join("horizon-mods.txt"), "mod.example:true\n").unwrap();
            // Distinct settings locations make either accidental fallback observable.
            fs::write(game.join("options.txt"), "wasm.compile_timeout_secs:120\n").unwrap();
            fs::write(
                player.join("options.txt"),
                format!(
                    "wasm.compile_timeout_secs:{}\n",
                    if side == "client" { value } else { "120" },
                ),
            )
            .unwrap();
            fs::write(
                game.join("server.properties"),
                format!(
                    "wasm.compile_timeout_secs={}\n",
                    if side == "server" { value } else { "120" },
                ),
            )
            .unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "compile_budget_tests::installed_settings_refusal_worker",
                    "--nocapture",
                ])
                .env("HORIZON_TEST_SETTINGS_ROOT", dir.path())
                .env("HORIZON_TEST_SETTINGS_SIDE", side)
                .env("HORIZON_TEST_SETTINGS_VALUE", value)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let console = String::from_utf8(output.stdout).unwrap();
            assert!(console.contains("wasm.compile_timeout_secs"), "{console}");
            assert!(console.contains("example"), "{console}");
            assert!(console.contains(value), "{console}");
        }
    }
}
