//! Bootstrap capture -> real registration -> real Game hosts, with the compiled wasm guest.
use crate::game::{Game, Options, api_values, execution_role::ExecutionRole, mods};
use horizon_mod_package::selection::LaunchArguments;
use mod_api::{Dist, Hooks, Host, ModEntry, PackageInfo, RegistrationContext, RuntimeEnvironment};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Observed {
    registration: Vec<RuntimeEnvironment>,
    later: Vec<RuntimeEnvironment>,
    messages: Vec<mod_api::Text>,
    tests: usize,
}
struct Probe(Arc<Mutex<Observed>>);
fn from_host(host: &dyn Host, tests: bool) -> RuntimeEnvironment {
    RuntimeEnvironment::new(
        host.dist(),
        host.game_version().to_owned(),
        host.horizon_version().to_owned(),
        tests,
    )
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, _: &mut dyn mod_api::Registrar, cx: &RegistrationContext) {
        self.0
            .lock()
            .unwrap()
            .registration
            .push(RuntimeEnvironment::new(
                cx.dist(),
                cx.game_version().to_owned(),
                cx.horizon_version().to_owned(),
                cx.game_tests_enabled(),
            ));
    }
    fn client_tick_pre(
        &self,
        host: &mut dyn mod_api::ClientHost,
        _: &mut mod_api::ClientTickPreEvent,
    ) {
        // The flag has only a server Host method; registration already pinned it.
        let mut observed = self.0.lock().unwrap();
        let tests = observed.registration[0].game_tests_enabled();
        observed.later.push(from_host(host, tests));
    }
    fn server_started(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerStartedEvent,
    ) {
        self.0
            .lock()
            .unwrap()
            .later
            .push(from_host(host, host.game_tests_enabled()));
    }
    fn register_game_tests(&self, _: &dyn Host, _: &mut mod_api::RegisterGameTestsEvent<'_>) {
        self.0.lock().unwrap().tests += 1;
    }
    fn broadcast_message(
        &self,
        _: &dyn Host,
        event: &mut mod_api::BroadcastMessageEvent<'_>,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().messages.push(event.line.clone());
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "requires prepared HORIZON_TEST_RESOURCES"]
fn native_process_environment_in_registration_and_game_hosts() {
    run(false);
}

#[test]
#[ignore = "requires prepared HORIZON_TEST_RESOURCES and rebuilt HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn native_and_wasm_process_environment_in_registration_and_game_hosts() {
    horizon_guest_stack::run_on_guest_stack(|| run(true));
}

