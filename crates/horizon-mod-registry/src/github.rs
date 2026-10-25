//! What the service asks of GitHub: the sign-in (OAuth, no scope: only the
//! public profile), and with the bot's token on the registry repository
//! alone, an entry, the `staging` release, a branch, a file, a pull request.
//! Blocking (ureq): called from tokio's blocking pool.
use base64::{Engine, engine::general_purpose::STANDARD};
use horizon_mods::index::registry::Entry;
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

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
    /// Fine-grained, this repository only: contents and pull requests.
    pub bot_token: String,
    agent: ureq::Agent,
}

/// A failed call: `status` is GitHub's, 0 when it wasn't reached.
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub message: String,
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
        },
        other => Failure {
            status: 0,
            message: other.to_string(),
        },
    }
}

fn read(response: ureq::Response) -> Result<Value> {
    response.into_json().map_err(|e| Failure {
        status: 0,
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
            .timeout(Duration::from_secs(60))
            .user_agent("horizon-mod-registry")
            .build();
        GitHub {
            at,
            repo,
            client_id,
            client_secret,
            bot_token,
            agent,
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
            });
        };
        let user = self
            .agent
            .get(&format!("{}/user", self.at.api))
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
            }),
        }
    }

    fn call(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(
                method,
                &format!("{}/repos/{}{path}", self.at.api, self.repo),
            )
            .set("Authorization", &format!("Bearer {}", self.bot_token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
    }

    /// `mods/<id>.toml` on main, with its blob sha; None when it isn't there.
    pub fn entry(&self, id: &str) -> Result<Option<(Entry, String)>> {
        let response = self
            .call("GET", &format!("/contents/mods/{id}.toml"))
            .query("ref", "main")
            .call();
        let file = match response {
            Ok(r) => read(r)?,
            Err(ureq::Error::Status(404, _)) => return Ok(None),
            Err(e) => return Err(failure(e)),
        };
        let bytes = file["content"]
            .as_str()
            .map(|c| c.replace('\n', ""))
            .and_then(|c| STANDARD.decode(c).ok())
            .unwrap_or_default();
        let entry = toml::from_str(&String::from_utf8_lossy(&bytes)).map_err(|e| Failure {
            status: 0,
            message: format!("mods/{id}.toml on main: {e}"),
        })?;
        Ok(Some((entry, file["sha"].as_str().unwrap_or("").to_owned())))
    }

    /// Uploads `path` as asset `name` of the `staging` prerelease, unless it
    /// is already there (the same file submitted again).
    pub fn stage(&self, name: &str, path: &Path) -> Result<()> {
        let release = self
            .call("GET", "/releases/tags/staging")
            .call()
            .map_err(failure)
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
        })?;
        self.agent
            .post(&format!(
                "{}/repos/{}/releases/{id}/assets",
                self.at.uploads, self.repo
            ))
            .query("name", name)
            .set("Authorization", &format!("Bearer {}", self.bot_token))
            .set("Accept", "application/vnd.github+json")
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&bytes)
            .map_err(failure)?;
        Ok(())
    }

    /// Makes branch `name` from main; false when it already exists.
    pub fn branch(&self, name: &str) -> Result<bool> {
        let main = self
            .call("GET", "/git/ref/heads/main")
            .call()
            .map_err(failure)
            .and_then(read)?;
        let sha = main["object"]["sha"].as_str().unwrap_or("");
        match self
            .call("POST", "/git/refs")
            .send_json(json!({"ref": format!("refs/heads/{name}"), "sha": sha}))
        {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(422, _)) => Ok(false),
            Err(e) => Err(failure(e)),
        }
    }

    /// Writes `path` on `branch` in one commit; `sha` is the blob it replaces.
    pub fn put(
        &self,
        path: &str,
        text: &str,
        branch: &str,
        sha: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let mut body = json!({
            "message": message,
            "content": STANDARD.encode(text),
            "branch": branch,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        self.call("PUT", &format!("/contents/{path}"))
            .send_json(body)
            .map_err(failure)?;
        Ok(())
    }

    /// Opens the pull request; its address.
    pub fn pull(&self, title: &str, head: &str, body: &str) -> Result<String> {
        let pr = self
            .call("POST", "/pulls")
            .send_json(json!({"title": title, "head": head, "base": "main", "body": body}))
            .map_err(failure)
            .and_then(read)?;
        Ok(pr["html_url"].as_str().unwrap_or("").to_owned())
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
