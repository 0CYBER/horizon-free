//! The game's side of the Launch API (docs/api/launch.md): the options another launcher may pass,
//! `--launch-api`'s reply, the exit codes, and docs/api/launch-reference.md, generated from them.
//! Update it with: `HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --lib launch_api`.

/// The Launch API's version (docs/api/stability.md): an addition makes the next patch.
pub const VERSION: &str = "0.1.1";

/// One option of the contract: its name, its value's placeholder (none for a flag), what it does.
pub struct LaunchOption {
    pub name: &'static str,
    pub value: Option<&'static str>,
    /// For the reference only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub what: &'static str,
}

const fn option(
    name: &'static str,
    value: Option<&'static str>,
    what: &'static str,
) -> LaunchOption {
    LaunchOption { name, value, what }
}

/// What a launcher may pass. The game takes more (`--help` lists them), but only these are
/// promised: they keep their meaning within a line of this API.
pub const OPTIONS: &[LaunchOption] = &[
    option(
        "--developer-single-player",
        None,
        "Restricts this run to account-free single-player: no session loading, external connections or LAN hosting. Not a sandbox for native mods.",
    ),
    option(
        "--resources",
        Some("DIR"),
        "The prepared resources (horizon-prepare's output). Default: `resources` beside the executable, else `.local/horizon-client/26.3`.",
    ),
    option(
        "--game-dir",
        Some("DIR"),
        "The game folder: `saves/`, `options.txt`, `resourcepacks/`, `horizon-mods/`, `screenshots/`. Created when missing. Default: the working directory. `--gameDir` is the same.",
    ),
    option(
        "--home",
        Some("DIR"),
        "Logs (`logs/latest.log`) and crash reports (`crash-reports/`). Default: `HORIZON_HOME`, else `horizon` in the platform data folder.",
    ),
    option(
        "--player-dir",
        Some("DIR"),
        "One player's own files, for several games playing in one game folder at the same time: `options.txt` and the settings beside it, `logs/`, `crash-reports/`, `horizon-player.txt`, `servers.dat`, `hotbar.nbt`, `screenshots/`. Saves, packs, mods and their config, the chat's command history and Play with Friends' relay settings stay in the game folder. A relative path is taken from the folder the game starts in. Created when missing. Default: the game folder, and `--home` for logs. `horizon-ctl logs` and the launcher read `--home`'s logs.",
    ),
    option(
        "--width",
        Some("N"),
        "The window's starting width (default 1280).",
    ),
    option(
        "--height",
        Some("N"),
        "The window's starting height (default 720).",
    ),
    option(
        "--fullscreen",
        None,
        "Opens the window fullscreen, whatever `options.txt` says; `options.txt` is not changed.",
    ),
    option(
        "--session-file",
        Some("PATH"),
        "The signed-in account, as the session JSON below. Read once, then deleted.",
    ),
    option(
        "--session-stdin",
        None,
        "The same JSON on standard input, so it never touches the disk.",
    ),
    option(
        "--quickPlaySingleplayer",
        Some("WORLD"),
        "Opens this world of `saves/` at start, only if it exists (else the title screen). WORLD is one folder name: a path is refused.",
    ),
    option(
        "--quickPlayMultiplayer",
        Some("HOST[:PORT]"),
        "Joins this server at start.",
    ),
    option(
        "--launch-api",
        None,
        "Prints the reply below and exits 0, before anything else runs.",
    ),
];

/// How the game ends, for the launcher that started it.
pub mod exit_code {
    /// The player quit, or `--help` and `--launch-api` printed.
    pub const OK: i32 = 0;
    /// The game stopped on an error; a crash report was saved.
    pub const ERROR: i32 = 1;
    /// The command line was refused (an unknown option, a missing or bad value); no crash report.
    pub const USAGE: i32 = 2;
    /// A panic; a crash report was saved.
    pub const PANIC: i32 = 101;
}

