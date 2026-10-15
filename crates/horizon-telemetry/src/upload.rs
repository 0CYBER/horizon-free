//! Sends the outbox, from the launcher at start and exit. Before anything else it reads the
//! consent again and deletes what the player no longer allows, so with everything off it
//! returns without opening a connection. Failures back off (a minute, doubling, at most a day)
//! and at most [`MAX_BATCHES_A_DAY`] batches leave a day.
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::batch::{self, Batch};
use crate::{Consent, outbox};

pub const MAX_BATCHES_A_DAY: u32 = 8;
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
pub enum Flush {
    /// Nothing to send, or nothing allowed: no connection was made.
    Nothing,
    /// Waiting after a failure, or for tomorrow's allowance.
    Later,
    Sent {
        items: usize,
        batches: usize,
    },
    /// The server could not be reached or failed; tried again later.
    Failed(String),
}

/// The uploader's own state, beside the consent.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    day: u32,
    batches: u32,
    failures: u32,
    retry_at: u64,
}

fn state_path(home: &Path) -> PathBuf {
    crate::dir(home).join("upload.json")
}

fn load(home: &Path) -> State {
    std::fs::read(state_path(home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(home: &Path, state: &State) {
    if let Ok(json) = serde_json::to_vec(state) {
        let _ = crate::write_atomic(&state_path(home), &json);
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(TIMEOUT).build()
}

/// Sends what the player allows to `endpoint` (`https://host`, without a trailing slash).
pub fn flush(home: &Path, endpoint: &str) -> Flush {
    let consent = Consent::load(home);
    // What was turned off since it was kept never leaves.
    let allowed = outbox::retain_allowed(home);
    if allowed.is_empty() {
        return Flush::Nothing;
    }
    let mut state = load(home);
    let today = crate::today();
    if state.day != today {
        state.day = today;
        state.batches = 0;
    }
    if crate::now_secs() < state.retry_at || state.batches >= MAX_BATCHES_A_DAY {
        return Flush::Later;
    }
    let agent = agent();
    let url = format!("{endpoint}/v1/batch");
    let (mut items, mut batches) = (0, 0);
    for group in batch::split(allowed) {
        if state.batches >= MAX_BATCHES_A_DAY {
            break;
        }
        let (paths, list): (Vec<PathBuf>, Vec<crate::Item>) = group.into_iter().unzip();
        let body = batch::encode(&Batch::new(&consent.install_id, list));
        let sent = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("Content-Encoding", "zstd")
            .send_bytes(&body);
        match sent {
            Ok(_) => {
                state.batches += 1;
                state.failures = 0;
                items += paths.len();
                batches += 1;
                paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
            }
            // Refused for what it is (not for load): sending it again would not help.
            Err(ureq::Error::Status(code, _)) if (400..500).contains(&code) && code != 429 => {
                state.batches += 1;
                paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
            }
            Err(e) => {
                state.failures = (state.failures + 1).min(16);
                state.retry_at = crate::now_secs() + (60u64 << (state.failures - 1)).min(86_400);
                save(home, &state);
                return Flush::Failed(e.to_string());
            }
        }
    }
    save(home, &state);
    if batches == 0 {
        Flush::Later
    } else {
        Flush::Sent { items, batches }
    }
}

/// Asks the server to delete everything sent under `install_id`.
pub fn delete_remote(endpoint: &str, install_id: &str) -> Result<(), String> {
    if !crate::consent::is_install_id(install_id) {
        return Err("no install ID: nothing was sent".into());
    }
    agent()
        .delete(&format!("{endpoint}/v1/installs/{install_id}"))
        .call()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The batch the next [`flush`] would send, as readable JSON ("View what would be sent").
pub fn preview(home: &Path) -> String {
    let consent = Consent::load(home);
    let items = outbox::retain_allowed(home)
        .into_iter()
        .map(|(_, item)| item)
        .collect();
    serde_json::to_string_pretty(&Batch::new(&consent.install_id, items)).unwrap_or_default()
}
