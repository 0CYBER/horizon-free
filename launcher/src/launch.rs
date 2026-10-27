//! What runs around the game for one instance (`instances::Meta::launch`): a command run before
//! it, a wrapper it runs through, and environment variables. All empty by default.
//!
//! [`wrap`] turns the game's command into one process that does all three, so the launcher still
//! follows one program (its output, its exit, Stop):
//!
//! - Unix: `/bin/sh -c '(<pre-launch>) || exit; exec "$@"' horizon <wrapper…> <game> <args…>`.
//!   The pre-launch command is the player's own shell line, run in the instance's folder; the
//!   game's words go through `"$@"`, so they are never parsed by the shell. `exec` makes the
//!   shell become the wrapper (or the game).
//! - Windows: `cmd /D /S /C "<pre-launch> && <wrapper…> <game> <args…>"`, each word quoted.
//!
//! Without a pre-launch command the wrapper is the program itself, and no shell runs.
use crate::backend::split_args;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::process::Command;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    /// A shell command run in the instance's folder before the game; the game starts only if it
    /// succeeds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pre_launch: String,
    /// A command line the game runs through, such as `nice -n 5` (quotes group a word).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wrapper: String,
    /// Environment variables for the game (and the two commands above).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Launch {
    pub fn is_empty(&self) -> bool {
        self == &Launch::default()
    }

    /// What it does, in one line, or None when nothing.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.pre_launch.trim().is_empty() {
            parts.push(format!("runs {} first", self.pre_launch.trim()));
        }
        if !self.wrapper.trim().is_empty() {
            parts.push(format!("through {}", self.wrapper.trim()));
        }
        match self.env.len() {
            0 => {}
            1 => parts.push("with 1 variable".into()),
            n => parts.push(format!("with {n} variables")),
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `KEY=value KEY2="two words"` (one line, or one per line) as variables.
pub fn parse_env(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    for word in split_args(text) {
        let (key, value) = word
            .split_once('=')
            .ok_or_else(|| format!("{word}: write it as NAME=value"))?;
        if !valid_key(key) {
            return Err(format!(
                "{key}: a variable's name is letters, digits and _, not starting with a digit"
            ));
        }
        env.insert(key.to_string(), value.to_string());
    }
    Ok(env)
}

/// The variables as [`parse_env`] reads them back.
pub fn env_line(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(
            |(k, v)| match v.is_empty() || v.contains([' ', '"', '\'', '\\']) {
                true => format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")),
                false => format!("{k}={v}"),
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

/// The Unix script: the pre-launch command (in a subshell, so its `exit` doesn't end the script
/// before the message), then the rest of the arguments as the program.
fn script(pre_launch: &str) -> String {
    format!(
        "(\n{pre_launch}\n) || {{ s=$?; echo \"The pre-launch command failed (exit $s), so the \
         game did not start.\" >&2; exit $s; }}\nexec \"$@\""
    )
}

/// A word quoted for a Windows command line, as programs built with the C runtime read it back.
fn windows_quote(word: &OsStr) -> String {
    let word = word.to_string_lossy();
    if !word.is_empty() && !word.contains([' ', '\t', '"']) {
        return word.into_owned();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in word.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// How [`wrap`] runs a command on each system, apart from the variables and the folder: the
/// program and its arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Run as given (`words[0]` is the program).
    Direct(Vec<OsString>),
    /// `/bin/sh -c <script> horizon <words…>`.
    Shell {
        script: String,
        words: Vec<OsString>,
    },
    /// `cmd /D /S /C "<line>"`: the line as cmd reads it.
    Cmd { line: String },
}

/// The plan for `words` (the game's program and arguments) with `launch`, on Windows or not.
pub fn plan(launch: &Launch, words: Vec<OsString>, windows: bool) -> Plan {
    let mut all: Vec<OsString> = split_args(&launch.wrapper)
        .into_iter()
        .map(OsString::from)
        .collect();
    all.extend(words);
    let pre = launch.pre_launch.trim();
    match (pre.is_empty(), windows) {
        (true, _) => Plan::Direct(all),
        (false, false) => Plan::Shell {
            script: script(pre),
            words: all,
        },
        (false, true) => Plan::Cmd {
            line: format!(
                "{pre} && {}",
                all.iter()
                    .map(|w| windows_quote(w))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        },
    }
}

/// The game's command run with `launch`: a new command with the same folder and variables,
/// plus the instance's variables. Call it last, after every argument is added.
pub fn wrap(cmd: Command, launch: &Launch) -> Command {
    if launch.is_empty() {
        return cmd;
    }
    let words: Vec<OsString> = std::iter::once(cmd.get_program().to_owned())
        .chain(cmd.get_args().map(OsStr::to_owned))
        .collect();
    let mut out = match plan(launch, words, cfg!(windows)) {
        Plan::Direct(words) => {
            let mut c = Command::new(&words[0]);
            c.args(&words[1..]);
            c
        }
        Plan::Shell { script, words } => {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(script).arg("horizon").args(words);
            c
        }
        Plan::Cmd { line } => {
            let mut c = Command::new("cmd.exe");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.raw_arg(format!("/D /S /C \"{line}\""));
            }
            #[cfg(not(windows))]
            c.args(["/D", "/S", "/C", &line]);
            c
        }
    };
    if let Some(dir) = cmd.get_current_dir() {
        out.current_dir(dir);
    }
    for (key, value) in cmd.get_envs() {
        match value {
            Some(value) => out.env(key, value),
            None => out.env_remove(key),
        };
    }
    out.envs(&launch.env);
    out
}

#[cfg(test)]
#[path = "launch/tests.rs"]
mod tests;
