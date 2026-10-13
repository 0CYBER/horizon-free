//! `horizon-launcher mcp`: the Model Context Protocol over stdin and stdout (JSON-RPC 2.0, one
//! message per line), so AI agents can install, prepare and run Horizon. Every tool goes through
//! `backend`. A server started here lives as long as the session.
use horizon_launcher::backend::{self, Job, Logged};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

const PROTOCOL: &str = "2025-06-18";

struct Session {
    server: Option<Logged>,
    client: Option<Logged>,
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
        { "name": "prepare_resources", "description": "Prepare a version's resources from the player's own Minecraft install (needs Minecraft 26.3 played once in the official launcher, Java 25 and Python 3.10+). Can take several minutes; returns the end of the log.",
          "inputSchema": { "type": "object", "properties": { "version": version, "minecraft_dir": { "type": "string", "description": "The official launcher's game folder. Default: the launcher's setting, else the platform default." } } } },
        { "name": "launch_client", "description": "Start the Horizon client (a game window on the player's screen). Returns once it has started.",
          "inputSchema": { "type": "object", "properties": { "version": version } } },
        { "name": "start_server", "description": "Start a Horizon dedicated server. It runs until stop_server or the end of this MCP session. Requires agreeing to the Minecraft EULA.",
          "inputSchema": { "type": "object", "properties": { "version": version, "dir": { "type": "string", "description": "The server's folder. Default: the launcher's server folder." }, "accept_eula": { "type": "boolean", "description": "Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA) on the player's behalf. Only with their consent." } } } },
        { "name": "server_command", "description": "Send a console command to the running server (for example \"list\" or \"say hello\"), and return the lines it printed in reply.",
          "inputSchema": { "type": "object", "properties": { "command": { "type": "string" } }, "required": ["command"] } },
        { "name": "server_log", "description": "The most recent lines of the server's console.",
          "inputSchema": { "type": "object", "properties": { "lines": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "How many lines (default 50)." } } } },
        { "name": "stop_server", "description": "Stop the running server cleanly (it saves the world), killing it if it does not stop within 30 seconds.", "inputSchema": none },
        { "name": "patch_notes", "description": "A release's patch notes as markdown.",
          "inputSchema": { "type": "object", "properties": { "tag": { "type": "string", "description": "Default: the newest release." } } } },
        { "name": "roadmap", "description": "What's planned next for Horizon, from the README's roadmap.", "inputSchema": none },
        { "name": "links", "description": "The Horizon links: the Discord invite, the GitHub Sponsors page and the website (an empty one is not offered).", "inputSchema": none },
        { "name": "menu_style", "description": "Show or set the game's menu style. \"horizon\" is Horizon's own menus, \"classic\" is vanilla's screens, which have no way back inside the game. The game reads it when it starts.",
          "inputSchema": { "type": "object", "properties": { "style": { "type": "string", "enum": ["horizon", "classic"], "description": "Omit to show the current style." } } } },
        { "name": "check_updates", "description": "Check GitHub for a Horizon release newer than every installed version, and for a newer launcher.", "inputSchema": none },
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
                    "installed": installed.iter().map(|t| json!({ "tag": t, "prepared": backend::is_prepared(t) })).collect::<Vec<_>>(),
                    "selected": backend::resolve_version(None).ok(),
                });
                if !args
                    .get("installed_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    out["releases"] = backend::releases()?
                        .iter()
                        .map(|r| json!({ "tag": r.tag, "name": r.name, "date": r.date, "available_here": r.asset.is_some() }))
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
                let cmd = backend::prepare_command(&tag, &minecraft, &|_| {})?;
                let run = Logged::start(Job::Prepare, cmd)?;
                let result = run.wait();
                let tail = run.tail(20).join("\n");
                result
                    .map(|_| format!("{tag} is prepared.\n{tail}"))
                    .map_err(|e| format!("{e}\n{tail}"))
            }
            "launch_client" => {
                if self.client.as_ref().is_some_and(|c| c.finished().is_none()) {
                    return Err("The client is already running.".into());
                }
                let tag = backend::resolve_version(arg_str(args, "version"))?;
                let client = Logged::start(Job::Client, backend::client(&tag)?)?;
                std::thread::sleep(Duration::from_secs(2));
                if let Some(Err(e)) = client.finished() {
                    return Err(format!("{e}\n{}", client.tail(20).join("\n")));
                }
                self.client = Some(client);
                Ok(format!("Started the {tag} client."))
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
            "roadmap" => backend::roadmap(),
            "links" => Ok(serde_json::to_string_pretty(&backend::links()).unwrap()),
            "menu_style" => {
                if let Some(name) = arg_str(args, "style") {
                    let style = backend::MenuStyle::parse(name)
                        .ok_or("style is \"horizon\" or \"classic\"")?;
                    backend::set_menu_style(style)?;
                }
                Ok(format!("Menu style: {}", backend::menu_style().name()))
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
            _ => Err(format!("Unknown tool {name}")),
        }
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
                "instructions": "Manage Horizon, a Rust reimplementation of Minecraft Java Edition: install versions, prepare them from the player's own Minecraft, start the client or a dedicated server.",
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "name is required".to_string()))?;
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
