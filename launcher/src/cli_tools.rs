//! Every launcher CLI command as an MCP tool, generated from the CLI's own definition (clap) so the
//! two can't drift: the tool `mods_search` is `horizon-launcher mods search`, its arguments are the
//! command's (snake_case), and a call runs that command with `--json` as a child process and
//! returns what it printed. A few commands keep a process or a sign-in going for the session;
//! mcp.rs runs those itself under the same names (`play`, `server`, `account_sign_in`...).
use clap::{ArgAction, Command};
use serde_json::{json, Map, Value};

/// CLI commands that are not generated tools: the MCP server itself; No AI, the player's switch
/// and never an agent's; and the ones mcp.rs runs in the session.
const NOT_TOOLS: &[&str] = &[
    "mcp",
    "agent-config",
    "no-ai",
    "play",
    "server",
    "account sign-in",
    "account list",
];

/// Commands that delete the player's things: their tools need `confirm: true`.
pub const DESTRUCTIVE: &[&str] = &[
    "remove",
    "modpack remove",
    "mods remove",
    "servers forget",
    "account sign-out",
    "data delete",
];

/// Commands that send something or reach other people: only when the player asks.
pub const OPEN_WORLD: &[&str] = &[
    "data allow",
    "data send",
    "data bug",
    "data benchmark",
    "servers create",
];

/// A generated tool: its name, the CLI words it runs and the clap command behind them.
pub struct Tool {
    pub name: String,
    pub path: Vec<String>,
    command: Command,
}

/// The tool name of CLI words: `mods search` gives `mods_search`.
pub fn tool_name(path: &[String]) -> String {
    path.join("_").replace('-', "_")
}

/// Every generated tool, in the CLI's order.
pub fn all() -> Vec<Tool> {
    let mut out = Vec::new();
    walk(&crate::cli::command(), &mut Vec::new(), &mut out);
    out
}

fn walk(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<Tool>) {
    for sub in cmd.get_subcommands() {
        path.push(sub.get_name().to_owned());
        if sub.has_subcommands() {
            walk(sub, path, out);
        } else if !NOT_TOOLS.contains(&path.join(" ").as_str()) {
            out.push(Tool {
                name: tool_name(path),
                path: path.clone(),
                command: sub.clone(),
            });
        }
        path.pop();
    }
}

impl Tool {
    fn words(&self) -> String {
        self.path.join(" ")
    }
    pub fn destructive(&self) -> bool {
        DESTRUCTIVE.contains(&self.words().as_str())
    }

    /// The tools/list entry.
    pub fn describe(&self) -> Value {
        let mut props = Map::new();
        let mut required = Vec::new();
        for arg in self.command.get_arguments().filter(|a| shown(a)) {
            let id = arg.get_id().as_str();
            let mut help = arg.get_help().map(ToString::to_string).unwrap_or_default();
            let defaults: Vec<String> = arg
                .get_default_values()
                .iter()
                .map(|d| d.to_string_lossy().into_owned())
                .collect();
            if !defaults.is_empty() {
                help = format!("{help} (default {})", defaults.join(", "))
                    .trim_start()
                    .to_owned();
            }
            let choices: Vec<String> = arg
                .get_possible_values()
                .iter()
                .map(|v| v.get_name().to_owned())
                .collect();
            let one = if !choices.is_empty() {
                json!({ "type": "string", "enum": choices })
            } else if defaults.first().is_some_and(|d| d.parse::<i64>().is_ok()) {
                json!({ "type": "integer" })
            } else {
                json!({ "type": "string" })
            };
            let mut schema = if matches!(arg.get_action(), ArgAction::SetTrue) {
                json!({ "type": "boolean" })
            } else if many(arg) {
                json!({ "type": "array", "items": one })
            } else {
                one
            };
            if !help.is_empty() {
                schema["description"] = help.into();
            }
            props.insert(id.to_owned(), schema);
            if arg.is_required_set() {
                required.push(id.to_owned());
            }
        }
        if self.destructive() {
            props.insert(
                "confirm".into(),
                json!({ "type": "boolean", "description": "true once the player agreed: it cannot be undone" }),
            );
            required.push("confirm".into());
        }
        let about = self
            .command
            .get_about()
            .map(ToString::to_string)
            .unwrap_or_default();
        let mut tool = json!({
            "name": self.name,
            "description": format!("{about}. CLI: horizon-launcher {}", self.words()),
            "inputSchema": { "type": "object", "properties": props, "required": required },
        });
        if self.destructive() {
            tool["annotations"] = json!({ "destructiveHint": true });
        } else if OPEN_WORLD.contains(&self.words().as_str()) {
            tool["annotations"] = json!({ "openWorldHint": true });
        }
        tool
    }

