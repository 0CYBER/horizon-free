//! Friends over the internet through Tailscale: is it installed and signed in, this computer's name and
//! address on the tailnet, and, with the player's own API access token, inviting a friend to this one
//! computer (device sharing) and limiting what shared friends reach to the server's port.
//!
//! The launcher never signs in for anyone and never uses Funnel: signing in happens in Tailscale's own app
//! or page, and the access token is one the player makes in the admin console and pastes here (it is
//! kept in the system keychain). `HORIZON_LAUNCHER_TAILSCALE` (the CLI) and `HORIZON_TAILSCALE_API` (the
//! API's address) point the tests at stand-ins.
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// The admin console: sign-in, keys, machines, the access controls.
pub const ADMIN: &str = "https://login.tailscale.com/admin";
/// Where an API access token is made.
pub const KEYS: &str = "https://login.tailscale.com/admin/settings/keys";
/// The access controls page.
pub const ACCESS: &str = "https://login.tailscale.com/admin/acls/file";
/// The keychain entry of the player's API access token.
pub const TOKEN_KEY: &str = "tailscale-api-token";

/// Where to download Tailscale for this system.
pub fn install_url() -> &'static str {
    if cfg!(target_os = "macos") {
        "https://tailscale.com/download/mac"
    } else if cfg!(windows) {
        "https://tailscale.com/download/windows"
    } else {
        "https://tailscale.com/download/linux"
    }
}

/// The command that signs this computer in on Linux (it opens a link to finish in the browser).
pub const LINUX_LOGIN: &str = "sudo tailscale up --operator=$USER";

/// The `tailscale` command, wherever the installer put it.
pub fn cli() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HORIZON_LAUNCHER_TAILSCALE") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let mut places: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| {
                    d.join(if cfg!(windows) {
                        "tailscale.exe"
                    } else {
                        "tailscale"
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        places.push("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into());
        places.push("/usr/local/bin/tailscale".into());
        places.push("/opt/homebrew/bin/tailscale".into());
    } else if cfg!(windows) {
        places.push(r"C:\Program Files\Tailscale\tailscale.exe".into());
    } else {
        places.push("/usr/bin/tailscale".into());
        places.push("/usr/local/bin/tailscale".into());
        places.push("/run/current-system/sw/bin/tailscale".into());
    }
    places.into_iter().find(|p| p.is_file())
}

/// This computer on the tailnet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// The stable node ID (`n…`), which the API takes as the device ID.
    pub id: String,
    pub host: String,
    /// The MagicDNS name (`host.tailnet.ts.net`), without the final dot.
    pub dns_name: String,
    pub ips: Vec<String>,
    pub tailnet: String,
    pub magic_dns: bool,
}

impl Node {
    pub fn ipv4(&self) -> Option<&str> {
        self.ips
            .iter()
            .map(String::as_str)
            .find(|ip| !ip.contains(':'))
    }

    /// What a friend types in Minecraft: the MagicDNS name when it is on, else the address.
    pub fn address(&self, port: u16) -> String {
        let host = if self.magic_dns && !self.dns_name.is_empty() {
            self.dns_name.as_str()
        } else {
            self.ipv4().unwrap_or_default()
        };
        if port == 25565 {
            host.to_string()
        } else {
            format!("{host}:{port}")
        }
    }
}

/// Where Tailscale stands on this computer.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// No `tailscale` command.
    Missing,
    /// Installed, but its service doesn't answer (the app isn't open, or it is switched off).
    Off(String),
    /// Installed and waiting for the player to sign in (with the link to finish it, when there is one).
    SignedOut {
        auth_url: Option<String>,
    },
    Running(Node),
}

/// Asks `tailscale status --json` (read-only).
pub fn status() -> State {
    status_of(cli())
}

/// `status` with a given `tailscale` command.
pub fn status_of(cli: Option<PathBuf>) -> State {
    let Some(cli) = cli else {
        return State::Missing;
    };
    let mut command = crate::process::command(&cli);
    command.args(["status", "--json"]);
    let out = match command.output() {
        Ok(out) => out,
        Err(e) => return State::Off(e.to_string()),
    };
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(v) if v.is_object() => parse_status(&v),
        _ => {
            let err = String::from_utf8_lossy(&out.stderr);
            let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            State::Off(line.trim().to_string())
        }
    }
}

