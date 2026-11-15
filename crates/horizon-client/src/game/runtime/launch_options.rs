//! The command line the game starts with (`Options`): what it asks for, read once at start,
//! then the game folder and the account it names.
use super::*;

#[derive(Clone)]
pub(crate) struct Options {
    pub(super) resources: PathBuf,
    pub(super) distance: i32,
    pub(super) distance_override: Option<i32>,
    pub(super) seed: i64,
    pub(super) developer_single_player: bool,
    pub(super) creative: bool,
    pub(super) test_terrain: bool,
    /// `--pregen`: enable the command even when the saved Extras switch is off.
    pub(super) pregen: bool,
    pub(super) frames: Option<u64>,
    /// A splitscreen player: the split window paces every player's frames, not each its own.
    pub(super) paced_by_split: bool,
    /// A splitscreen player other than P1: its windows are the split's (P1 alone decides
    /// fullscreen; this game neither asks for it nor adopts it into its options).
    pub(super) split_guest: bool,
    /// A splitscreen guest drawn in Window 1, the game's own window: its F11 leaves that
    /// window to P1.
    pub(super) split_in_main_window: bool,
    /// `--world-frames N`: exit after N frames drawn in the world (smoke runs; the world may load slowly).
    pub(super) world_frames: Option<u64>,
    pub(super) screenshot_after: Option<u64>,
    pub(super) timeout: Option<Duration>,
    pub(super) report: Option<PathBuf>,
    pub(super) world: Option<String>,
    pub(super) look: Option<(f32, f32)>,
    pub(super) time: Option<i64>,
    pub(super) weather: Option<String>,
    pub(super) dimension: Option<String>,
    /// Options.pauseOnLostFocus off (F3+P), for scripted testing.
    pub(super) no_pause: bool,
    /// Control socket path (`--control`), for `horizon-ctl`.
    pub(super) control: Option<PathBuf>,
    /// `HORIZON_SPIKES=MS`: the slow ring's threshold, with spans on every thread; nothing is logged.
    pub(super) spikes: Option<f32>,
    /// Render into a texture instead of the window (`--offscreen`), so a
    /// GPU the display cannot present from can still be benchmarked. The
    /// window stays hidden and never grabs the system pointer.
    pub(super) offscreen: bool,
    /// `--headless` (#4951): no display, no window and no renderer. The game runs its own
    /// loop (`game/runtime/headless.rs`) instead of winit's, so it starts where no display server
    /// is (an SSH session, a server). It counts as `offscreen` for everything else (an
    /// automated run that never shows a window or grabs the pointer).
    pub(super) headless: bool,
    /// `--width` and `--height` (vanilla's own arguments): the window's
    /// starting size, 1280x720 by default.
    pub(super) window_size: [u32; 2],
    /// Multiplayer: `--server HOST[:PORT]` joins at start; `--name` is the offline player name.
    pub(super) server: Option<String>,
    /// `--name`, `HORIZON_PLAYER_NAME` or the signed-in account's name; else
    /// [`Options::player_name`] uses the kept offline name.
    pub(super) player_name: Option<String>,
    /// `--dedicated`: run as a dedicated server; vanilla's server arguments
    /// (`--nogui`, `--port`, `--world`, `--universe`, `--initSettings`) follow.
    pub(super) dedicated: Option<Vec<String>>,
    /// `--transfers SETTINGS`: a dedicated server takes a proxy's hand-over requests
    /// (`server.toml`'s `[transfer]`, which horizon-server passes; lan/arrivals.rs).
    pub(super) transfers: Option<horizon_client::lan::arrivals::Settings>,
    /// `--benchmark SCENARIO`: run the benchmark, write its report and quit.
    pub(super) benchmark: Option<String>,
    /// `--benchmark-set SWITCH=VALUE`: a graphics choice for the travel benchmark.
    pub(super) benchmark_set: Option<(String, String)>,
    /// `--profile`: capture the benchmark run in a .hzperf file.
    pub(super) benchmark_profile: bool,
    /// Content selection and the compiled native runtime shared with resource loaders.
    pub(super) mods: mods::ModsState,
    /// What this Game executes; independent of its current server connection.
    pub(super) execution_role: execution_role::ExecutionRole,
    /// `--fullscreen` (vanilla's): the window opens fullscreen whatever options.txt says,
    /// as vanilla's DisplayData does, without saving it there.
    pub(super) fullscreen: bool,
    /// `--game-dir DIR`: the folder `read` makes the working directory (saves, options.txt...);
    /// none for a splitscreen player's game, which shares P1's working directory.
    pub(super) game_dir: Option<PathBuf>,
    /// `--quickPlaySingleplayer`, checked once the game folder is known.
    pub(super) quick_play: Option<String>,
    pub(super) session_file: Option<String>,
    /// `--session-stdin`: the session file's JSON comes on standard input instead.
    pub(super) session_stdin: bool,
}

