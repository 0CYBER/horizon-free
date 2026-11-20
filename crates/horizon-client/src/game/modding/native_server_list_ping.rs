//! `server_list_ping`: the game's half of a status request. The connection thread cannot reach
//! the mods, so it queues the request (`lan::Shared::status_for`) and waits a moment; the game
//! thread asks the mods here, once a tick, and answers each request.
use super::Game;
use crate::game::api_values::{TryIntoApi, TryIntoEngine};
use horizon_client::{
    lan::{self, StatusAsk, StatusParts},
    server::json::Json,
    text::Text,
};

/// The first bytes of every PNG file.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

impl Game<'_> {
    /// Tells the connection threads whether a mod listens, and puts each waiting status request
    /// to the mods.
    pub(in crate::game) fn answer_status_pings(&mut self) {
        let Some(shared) = self.lan.as_ref().map(|host| host.hub.lan.shared.clone()) else {
            return;
        };
        let listened = mod_api::__private::listened(mod_api::HookId::ServerListPing);
        shared.set_status_hook(listened);
        for ask in shared.take_status_asks() {
            // A request that was queued just before the last listener went is answered as it was.
            let answer = if listened {
                self.ask_server_list_ping(&ask)
            } else {
                Some(ask.parts.clone())
            };
            // A connection that gave up has dropped its end.
            let _ = ask.reply.send(answer);
        }
    }

    /// What the mods make of one status request: `None` when one cancels it. A text the mods made
    /// that cannot be converted leaves the whole answer as it was.
    fn ask_server_list_ping(&mut self, ask: &StatusAsk) -> Option<StatusParts> {
        let StatusParts {
            description,
            favicon,
            online,
            max_players,
            sample,
            hide_players,
        } = ask.parts.clone();
        let plain = match &description {
            Json::String(text) => text.clone(),
            _ => String::new(),
        };
        let Ok(mut motd) = Text::untrusted(plain).try_into_api() else {
            return Some(ask.parts.clone());
        };
        let (mut favicon, mut online, mut max_players) = (favicon, online, max_players);
        let (mut sample, mut hide_players, mut cancel) = (sample, hide_players, false);
        self.native_server_list_ping(mod_api::__private::ServerListPingCall {
            protocol: ask.protocol,
            virtual_host: &ask.virtual_host,
            motd: &mut motd,
            favicon: &mut favicon,
            online: &mut online,
            max_players: &mut max_players,
            sample: &mut sample,
            hide_players: &mut hide_players,
            cancel: &mut cancel,
        });
        if cancel {
            return None;
        }
        let Ok(motd) = motd.try_into_engine(&()) else {
            return Some(ask.parts.clone());
        };
        sample.truncate(lan::STATUS_SAMPLE_LIMIT);
        Some(StatusParts {
            description: lan::conn::component_value(&lan::data::component_nbt(&motd.component())),
            // Bytes that are not a PNG send no icon.
            favicon: favicon.filter(|bytes| bytes.starts_with(&PNG_SIGNATURE)),
            online,
            max_players,
            sample,
            hide_players,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use mod_api::{Flow, Hooks, ServerHost, ServerListPingEvent};

    /// Rewrites the answer for the host `rewrite.example` and cancels it for `silent.example`.
    struct Pinger;
    #[mod_api::__subscriptions]
    impl Hooks for Pinger {
        fn server_list_ping(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut ServerListPingEvent<'_>,
        ) -> Flow {
            match event.virtual_host {
                "silent.example" => return Flow::Cancel,
                "rewrite.example" => {
                    *event.motd = mod_api::Text::literal(format!("Hello {}", event.protocol));
                    *event.favicon = Some(vec![1, 2, 3]);
                    event.online = 42;
                    event.max_players = 100;
                    event.sample.push(("Zed".to_owned(), 5));
                }
                _ => {}
            }
            Flow::Continue
        }
    }

    /// Asks the game's server like a connection thread does, answering the queue from the game
    /// thread meanwhile.
    fn ping(game: &mut super::Game<'_>, protocol: i32, host: &'static str) -> Option<String> {
        let shared = game.lan.as_ref().unwrap().hub.lan.shared.clone();
        game.answer_status_pings();
        let asker = std::thread::spawn(move || shared.status_for(protocol, 775, host));
        while !asker.is_finished() {
            game.answer_status_pings();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        asker.join().unwrap()
    }

    /// A listener rewrites the status answer, sees the protocol and the host the client typed,
    /// a non-PNG icon is dropped, and a cancel is no answer at all.
    #[test]
    #[ignore = "daily"]
    fn a_listener_rewrites_or_cancels_a_status_answer() {
        let mods = vec![mod_api::ModEntry::new("pinger", Box::new(Pinger))].into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                assert_eq!(
                    ping(game, 774, "rewrite.example").unwrap(),
                    r#"{"description":"Hello 774","players":{"max":100,"online":42,"sample":[{"id":"00000000-0000-0000-0000-000000000005","name":"Zed"}]},"version":{"name":"26.3","protocol":775}}"#
                );
                assert_eq!(ping(game, 774, "silent.example"), None);
                let plain = ping(game, 774, "other.example").unwrap();
                assert!(
                    plain.contains(r#""version":{"name":"26.3","protocol":775}"#),
                    "{plain}"
                );
                assert!(!plain.contains("Hello"), "{plain}");
            });
        });
    }
}