pub fn parse_status(v: &Value) -> State {
    let backend = v["BackendState"].as_str().unwrap_or_default();
    match backend {
        "Running" => {
            let me = &v["Self"];
            let ips = me["TailscaleIPs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|ip| ip.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let tailnet = &v["CurrentTailnet"];
            State::Running(Node {
                id: me["ID"].as_str().unwrap_or_default().to_string(),
                host: me["HostName"].as_str().unwrap_or_default().to_string(),
                dns_name: me["DNSName"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_end_matches('.')
                    .to_string(),
                ips,
                tailnet: tailnet["Name"].as_str().unwrap_or_default().to_string(),
                magic_dns: tailnet["MagicDNSEnabled"].as_bool().unwrap_or(false),
            })
        }
        "NeedsLogin" | "NeedsMachineAuth" | "NoState" => State::SignedOut {
            auth_url: v["AuthURL"]
                .as_str()
                .filter(|u| u.starts_with("https://"))
                .map(str::to_string),
        },
        "Stopped" => State::Off("Tailscale is switched off.".into()),
        other => State::Off(format!("Tailscale is {}.", other.to_lowercase())),
    }
}

/// Opens the Tailscale app, where the player signs in (macOS and Windows; Linux has no app).
pub fn open_app() -> Result<(), String> {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").args(["-a", "Tailscale"]).status()
    } else if cfg!(windows) {
        Command::new(r"C:\Program Files\Tailscale\tailscale-ipn.exe").status()
    } else {
        return Err(format!(
            "Run `{LINUX_LOGIN}` in a terminal and open the link it prints."
        ));
    };
    match result {
        Ok(s) if s.success() => Ok(()),
        Ok(_) | Err(_) => {
            Err("Tailscale's app didn't open: start it from your applications.".into())
        }
    }
}

// ----- The API --------------------------------------------------------------------------------------

/// An invitation to use this computer, as the API lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Invite {
    pub id: String,
    pub url: String,
    pub email: Option<String>,
    pub created: String,
    pub accepted: bool,
    /// Who accepted it (their login name).
    pub accepted_by: Option<String>,
}

impl Invite {
    fn parse(v: &Value) -> Self {
        Self {
            id: match &v["id"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
            url: v["inviteUrl"].as_str().unwrap_or_default().to_string(),
            email: v["email"]
                .as_str()
                .filter(|e| !e.is_empty())
                .map(str::to_string),
            created: v["created"].as_str().unwrap_or_default().to_string(),
            accepted: v["accepted"].as_bool().unwrap_or(false),
            accepted_by: v["acceptedBy"]["loginName"].as_str().map(str::to_string),
        }
    }
}

/// Tailscale's API with the player's access token.
pub struct Api {
    base: String,
    token: String,
}

impl Api {
    pub fn new(token: &str) -> Self {
        let base = std::env::var("HORIZON_TAILSCALE_API")
            .unwrap_or_else(|_| "https://api.tailscale.com".into());
        Self::at(&base, token)
    }

    /// The API at another address (a stand-in in tests).
    pub fn at(base: &str, token: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.trim().to_string(),
        }
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        if_match: Option<&str>,
    ) -> Result<(Value, Option<String>), String> {
        let url = format!("{}/api/v2{path}", self.base);
        let agent = crate::proxy::agent(
            ureq::AgentBuilder::new().timeout(Duration::from_secs(20)),
            &url,
        )
        .build();
        let mut request = agent
            .request(method, &url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/json");
        if let Some(etag) = if_match {
            request = request.set("If-Match", etag);
        }
        let response = match body {
            Some(b) => request.send_json(b.clone()),
            None => request.call(),
        };
        match response {
            Ok(r) => {
                let etag = r.header("ETag").map(str::to_string);
                let text = r.into_string().map_err(|e| e.to_string())?;
                let v = if text.trim().is_empty() {
                    Value::Null
                } else {
                    serde_json::from_str(&text).map_err(|e| e.to_string())?
                };
                Ok((v, etag))
            }
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let said = v["message"].as_str().unwrap_or_default();
                Err(match code {
                    401 => "Tailscale didn't accept the access token: it may have expired. Make a \
                            new one and paste it again."
                        .to_string(),
                    403 => format!(
                        "This access token may not do that{}. Make one as an admin of your tailnet.",
                        if said.is_empty() {
                            String::new()
                        } else {
                            format!(" ({said})")
                        }
                    ),
                    412 => "The access controls changed meanwhile: look again before changing them."
                        .to_string(),
                    _ if !said.is_empty() => format!("Tailscale says: {said}"),
                    _ => format!("Tailscale answered {code}."),
                })
            }
            Err(e) => Err(format!("Tailscale's API didn't answer: {e}")),
        }
    }

