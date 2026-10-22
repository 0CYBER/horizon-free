//! `horizon-launcher mcp`: the Model Context Protocol over stdin and stdout (JSON-RPC 2.0, one
//! message per line), so AI agents can install, prepare and run Horizon. Every tool goes through
//! `backend`. A server started here lives as long as the session. The running game's own tools
//! come from its `horizon-ctl` as `game_*` (game_tools.rs).
use crate::game_tools::{self, GameTools};
use horizon_launcher::account::{self, Accounts, SignIn, SignInState};
use horizon_launcher::backend::{self, Job, Logged};
use horizon_launcher::telemetry;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

pub const PROTOCOL: &str = "2025-06-18";

struct Session {
    server: Option<Logged>,
    client: Option<Logged>,
    sign_in: Option<SignIn>,
    /// The selected version's `horizon-ctl mcp`, once a version is installed.
    game: Option<GameTools>,
}

fn tools() -> Value {
    let version = json!({ "type": "string", "description": "An installed version tag such as v2026.10.13. Default: the selected version." });
    let none = json!({ "type": "object", "properties": {} });
    json!([
        { "name": "list_versions", "description": "List Horizon releases on GitHub (newest first) and the versions installed here, with which is selected and which are prepared.",
          "inputSchema": { "type": "object", "properties": { "installed_only": { "type": "boolean", "description": "Skip GitHub and list only installed versions." } } } },
        { "name": "install_version", "description": "Download and install a Horizon release for this platform. Takes a minute or two.",
          "inputSchema": { "type": "object", "properties": { "tag": { "type": "string", "description": "A release tag such as v2026.10.13, or \"latest\"." } }, "required": ["tag"] } },
        { "name": "remove_version", "description": "Remove an installed Horizon version and its prepared resources.",
          "inputSchema": { "type": "object", "properties": { "tag": { "type": "string" } }, "required": ["tag"] } },
        { "name": "prepare_resources", "description": "Prepare a version's resources from the player's own Minecraft install (needs the Minecraft release that version is built for, played once in the official launcher, and its Java; versions before horizon-prepare also need Python 3.10+). Can take several minutes; returns the end of the log.",
          "inputSchema": { "type": "object", "properties": { "version": version, "minecraft_dir": { "type": "string", "description": "The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App, ATLauncher or GDLauncher. Default: the launcher's setting, then every launcher's default folder." } } } },
        { "name": "launch_client", "description": "Start the Horizon client (a game window on the player's screen) under agent control: the game_* tools then play it. Returns once it has started; game_wait_ready waits for the world.",
          "inputSchema": { "type": "object", "properties": { "version": version,
            "world": { "type": "string", "description": "Open or create this world (a folder under saves) at once." },
            "creative": { "type": "boolean", "description": "Creative mode for a new world." },
            "offscreen": { "type": "boolean", "description": "Render without showing a window (game_screenshot still works)." },
            "args": { "type": "array", "items": { "type": "string" }, "description": "Extra game arguments, passed through as-is (for example \"--server-thread\")." } } } },
        { "name": "start_server", "description": "Start a Horizon dedicated server. It runs until stop_server or the end of this MCP session. Requires agreeing to the Minecraft EULA.",
          "inputSchema": { "type": "object", "properties": { "version": version, "dir": { "type": "string", "description": "The server's folder. Default: the launcher's server folder." }, "accept_eula": { "type": "boolean", "description": "Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA) on the player's behalf. Only with their consent." } } } },
        { "name": "server_command", "description": "Send a console command to the running server (for example \"list\" or \"say hello\"), and return the lines it printed in reply.",
          "inputSchema": { "type": "object", "properties": { "command": { "type": "string" } }, "required": ["command"] } },
        { "name": "server_log", "description": "The most recent lines of the server's console.",
          "inputSchema": { "type": "object", "properties": { "lines": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "How many lines (default 50)." } } } },
        { "name": "stop_server", "description": "Stop the running server cleanly (it saves the world), killing it if it does not stop within 30 seconds.", "inputSchema": none },
        { "name": "patch_notes", "description": "A release's patch notes as markdown.",
          "inputSchema": { "type": "object", "properties": { "tag": { "type": "string", "description": "Default: the newest release." } } } },
        { "name": "links", "description": "The Horizon links: the Discord invite, the GitHub Sponsors page and the website (an empty one is not offered).", "inputSchema": none },
        { "name": "menu_style", "description": "Show or set the game's menu style. \"horizon\" is Horizon's own menus, \"classic\" is vanilla's screens, which have no way back inside the game. The game reads it when it starts.",
          "inputSchema": { "type": "object", "properties": { "style": { "type": "string", "enum": ["horizon", "classic"], "description": "Omit to show the current style." } } } },
        { "name": "logs", "description": "Read the game's log: the last lines of logs/latest.log in the game folder, or with crash the newest crash report (what the game wrote when it last crashed). Read-only.",
          "inputSchema": { "type": "object", "properties": { "lines": { "type": "integer", "minimum": 1, "maximum": 5000, "description": "How many lines of latest.log (default 100)." }, "crash": { "type": "boolean", "description": "Return the newest crash report instead." } } } },
        { "name": "accounts", "description": "The Microsoft accounts signed in to the launcher (names and UUIDs, never tokens), which one the game plays as (none = offline), and how a sign-in started with sign_in is going.", "inputSchema": none },
        { "name": "sign_in", "description": "Start signing in a Microsoft account: returns a link and a code the player enters in their browser. The sign-in finishes in the background; check it with accounts. Only when the player asks to sign in.", "inputSchema": none },
        { "name": "sign_out", "description": "Sign an account out of the launcher (its tokens leave the keychain).",
          "inputSchema": { "type": "object", "properties": { "account": { "type": "string", "description": "Its name or UUID." } }, "required": ["account"] } },
        { "name": "select_account", "description": "Choose the account the game plays as, or \"offline\" to play without one.",
          "inputSchema": { "type": "object", "properties": { "account": { "type": "string", "description": "A name or UUID, or \"offline\"." } }, "required": ["account"] } },
        { "name": "check_updates", "description": "Check GitHub for a Horizon release newer than every installed version, and for a newer launcher.", "inputSchema": none },
        { "name": "data_collection", "description": "The player's opt-in data collection: which kinds they allow (performance, crashes, bugs, benchmarks; all off until they answer), the install ID, how many items wait, and the data server. Read-only.", "inputSchema": none },
        { "name": "set_data_collection", "description": "Turn kinds of data collection on or off. Only when the player asks: it records their consent.",
          "inputSchema": { "type": "object", "properties": { "kinds": { "type": "array", "items": { "type": "string", "enum": ["performance", "crashes", "bugs", "benchmarks", "all"] } }, "on": { "type": "boolean" } }, "required": ["kinds", "on"] } },
        { "name": "data_preview", "description": "Exactly what would be sent (the JSON), or an example of each kind when nothing waits. Sends nothing.", "inputSchema": none },
        { "name": "delete_my_data", "description": "Ask the server to delete everything sent from this install, remove what waits, and take a new install ID. Only when the player asks.", "inputSchema": none },
        { "name": "report_bug", "description": "Send a bug report in the player's words, only once they have agreed to send it (bug reports must be allowed). With include_log the end of the game's log goes too, scrubbed and without chat: show it to the player first (the logs tool). Returns exactly what was kept.",
          "inputSchema": { "type": "object", "properties": { "text": { "type": "string" }, "include_log": { "type": "boolean" } }, "required": ["text"] } },
    ])
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