const EXIT_CODES: &[(i32, &str)] = &[
    (
        exit_code::OK,
        "The player quit, or `--help` or `--launch-api` printed.",
    ),
    (
        exit_code::ERROR,
        "The game stopped on an error. A crash report was saved.",
    ),
    (
        exit_code::USAGE,
        "The command line was refused: an unknown option, a missing or bad value. No crash report.",
    ),
    (
        exit_code::PANIC,
        "The game panicked. A crash report was saved.",
    ),
];

/// A command line the game refuses: exit code 2, no crash report.
#[derive(Debug)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// What content this build loads (docs/api/content.md), from the constants it checks content
/// against, so a launcher offers only what will work.
fn content() -> serde_json::Value {
    use horizon_mod_package::manifest;
    serde_json::json!({
        // 0.1 was the WebAssembly code line: no build runs it any more.
        "mod": { "api": [manifest::API] },
        "native_mod": { "exact_build": true, "rebuild": true },
        // The line it draws: a pack for another is refused.
        "shader": { "schema": horizon_shader_api::SCHEMA, "api": [horizon_shader_api::API] },
        "resourcepack": { "format": horizon_core::version::RESOURCE_PACK_FORMAT },
        "datapack": { "format": horizon_core::version::DATA_PACK_FORMAT },
        "modpack": { "format": horizon_mod_package::modpack::FORMAT },
        "index": { "schema": horizon_mod_package::index::SCHEMA },
    })
}

/// `--launch-api`'s reply for a build of this `horizon` version.
pub fn reply(horizon: &str) -> String {
    serde_json::to_string_pretty(&reply_value(horizon)).expect("a JSON value")
}

fn reply_value(horizon: &str) -> serde_json::Value {
    let options: Vec<_> = OPTIONS
        .iter()
        .map(|o| serde_json::json!({ "name": o.name, "value": o.value }))
        .collect();
    serde_json::json!({
        "launch_api": VERSION,
        "horizon": horizon,
        "minecraft": horizon_core::VANILLA_VERSION,
        "options": options,
        "exit_codes": EXIT_CODES.iter().map(|(code, _)| code).collect::<Vec<_>>(),
        "crash_report_line": horizon_client::crash::SAVED_LINE,
        "content": content(),
    })
}

/// The reply with a placeholder for the game build version.
#[cfg(test)]
fn reference_reply() -> String {
    let reply = reply_value("<version>");
    serde_json::to_string_pretty(&reply).expect("a JSON value")
}