fn run(wasm: bool) {
    if std::env::var_os("HORIZON_RUNTIME_ENV_CASE").is_none() {
        let test = if wasm {
            "native_and_wasm_process_environment_in_registration_and_game_hosts"
        } else {
            "native_process_environment_in_registration_and_game_hosts"
        };
        let resources = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resource root required"),
        )
        .canonicalize()
        .unwrap();
        let owner = tempfile::tempdir().unwrap();
        let cases = [
            "client_off",
            "client_flag",
            "client_env_one",
            "client_env_true",
            "client_env_false",
            "dedicated_off",
            "dedicated_before",
            "dedicated_after",
            "dedicated_env_one",
            "dedicated_operand",
            "integrated_off",
            "integrated_flag",
            "integrated_env_true",
        ];
        let run = |case: &str| {
            let scratch = owner.path().join(case);
            std::fs::create_dir(&scratch).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    &format!("game::native_hooks::runtime_environment::tests::{test}"),
                    "--nocapture",
                ])
                .current_dir(&scratch)
                .env("HORIZON_RUNTIME_ENV_CASE", case)
                .env("HORIZON_TEST_RESOURCES", &resources)
                .env("HORIZON_NO_AUDIO", "1")
                .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
                .env_remove("HORIZON_GAME_TESTS");
            for key in [
                "HORIZON_HOME",
                "HORIZON_LAUNCHER_HOME",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
            ] {
                command.env(key, &scratch);
            }
            if case.ends_with("env_one") {
                command.env("HORIZON_GAME_TESTS", "1");
            } else if case.ends_with("env_true") {
                command.env("HORIZON_GAME_TESTS", "true");
            } else if case.ends_with("env_false") {
                command.env("HORIZON_GAME_TESTS", "false");
            }
            command.output().unwrap()
        };
        // The wasm case runs alone in the fixture job: four cases at a time there, each still in its
        // own process. The native case shares the slow shards' memory, so one at a time.
        for group in cases.chunks(if wasm { 4 } else { 1 }) {
            let run = &run;
            let outputs: Vec<_> = std::thread::scope(|scope| {
                let running: Vec<_> = group
                    .iter()
                    .map(|&case| scope.spawn(move || run(case)))
                    .collect();
                running
                    .into_iter()
                    .map(|child| child.join().unwrap())
                    .collect()
            });
            for (case, output) in group.iter().zip(outputs) {
                assert!(
                    output.status.success(),
                    "{case}: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                    "child must execute the exact acceptance test, not zero tests"
                );
            }
        }
        return;
    }
    let case = std::env::var("HORIZON_RUNTIME_ENV_CASE").unwrap();
    let dedicated = case.starts_with("dedicated");
    let enabled = case.ends_with("flag")
        || case.ends_with("before")
        || case.ends_with("after")
        || case.ends_with("env_one")
        || case.ends_with("env_true");
    let mut args = Vec::new();
    if case.ends_with("flag") || case.ends_with("before") {
        args.push("--game-tests".to_owned());
    }
    if dedicated {
        args.push("--dedicated".to_owned());
    }
    if case.ends_with("after") {
        args.push("--game-tests".to_owned());
    }
    if case.ends_with("operand") {
        args.extend(["--world".to_owned(), "--game-tests".to_owned()]);
    }
    let cwd = std::env::current_dir().unwrap();
    let launch = LaunchArguments::parse(args.into_iter(), &cwd, &cwd, None).unwrap();
    let options = Options::from_launch(launch.clone()).unwrap().unwrap();
    if case.ends_with("operand") {
        assert_eq!(
            options.dedicated.as_deref().unwrap(),
            ["--world", "--game-tests"]
        );
    }
    // The same entry capture used before the executable loads any code.
    crate::entry::runtime_environment::capture(&launch);
    let expected = RuntimeEnvironment::new(
        if dedicated {
            Dist::DedicatedServer
        } else {
            Dist::Client
        },
        horizon_core::VANILLA_VERSION.to_owned(),
        crate::crash::version(),
        enabled,
    );
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut entries = vec![ModEntry::new(
        "runtime_probe",
        Box::new(Probe(observed.clone())),
    )];
    let mut inventory = vec![PackageInfo::new(
        "runtime_probe".into(),
        "1.0.0".into(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec!["runtime_probe".into()],
        vec![],
    )];
    // Retain the loaded code for the entire registration and host dispatch lifetime.
    let mut loaded = wasm.then(|| {
        use horizon_mod_package::manifest::Package;
        let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
        let mut manifest = Package::read(&path).unwrap().manifest;
        manifest.inject.clear();
        inventory.push(PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            vec![],
        ));
        let loaded = horizon_mod_wasm::load(
            &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
            vec![horizon_mod_wasm::WasmPackage {
                manifest,
                module: Package::read_wasm(&path).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        loaded.initialize(&inventory).unwrap();
        loaded
    });
    if let Some(loaded) = &mut loaded {
        entries.append(&mut loaded.mods);
    }
    api_values::install_test_platform();
    mod_platform::__private::install_packages(&inventory, &[]);
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    // Freeze the real bootstrap snapshot before with_mods runs registration hooks.
    mods::install_registration_data(&root).unwrap();
    mod_platform::testing::with_mods(entries.into(), || {
        let role = if dedicated {
            ExecutionRole::DedicatedServer
        } else if case.starts_with("integrated") {
            ExecutionRole::IntegratedServer
        } else {
            ExecutionRole::Client
        };
        crate::game::native_entity_refs::tests::with_game_role(
            root,
            role,
            |game: &mut Game<'_>| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                if role.is_client() {
                    game.native_client_tick_pre();
                    // A client process's integrated logical server must read that same snapshot.
                    game.options.execution_role = ExecutionRole::IntegratedServer;
                }
                game.native_server_started(dedicated);
                game.native_register_game_tests(&mut mod_api::GameTestFunctions::default());
            },
        );
    });
    let observed = observed.lock().unwrap();
    assert!(
        !observed.registration.is_empty(),
        "Hooks::register must execute"
    );
    assert!(observed.registration.iter().all(|value| value == &expected));
    assert_eq!(
        observed.later.len(),
        if case.starts_with("client") { 2 } else { 1 },
        "client Host and logical server Host hooks must each execute"
    );
    assert!(observed.later.iter().all(|value| value == &expected));
    assert_eq!(
        observed.tests, 1,
        "game-test registration runs with the flag off and on"
    );
    if wasm {
        assert_eq!(
            observed.messages,
            [mod_api::Text::literal(format!(
                "runtime_environment|{:?}|{}|{}|{}|{}",
                expected.dist(),
                expected.game_version(),
                expected.horizon_version(),
                expected.game_tests_enabled(),
                horizon_core::version::DATA_VERSION,
            ))],
            "compiled guest must finish registration and later Host reads without a trap"
        );
    }
}

#[test]
fn normal_options_preserve_dedicated_flag_valued_operands() {
    let cwd = std::env::current_dir().unwrap();
    for option in ["world", "universe", "port", "serverId", "pidFile"] {
        let tail = vec![format!("--{option}"), "--game-tests".to_owned()];
        let launch = LaunchArguments::parse(
            std::iter::once("--dedicated".to_owned()).chain(tail.clone()),
            &cwd,
            &cwd,
            None,
        )
        .unwrap();
        let options = Options::from_launch(launch).unwrap().unwrap();
        assert_eq!(options.dedicated.as_ref(), Some(&tail));
        assert_eq!(
            crate::entry::dedicated_arguments::parse(options.dedicated.as_ref().unwrap())
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [(option, Some("--game-tests"))],
        );
    }
    let options = Options::parse(["--world", "--game-tests"].map(str::to_owned).into_iter())
        .unwrap()
        .unwrap();
    assert_eq!(options.world.as_deref(), Some("--game-tests"));
}
