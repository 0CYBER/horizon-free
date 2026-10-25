//! Connecting an AI agent (docs/agent-control.md): the launcher's MCP server, `horizon-launcher
//! mcp`, as each common client wants it configured. `horizon-launcher agent-config` prints it and
//! the Settings page's "Connect an AI agent" shows it. Nothing here runs a client's command: the
//! window runs one only when the player clicks its button, and shows it first.
use serde_json::{json, Value};
use std::path::Path;

/// The name the server is added under in every client.
pub const SERVER_NAME: &str = "horizon";

pub struct Client {
    /// `claude-code`, `codex`, `cursor` or `generic`.
    pub id: &'static str,
    pub name: &'static str,
    /// Where its configuration lives.
    pub file: &'static str,
    /// What goes in that file.
    pub config: String,
    /// The client's own command that adds the server, when it has a CLI: (program, arguments).
    pub add: Option<(&'static str, Vec<String>)>,
}

impl Client {
    /// The add command as a shell line, to show and to copy.
    pub fn add_line(&self) -> Option<String> {
        self.add
            .as_ref()
            .map(|(program, args)| shell_line(program, args))
    }
}

/// The `mcpServers` JSON most clients read (Claude Code's .mcp.json, Cursor, Claude Desktop...).
fn mcp_servers_json(exe: &Path) -> String {
    let config = json!({ "mcpServers": { SERVER_NAME: { "command": exe, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The clients, for the launcher at `exe`.
pub fn clients(exe: &Path) -> Vec<Client> {
    let exe_text = exe.display().to_string();
    let add = |args: &[&str]| -> Vec<String> {
        let mut words: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        words.extend([exe_text.clone(), "mcp".into()]);
        words
    };
    vec![
        Client {
            id: "claude-code",
            name: "Claude Code",
            file: "~/.claude.json (or .mcp.json in a project)",
            config: mcp_servers_json(exe),
            add: Some((
                "claude",
                add(&["mcp", "add", "--scope", "user", SERVER_NAME, "--"]),
            )),
        },
        Client {
            id: "codex",
            name: "Codex",
            file: "~/.codex/config.toml",
            config: format!(
                "[mcp_servers.{SERVER_NAME}]\ncommand = {}\nargs = [\"mcp\"]\n",
                toml_string(&exe_text)
            ),
            add: Some(("codex", add(&["mcp", "add", SERVER_NAME, "--"]))),
        },
        Client {
            id: "cursor",
            name: "Cursor",
            file: "~/.cursor/mcp.json",
            config: mcp_servers_json(exe),
            add: None,
        },
        Client {
            id: "generic",
            name: "Any MCP client",
            file: "its MCP settings (a stdio server)",
            config: mcp_servers_json(exe),
            add: None,
        },
    ]
}

/// Where a client's CLI is: on the PATH, else in the folders installers use (a launcher started
/// from the desktop gets a short PATH on macOS). The bare name when it is nowhere.
pub fn locate(program: &str) -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    let names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd"]
            .iter()
            .map(|e| format!("{program}.{e}"))
            .collect()
    } else {
        vec![program.to_owned()]
    };
    let extra = [
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".cargo/bin"),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        home.join("AppData/Roaming/npm"),
    ];
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
        .unwrap_or_else(|| program.into())
}

/// Starts `exe mcp` as a client would, lists its tools and returns how many answered: the
/// Settings page's "Test the connection".
pub fn self_test(exe: &Path) -> Result<usize, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = crate::process::command(exe)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
    let lines = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "horizon-launcher-test", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    ];
    if let Some(mut stdin) = child.stdin.take() {
        for line in &lines {
            writeln!(stdin, "{line}").map_err(|e| e.to_string())?;
        }
        // Dropping stdin ends the session: the server answers, then exits.
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|r| r["id"] == 2)
        .and_then(|r| r["result"]["tools"].as_array().map(Vec::len))
        .ok_or_else(|| "The MCP server did not list its tools.".into())
}

/// `program args...` quoted for a POSIX shell where a word needs it.
pub fn shell_line(program: &str, args: &[String]) -> String {
    let quote = |w: &str| {
        if !w.is_empty()
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c))
        {
            w.to_owned()
        } else {
            format!("'{}'", w.replace('\'', "'\\''"))
        }
    };
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Everything `agent-config --json` prints.
pub fn json(exe: &Path) -> Value {
    json!({
        "server": { "name": SERVER_NAME, "command": exe, "args": ["mcp"] },
        "clients": clients(exe).iter().map(|c| json!({
            "id": c.id, "name": c.name, "file": c.file, "config": c.config, "add": c.add_line(),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_name_the_launcher_and_quote_its_path() {
        let exe = Path::new("/Applications/Horizon Launcher.app/Contents/MacOS/horizon-launcher");
        let all = clients(exe);
        let claude = &all[0];
        assert_eq!(
            claude.add_line().unwrap(),
            "claude mcp add --scope user horizon -- '/Applications/Horizon Launcher.app/Contents/MacOS/horizon-launcher' mcp"
        );
        let parsed: Value = serde_json::from_str(&claude.config).unwrap();
        assert_eq!(parsed["mcpServers"]["horizon"]["args"], json!(["mcp"]));
        assert_eq!(
            parsed["mcpServers"]["horizon"]["command"],
            exe.display().to_string()
        );
        let codex = &all[1];
        assert!(codex
            .config
            .contains("command = \"/Applications/Horizon Launcher.app"));
        assert_eq!(
            toml_string(r#"C:\Horizon\"x".exe"#),
            r#""C:\\Horizon\\\"x\".exe""#
        );
        assert!(all.iter().any(|c| c.id == "cursor" && c.add.is_none()));
    }
}
