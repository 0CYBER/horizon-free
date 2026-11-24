//! What the service asks of GitHub: the sign-in (OAuth, no scope: only the
//! public profile), and with the bot's token on the registry repository
//! alone, an entry, the `staging` release, a branch, a file, a pull request.
//! New native builds also compare their base with main in the Horizon game repository.
//! The bot is a token, or the registry's GitHub App (`app`).
//! Blocking (ureq): called from tokio's blocking pool.
mod app;

pub use app::AppCredentials;
use base64::{Engine, engine::general_purpose::STANDARD};
use horizon_platform::mod_package::index::{Revoked, registry::Entry};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

/// Where GitHub is: the real one, or a test's mock.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// `https://github.com`: OAuth.
    pub web: String,
    /// `https://api.github.com`.
    pub api: String,
    /// `https://uploads.github.com`: release assets.
    pub uploads: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            web: "https://github.com".into(),
            api: "https://api.github.com".into(),
            uploads: "https://uploads.github.com".into(),
        }
    }
}

#[derive(Clone)]
pub struct GitHub {
    pub at: Endpoints,
    /// `owner/name` of the registry repository.
    pub repo: String,
    pub client_id: String,
    pub client_secret: String,
    bot: Bot,
    /// How long one call may take, and the file's upload: past it, the
    /// submitter is told GitHub is slow rather than left waiting.
    pub call_timeout: Duration,
    pub upload_timeout: Duration,
    agent: ureq::Agent,
}

/// How the service writes to the registry repository.
#[derive(Clone)]
enum Bot {
    /// Fine-grained, this repository only: its pull requests open as the token's owner.
    Token(String),
    /// The App's installation: its pull requests open as the App's bot account.
    App(Arc<app::Installation>),
}

/// What a call sends.
#[derive(Clone, Copy)]
enum Payload<'a> {
    Empty,
    Json(&'a Value),
    Bytes(&'a [u8]),
}

/// A failed call: `status` is GitHub's, 0 when it wasn't reached.
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub message: String,
    /// GitHub didn't answer in time.
    pub timed_out: bool,
}

/// Whether `e`, or what caused it, is a timeout.
fn timed_out(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut cause = Some(e);
    while let Some(e) = cause {
        if e.downcast_ref::<std::io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )
        }) {
            return true;
        }
        cause = e.source();
    }
    false
}

type Result<T> = std::result::Result<T, Failure>;

fn failure(e: ureq::Error) -> Failure {
    match e {
        ureq::Error::Status(status, response) => Failure {
            status,
            message: response
                .into_string()
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect(),
            timed_out: false,
        },
        other => Failure {
            status: 0,
            timed_out: timed_out(&other),
            message: other.to_string(),
        },
    }
}

fn read(response: ureq::Response) -> Result<Value> {
    response.into_json().map_err(|e| Failure {
        status: 0,
        timed_out: timed_out(&e),
        message: e.to_string(),
    })
}

impl GitHub {
    pub fn new(
        at: Endpoints,
        repo: String,
        client_id: String,
        client_secret: String,
        bot_token: String,
    ) -> GitHub {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .user_agent("horizon-mod-registry")
            .build();
        GitHub {
            at,
            repo,
            client_id,
            client_secret,
            bot: Bot::Token(bot_token),
            call_timeout: Duration::from_secs(30),
            upload_timeout: Duration::from_secs(300),
            agent,
        }
    }

    /// Writes as the GitHub App instead of with the token.
    pub fn as_app(mut self, credentials: AppCredentials) -> GitHub {
        self.bot = Bot::App(Arc::new(app::Installation::new(credentials)));
        self
    }

    /// Who writes to the registry, for the log: never the token itself.
    pub fn bot(&self) -> String {
        match &self.bot {
            Bot::Token(_) => "GITHUB_BOT_TOKEN".into(),
            Bot::App(app) => format!("the GitHub App {}", app.id()),
        }
    }

    /// Where the sign-in starts.
    pub fn authorize_url(&self, redirect: &str, state: &str) -> String {
        format!(
            "{}/login/oauth/authorize?client_id={}&redirect_uri={}&state={state}&allow_signup=true",
            self.at.web,
            self.client_id,
            encode(redirect)
        )
    }

