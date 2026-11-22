use super::*;

fn os(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

#[test]
fn variables_are_read_and_written_back() {
    let env = parse_env("MANGOHUD=1  DRI_PRIME=1\nNAME=\"two words\" EMPTY=").unwrap();
    assert_eq!(env["MANGOHUD"], "1");
    assert_eq!(env["NAME"], "two words");
    assert_eq!(env["EMPTY"], "");
    assert_eq!(parse_env(&env_line(&env)).unwrap(), env);
    assert_eq!(
        env_line(&env),
        "DRI_PRIME=1 EMPTY=\"\" MANGOHUD=1 NAME=\"two words\""
    );
    assert!(parse_env("JUSTANAME").unwrap_err().contains("NAME=value"));
    assert!(parse_env("1X=2")
        .unwrap_err()
        .contains("letters, digits and _"));
    assert!(parse_env("").unwrap().is_empty());
}

#[test]
fn the_java_heap_slider_reaches_the_computers_memory() {
    assert_eq!(memory_at(1., 16384), 16384);
    assert_eq!(memory_at(0., 16384), 1);
    assert_eq!(memory_at(0.25, 16384), 4096);
    assert_eq!(memory_at(0.5, 8000), 4032);
    assert_eq!(memory_fraction(16384, 16384), 1.);
    assert_eq!(memory_fraction(4096, 16384), 0.25);
    // Saved on a computer with more memory.
    assert_eq!(memory_fraction(32768, 16384), 1.);
}

#[test]
fn a_typed_memory_limit_is_read_in_mb_or_gb_with_no_lowest_value() {
    assert_eq!(parse_memory("2048"), Ok(Some(2048)));
    assert_eq!(parse_memory(" 512 MB "), Ok(Some(512)));
    assert_eq!(parse_memory("4gb"), Ok(Some(4096)));
    assert_eq!(parse_memory("2 G"), Ok(Some(2048)));
    assert_eq!(parse_memory("1"), Ok(Some(1)));
    assert_eq!(parse_memory(""), Ok(None));
    assert_eq!(parse_memory("off"), Ok(None));
    for bad in ["lots", "-5", "1.5gb", "4 tb", "99999999999", "9999999gb"] {
        assert_eq!(parse_memory(bad), Err(NotMemory), "{bad}");
    }
}

#[test]
fn a_summary_says_what_runs() {
    assert_eq!(Launch::default().summary(), None);
    let launch = Launch {
        pre_launch: "./sync.sh".into(),
        wrapper: "nice -n 5".into(),
        env: parse_env("A=1 B=2").unwrap(),
        ..Launch::default()
    };
    assert_eq!(
        launch.summary().unwrap(),
        "runs ./sync.sh first, through nice -n 5, with 2 variables"
    );
}

#[test]
fn each_system_gets_its_plan() {
    let game = os(&["/opt/horizon-client", "--world", "My World"]);
    let wrapper = Launch {
        wrapper: "nice -n 5".into(),
        ..Launch::default()
    };
    assert_eq!(
        plan(&wrapper, game.clone(), false, &Found::default()),
        Plan::Direct(os(&[
            "nice",
            "-n",
            "5",
            "/opt/horizon-client",
            "--world",
            "My World"
        ]))
    );
    let both = Launch {
        pre_launch: "echo hi".into(),
        ..wrapper.clone()
    };
    let Plan::Shell { script, words } = plan(&both, game.clone(), false, &Found::default()) else {
        panic!()
    };
    assert!(script.starts_with("(\necho hi\n) || "));
    assert!(script.ends_with("\nexec \"$@\""));
    assert_eq!(words[0], "nice");
    assert_eq!(
        plan(
            &both,
            os(&[r"C:\Horizon\horizon-client.exe", "--world", "My World"]),
            true,
            &Found::default()
        ),
        Plan::Cmd {
            line: "echo hi".into(),
            words: os(&[
                "nice",
                "-n",
                "5",
                r"C:\Horizon\horizon-client.exe",
                "--world",
                "My World"
            ])
        }
    );
}

/// On Windows the game's words never reach cmd: only the player's own pre-launch line is cmd's,
/// so a server address, a world's name or the game's folder with cmd's special characters can't
/// run anything.
#[test]
fn windows_game_words_never_reach_cmd() {
    let launch = Launch {
        pre_launch: "backup.bat".into(),
        ..Launch::default()
    };
    for special in ["&", "|", "^", "%", "\"", "<", ">", "(", ")", "!"] {
        let game = os(&[
            &format!(r"C:\Horizon {special} Co\horizon-client.exe"),
            "--world",
            &format!("a{special}b"),
            "--server",
            // cmd reads a comma as a space: no space for the quoting to catch.
            &format!(r"example.com{special}echo,PWNED>%TEMP%\horizon-poc.txt"),
        ]);
        let Plan::Cmd { line, .. } = plan(&launch, game, true, &Found::default()) else {
            panic!()
        };
        assert_eq!(line, "backup.bat", "{special}");
    }
}

#[test]
fn nothing_to_add_leaves_the_command_as_it_is() {
    let mut cmd = Command::new("/opt/game");
    cmd.arg("--x").env("A", "1");
    let out = wrap(cmd, &Launch::default()).unwrap();
    assert_eq!(out.get_program(), "/opt/game");
    assert_eq!(out.get_args().collect::<Vec<_>>(), ["--x"]);
}

#[test]
fn only_the_end_of_a_noisy_commands_output_is_kept() {
    let mut tail = hook::Tail::default();
    for i in 0..100_000 {
        tail.push(format!("line {i}\n").as_bytes());
    }
    assert_eq!(tail.len(), hook::KEPT);
    let text = tail.text();
    assert!(text.ends_with("line 99999"), "{text}");
    assert_eq!(text.lines().count(), 20);
    assert!(text.lines().all(|l| l.starts_with("line ")), "{text}");
}

#[test]
fn a_shell_wrapper_on_windows_is_refused_for_a_server_or_a_world() {
    let with = |wrapper: &str| Launch {
        wrapper: wrapper.into(),
        ..Launch::default()
    };
    let outside = [
        os(&["game", "--server", "example.com"]),
        os(&["game", "--world", "My World"]),
        os(&["java", "--quickPlayMultiplayer", "example.com"]),
    ];
    let plain = os(&["game", "--render-distance", "12"]);
    for shell in [
        "cmd /c",
        r"'C:\Windows\System32\CMD.EXE' /k",
        "powershell -Command",
        "pwsh.exe -c",
        "powershell",
        "powershell -File wrap.ps1",
        "pwsh -f wrap.ps1",
        // After -Command, -File is only more of the command.
        "powershell -Command Write-Output -File x",
        r"'C:\Program Files\PowerShell\7\PWSH.EXE' -File x",
        // Windows drops a name's trailing dots and spaces.
        "'cmd.exe.' /c",
        r"'C:\Windows\System32\CMD.EXE ' /c",
        "'pwsh. ' -c",
        "'powershell.exe..'",
    ] {
        for words in &outside {
            assert_eq!(refused(&with(shell), words, true), Some(SHELL_WRAPPER));
            assert_eq!(refused(&with(shell), words, false), None, "{shell}");
        }
        assert_eq!(refused(&with(shell), &plain, true), None, "{shell}");
    }
    for program in [
        "nice -n 5",
        r"'C:\Tools\run.bat'",
        "cmdtool",
        "pwsh-helper",
        "cmdx.exe",
        "mycmd.exe",
    ] {
        for words in &outside {
            assert_eq!(refused(&with(program), words, true), None, "{program}");
        }
    }
}

/// The tests that run a pre-launch command, one at a time: they count the output readers
/// running (`hook::READING`).
#[cfg(windows)]
static HOOKS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(windows)]
fn one_hook() -> std::sync::MutexGuard<'static, ()> {
    HOOKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new empty folder for one test.