    /// The invitations to this computer.
    pub fn invites(&self, device: &str) -> Result<Vec<Invite>, String> {
        let (v, _) = self.send(
            "GET",
            &format!("/device/{device}/device-invites"),
            None,
            None,
        )?;
        Ok(v.as_array()
            .map(|a| a.iter().map(Invite::parse).collect())
            .unwrap_or_default())
    }

    /// A new invitation to this computer only: a link to send, or sent by email by Tailscale. One
    /// friend each (not reusable), never as an exit node.
    pub fn invite(&self, device: &str, email: Option<&str>) -> Result<Invite, String> {
        let mut one = json!({ "multiUse": false, "allowExitNode": false });
        if let Some(email) = email.filter(|e| !e.trim().is_empty()) {
            one["email"] = json!(email.trim());
        }
        let (v, _) = self.send(
            "POST",
            &format!("/device/{device}/device-invites"),
            Some(&json!([one])),
            None,
        )?;
        v.as_array()
            .and_then(|a| a.first())
            .map(Invite::parse)
            .ok_or_else(|| "Tailscale made no invitation.".to_string())
    }

    /// Withdraws an invitation (an accepted one stops sharing the computer with that friend).
    pub fn revoke(&self, invite: &str) -> Result<(), String> {
        self.send("DELETE", &format!("/device-invites/{invite}"), None, None)
            .map(|_| ())
    }

    /// The tailnet's access controls (as JSON) and their version.
    pub fn policy(&self) -> Result<(Value, Option<String>), String> {
        self.send("GET", "/tailnet/-/acl", None, None)
    }

    /// Checks new access controls without applying them.
    pub fn validate(&self, policy: &Value) -> Result<(), String> {
        let (v, _) = self.send("POST", "/tailnet/-/acl/validate", Some(policy), None)?;
        match v["message"].as_str() {
            Some(m) if !m.is_empty() => Err(format!("Tailscale wouldn't take the change: {m}")),
            _ => Ok(()),
        }
    }

    /// Applies new access controls, only if nobody changed them since `etag`.
    pub fn set_policy(&self, policy: &Value, etag: Option<&str>) -> Result<(), String> {
        self.send("POST", "/tailnet/-/acl", Some(policy), etag)
            .map(|_| ())
    }
}

// ----- Limiting what shared friends reach -------------------------------------------------------------

/// What the tailnet's access controls let shared friends reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reach {
    /// Tailscale's default: everyone, shared friends included, reaches every port of this computer.
    Everything,
    /// Shared friends reach only the server's port (the rule this launcher adds).
    ServerOnly,
    /// Access controls someone wrote: the launcher doesn't touch them.
    Custom,
}

const MEMBERS: [&str; 2] = ["autogroup:member", "autogroup:tagged"];
const SHARED: &str = "autogroup:shared";

fn is_star(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.len() == 1 && a[0] == "*")
}

fn is_members(v: &Value) -> bool {
    v.as_array().is_some_and(|a| {
        a.len() == MEMBERS.len() && MEMBERS.iter().all(|m| a.iter().any(|x| x == m))
    })
}