    /// The signed-in account (login, id) for an OAuth `code`. Its token is
    /// used once and dropped.
    pub fn sign_in(&self, code: &str, redirect: &str) -> Result<(String, u64)> {
        let token = self
            .agent
            .post(&format!("{}/login/oauth/access_token", self.at.web))
            .timeout(self.call_timeout)
            .set("Accept", "application/json")
            .send_json(json!({
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
                "redirect_uri": redirect,
            }))
            .map_err(failure)
            .and_then(read)?;
        let Some(token) = token["access_token"].as_str() else {
            return Err(Failure {
                status: 401,
                message: "GitHub refused the sign-in".into(),
                timed_out: false,
            });
        };
        let user = self
            .agent
            .get(&format!("{}/user", self.at.api))
            .timeout(self.call_timeout)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "application/vnd.github+json")
            .call()
            .map_err(failure)
            .and_then(read)?;
        match (user["login"].as_str(), user["id"].as_u64()) {
            (Some(login), Some(id)) => Ok((login.to_owned(), id)),
            _ => Err(Failure {
                status: 0,
                message: "GitHub sent no login".into(),
                timed_out: false,
            }),
        }
    }

    /// Refuses a name that could leave its place in an API path: the bot's token must only reach
    /// this repository's own files and refs.
    fn path_safe(name: &str) -> Result<()> {
        let plain = !name.is_empty()
            && !name.starts_with('/')
            && !name
                .split('/')
                .any(|part| part.is_empty() || part.starts_with('.'))
            && name.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+' | b'/')
            });
        if plain {
            Ok(())
        } else {
            Err(Failure {
                status: 0,
                message: format!("{name:?} is not a plain path"),
                timed_out: false,
            })
        }
    }

    fn call(&self, method: &str, path: &str) -> ureq::Request {
        self.api_request(
            method,
            &format!("{}/repos/{}{path}", self.at.api, self.repo),
        )
    }

    fn api_request(&self, method: &str, url: &str) -> ureq::Request {
        self.agent
            .request(method, url)
            .timeout(self.call_timeout)
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
    }

    /// Sends `request` as the bot. A refused App token (revoked, or ended early) is dropped and the call made
    /// once more with a new one.
    fn send(&self, request: ureq::Request, payload: Payload<'_>) -> Result<ureq::Response> {
        let attempt = |token: &str| {
            let request = request
                .clone()
                .set("Authorization", &format!("Bearer {token}"));
            match payload {
                Payload::Empty => request.call(),
                Payload::Json(body) => request.send_json(body),
                Payload::Bytes(bytes) => request.send_bytes(bytes),
            }
            .map_err(failure)
        };
        match &self.bot {
            Bot::Token(token) => attempt(token),
            Bot::App(app) => {
                let token = app.token(&self.agent, &self.at.api, self.call_timeout)?;
                match attempt(&token) {
                    Err(e) if e.status == 401 => {
                        app.forget(&token);
                        let token = app.token(&self.agent, &self.at.api, self.call_timeout)?;
                        let again = attempt(&token);
                        // Refused again: the next call starts with another token, still one retry each.
                        if again.as_ref().is_err_and(|e| e.status == 401) {
                            app.forget(&token);
                        }
                        again
                    }
                    other => other,
                }
            }
        }
    }

    /// `mods/<id>.toml` on main, with its blob sha; None when it isn't there.
    pub fn entry(&self, id: &str) -> Result<Option<(Entry, String)>> {
        Self::path_safe(id)?;
        let response = self.send(
            self.call("GET", &format!("/contents/mods/{id}.toml"))
                .query("ref", "main"),
            Payload::Empty,
        );
        let file = match response {
            Ok(r) => read(r)?,
            Err(e) if e.status == 404 => return Ok(None),
            Err(e) => return Err(e),
        };
        let bytes = file["content"]
            .as_str()
            .map(|c| c.replace('\n', ""))
            .and_then(|c| STANDARD.decode(c).ok())
            .unwrap_or_default();
        let entry = toml::from_str(&String::from_utf8_lossy(&bytes)).map_err(|e| Failure {
            status: 0,
            message: format!("mods/{id}.toml on main: {e}"),
            timed_out: false,
        })?;
        Ok(Some((entry, file["sha"].as_str().unwrap_or("").to_owned())))
    }

    /// The blob sha of `path` on main, None when it isn't there: what replacing it needs.
    pub fn blob(&self, path: &str) -> Result<Option<String>> {
        Self::path_safe(path)?;
        match self.send(
            self.call("GET", &format!("/contents/{path}"))
                .query("ref", "main"),
            Payload::Empty,
        ) {
            Ok(r) => Ok(read(r)?["sha"].as_str().map(str::to_owned)),
            Err(e) if e.status == 404 => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Uploads `path` as asset `name` of the `staging` prerelease, unless it
    /// is already there (the same file submitted again).
    pub fn stage(&self, name: &str, path: &Path) -> Result<()> {
        let release = self
            .send(self.call("GET", "/releases/tags/staging"), Payload::Empty)
            .and_then(read)?;
        let there = release["assets"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x["name"] == name));
        if there {
            return Ok(());
        }
        let id = release["id"].as_u64().unwrap_or(0);
        let bytes = std::fs::read(path).map_err(|e| Failure {
            status: 0,
            message: e.to_string(),
            timed_out: false,
        })?;
        let upload = self
            .agent
            .post(&format!(
                "{}/repos/{}/releases/{id}/assets",
                self.at.uploads, self.repo
            ))
            .query("name", name)
            .timeout(self.upload_timeout)
            .set("Accept", "application/vnd.github+json")
            .set("Content-Type", "application/octet-stream");
        self.send(upload, Payload::Bytes(&bytes))?;
        Ok(())
    }

    /// Makes branch `name` from main; false when it already exists.
    pub fn branch(&self, name: &str) -> Result<bool> {
        Self::path_safe(name)?;
        let main = self
            .send(self.call("GET", "/git/ref/heads/main"), Payload::Empty)
            .and_then(read)?;
        let sha = main["object"]["sha"].as_str().unwrap_or("");
        match self.send(
            self.call("POST", "/git/refs"),
            Payload::Json(&json!({"ref": format!("refs/heads/{name}"), "sha": sha})),
        ) {
            Ok(_) => Ok(true),
            Err(e) if e.status == 422 => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// The address of the open pull request from `branch`, if there is one.
    pub fn open_pull(&self, branch: &str) -> Result<Option<String>> {
        let owner = self.repo.split('/').next().unwrap_or("");
        let pulls = self
            .send(
                self.call("GET", "/pulls")
                    .query("head", &format!("{owner}:{branch}"))
                    .query("state", "open"),
                Payload::Empty,
            )
            .and_then(read)?;
        Ok(pulls
            .as_array()
            .and_then(|p| p.first())
            .map(|p| p["html_url"].as_str().unwrap_or("").to_owned()))
    }

    /// The messages of `branch`'s commits that aren't on main.
    pub fn commits(&self, branch: &str) -> Result<Vec<String>> {
        Self::path_safe(branch)?;
        let compare = self
            .send(
                self.call("GET", &format!("/compare/main...{branch}")),
                Payload::Empty,
            )
            .and_then(read)?;
        Ok(compare["commits"]
            .as_array()
            .map(|c| {
                c.iter()
                    .map(|c| c["commit"]["message"].as_str().unwrap_or("").to_owned())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Moves branch `name` back to main, dropping its own commits.
    pub fn reset(&self, name: &str) -> Result<()> {
        Self::path_safe(name)?;
        let main = self
            .send(self.call("GET", "/git/ref/heads/main"), Payload::Empty)
            .and_then(read)?;
        let sha = main["object"]["sha"].as_str().unwrap_or("");
        self.send(
            self.call("PATCH", &format!("/git/refs/heads/{name}")),
            Payload::Json(&json!({"sha": sha, "force": true})),
        )?;
        Ok(())
    }

    /// Writes `path` on `branch` in one commit; `sha` is the blob it replaces.
    pub fn put(
        &self,
        path: &str,
        content: impl AsRef<[u8]>,
        branch: &str,
        sha: Option<&str>,
        message: &str,
    ) -> Result<()> {
        Self::path_safe(path)?;
        Self::path_safe(branch)?;
        let mut body = json!({
            "message": message,
            "content": STANDARD.encode(content),
            "branch": branch,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        self.send(
            self.call("PUT", &format!("/contents/{path}")),
            Payload::Json(&body),
        )?;
        Ok(())
    }

    /// Opens the pull request; its address and number.
    pub fn pull(&self, title: &str, head: &str, body: &str) -> Result<(String, u64)> {
        let pr = self
            .send(
                self.call("POST", "/pulls"),
                Payload::Json(&json!({"title": title, "head": head, "base": "main", "body": body})),
            )
            .and_then(read)?;
        Ok((
            pr["html_url"].as_str().unwrap_or("").to_owned(),
            pr["number"].as_u64().unwrap_or(0),
        ))
    }

    /// Adds `label` to pull request `number` (GitHub creates it when new).
    pub fn label(&self, number: u64, label: &str) -> Result<()> {
        self.send(
            self.call("POST", &format!("/issues/{number}/labels")),
            Payload::Json(&json!({"labels": [label]})),
        )?;
        Ok(())
    }

    /// `revoked.toml` on main: the files taken down (none when it isn't there).
    pub fn revoked(&self) -> Result<Vec<Revoked>> {
        #[derive(serde::Deserialize)]
        struct File {
            #[serde(default)]
            revoked: Vec<Revoked>,
        }
        let response = self.send(
            self.call("GET", "/contents/revoked.toml")
                .query("ref", "main"),
            Payload::Empty,
        );
        let file = match response {
            Ok(r) => read(r)?,
            Err(e) if e.status == 404 => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let bytes = file["content"]
            .as_str()
            .map(|c| c.replace('\n', ""))
            .and_then(|c| STANDARD.decode(c).ok())
            .unwrap_or_default();
        let parsed: File =
            toml::from_str(&String::from_utf8_lossy(&bytes)).map_err(|e| Failure {
                status: 0,
                message: format!("revoked.toml on main: {e}"),
                timed_out: false,
            })?;
        Ok(parsed.revoked)
    }

    /// Checks whether release asset `name` exists in release tag `tag`.
    pub fn release_asset_exists(&self, tag: &str, name: &str) -> Result<bool> {
        let response = self.send(
            self.call("GET", &format!("/releases/tags/{tag}")),
            Payload::Empty,
        );
        let release = match response {
            Ok(r) => read(r)?,
            Err(e) if e.status == 404 => return Ok(false),
            Err(e) => return Err(e),
        };
        Ok(release["assets"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x["name"] == name)))
    }

    /// Whether the exact, format-checked `base` commit is reachable from Horizon's main.
    pub(super) fn native_base_on_main(&self, base: &str) -> Result<bool> {
        // Making the requested commit the head keeps a trusted base's commit and file lists
        // empty, even if main has advanced a long way since that release.
        let compare = self
            .send(
                self.api_request(
                    "GET",
                    &format!(
                        "{}/repos/{}/compare/main...{base}",
                        self.at.api,
                        horizon_addresses::HORIZON_GITHUB_REPO,
                    ),
                )
                .query("per_page", "1"),
                Payload::Empty,
            )
            .and_then(read)?;
        let status = compare["status"].as_str();
        let merge_base = compare["merge_base_commit"]["sha"].as_str();
        match (status, merge_base) {
            (Some("behind" | "identical" | "ahead" | "diverged"), Some(sha))
                if super::is_hex_str(sha, 40) =>
            {
                Ok(matches!(status, Some("behind" | "identical")) && sha == base)
            }
            _ => Err(Failure {
                status: 0,
                message: "GitHub returned an incomplete or invalid native build comparison".into(),
                timed_out: false,
            }),
        }
    }

    /// Dispatches the `native-build.yml` workflow on `ref_name` with inputs for target, base, and mods.
    pub fn dispatch_native_build(
        &self,
        ref_name: &str,
        target: &str,
        base: &str,
        mods: &str,
    ) -> Result<()> {
        self.send(
            self.call("POST", "/actions/workflows/native-build.yml/dispatches"),
            Payload::Json(&json!({
                "ref": ref_name,
                "inputs": {
                    "targets": target,
                    "base": base,
                    "mods": mods,
                }
            })),
        )?;
        Ok(())
    }
}

/// Percent-encodes a query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