#[cfg(windows)]
fn fresh(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-launch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Words with every character cmd or the C runtime reads specially, and a variable that is set
/// (`HORIZON_CANARY`): a game gets each as it is.
#[cfg(windows)]
const HOSTILE: [&str; 16] = [
    "--server",
    "example.com&echo,PWNED>poc.txt",
    "a&b",
    "a|b",
    "a^b",
    "a<b>c",
    "say \"hi\"",
    "\"",
    "\"&echo,PWNED>poc.txt",
    "%HORIZON_CANARY%",
    "!HORIZON_CANARY!",
    "(x);=,`~'",
    r"C:\trailing\",
    r#"a\"b"#,
    "two words",
    "",
];

/// The game of the tests below: this test binary, which writes the words after `--` to the file
/// `HORIZON_TEST_ARGV` names. Run on its own, it does nothing.
#[test]
fn argv_actor() {
    let Some(file) = std::env::var_os("HORIZON_TEST_ARGV") else {
        return;
    };
    let words: Vec<String> = std::env::args().skip_while(|w| w != "--").skip(1).collect();
    std::fs::write(file, serde_json::to_string(&words).unwrap()).unwrap();
}

/// A program the pre-launch command starts: it writes its process id to the file
/// `HORIZON_TEST_SLEEPER` names, then waits two minutes. Run on its own, it does nothing.
#[test]
fn sleeper_actor() {
    let Some(file) = std::env::var_os("HORIZON_TEST_SLEEPER") else {
        return;
    };
    let part = std::path::Path::new(&file).with_extension("part");
    std::fs::write(&part, std::process::id().to_string()).unwrap();
    std::fs::rename(&part, &file).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(120));
}