    /// The CLI words for a call's arguments.
    pub fn argv(&self, args: &Value) -> Result<Vec<String>, String> {
        if self.destructive() && args.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "{} cannot be undone: ask the player, then call it with confirm: true",
                self.name
            ));
        }
        let text = |v: &Value| match v {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            other => Err(format!("{other} is not a value")),
        };
        let mut words = self.path.clone();
        let mut positional = Vec::new();
        let mut last = Vec::new();
        for arg in self.command.get_arguments().filter(|a| shown(a)) {
            let id = arg.get_id().as_str();
            let Some(value) = args.get(id).filter(|v| !v.is_null()) else {
                if arg.is_required_set() {
                    return Err(format!("{id} is required"));
                }
                continue;
            };
            let values = match value {
                Value::Array(items) => items.iter().map(text).collect::<Result<Vec<_>, _>>()?,
                v => vec![text(v)?],
            };
            if matches!(arg.get_action(), ArgAction::SetTrue) {
                if value.as_bool().ok_or(format!("{id} is true or false"))? {
                    words.push(format!("--{}", arg.get_long().unwrap_or(id)));
                }
            } else if arg.is_positional() {
                let into = if arg.is_last_set() {
                    &mut last
                } else {
                    &mut positional
                };
                into.push((arg.get_index().unwrap_or(0), values));
            } else {
                let long = arg.get_long().unwrap_or(id);
                for v in values {
                    // `=` keeps a value that starts with a dash (game arguments) a value.
                    words.push(format!("--{long}={v}"));
                }
            }
        }
        positional.sort_by_key(|(index, _)| *index);
        words.extend(positional.into_iter().flat_map(|(_, v)| v));
        if !last.is_empty() {
            words.push("--".into());
            words.extend(last.into_iter().flat_map(|(_, v)| v));
        }
        Ok(words)
    }

    /// Runs the command as `horizon-launcher --json ...` and returns what it printed.
    pub fn call(&self, args: &Value) -> Result<String, String> {
        let words = self.argv(args)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let out = std::process::Command::new(exe)
            .arg("--json")
            .args(&words)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("Could not run horizon-launcher: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        let text = match (stdout.is_empty(), stderr.is_empty()) {
            (false, true) => stdout,
            (true, false) => stderr,
            (true, true) => "Done.".into(),
            (false, false) => format!("{stdout}\n{stderr}"),
        };
        if out.status.success() {
            Ok(text)
        } else {
            Err(text)
        }
    }
}

/// Whether an argument is part of a tool (not --help, --version or the global --json).
fn shown(arg: &clap::Arg) -> bool {
    !arg.is_hide_set() && !matches!(arg.get_id().as_str(), "help" | "version" | "json")
}

/// Whether an argument takes several values (a list).
fn many(arg: &clap::Arg) -> bool {
    matches!(arg.get_action(), ArgAction::Append)
        || arg.get_num_args().is_some_and(|n| n.max_values() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> Tool {
        all().into_iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn every_cli_command_is_a_tool_named_after_its_words() {
        let names: Vec<String> = all().into_iter().map(|t| t.name).collect();
        for want in [
            "versions",
            "install",
            "remove",
            "prepare",
            "notes",
            "update",
            "update_channel",
            "game_args",
            "settings",
            "logs",
            "mods_search",
            "mods_install",
            "modpack_import",
            "servers_create",
            "servers_run",
            "data_status",
            "data_bug",
            "account_select",
            "benchmark",
        ] {
            assert!(names.iter().any(|n| n == want), "{want} missing: {names:?}");
        }
        for not in ["mcp", "no_ai", "play", "server", "account_sign_in"] {
            assert!(!names.iter().any(|n| n == not), "{not}");
        }
    }

    #[test]
    fn arguments_become_cli_words() {
        let search = tool("mods_search");
        let schema = search.describe();
        assert_eq!(schema["inputSchema"]["properties"]["text"]["type"], "array");
        let kinds = &schema["inputSchema"]["properties"]["kind"]["enum"];
        assert!(
            kinds.as_array().unwrap().contains(&json!("shader")),
            "{kinds}"
        );
        assert_eq!(
            search
                .argv(&json!({ "text": ["fresh", "animations"], "kind": "shader", "everything": true }))
                .unwrap(),
            ["mods", "search", "--kind=shader", "--everything", "fresh", "animations"]
        );
        let run = tool("servers_run");
        assert_eq!(
            run.argv(&json!({ "server": "My server", "args": ["status"] }))
                .unwrap(),
            ["servers", "run", "My server", "--", "status"]
        );
        assert!(run
            .argv(&json!({ "server": "x" }))
            .unwrap_err()
            .contains("args is required"));
        let logs = tool("logs").describe();
        assert_eq!(
            logs["inputSchema"]["properties"]["lines"]["type"],
            "integer"
        );
    }

    #[test]
    fn destructive_tools_need_confirm() {
        let remove = tool("remove");
        assert_eq!(remove.describe()["annotations"]["destructiveHint"], true);
        assert!(remove
            .argv(&json!({ "tag": "v1" }))
            .unwrap_err()
            .contains("confirm"));
        assert_eq!(
            remove
                .argv(&json!({ "tag": "v1", "confirm": true }))
                .unwrap(),
            ["remove", "v1"]
        );
        for words in DESTRUCTIVE.iter().chain(OPEN_WORLD) {
            assert!(
                all().iter().any(|t| t.words() == *words),
                "{words} is not a tool"
            );
        }
    }
}
