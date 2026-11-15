use super::service::post_presence;
use super::{Presence, PresenceStatus, lock};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

/// `Options.sharePresence` (`PresenceSharing`): how much of the player the service sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharePresence {
    None,
    Limited,
    All,
}

impl SharePresence {
    /// `options.txt`'s `sharePresence` (0 none, 1 limited, 2 all; `Options` defaults to
    /// all).
    pub fn of(setting: u8) -> Self {
        match setting {
            0 => Self::None,
            1 => Self::Limited,
            _ => Self::All,
        }
    }
}

/// `PresenceHandler`.
pub(super) struct PresenceHandler {
    latest: Vec<Presence>,
    last_post: Instant,
    /// `updatePresence`: post at the next tick.
    update: bool,
    /// `PresenceHandler.getPresenceStatus`'s answer, from the client's state.
    playing: PresenceStatus,
}

impl Default for PresenceHandler {
    fn default() -> Self {
        Self {
            latest: Vec::new(),
            // `lastPresencePost = Instant.now()`: the first post waits an interval.
            last_post: Instant::now(),
            // Its constructor sets `updatePresence = true`.
            update: true,
            playing: PresenceStatus::Online,
        }
    }
}

/// `PresenceHandler.getLatestPresence`.
pub fn latest_presence() -> Vec<Presence> {
    lock().presence.latest.clone()
}

/// `Options.sharePresence`: what presence the handler posts.
pub fn set_share_presence(share: SharePresence) {
    lock().share = share;
}

/// `PresenceHandler.getPresenceStatus`, supplied by the client from its world state.
pub fn set_presence_status(status: PresenceStatus) {
    lock().presence.playing = status;
}

/// `Minecraft.runTick`: `PresenceHandler.tick`, once per client tick.
pub fn tick() {
    let due = {
        let inner = lock();
        if !inner.manager.friend_list_enabled || inner.handler.latest.friends.is_empty() {
            return;
        }
        let interval = inner
            .service
            .presence_poll
            .unwrap_or(Duration::from_secs(60));
        let since = inner.presence.last_post.elapsed();
        (inner.presence.update && since >= interval) || since >= interval * 5
    };
    if due {
        update_presence();
    }
}

/// `PresenceHandler.updatePresence`.
fn update_presence() {
    let status = {
        let mut inner = lock();
        inner.presence.update = false;
        inner.presence.last_post = Instant::now();
        match inner.share {
            SharePresence::None => PresenceStatus::Offline,
            SharePresence::Limited => PresenceStatus::Online,
            SharePresence::All => inner.presence.playing,
        }
    };
    let _ = thread::Builder::new()
        .name("Friends presence".into())
        .spawn(move || {
            if let Some(presence) = post_presence(status) {
                lock().presence.latest = presence;
            }
        });
}

/// `PresenceHandler.tryUpdatePresence`: the world was entered (or the overlay opened).
pub fn try_update_presence() {
    lock().presence.update = true;
}

/// `PresenceHandler.sendOfflinePresence`: the game is closing.
pub fn send_offline_presence() {
    static SENT: AtomicBool = AtomicBool::new(false);
    {
        let inner = lock();
        if !inner.manager.friend_list_enabled || inner.handler.latest.friends.is_empty() {
            return;
        }
    }
    // `Minecraft.close` happens once; a second quit path must not post twice.
    if SENT.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = thread::Builder::new()
        .name("Friends offline".into())
        .spawn(|| {
            let _ = post_presence(PresenceStatus::Offline);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_presence_option_maps_to_its_levels() {
        assert_eq!(SharePresence::of(0), SharePresence::None);
        assert_eq!(SharePresence::of(1), SharePresence::Limited);
        assert_eq!(SharePresence::of(2), SharePresence::All);
        assert_eq!(SharePresence::of(9), SharePresence::All);
    }
}