/// This test binary run as one of the actors above, with `words` after `--`.
#[cfg(windows)]
fn actor(name: &str, words: &[&str]) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args([
        "--include-ignored",
        "--exact",
        &format!("launch::tests::{name}"),
        "--",
    ])
    .args(words);
    c
}

#[cfg(windows)]
#[test]
fn the_pre_launch_command_runs_first_and_the_game_gets_its_words_as_they_are() {
    let _one = one_hook();
    let dir = fresh("argv");
    let mut game = actor("argv_actor", &HOSTILE);
    game.current_dir(&dir)
        .env("HORIZON_TEST_ARGV", dir.join("argv.json"))
        .env("HORIZON_CANARY", "expanded")
        .env("FROM_LAUNCHER", "yes")
        .env(crate::run_leases::GAME, "the sign-in");
    let launch = Launch {
        pre_launch: "echo %FROM_LAUNCHER% %FROM_INSTANCE% %HORIZON_LAUNCHER_GAME_COMMAND%> pre.txt"
            .into(),
        env: parse_env("FROM_INSTANCE=works").unwrap(),
        ..Launch::default()
    };
    let mut out = wrap(game, &launch).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("pre.txt")).unwrap(),
        "yes works %HORIZON_LAUNCHER_GAME_COMMAND%\r\n"
    );
    assert_eq!(out.get_current_dir(), Some(dir.as_path()));
    let ran = out.output().unwrap();
    assert!(ran.status.success(), "{ran:?}");
    let words: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("argv.json")).unwrap()).unwrap();
    assert_eq!(words, HOSTILE);
    assert!(!dir.join("poc.txt").exists());

    let failing = Launch {
        pre_launch: "echo nothing to back up 1>&2 & exit /b 3".into(),
        ..Launch::default()
    };
    let mut game = Command::new(dir.join("horizon-client.exe"));
    game.current_dir(&dir);
    let error = wrap(game, &failing).unwrap_err();
    assert!(
        error.contains("pre-launch command failed (exit 3)"),
        "{error}"
    );
    assert!(error.ends_with("nothing to back up"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Has process `pid` ended, or does it within `within`?
#[cfg(windows)]
fn ended(pid: u32, within: std::time::Duration) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    // SAFETY: the handle is checked before use and closed after.
    unsafe {
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return true;
        }
        let waited = WaitForSingleObject(process, within.as_millis() as u32);
        CloseHandle(process);
        waited == WAIT_OBJECT_0
    }
}