/// The allow-all rule, in either of its forms: `acls` (`src`/`dst` or the older `users`/`ports`) or
/// `grants`.
fn allow_all(rule: &Value, grant: bool) -> bool {
    if grant {
        is_star(&rule["src"]) && is_star(&rule["dst"]) && is_star(&rule["ip"])
    } else {
        rule["action"] == "accept"
            && (is_star(&rule["src"]) || is_star(&rule["users"]))
            && [&rule["dst"], &rule["ports"]]
                .iter()
                .any(|d| d.as_array().is_some_and(|a| a.len() == 1 && a[0] == "*:*"))
    }
}

fn members_all(rule: &Value, grant: bool) -> bool {
    if grant {
        is_members(&rule["src"]) && is_star(&rule["dst"]) && is_star(&rule["ip"])
    } else {
        rule["action"] == "accept"
            && is_members(&rule["src"])
            && rule["dst"]
                .as_array()
                .is_some_and(|a| a.len() == 1 && a[0] == "*:*")
    }
}

fn shared_rule(ip: &str, port: u16, grant: bool) -> Value {
    if grant {
        json!({ "src": [SHARED], "dst": [ip], "ip": [format!("tcp:{port}")] })
    } else {
        json!({ "action": "accept", "src": [SHARED], "dst": [format!("{ip}:{port}")] })
    }
}

/// The rules (`acls` or `grants`) when the policy is one of the two shapes this launcher knows: the
/// default allow-all alone, or that limited by `limit_shared` (with only shared rules beside it).
fn known(policy: &Value) -> Option<(&'static str, &Vec<Value>)> {
    let acls = policy["acls"].as_array().filter(|a| !a.is_empty());
    let grants = policy["grants"].as_array().filter(|a| !a.is_empty());
    match (acls, grants) {
        (Some(a), None) => Some(("acls", a)),
        (None, Some(g)) => Some(("grants", g)),
        _ => None,
    }
}

/// What shared friends reach under `policy`, for this computer's address and the server's port.
pub fn reach(policy: &Value, ip: &str, port: u16) -> Reach {
    let Some((key, rules)) = known(policy) else {
        return Reach::Custom;
    };
    let grant = key == "grants";
    if rules.len() == 1 && allow_all(&rules[0], grant) {
        return Reach::Everything;
    }
    let ours = shared_rule(ip, port, grant);
    if rules.len() == 2 && members_all(&rules[0], grant) && rules[1] == ours {
        return Reach::ServerOnly;
    }
    Reach::Custom
}

/// The default policy changed so shared friends reach only `ip:port`, everyone else as before; None
/// when the policy isn't the default (then the player adds `snippet` by hand).
pub fn limit_shared(policy: &Value, ip: &str, port: u16) -> Option<Value> {
    if reach(policy, ip, port) != Reach::Everything {
        return None;
    }
    let (key, _) = known(policy)?;
    let grant = key == "grants";
    let members = if grant {
        json!({ "src": MEMBERS, "dst": ["*"], "ip": ["*"] })
    } else {
        json!({ "action": "accept", "src": MEMBERS, "dst": ["*:*"] })
    };
    let mut new = policy.clone();
    new[key] = json!([members, shared_rule(ip, port, grant)]);
    Some(new)
}

/// Undoes `limit_shared`: Tailscale's default rule back. None if the policy isn't the limited one.
pub fn unlimit_shared(policy: &Value, ip: &str, port: u16) -> Option<Value> {
    if reach(policy, ip, port) != Reach::ServerOnly {
        return None;
    }
    let (key, _) = known(policy)?;
    let mut new = policy.clone();
    new[key] = if key == "grants" {
        json!([{ "src": ["*"], "dst": ["*"], "ip": ["*"] }])
    } else {
        json!([{ "action": "accept", "src": ["*"], "dst": ["*:*"] }])
    };
    Some(new)
}

/// The two rules to put in access controls someone wrote, by hand in the admin console.
pub fn snippet(ip: &str, port: u16) -> String {
    format!(
        "// Friends this computer is shared with reach only the Minecraft server.\n\
         // Replace \"*\" with these two in the rule that lets everyone reach everything:\n\
         //   \"src\": [\"autogroup:member\", \"autogroup:tagged\"]\n\
         {{\"action\": \"accept\", \"src\": [\"autogroup:shared\"], \"dst\": [\"{ip}:{port}\"]}},"
    )
}

#[cfg(test)]
mod tests;