impl Options {
    /// Whether this `Game` is the process's client start: not a dedicated server, and not
    /// a splitscreen guest, whose `run` is another local player joining the same process.
    pub(super) fn is_client_start(&self) -> bool {
        self.execution_role.is_client() && !self.split_guest
    }
    /// The name to join servers with: the one given, else the offline name kept
    /// next to options.txt (created on first use).
    pub(super) fn player_name(&self) -> String {
        self.player_name.clone().unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| horizon_client::offline_name::generate(0),
                |_| {
                    horizon_client::offline_name::load_or_create(
                        &horizon_client::logging::player_dir(),
                    )
                },
            )
        })
    }
    /// The command line, then what it asks of the process: the game folder, the account.
    /// A bad command line is a [`launch_api::UsageError`] (exit code 2, no crash report).
    pub(super) fn read(mut o: Self) -> Result<Self> {
        if o.pregen {
            horizon_client::command::vanilla::set_horizon_commands(true);
        }
        // The launcher's account: online-mode servers need it. A bad file
        // only leaves the game offline.
        let session = if o.developer_single_player {
            Ok(None)
        } else if o.session_stdin {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| e.to_string())
                .and_then(|_| horizon_client::auth::AuthSession::parse(&text))
                .map(Some)
        } else {
            o.session_file
                .take()
                .filter(|p| !p.is_empty())
                .map(|path| horizon_client::auth::AuthSession::load(path.as_ref()))
                .transpose()
        };
        match session {
            Ok(Some(session)) => {
                o.player_name = Some(session.name.clone());
                horizon_client::auth::set_current(session);
                // The chat key pair, ready before the first join.
                horizon_client::auth::prepare_chat_keys(true);
            }
            Ok(None) => {}
            Err(e) => eprintln!("Horizon: not signed in: {e}"),
        }
        if !o.developer_single_player {
            split::launcher::read_now();
        }
        if let Some(dir) = o.game_dir.take() {
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::env::set_current_dir(&dir))
                .map_err(|e| format!("game folder {}: {e}", dir.display()))?;
        }
        // Vanilla's quick play (Main, QuickPlay.joinSingleplayerWorld): only a
        // world that exists opens; else the title screen. --world also creates one.
        if let Some(world) = o.quick_play.take() {
            if std::env::current_dir()?
                .join("saves")
                .join(&world)
                .join("level.dat")
                .is_file()
            {
                o.world = Some(world);
            } else {
                eprintln!("Quick play: there is no world {world} in saves/");
            }
        }
        Ok(o)
    }
    /// Explicit timeouts apply to every run; only frame-limited runs get a default.
    pub(super) fn time_limit(&self) -> Option<Duration> {
        self.timeout
            .or(self.frames.map(|_| Duration::from_secs(120)))
    }
    /// The command line alone: no file is read and nothing changes. `None` for `--help` and
    /// `--launch-api`, which print and end the run.
    #[cfg(test)]
    pub(super) fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>> {
        Self::from_launch(LaunchArguments::parse(
            args,
            &std::env::current_dir()?,
            &horizon_client::logging::game_home(),
            std::env::var_os("HORIZON_HOME").as_deref(),
        )?)
    }
    pub(super) fn from_launch(launch: LaunchArguments) -> Result<Option<Self>> {
        let installation_side = launch.installation_side();
        let LaunchArguments { context, rest } = launch;
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                std::path::Path::new(".local/horizon-client").join(horizon_core::VANILLA_VERSION)
            },
            distance: 12,
            distance_override: None,
            seed: 0,
            developer_single_player: false,
            creative: false,
            test_terrain: false,
            pregen: false,
            frames: None,
            paced_by_split: false,
            split_guest: false,
            split_in_main_window: false,
            world_frames: None,
            screenshot_after: None,
            timeout: None,
            report: None,
            world: None,
            look: None,
            time: None,
            weather: None,
            dimension: None,
            no_pause: false,
            control: None,
            offscreen: false,
            headless: false,
            window_size: [1280, 720],
            spikes: std::env::var("HORIZON_SPIKES")
                .ok()
                .and_then(|v| v.parse().ok()),
            server: None,
            player_name: std::env::var("HORIZON_PLAYER_NAME")
                .ok()
                .filter(|n| !n.is_empty()),
            dedicated: None,
            transfers: None,
            benchmark: None,
            benchmark_set: None,
            benchmark_profile: false,
            mods: mods::ModsState {
                installation_side,
                overrides_dir: context.game_dir.join(horizon_mod_package::mod_list::DIR),
                dev: context.dev_mods,
                ..Default::default()
            },
            execution_role: execution_role::ExecutionRole::Client,
            fullscreen: false,
            game_dir: Some(context.game_dir),
            quick_play: None,
            session_file: std::env::var("HORIZON_SESSION_FILE").ok(),
            session_stdin: false,
        };
        for argument in rest {
            match argument {
                Argument::Switch(arg) => match arg.as_str() {
                    "--help" | "-h" => {
                        println!(
                            "Horizon — Minecraft Java Edition {} in Rust\n\n  --resources PATH        Prepared resources (horizon-prepare)\n  --render-distance N     Chunks, 2..128 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --width N, --height N   Window size (default 1280 x 720)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --game-tests            Enable test-only mod content (or HORIZON_GAME_TESTS=1/true)\n  --frames N              Exit after N frames (testing)\n  --world-frames N        Exit after N frames drawn in the world (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --benchmark SCENARIO    Run a benchmark (standard, village, forest, ocean, mountain, travel, ab:SWITCH), write its report to benchmarks/ and quit\n  --profile               With --benchmark: also write a performance capture that embeds the report\n  --benchmark-set S=V     With --benchmark travel: set gpu-culling, section-cull, section-sides or section-draws\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n  --server HOST[:PORT]    Join a multiplayer server (offline mode) at start\n  --quickPlaySingleplayer WORLD  Open this world of saves/ at start, only if it exists (vanilla's)\n  --quickPlayMultiplayer HOST[:PORT]  The same as --server (vanilla's name)\n  --name NAME             Player name for multiplayer (default: generated once, kept in horizon-player.txt)\n  --session-file PATH     Signed-in account from the launcher (read once, then deleted)\n  --session-stdin         The same account as JSON on standard input\n  --game-dir DIR          Game folder: saves, options.txt, packs, mods (default: the working directory)\n  --home DIR              Logs and crash reports (default $HORIZON_HOME, else the platform data folder)\n  --player-dir DIR        This player's own files when several play in one game folder: options.txt, logs, horizon-player.txt, servers.dat, screenshots\n  --fullscreen            Open the window fullscreen\n  --launch-api            Print the Launch API version and options as JSON (docs/api/launch.md)\n  --dev-mod PATH          A content mod (.hmod or folder) to develop: always on\n  --pregen                Add Horizon's /pregen command (docs/pregeneration.md)\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause.",
                            horizon_core::VANILLA_VERSION
                        );
                        return Ok(None);
                    }
                    "--launch-api" => {
                        println!("{}", launch_api::reply(&horizon_client::crash::version()));
                        return Ok(None);
                    }
                    "--fullscreen" => o.fullscreen = true,
                    "--profile" => o.benchmark_profile = true,
                    "--session-stdin" => o.session_stdin = true,
                    "--developer-single-player" => o.developer_single_player = true,
                    "--creative" => o.creative = true,
                    "--test-terrain" => o.test_terrain = true,
                    "--no-pause" => o.no_pause = true,
                    "--offscreen" => o.offscreen = true,
                    "--headless" => {
                        o.headless = true;
                        o.offscreen = true;
                    }
                    "--control" => o.control = Some(horizon_client::control::default_path()),
                    // Single player always runs on its server thread: a launcher's saved arguments
                    // may still name the old switches.
                    "--server-thread" | "--game-tests" => {}
                    "--in-frame" | "--no-server-thread" => eprintln!(
                        "Horizon: {arg} was removed: single player runs on its server thread."
                    ),
                    // Horizon's own commands (/pregen), off by default as on vanilla.
                    "--pregen" => {
                        o.pregen = true;
                    }
                    _ => return Err(format!("{arg}: unknown option").into()),
                },
                Argument::Dedicated(args) => {
                    o.execution_role = execution_role::ExecutionRole::DedicatedServer;
                    o.dedicated = Some(args);
                    break;
                }
                Argument::Value(arg, value) => {
                    // A launcher shows the error to the player: it names the option.
                    let read = || -> Result<()> {
                        match arg.as_str() {
                            "--resources" => o.resources = value.into(),
                            "--render-distance" | "--radius" => {
                                o.distance = value.parse()?;
                                o.distance_override = Some(o.distance);
                                let max = horizon_client::options_file::MAX_RENDER_DISTANCE;
                                if !(2..=max).contains(&o.distance) {
                                    return Err(format!("render distance must be 2..{max}").into());
                                }
                            }
                            "--seed" => {
                                o.seed = value.parse().unwrap_or_else(|_| {
                                    // Text seeds hash like String.hashCode.
                                    value
                                        .encode_utf16()
                                        .fold(0i32, |h, c| {
                                            h.wrapping_mul(31).wrapping_add(i32::from(c))
                                        })
                                        .into()
                                })
                            }
                            "--frames" => o.frames = Some(value.parse()?),
                            "--world-frames" => {
                                o.world_frames = Some(value.parse()?);
                                o.frames.get_or_insert(u64::MAX);
                            }
                            "--timeout-seconds" => {
                                o.timeout = Some(Duration::from_secs(value.parse()?))
                            }
                            "--smoke-report" => o.report = Some(value.into()),
                            "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                            "--width" => o.window_size[0] = value.parse::<u32>()?.max(1),
                            "--height" => o.window_size[1] = value.parse::<u32>()?.max(1),
                            // Vanilla opens only a level id that is one folder of saves/. --world, for
                            // harnesses, also takes a path (tools/bundle_tooltip_e2e.py).
                            "--world" => o.world = Some(value),
                            "--quickPlaySingleplayer" => {
                                let mut parts = std::path::Path::new(&value).components();
                                if !matches!(
                                    (parts.next(), parts.next()),
                                    (Some(std::path::Component::Normal(_)), None)
                                ) {
                                    return Err("not a folder name of saves/".into());
                                }
                                o.quick_play = Some(value);
                            }
                            // main has set this player's own folder (logging::set_player_dir).
                            "--player-dir" => {}

                            "--benchmark" => {
                                horizon_client::benchmark::Scenario::parse(&value)?;
                                o.benchmark = Some(value);
                            }
                            "--benchmark-set" => {
                                o.benchmark_set =
                                    Some(horizon_client::benchmark::parse_set(&value)?)
                            }
                            "--server" | "--quickPlayMultiplayer" => o.server = Some(value),
                            "--name" => o.player_name = Some(value),
                            "--session-file" => o.session_file = Some(value),
                            "--control-socket" => o.control = Some(value.into()),
                            "--transfers" => {
                                o.transfers =
                                    Some(horizon_client::lan::arrivals::Settings::parse(&value)?)
                            }
                            "--time" => o.time = Some(value.parse()?),
                            "--weather" => o.weather = Some(value),
                            "--dimension" => {
                                o.dimension = Some(if value.contains(':') {
                                    value
                                } else {
                                    format!("minecraft:{value}")
                                })
                            }
                            "--look" => {
                                let (y, p) =
                                    value.split_once(',').ok_or("--look takes YAW,PITCH")?;
                                o.look = Some((y.parse()?, p.parse()?));
                            }
                            _ => return Err("unknown option".into()),
                        }
                        Ok(())
                    };
                    read().map_err(|e| format!("{arg}: {e}"))?;
                }
            }
        }
        if o.benchmark_set.is_some() && o.benchmark.as_deref() != Some("travel") {
            return Err("--benchmark-set needs --benchmark travel".into());
        }
        if o.benchmark_profile && o.benchmark.is_none() {
            return Err("--profile needs --benchmark".into());
        }
        o.validate_developer_single_player()?;
        Ok(Some(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(args: &[&str]) -> Option<Duration> {
        Options::parse(args.iter().map(|arg| arg.to_string()))
            .unwrap()
            .unwrap()
            .time_limit()
    }

    /// Upstream's regression: a control-socket run must not outlive its explicit deadline.
    #[test]
    fn timeout_seconds_limits_every_run() {
        let secs = |n| Some(Duration::from_secs(n));
        assert_eq!(limit(&[]), None);
        assert_eq!(limit(&["--control-socket", "c.sock"]), None);
        assert_eq!(
            limit(&["--control-socket", "c.sock", "--timeout-seconds", "5"]),
            secs(5)
        );
        assert_eq!(limit(&["--timeout-seconds", "600"]), secs(600));
        assert_eq!(limit(&["--frames", "10"]), secs(120));
        assert_eq!(limit(&["--world-frames", "10"]), secs(120));
        assert_eq!(
            limit(&["--frames", "10", "--timeout-seconds", "30"]),
            secs(30)
        );
    }
}