#[cfg(windows)]
#[test]
fn stop_ends_the_pre_launch_command_with_what_it_started() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _one = one_hook();
    let dir = fresh("stop");
    let pid_file = dir.join("sleeper.pid");
    // cmd waits for the sleeper, a program it started.
    let launch = Launch {
        pre_launch: format!(
            "\"{}\" --exact launch::tests::sleeper_actor",
            std::env::current_exe().unwrap().display()
        ),
        ..Launch::default()
    };
    let mut game = Command::new(dir.join("horizon-client.exe"));
    game.current_dir(&dir)
        .env("HORIZON_TEST_SLEEPER", &pid_file);
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stop_there = std::sync::Arc::clone(&stop);
    let start = std::thread::spawn(move || wrap_stoppable(game, &launch, &stop_there));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !pid_file.exists() {
        assert!(
            std::time::Instant::now() < until,
            "the sleeper never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    stop.store(true, Ordering::Relaxed);
    let stopped = start.join().unwrap();
    assert!(stopped.unwrap_err().contains("was stopped"));
    assert_eq!(hook::READING.load(Ordering::SeqCst), 0);
    assert!(
        ended(pid, std::time::Duration::from_secs(10)),
        "it outlived Stop"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The window's hand-off: Stop pressed once the pre-launch command had succeeded still keeps
/// the game from starting, and an earlier failure keeps its own message.
#[test]
fn a_start_stopped_after_its_pre_launch_command_does_not_start_the_game() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let stop = AtomicBool::new(false);
    assert_eq!(unless_stopped(Ok(1), None), Ok(1));
    assert_eq!(unless_stopped(Ok(1), Some(&stop)), Ok(1));
    stop.store(true, Ordering::Relaxed);
    assert_eq!(
        unless_stopped(Ok(1), Some(&stop)),
        Err("The pre-launch command was stopped, so the game did not start.".into())
    );
    assert_eq!(
        unless_stopped::<i32>(Err("no Java".into()), Some(&stop)),
        Err("no Java".into())
    );
}

/// A program the pre-launch command leaves running holds its output open: the game goes on
/// without waiting for it, and the output readers end.
#[cfg(windows)]
#[test]
fn a_program_left_running_does_not_hold_up_the_game() {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    let _one = one_hook();
    let dir = fresh("background");
    for (exit, starts) in [(0, true), (3, false)] {
        let pid_file = dir.join(format!("sleeper-{exit}.pid"));
        // The command ends once the sleeper, which holds its output, has started.
        let launch = Launch {
            pre_launch: format!(
                "start /b \"\" \"{}\" --exact launch::tests::sleeper_actor & for /l %i in \
                 (1,1,100000000) do @if exist \"{}\" exit /b {exit}",
                std::env::current_exe().unwrap().display(),
                pid_file.display()
            ),
            ..Launch::default()
        };
        let mut game = Command::new(dir.join("horizon-client.exe"));
        game.current_dir(&dir)
            .env("HORIZON_TEST_SLEEPER", &pid_file);
        let result = wrap(game, &launch);
        let since = std::fs::metadata(&pid_file)
            .and_then(|m| m.modified())
            .map(|started| started.elapsed().unwrap_or_default());
        assert_eq!(result.is_ok(), starts, "{result:?}");
        assert!(
            since
                .as_ref()
                .is_ok_and(|s| *s < Duration::from_millis(1500)),
            "{since:?}"
        );
        assert_eq!(hook::READING.load(Ordering::SeqCst), 0);
        // The sleeper isn't the pre-launch command's to end: the test ends it.
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        assert!(!ended(pid, Duration::ZERO), "the sleeper was ended");
        end(pid);
        // Gone before its folder is removed.
        assert!(ended(pid, Duration::from_secs(10)));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(windows)]
fn end(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: the handle is checked before use and closed after.
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}

#[cfg(windows)]
#[test]
fn a_noisy_pre_launch_command_fails_with_the_end_of_its_output() {
    let _one = one_hook();
    let dir = fresh("noisy");
    let launch = Launch {
        pre_launch: "(for /l %i in (1,1,4000) do @echo noisy line %i 1>&2) & exit /b 4".into(),
        ..Launch::default()
    };
    let mut game = Command::new(dir.join("horizon-client.exe"));
    game.current_dir(&dir);
    let error = wrap(game, &launch).unwrap_err();
    assert!(error.contains("failed (exit 4)"), "{error}");
    assert!(error.ends_with("noisy line 4000"), "{error}");
    assert_eq!(error.lines().count(), 21, "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A `.bat` wrapper gets the game's words quoted for cmd (Rust's standard library does that),
/// or Rust refuses to start it: none of them runs as a command.
#[cfg(windows)]
#[test]
fn a_bat_wrapper_gets_the_games_words_quoted_or_does_not_start() {
    let dir = fresh("bat");
    let bat = dir.join("wrap.bat");
    std::fs::write(&bat, "@echo(%*> \"%~dp0bat-args.txt\"\r\n").unwrap();
    let launch = Launch {
        wrapper: format!("'{}'", bat.display()),
        ..Launch::default()
    };
    let wrapped = |words: &[&str]| {
        let mut game = Command::new(dir.join("horizon-client.exe"));
        game.args(words)
            .current_dir(&dir)
            .env("HORIZON_CANARY", "expanded");
        wrap(game, &launch).unwrap().output()
    };
    let ran = wrapped(&HOSTILE).unwrap();
    assert!(ran.status.success(), "{ran:?}");
    let line = std::fs::read_to_string(dir.join("bat-args.txt")).unwrap();
    for word in HOSTILE.iter().filter(|w| !w.contains('"')) {
        assert!(line.contains(word), "{word} in {line}");
    }
    assert!(!line.contains("expanded"), "{line}");
    assert!(!dir.join("poc.txt").exists(), "{line}");
    // What cmd can't be given at all, Rust refuses.
    match wrapped(&["--server", "a\nb"]) {
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput),
        Ok(ran) => panic!("a line break reached the .bat: {ran:?}"),
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn the_wrapped_game_runs_after_its_pre_launch_command_in_its_folder() {
    let dir = std::env::temp_dir().join(format!("horizon-launch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut game = Command::new("/bin/sh");
    // The "game" prints its words and two variables: one from the launcher, one the instance's.
    game.args([
        "-c",
        "echo \"$1|$2|$FROM_LAUNCHER|$FROM_INSTANCE\"",
        "game",
        "a b",
        "$HOME",
    ])
    .current_dir(&dir)
    .env("FROM_LAUNCHER", "yes");
    let launch = Launch {
        pre_launch: "echo ran > pre.txt".into(),
        wrapper: "/usr/bin/env WRAPPED=1".into(),
        env: parse_env("FROM_INSTANCE=\"it works\"").unwrap(),
        ..Launch::default()
    };
    let out = wrap(game, &launch).unwrap().output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "a b|$HOME|yes|it works\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("pre.txt")).unwrap(),
        "ran\n"
    );

    let failing = Launch {
        pre_launch: "exit 3".into(),
        ..Launch::default()
    };
    let mut game = Command::new("/bin/sh");
    game.args(["-c", "echo started > started.txt"])
        .current_dir(&dir);
    let out = wrap(game, &failing).unwrap().output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("pre-launch command failed (exit 3)"));
    assert!(!dir.join("started.txt").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_linux_switches_act_only_where_their_tools_are() {
    let all = Found {
        gamemode: true,
        mangohud: true,
        two_gpus: true,
        nvidia: false,
    };
    let launch = Launch {
        wrapper: "nice".into(),
        gamemode: true,
        mangohud: true,
        dedicated_gpu: true,
        ..Launch::default()
    };
    assert_eq!(
        plan(&launch, os(&["game"]), false, &all),
        Plan::Direct(os(&["gamemoderun", "mangohud", "nice", "game"]))
    );
    assert_eq!(switch_env(&launch, &all), [("DRI_PRIME", "1")]);
    let nvidia = Found {
        nvidia: true,
        ..all
    };
    assert_eq!(
        switch_env(&launch, &nvidia),
        [
            ("__NV_PRIME_RENDER_OFFLOAD", "1"),
            ("__VK_LAYER_NV_optimus", "NVIDIA_only"),
            ("__GLX_VENDOR_LIBRARY_NAME", "nvidia")
        ]
    );
    // Not installed (or not Linux): switched on, but nothing is added.
    let none = Found::default();
    assert_eq!(
        plan(&launch, os(&["game"]), false, &none),
        Plan::Direct(os(&["nice", "game"]))
    );
    assert!(switch_env(&launch, &none).is_empty());
    assert!(!none.any() && all.any());
    assert_eq!(
        launch.summary().unwrap(),
        "through nice, with GameMode, with MangoHud, on the dedicated GPU"
    );
    // Off switches aren't written.
    let text = serde_json::to_string(&Launch::default()).unwrap();
    assert_eq!(text, "{}");
}

#[test]
fn the_post_exit_command_runs_with_the_instance_variables() {
    let dir = std::env::temp_dir().join(format!("horizon-post-exit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(after_exit(&Launch::default(), &dir, &[]).is_none());
    let launch = Launch {
        post_exit: if cfg!(windows) {
            // The redirection first: `1> after.txt` would redirect stdout and drop the 1.
            "> after.txt echo %INST_NAME% %INST_ID% %INST_JAVA% %MINE%".into()
        } else {
            "echo \"$INST_NAME $INST_ID $INST_JAVA $MINE\" > after.txt".into()
        },
        env: parse_env("MINE=1").unwrap(),
        ..Launch::default()
    };
    assert_eq!(
        launch.summary().unwrap(),
        format!("runs {} after, with 1 variable", launch.post_exit)
    );
    let vars = instance_vars("My Pack", "my-pack", &dir, Some(OsStr::new("java")));
    let names: Vec<&str> = vars.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        names,
        [
            "INST_NAME",
            "INST_ID",
            "INST_DIR",
            "INST_MC_DIR",
            "INST_JAVA"
        ]
    );
    assert!(after_exit(&launch, &dir, &vars)
        .unwrap()
        .status()
        .unwrap()
        .success());
    let written = std::fs::read_to_string(dir.join("after.txt")).unwrap();
    assert_eq!(written.trim(), "My Pack my-pack java 1");
    // A Horizon instance has no Java.
    assert!(!instance_vars("H", "horizon", &dir, None)
        .iter()
        .any(|(k, _)| *k == "INST_JAVA"));
    let _ = std::fs::remove_dir_all(&dir);
}
