//! Play with friends, host side: "Invite friends" opens the world to LAN if
//! it isn't (with Open to LAN's defaults) and shares that port behind an
//! invite code through the relay (horizon-friends). The integrated server is
//! the LAN one, unchanged: tunnelled guests are LAN guests that connect from
//! the loopback address.
use super::{Game, lan::LanSettings};
use horizon_client::{Result, front::FriendsView, text::Text};
use horizon_friends::{Code, Host, HostConfig, HostEvent, HostState, PathMode};
use horizon_protocol::{GAME_VERSION, PROTOCOL_VERSION};
use std::time::{Duration, Instant};

pub(super) struct Sharing {
    host: Host,
    /// Sharing opened the world to LAN, so stopping closes it again.
    opened_lan: bool,
    code: Option<Code>,
    /// Wrong codes tried against the current code.
    wrong: u32,
    /// Sessions carried directly and through the relay so far.
    paths: (u32, u32),
    notice: Option<String>,
    copied: Option<Instant>,
}

pub const HELP: &[&str] = &[
    "friends host                   share the open world behind an invite code (needs HORIZON_RELAY_URL)",
    "friends stop                   stop sharing",
    "friends status                 the code, the relay state and the guests",
];

impl Game<'_> {
    /// Start sharing the open world.
    pub(super) fn friends_start(&mut self) -> Result<()> {
        if self.friends.is_some() {
            return Ok(());
        }
        if self.remote.is_some() || self.title.is_some() {
            return Err("open a singleplayer world first".into());
        }
        let relay = horizon_friends::relay_url()
            .ok_or("inviting friends is not available yet in this build")?;
        let opened_lan = self.lan.is_none();
        if opened_lan {
            // ShareToLanScreen's defaults, as the world options screen offers them.
            self.lan_open(LanSettings {
                port: horizon_client::lan::available_port(),
                guest_commands: false,
                force_game_mode: true,
            })?;
        }
        let port = self.lan_port().ok_or("the world isn't open to LAN")?;
        let host = match Host::start(HostConfig {
            relay,
            lan: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            protocol: PROTOCOL_VERSION,
            game: GAME_VERSION.to_owned(),
            path: PathMode::from_env(),
        }) {
            Ok(h) => h,
            Err(e) => {
                if opened_lan {
                    self.lan_close(true);
                }
                return Err(e.into());
            }
        };
        self.friends = Some(Sharing {
            host,
            opened_lan,
            code: None,
            wrong: 0,
            paths: (0, 0),
            notice: None,
            copied: None,
        });
        Ok(())
    }

    /// Stop sharing: the code ends; the world closes to LAN again if sharing
    /// opened it.
    pub(super) fn friends_stop(&mut self) {
        let Some(mut sharing) = self.friends.take() else {
            return;
        };
        sharing.host.stop();
        if sharing.opened_lan {
            self.lan_close(true);
        } else {
            self.system_message(Text::literal("Stopped sharing the invite code."));
        }
    }

    /// The LAN server is closing: the code goes with it.
    pub(super) fn friends_lan_closed(&mut self) {
        if let Some(mut sharing) = self.friends.take() {
            sharing.host.stop();
        }
    }

    /// New codes and the relay's state, once a tick and while the pause menu
    /// is drawn.
    pub(super) fn tick_friends(&mut self) {
        let Some(sharing) = self.friends.as_mut() else {
            return;
        };
        let mut announce = None;
        for event in sharing.host.poll() {
            match event {
                HostEvent::Code(code) => {
                    if sharing.code.is_some() {
                        sharing.notice =
                            Some(if sharing.wrong >= horizon_friends::host::MAX_WRONG {
                                "Someone kept trying a wrong code, so the code changed.".to_owned()
                            } else {
                                "The invite service restarted, so the code changed.".to_owned()
                            });
                    }
                    sharing.wrong = 0;
                    announce = Some(code.to_string());
                    sharing.code = Some(code);
                }
                HostEvent::WrongCode { count } => {
                    sharing.wrong = count;
                    sharing.notice = Some(format!("Someone tried a wrong code ({count} of 5)."));
                }
                HostEvent::Connected { direct: true, .. } => sharing.paths.0 += 1,
                HostEvent::Connected { direct: false, .. } => sharing.paths.1 += 1,
                HostEvent::Disconnected { .. } => {}
            }
        }
        if let HostState::Retrying(_) = sharing.host.state() {
            sharing.code = None;
        }
        if let Some(code) = announce {
            self.system_message(Text::literal(format!(
                "Friends can join with the code {code}"
            )));
        }
    }

    pub(super) fn friends_copy(&mut self) {
        let Some(code) = self.friends.as_ref().and_then(|s| s.code.clone()) else {
            return;
        };
        use horizon_client::clipboard::Clipboard;
        self.clipboard.set(&code.to_string());
        if let Some(s) = self.friends.as_mut() {
            s.copied = Some(Instant::now());
        }
    }

    /// What the pause menu shows.
    pub(super) fn friends_view(&self) -> FriendsView {
        let guests = self
            .lan
            .as_ref()
            .map(|h| h.hub.guests.iter().map(|g| g.name.clone()).collect())
            .unwrap_or_default();
        let Some(s) = &self.friends else {
            return FriendsView {
                shown: self.remote.is_none(),
                available: horizon_friends::relay_url().is_some(),
                ..FriendsView::default()
            };
        };
        let status = match s.host.state() {
            HostState::Registering => "Getting a code…".to_owned(),
            HostState::Retrying(e) => format!("Couldn't reach the invite service; retrying ({e})"),
            HostState::Sharing(_) | HostState::Stopped => String::new(),
        };
        FriendsView {
            shown: true,
            available: true,
            sharing: true,
            code: s.code.as_ref().map(Code::to_string),
            status,
            guests,
            notice: s.notice.clone(),
            copied: s
                .copied
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
        }
    }

    /// `horizon-ctl friends ...`.
    pub(super) fn friends_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words[0] != "friends" {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("host") => {
                self.friends_start()?;
                Ok(Some(json!({ "ok": true, "lan_port": self.lan_port() })))
            }
            Some("stop") => {
                self.friends_stop();
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => {
                self.tick_friends();
                let view = self.friends_view();
                let state = self
                    .friends
                    .as_ref()
                    .map(|s| format!("{:?}", s.host.state()));
                Ok(Some(json!({
                    "available": view.available,
                    "sharing": view.sharing,
                    "code": view.code,
                    "state": state,
                    "sessions": self.friends.as_ref().map_or(0, |s| s.host.sessions()),
                    "direct": self.friends.as_ref().map_or(0, |s| s.paths.0),
                    "relayed": self.friends.as_ref().map_or(0, |s| s.paths.1),
                    "guests": view.guests,
                    "lan_port": self.lan_port(),
                })))
            }
            _ => Err("friends host|stop|status".into()),
        }
    }
}
