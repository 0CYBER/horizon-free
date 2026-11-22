//! `UserApiService`'s block list: the players the account has blocked outside
//! the game (Mojang's privacy settings), read from the player service's
//! `getBlocklist` endpoint (`MinecraftServicesUserApiService`).
//!
//! `PlayerSocialManager.startOnlineMode` starts one refresh when the user
//! properties have arrived; `social::init` is that moment here. The service's
//! rules carry over: a request may run once per 120 seconds
//! (`REQUEST_COOLDOWN_SECONDS`), a failed fetch leaves the set unknown (never
//! blocking anybody), and the NIL UUID is never blocked. One divergence,
//! noted where it happens: vanilla's `isBlockedPlayer` fetches lazily on the
//! calling thread, which would stall the game; here a due refresh is kicked
//! onto a thread instead and the caller keeps the last answer.
use crate::auth;
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

/// `MinecraftServicesUserApiService.REQUEST_COOLDOWN_SECONDS`: one
/// `getBlocklist` request may start every two minutes.
const REQUEST_COOLDOWN: Duration = Duration::from_secs(120);

struct State {
    /// `blockList`: the fetched set, or unknown (`None`) while no fetch has
    /// landed — a failed one clears it, as `refreshBlockList`'s assignment does.
    set: Option<HashSet<u128>>,
    fetching: bool,
    /// `nextAcceptableRequest`.
    next_acceptable: Option<Instant>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        set: None,
        fetching: false,
        next_acceptable: None,
    })
});

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `MinecraftServicesUserApiService.canMakeRequest`: nothing sent yet, or the
/// cooldown has passed.
fn can_make_request(state: &State) -> bool {
    state.next_acceptable.is_none_or(|at| Instant::now() > at)
}

/// `PlayerSocialManager.startOnlineMode`'s `refreshBlockList`: fetch when no
/// set is known (whatever the cooldown says, as the service's null check
/// skips it) or once the cooldown has passed. Runs on its own thread, like
/// vanilla's `thenRunAsync`.
pub(super) fn refresh() {
    let mut state = lock();
    if state.fetching {
        return;
    }
    if state.set.is_some() && !can_make_request(&state) {
        return;
    }
    state.fetching = true;
    drop(state);
    let spawned = std::thread::Builder::new()
        .name("Block list".into())
        .spawn(|| {
            let set = fetch_block_list();
            let mut state = lock();
            state.fetching = false;
            state.set = set;
        });
    if spawned.is_err() {
        lock().fetching = false;
    }
}

/// `MinecraftServicesUserApiService.forceFetchBlockList`: the cooldown is set
/// before the request, a good answer is its `blockedProfiles` (empty when the
/// service said nothing), and any failure is an unknown set.
fn fetch_block_list() -> Option<HashSet<u128>> {
    lock().next_acceptable = Some(Instant::now() + REQUEST_COOLDOWN);
    let session = auth::current()?;
    let doc = auth::discovery().ok()?;
    let url = auth::endpoint(&doc, "player", "getBlocklist")?;
    let answer = auth::agent()
        .get(&url)
        .set(
            "Authorization",
            &format!("Bearer {}", session.access_token()),
        )
        .call();
    let body = match answer {
        Ok(response) => response.into_string().ok()?,
        Err(_) => return None,
    };
    parse_block_list(&body)
}

/// `BlockListResponse.blockedProfiles`: the UUIDs, undashed as
/// `UUIDTypeAdapter` writes them. Malformed JSON is an error, like the
/// service's parse.
pub(super) fn parse_block_list(body: &str) -> Option<HashSet<u128>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let list = v.get("blockedProfiles")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|p| p.as_str())
            .filter_map(auth::parse_uuid)
            .collect(),
    )
}

/// `UserApiService.isBlockedPlayer`: never the NIL UUID; a due refresh on an
/// unknown set is kicked onto a thread where vanilla would fetch on the
/// calling thread. Until an answer lands, nobody counts as blocked.
pub(super) fn is_blocked_player(id: u128) -> bool {
    if id == 0 {
        return false;
    }
    let state = lock();
    let Some(set) = &state.set else {
        drop(state);
        if can_make_request(&lock()) {
            refresh();
        }
        return false;
    };
    set.contains(&id)
}

/// `UserApiService`'s set of blocked players, empty while it is unknown.
pub(super) fn blocked_players() -> HashSet<u128> {
    lock().set.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_profiles_read_undashed_uuids() {
        let body = r#"{"blockedProfiles":["069a79f444e94726a5befca90e38aaf5",
            "e9970cf5-211e-3998-95ba-4a5b7ea7fe06"]}"#;
        let set = parse_block_list(body).unwrap();
        assert!(set.contains(&0x069a79f4_44e94726_a5befca9_0e38aaf5));
        assert!(set.contains(&0xe9970cf5_211e3998_95ba4a5b_7ea7fe06));
        assert_eq!(set.len(), 2);
        // Malformed answers are an error, as the service's parse throws.
        assert!(parse_block_list("nope").is_none());
        assert!(parse_block_list(r#"{"blockedProfiles":{}}"#).is_none());
        assert!(parse_block_list("{}").is_none());
        let empty = parse_block_list(r#"{"blockedProfiles":[]}"#).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn the_nil_uuid_is_never_blocked() {
        assert!(!is_blocked_player(0));
        assert!(!blocked_players().contains(&0));
    }
}