/// docs/api/launch-reference.md.
#[cfg(test)]
pub fn markdown() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "<!-- Generated from crates/horizon-client/src/game/runtime/launch_api.rs; a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --lib launch_api -->"
    );
    let _ = writeln!(w, "# Launch API reference\n");
    let _ = writeln!(
        w,
        "What `horizon-client` promises a launcher that starts it, Launch API {VERSION}. The guide: \
         [launch.md](launch.md); stability and versions: [stability.md](stability.md).\n"
    );
    let _ = writeln!(w, "## Options\n");
    let _ = writeln!(
        w,
        "Any other option is refused with exit code 2, so ask `--launch-api` before passing one \
         a build may not know.\n"
    );
    let _ = writeln!(w, "| Option | What it does |\n| --- | --- |");
    for o in OPTIONS {
        let usage = match o.value {
            Some(value) => format!("`{} {value}`", o.name),
            None => format!("`{}`", o.name),
        };
        let _ = writeln!(w, "| {usage} | {} |", o.what);
    }
    let _ = writeln!(w, "\n## `--launch-api`\n");
    let _ = writeln!(
        w,
        "Prints one JSON object on stdout and exits 0. `horizon` is the build's version; readers \
         ignore fields they don't know.\n"
    );
    let _ = writeln!(w, "```json\n{}\n```\n", reference_reply());
    let _ = writeln!(
        w,
        "`content` is what this build loads ([content.md](content.md)), so a launcher offers only \
         what works on it: content \
         mods of the manifest `api` lines in `mod.api`; native mods built for this exact game build \
         (`native_mod.exact_build`), which need the game rebuilt (`rebuild`); shader packs of \
         `shader.schema` and the lines in `shader.api` (a pack for another is refused); resource \
         and data packs of `format` (`[major, minor]`; others still load, marked too old or too \
         new as vanilla marks them); modpacks of `modpack.format`; the mod index of \
         `index.schema`.\n"
    );
    let _ = writeln!(w, "## Exit codes\n\n| Code | Meaning |\n| --- | --- |");
    for (code, what) in EXIT_CODES {
        let _ = writeln!(w, "| {code} | {what} |");
    }
    let _ = writeln!(
        w,
        "\nWhen a crash report is saved, the last line on stderr is `{}` followed by the \
         report's path.",
        horizon_client::crash::SAVED_LINE.trim_end()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> horizon_client::Result<Option<super::super::Options>> {
        super::super::Options::parse(args.iter().map(|a| a.to_string()))
    }

    /// Every promised option is one the game accepts.
    #[test]
    fn launch_api_options_are_accepted() {
        for o in OPTIONS {
            let args: Vec<&str> = std::iter::once(o.name)
                .chain(o.value.map(|v| if v == "N" { "100" } else { "x" }))
                .collect();
            assert!(parse(&args).is_ok(), "{} is refused", o.name);
        }
    }

    #[test]
    fn launch_api_flags_are_read() {
        let o = parse(&["--fullscreen", "--game-dir", "g", "--session-stdin"])
            .unwrap()
            .unwrap();
        assert!(o.fullscreen && o.session_stdin);
        assert_eq!(o.game_dir, Some(std::env::current_dir().unwrap().join("g")));
        assert!(parse(&["--launch-api"]).unwrap().is_none());
        assert!(parse(&["--no-such-option"]).is_err());
    }

    #[test]
    #[ignore = "daily"]
    fn developer_offline_never_consumes_inherited_sessions() {
        if let Some(file) = std::env::var_os("OFFLINE_SESSION_TEST_FILE") {
            assert!(parse(&["--developer-single-player"]).is_err());
            assert_eq!(std::fs::read(file).unwrap(), b"unconsumed session sentinel");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("session.json");
        std::fs::write(&file, b"unconsumed session sentinel").unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "game::launch_api::tests::developer_offline_never_consumes_inherited_sessions",
                "--nocapture",
            ])
            .env("OFFLINE_SESSION_TEST_FILE", &file)
            .env("HORIZON_SESSION_FILE", &file)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(
            std::fs::read(&file).unwrap(),
            b"unconsumed session sentinel"
        );
    }

    #[test]
    fn developer_single_player_rejects_network_and_account_intents() {
        let game = parse(&["--developer-single-player", "--world", "DeveloperProof"])
            .unwrap()
            .unwrap();
        assert!(game.developer_single_player);
        assert!(game.execution_role.is_client());
        for args in [
            vec!["--developer-single-player", "--server", "localhost"],
            vec![
                "--developer-single-player",
                "--quickPlayMultiplayer",
                "localhost",
            ],
            vec![
                "--developer-single-player",
                "--session-file",
                "account.json",
            ],
            vec!["--developer-single-player", "--session-stdin"],
            vec!["--developer-single-player", "--dedicated"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        assert!(parse(&["--server", "localhost"]).is_ok());
    }

    #[test]
    fn game_uses_the_command_selection_context() {
        use horizon_mod_package::selection::{LaunchArguments, content_packages};
        use std::{collections::BTreeSet, fs, process::Command};
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("launch-selection-{}-{nonce}", std::process::id()));
        for (file, id) in [
            ("horizon-mods/base.hmod", "base"),
            ("other/horizon-mods/other.hmod", "other"),
            ("workspace/dev.hmod", "dev"),
        ] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n"
            );
            fs::write(
                path,
                horizon_mod_package::Package::write_files(&manifest, &[]).unwrap(),
            )
            .unwrap();
        }
        let enabled = BTreeSet::from(["base".into(), "other".into()]);
        struct Case<'a> {
            args: &'a [&'a str],
            environment: Option<&'a str>,
            home: &'a str,
            ids: &'a [&'a str],
        }
        for case in [
            Case {
                args: &[],
                environment: None,
                home: "default",
                ids: &["base"],
            },
            Case {
                args: &[],
                environment: Some("environment"),
                home: "environment",
                ids: &["base"],
            },
            Case {
                args: &["--home", "cache"],
                environment: Some("environment"),
                home: "cache",
                ids: &["base"],
            },
            Case {
                args: &["--game-dir", "other"],
                environment: None,
                home: "default",
                ids: &["other"],
            },
            Case {
                args: &["--dev-mod", "workspace/dev.hmod"],
                environment: None,
                home: "default",
                ids: &["base", "dev"],
            },
        ] {
            let mut command = Command::new("horizon-client");
            command.current_dir(&root).args(case.args);
            if let Some(home) = case.environment {
                command.env("HORIZON_HOME", home);
            } else {
                command.env_remove("HORIZON_HOME");
            }
            let launch = LaunchArguments::parse(
                command.get_args().map(|a| a.to_str().unwrap().to_string()),
                command.get_current_dir().unwrap(),
                &root.join("default"),
                command
                    .get_envs()
                    .find_map(|(name, value)| if name == "HORIZON_HOME" { value } else { None }),
            )
            .unwrap();
            assert_eq!(launch.context.home, root.join(case.home));
            let game = super::super::Options::from_launch(launch).unwrap().unwrap();
            let selected: Vec<_> =
                content_packages(game.game_dir.as_deref().unwrap(), &enabled, &game.mods.dev)
                    .map(|p| p.package.manifest.id)
                    .collect();
            assert_eq!(selected, case.ids);
        }
        fs::remove_dir_all(root).unwrap();
        let game = parse(&["--resources", "--home", "--creative"])
            .unwrap()
            .unwrap();
        assert_eq!(game.resources, std::path::Path::new("--home"));
        assert!(game.creative);
        assert!(parse(&["--help", "--missing"]).unwrap().is_none());
    }

    /// A launcher shows the error to the player, so it names the option.
    #[test]
    fn launch_api_bad_value_names_its_option() {
        let error = |args: &[&str]| parse(args).err().unwrap().to_string();
        assert_eq!(
            error(&["--width", "wide"]),
            "--width: invalid digit found in string"
        );
        assert_eq!(error(&["--bogus", "x"]), "--bogus: unknown option");
    }

    /// Quick play opens one folder of saves/, as on vanilla: nothing outside it.
    #[test]
    fn launch_api_quick_play_is_one_folder_of_saves() {
        for name in ["../x", "a/b", "/abs", ".", ".."] {
            assert!(
                parse(&["--quickPlaySingleplayer", name]).is_err(),
                "{name} is accepted"
            );
        }
        assert!(parse(&["--quickPlaySingleplayer", "E2E World"]).is_ok());
    }

    /// The content a build says it loads is what its checks take: content mods of api 0.2,
    /// never the removed 0.1, and the shader line it draws.
    #[test]
    fn launch_api_content_is_what_the_build_loads() {
        let reply: serde_json::Value = serde_json::from_str(&reply("x")).unwrap();
        let content = &reply["content"];
        let mods = content["mod"]["api"].as_array().unwrap();
        assert!(
            mods.contains(&"0.2".into()) && !mods.contains(&"0.1".into()),
            "{mods:?}"
        );
        assert_eq!(
            content["shader"]["api"],
            serde_json::json!([horizon_shader_api::API])
        );
        assert_eq!(content["native_mod"]["rebuild"], true);
        assert_eq!(
            content["resourcepack"]["format"],
            serde_json::json!(horizon_core::version::RESOURCE_PACK_FORMAT)
        );
    }

    /// docs/api/launch-reference.md is what `markdown` writes.
    #[test]
    fn launch_api_reference_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("docs/api/launch-reference.md");
        let want = markdown();
        if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            have == want,
            "docs/api/launch-reference.md is out of date: HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --lib launch_api"
        );
    }
}