impl Session {
    fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        if matches!(name, "list_versions" | "check_updates") {
            backend::reuse_prepared_everywhere();
        }
        match name {
            "list_versions" => {
                let installed = backend::installed();
                let mut out = json!({
                    "installed": installed.iter().map(|t| json!({ "tag": t, "prepared": backend::is_prepared(t), "sounds": backend::has_sounds(t) })).collect::<Vec<_>>(),
                    "selected": backend::resolve_version(None).ok(),
                });
                if !args
                    .get("installed_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    out["releases"] = backend::releases()?
                        .iter()
                        .map(|r| json!({ "tag": r.tag, "name": r.name, "date": r.date, "channel": if r.beta { "beta" } else { "stable" }, "available_here": r.asset.is_some() }))
                        .collect();
                }
                Ok(serde_json::to_string_pretty(&out).unwrap())
            }
            "install_version" => {
                let tag = backend::install_tag(
                    arg_str(args, "tag").ok_or("tag is required")?,
                    false,
                    |_| {},
                )?;
                Ok(if backend::is_prepared(&tag) {
                    format!("Installed {tag}. It is now the selected version, prepared already from another installed version.")
                } else {
                    format!("Installed {tag}. It is now the selected version; prepare it before playing.")
                })
            }
            "remove_version" => {
                let tag = arg_str(args, "tag").ok_or("tag is required")?;
                backend::remove_version(tag)?;
                Ok(format!("Removed {tag}."))
            }
            "prepare_resources" => {
                let tag = backend::resolve_version(arg_str(args, "version"))?;
                let minecraft =
                    backend::minecraft_dir(arg_str(args, "minecraft_dir").map(PathBuf::from))?;
                let (preparer, minecraft) = backend::preparer(&tag, &minecraft, &|_| {})?;
                let sound = std::cell::RefCell::new(Vec::new());
                backend::sounds(&tag, &minecraft, &|line| sound.borrow_mut().push(line));
                let (result, tail) = match preparer {
                    backend::Preparer::Process(cmd) => {
                        let run = Logged::start(Job::Prepare, cmd)?;
                        let result = run.wait();
                        (result, run.tail(20).join("\n"))
                    }
                    backend::Preparer::InProcess(options) => {
                        let lines = std::cell::RefCell::new(std::collections::VecDeque::new());
                        let result = backend::prepare_in_process(&options, &|line| {
                            let mut lines = lines.borrow_mut();
                            lines.push_back(line);
                            if lines.len() > 20 {
                                lines.pop_front();
                            }
                        });
                        let tail = lines
                            .into_inner()
                            .into_iter()
                            .collect::<Vec<_>>()
                            .join("\n");
                        (result, tail)
                    }
                };
                let tail = format!("{}\n{tail}", sound.into_inner().join("\n"));
                result
                    .map(|_| format!("{tag} is prepared.\n{tail}"))
                    .map_err(|e| format!("{e}\n{tail}"))
            }
            "launch_client" => {
                if self.client.as_ref().is_some_and(|c| c.finished().is_none()) {
                    return Err("The client is already running.".into());
                }
                let tag = backend::resolve_version(arg_str(args, "version"))?;
                let mut cmd = backend::client(&tag)?;
                cmd.args(game_tools::control_args());
                if let Some(world) = arg_str(args, "world") {
                    cmd.arg("--world").arg(world);
                }
                for flag in ["creative", "offscreen"] {
                    if args.get(flag).and_then(Value::as_bool).unwrap_or(false) {
                        cmd.arg(format!("--{flag}"));
                    }
                }
                if let Some(extra) = args.get("args").and_then(Value::as_array) {
                    cmd.args(extra.iter().filter_map(Value::as_str));
                }
                let client = Logged::start(Job::Client, cmd)?;
                // Started once its control socket answers, as horizon-ctl launch waits.
                let deadline = std::time::Instant::now() + Duration::from_secs(120);
                while !game_tools::listening() {
                    if let Some(result) = client.finished() {
                        return Err(format!(
                            "The client stopped: {}\n{}",
                            result.err().unwrap_or_default(),
                            client.tail(20).join("\n")
                        ));
                    }
                    if std::time::Instant::now() > deadline {
                        return Err(format!(
                            "The client did not open its control socket within 2 minutes.\n{}",
                            client.tail(20).join("\n")
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                self.client = Some(client);
                Ok(format!(
                    "Started the {tag} client. The game_* tools play it; game_wait_ready waits until the world is ready."
                ))
            }
            "start_server" => {
                if self.server.as_ref().is_some_and(|s| s.finished().is_none()) {
                    return Err("A server is already running. Stop it first.".into());
                }
                let tag = backend::resolve_version(arg_str(args, "version"))?;
                let dir = backend::server_dir(arg_str(args, "dir").map(PathBuf::from));
                let accept = args
                    .get("accept_eula")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let server = Logged::start(Job::Server, backend::server(&tag, &dir, accept)?)?;
                std::thread::sleep(Duration::from_secs(3));
                let tail = server.tail(15).join("\n");
                if let Some(result) = server.finished() {
                    return Err(format!(
                        "The server stopped: {}\n{tail}",
                        result.err().unwrap_or_default()
                    ));
                }
                self.server = Some(server);
                Ok(format!(
                    "Started the {tag} server in {}.\n{tail}",
                    dir.display()
                ))
            }
            "server_command" => {
                let server = self.running_server()?;
                let before = server.log.lock().unwrap().len();
                let command = arg_str(args, "command").ok_or("command is required")?;
                server.running.send(command.trim_start_matches('/'));
                std::thread::sleep(Duration::from_millis(700));
                let log = server.log.lock().unwrap();
                let reply: Vec<_> = log.iter().skip(before.min(log.len())).cloned().collect();
                Ok(if reply.is_empty() {
                    "(no output yet)".into()
                } else {
                    reply.join("\n")
                })
            }
            "server_log" => {
                let server = self
                    .server
                    .as_ref()
                    .ok_or("No server was started in this session.")?;
                let lines = args.get("lines").and_then(Value::as_u64).unwrap_or(50) as usize;
                let state = if server.finished().is_some() {
                    "stopped"
                } else {
                    "running"
                };
                Ok(format!(
                    "[server {state}]\n{}",
                    server.tail(lines).join("\n")
                ))
            }
            "stop_server" => {
                let mut server = self
                    .server
                    .take()
                    .ok_or("No server was started in this session.")?;
                server.stop(Duration::from_secs(30));
                Ok(format!(
                    "The server stopped.\n{}",
                    server.tail(5).join("\n")
                ))
            }
            "patch_notes" => {
                let r = backend::release_notes(arg_str(args, "tag"))?;
                Ok(format!(
                    "# {} ({}, {})\n\n{}",
                    if r.name.is_empty() { &r.tag } else { &r.name },
                    r.tag,
                    r.date,
                    r.body
                ))
            }
            "links" => Ok(serde_json::to_string_pretty(&backend::links()).unwrap()),
            "logs" => {
                let lines = args.get("lines").and_then(Value::as_u64).unwrap_or(100);
                let crash = args.get("crash").and_then(Value::as_bool).unwrap_or(false);
                let (path, text) = backend::read_logs(lines.clamp(1, 5000) as usize, crash)?;
                Ok(format!("{}\n\n{text}", path.display()))
            }
            "menu_style" => {
                if let Some(name) = arg_str(args, "style") {
                    let style = backend::MenuStyle::parse(name)
                        .ok_or("style is \"horizon\" or \"classic\"")?;
                    backend::set_menu_style(style)?;
                }
                Ok(format!("Menu style: {}", backend::menu_style().name()))
            }
            "data_collection" => Ok(serde_json::to_string_pretty(&telemetry::status_json())
                .map_err(|e| e.to_string())?),
            "set_data_collection" => {
                let kinds: Vec<String> = args
                    .get("kinds")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let on = args
                    .get("on")
                    .and_then(Value::as_bool)
                    .ok_or("on is required")?;
                telemetry::set(&telemetry::parse_kinds(&kinds)?, on)?;
                Ok(serde_json::to_string_pretty(&telemetry::status_json())
                    .map_err(|e| e.to_string())?)
            }
            "data_preview" => {
                let (what, text) = telemetry::preview();
                Ok(format!("{what}\n{text}"))
            }
            "delete_my_data" => telemetry::delete_my_data(),
            "report_bug" => {
                let text = arg_str(args, "text").ok_or("text is required")?;
                let with_log = args
                    .get("include_log")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let item = telemetry::bug_report(text, with_log)?;
                let done = telemetry::submit(&item)?;
                Ok(format!(
                    "{done}\n{}",
                    serde_json::to_string_pretty(&item).map_err(|e| e.to_string())?
                ))
            }
            "check_updates" => {
                let u = backend::check_updates()?;
                Ok(serde_json::to_string_pretty(&json!({
                    "horizon_update": u.horizon.as_ref().map(|r| &r.tag),
                    "installed": backend::installed(),
                    "launcher_update": u.launcher.as_ref().and_then(|r| backend::launcher_version(&r.tag)),
                    "launcher_version": backend::VERSION,
                }))
                .unwrap())
            }
            "accounts" => {
                let accounts = Accounts::load();
                let sign_in = self.sign_in.as_ref().map(|s| match s.state() {
                    SignInState::Starting => json!({ "state": "starting" }),
                    SignInState::Code(c) => json!({ "state": "waiting for the player", "link": c.verification_uri, "code": c.user_code }),
                    SignInState::Done(Ok(a)) => json!({ "state": "signed in", "name": a.name }),
                    SignInState::Done(Err(e)) => json!({ "state": "failed", "error": e }),
                });
                Ok(serde_json::to_string_pretty(&json!({
                    "accounts": accounts.accounts.iter().map(|a| json!({ "name": a.name, "uuid": a.uuid })).collect::<Vec<_>>(),
                    "plays_as": accounts.selected().map(|a| a.name.clone()),
                    "sign_in_configured": account::client_id().is_some(),
                    "keychain": account::store().persistent(),
                    "sign_in": sign_in,
                }))
                .unwrap())
            }
            "sign_in" => {
                account::client_id().ok_or(account::NOT_CONFIGURED)?;
                if let Some(s) = self.sign_in.take() {
                    s.cancel();
                }
                let sign_in = SignIn::start();
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                let state = loop {
                    match sign_in.state() {
                        SignInState::Starting if std::time::Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(100))
                        }
                        state => break state,
                    }
                };
                self.sign_in = Some(sign_in);
                match state {
                    SignInState::Code(c) => Ok(format!(
                        "Ask the player to open {} and enter the code {}. Then check accounts.",
                        c.verification_uri, c.user_code
                    )),
                    SignInState::Done(Err(e)) => Err(e),
                    _ => Err("Microsoft did not answer. Try again.".into()),
                }
            }
            "sign_out" | "select_account" => {
                let who = arg_str(args, "account").ok_or("account is required")?;
                let mut accounts = Accounts::load();
                if name == "select_account" && who.eq_ignore_ascii_case("offline") {
                    accounts.selected = None;
                    accounts.save()?;
                    return Ok("The game plays offline.".into());
                }
                let a = accounts
                    .find(who)
                    .cloned()
                    .ok_or_else(|| format!("No signed-in account {who}."))?;
                if name == "sign_out" {
                    accounts.remove(account::store(), &a.uuid)?;
                    accounts.save()?;
                    Ok(format!("Signed out {}.", a.name))
                } else {
                    accounts.selected = Some(a.uuid);
                    accounts.save()?;
                    Ok(format!("The game plays as {}.", a.name))
                }
            }
            _ => Err(format!("Unknown tool {name}")),
        }
    }

    /// The game's tools, starting the selected version's horizon-ctl the first time.
    fn game(&mut self) -> Option<&mut GameTools> {
        if self.game.is_none() {
            self.game = backend::resolve_version(None)
                .and_then(|tag| GameTools::start(&tag))
                .ok();
        }
        self.game.as_mut()
    }

    fn running_server(&mut self) -> Result<&mut Logged, String> {
        match &mut self.server {
            Some(s) if s.finished().is_none() => Ok(s),
            _ => Err("No server is running. Start one with start_server.".into()),
        }
    }

    fn handle(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "horizon-launcher", "version": backend::VERSION },
                "instructions": "Manage Horizon, a Rust reimplementation of Minecraft Java Edition: install versions, prepare them from the player's own Minecraft, start the client or a dedicated server. Once launch_client started the game, the game_* tools play it: game_state, game_ui (every widget of the open screen), game_click, game_goto, game_screenshot...",
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut list = tools().as_array().cloned().unwrap_or_default();
                if let Some(game) = self.game() {
                    list.extend(game.tools().iter().cloned());
                }
                Ok(json!({ "tools": list }))
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "name is required".to_string()))?;
                if name.starts_with(game_tools::PREFIX) {
                    let args = params.get("arguments").cloned().unwrap_or(json!({}));
                    let game = self.game().ok_or((
                        -32602,
                        "No installed version with horizon-ctl: install one first".to_string(),
                    ))?;
                    if !game.tools().iter().any(|t| t["name"] == name) {
                        return Err((-32602, format!("Unknown tool {name}")));
                    }
                    return game.call(name, &args).map_err(|e| (-32603, e));
                }
                if !tools()
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["name"] == name)
                {
                    return Err((-32602, format!("Unknown tool {name}")));
                }
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let (text, error) = match self.call(name, &args) {
                    Ok(text) => (text, false),
                    Err(e) => (e, true),
                };
                Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": error }))
            }
            _ => Err((-32601, format!("Method not found: {method}"))),
        }
    }
}

pub fn serve() -> Result<(), String> {
    let mut session = Session {
        server: None,
        client: None,
        sign_in: None,
        game: None,
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
            ),
            Ok(msg) => {
                let method = msg
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                // Notifications (no id) get no reply.
                msg.get("id").cloned().map(|id| match session.handle(method, &params) {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
                })
            }
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")
                .and_then(|_| stdout.flush())
                .map_err(|e| e.to_string())?;
        }
    }
    // The session is over: the server it started goes with it.
    if let Some(mut server) = session.server.take() {
        server.stop(Duration::from_secs(30));
    }
    Ok(())
}
