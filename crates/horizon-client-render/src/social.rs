//! Vanilla's friends machinery without its screens: authlib's
//! `MinecraftServicesFriendsService` (`getFriends`, `updateFriends`,
//! `updateAttributes`, `updatePresence`), `RemoteFriendListUpdateHandler`'s worker and
//! state, `PlayerSocialManager`'s flags, and `PresenceHandler`'s posting.
//!
//! `title::poll_account` starts this the way `Minecraft`'s constructor does: the
//! account's `friendsPreferences` (`UserFlag.FRIENDS_ENABLED`,
//! `UserFlag.ACCEPT_FRIEND_INVITES`) decide whether the worker reads the lists. No
//! screen shows them yet; the overlay and the Online options are later parts.
//!
//! The requests keep authlib's cache rules: `If-None-Match` with the last `ETag`, a 304
//! or an empty answer keeps the cached lists, `Retry-After` sets the poll interval, and
//! `getFriendData` waits out a ten-second cooldown between reads. The presence statuses
//! are `PresenceHandler.getPresenceStatus`'s, supplied by the client each tick.
mod list_updates;
mod model;
mod presence;
mod service;

use list_updates::{Handler, spawn_updater};
pub use list_updates::{State, set_foreground};
pub use model::{
    Friend, FriendData, Presence, PresenceStatus, ResultCode, parse_friends, parse_presence,
};

use presence::PresenceHandler;
pub use presence::{
    SharePresence, latest_presence, send_offline_presence, set_presence_status, set_share_presence,
    tick, try_update_presence,
};
use service::{Service, send_json, session, url};

use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// `PlayerSocialManager`'s flags.
#[derive(Default)]
struct Manager {
    friend_list_enabled: bool,
    allow_friend_requests: bool,
}

struct Inner {
    service: Service,
    handler: Handler,
    presence: PresenceHandler,
    manager: Manager,
    share: SharePresence,
    started: bool,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            service: Service::default(),
            handler: Handler::default(),
            presence: PresenceHandler::default(),
            manager: Manager::default(),
            share: SharePresence::All,
            started: false,
        }
    }
}

fn inner() -> &'static Arc<Mutex<Inner>> {
    static INNER: OnceLock<Arc<Mutex<Inner>>> = OnceLock::new();
    INNER.get_or_init(|| Arc::new(Mutex::new(Inner::default())))
}

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    inner().lock().unwrap_or_else(|e| e.into_inner())
}

/// `Minecraft`'s constructor: make the social manager, whose `friendListEnabled` comes
/// from the account's `friendsPreferences`, and start the updater when it is on.
/// Called once; later calls leave the running manager alone.
pub fn init(friends_enabled: bool, allow_requests: bool) {
    {
        let mut inner = lock();
        if inner.started {
            return;
        }
        inner.started = true;
        inner.manager.friend_list_enabled = friends_enabled;
        inner.manager.allow_friend_requests = allow_requests;
        inner.handler.enabled = friends_enabled;
    }
    spawn_updater();
}

/// `PlayerSocialManager.setFriendListEnabled`: the Online options switch.
pub fn set_friend_list_enabled(enabled: bool) {
    let mut inner = lock();
    inner.manager.friend_list_enabled = enabled;
    inner.handler.enabled = enabled;
    if enabled {
        inner.handler.last_update = None;
    }
}

/// `PlayerSocialManager.isFriendListEnabled`.
pub fn is_friend_list_enabled() -> bool {
    lock().manager.friend_list_enabled
}

/// `PlayerSocialManager.isAllowFriendRequests`.
pub fn allow_friend_requests() -> bool {
    lock().manager.allow_friend_requests
}

/// `PlayerSocialManager.setAllowFriendRequests`.
pub fn set_allow_friend_requests(allow: bool) {
    lock().manager.allow_friend_requests = allow;
}

/// `PlayerSocialManager.getFriends`.
pub fn friends() -> Vec<Friend> {
    lock().handler.latest.friends.clone()
}

/// `PlayerSocialManager.getIncomingRequests`.
pub fn incoming_requests() -> Vec<Friend> {
    lock().handler.latest.incoming.clone()
}

/// `PlayerSocialManager.getOutgoingRequests`.
pub fn outgoing_requests() -> Vec<Friend> {
    lock().handler.latest.outgoing.clone()
}

/// `PlayerSocialManager.getFriendListState`.
pub fn friend_list_state() -> State {
    lock().handler.state
}

/// `FriendsService.sendFriendRequest(name)`.
pub fn send_friend_request(name: &str) -> ResultCode {
    friend_action(json!({"name": name, "updateType": "ADD"}))
}

/// `FriendsService.removeFriend`/`declineIncomingFriendRequest`/
/// `revokeOutgoingFriendRequest` (`updateType` REMOVE).
pub fn remove_friend(id: u128) -> ResultCode {
    friend_action(json!({"profileId": uuid(id), "updateType": "REMOVE"}))
}

/// `FriendsService.acceptIncomingFriendRequest`: ADD by profile id.
pub fn accept_incoming_friend_request(id: u128) -> ResultCode {
    friend_action(json!({"profileId": uuid(id), "updateType": "ADD"}))
}

/// `FriendsService.updateFriendSettings`: the account's flags, server-side.
pub fn update_friend_settings(friends_enabled: bool, allow_requests: bool) -> ResultCode {
    let Some(session) = session() else {
        return ResultCode::Unauthorized;
    };
    let body = json!({
        "friendsPreferences": {
            "friends": toggle(friends_enabled),
            "acceptInvites": toggle(allow_requests),
        }
    });
    let Some(url) = url("updateAttributes") else {
        return ResultCode::ServiceNotAvailable;
    };
    match send_json(&session, "POST", &url, &body.to_string(), None) {
        Ok(_) => ResultCode::Success,
        Err(failure) => failure.code,
    }
}

fn toggle(on: bool) -> &'static str {
    if on { "ENABLED" } else { "DISABLED" }
}

/// `MinecraftServicesFriendsService.putFriendAction`: the answer already is the new
/// lists, as authlib caches it, and the handler reads again.
fn friend_action(body: Value) -> ResultCode {
    let Some(session) = session() else {
        return ResultCode::Unauthorized;
    };
    let Some(url) = url("updateFriends") else {
        return ResultCode::ServiceNotAvailable;
    };
    let answer = match send_json(&session, "PUT", &url, &body.to_string(), None) {
        Ok(answer) => answer,
        Err(failure) => return failure.code,
    };
    let code = if answer.body.is_empty() {
        ResultCode::Success
    } else {
        match parse_friends(&answer.body) {
            Some(data) => {
                let mut inner = lock();
                inner.service.friends_cache = data;
                inner.service.friends_etag = None;
                inner.service.cooldown = Some(Instant::now() + Duration::from_secs(10));
                ResultCode::Success
            }
            None => ResultCode::Error,
        }
    };
    if code == ResultCode::Success {
        // `PlayerSocialManager.runAction` forces a read after a successful action.
        lock().handler.forced = true;
    }
    code
}

fn uuid(id: u128) -> String {
    let hex = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
