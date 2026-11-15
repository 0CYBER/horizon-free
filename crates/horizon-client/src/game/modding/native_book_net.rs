//! Private custom-book synchronization, admitted by its own configuration capability.
use super::Game;
use horizon_client::recipe_book::{
    settings_codec,
    settings_sync::{CHANNEL, Message},
};
use horizon_protocol::packets::{
    configuration::CustomPayload,
    play::{Clientbound as C, Serverbound as S},
};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
};
static EPOCH: AtomicU64 = AtomicU64::new(1);
fn registered() -> BTreeSet<String> {
    mod_api::RecipeBookTypeKey::registered()
        .into_iter()
        .map(|key| key.as_str().to_owned())
        .collect()
}
impl Game<'_> {
    pub(super) fn tick_book_snapshot_retry(&mut self) {
        self.check_mod_book_placement_timeout();
        if self
            .native_net
            .book_retry_at
            .is_some_and(|at| std::time::Instant::now() >= at)
        {
            let message = if self.native_net.book_view.epoch.is_none() {
                Message::Offer(registered().into_iter().collect())
            } else {
                Message::Resnapshot
            };
            self.native_net.book_retry_at = None;
            if let Some(data) = settings_codec::encode(&message) {
                self.send(S::CustomPayload(CustomPayload {
                    channel: CHANNEL.into(),
                    data,
                }));
            }
        }
    }

    pub(super) fn sync_replaced_book_settings(&mut self, uuid: u128) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guests.iter().find(|guest| guest.uuid == uuid) else {
            return;
        };
        let Some(authority) = self
            .native_net
            .book_connections
            .get_mut(&guest.conn)
            .and_then(|connection| connection.authority.as_mut())
        else {
            return;
        };
        let Some(revision) = authority.revision.checked_add(1) else {
            return;
        };
        authority.revision = revision;
        if let Some(data) = settings_codec::encode(&authority.snapshot(&guest.recipe_book.custom)) {
            guest.send(&C::CustomPayload(CustomPayload {
                channel: CHANNEL.into(),
                data,
            }));
        }
    }

    pub(super) fn reset_book_view(&mut self) {
        self.native_net.placement.client = Default::default();
        self.native_net.placement.deadline = None;
        self.clear_mod_book_placement_screen();
        self.native_net.book_view = Default::default();
        self.native_net.book_request = 0;
        self.native_net.book_ready = false;
        self.native_net.book_retry_at = None;
        self.build_screen();
    }
    pub(super) fn clear_guest_book_connection(&mut self, conn: u32) {
        self.guest_combat.remove(&conn);
        self.native_net.book_connections.remove(&conn);
        self.native_net.placement.servers.remove(&conn);
    }
    pub(super) fn notify_vanilla_book_settings(
        &mut self,
        uuid: u128,
        name: &str,
        book: i32,
        open: bool,
        filtering: bool,
    ) {
        let book = match book {
            0 => mod_api::RecipeBookIdentity::Crafting,
            1 => mod_api::RecipeBookIdentity::Furnace,
            2 => mod_api::RecipeBookIdentity::BlastFurnace,
            3 => mod_api::RecipeBookIdentity::Smoker,
            _ => return,
        };
        self.native_player_recipe_book_settings_change(uuid, name, book, open, filtering);
    }

    /// Authoritative view for a custom menu panel. None disables the panel
    /// until negotiation and the initial snapshot finish.
    pub fn custom_book_settings(
        &self,
        book: &mod_api::RecipeBookTypeKey,
    ) -> Option<mod_api::RecipeBookSettings> {
        let view = &self.native_net.book_view;
        if !view.ready || !view.contains(book.as_str()) {
            return None;
        }
        let flags = view.books.get(book.as_str());
        Some(mod_api::RecipeBookSettings {
            open: flags.open,
            filtering: flags.filtering,
        })
    }
    /// Queue intent without changing the authoritative or saved pair.
    pub fn request_custom_book_settings(
        &mut self,
        book: &mod_api::RecipeBookTypeKey,
        flags: mod_api::RecipeBookSettings,
    ) -> bool {
        if self.custom_book_settings(book).is_none() {
            return false;
        }
        let Some(number) = self.native_net.book_request.checked_add(1) else {
            return false;
        };
        let Some(epoch) = self.native_net.book_view.epoch else {
            return false;
        };
        let message = Message::Request {
            epoch,
            number,
            key: book.as_str().into(),
            flags: horizon_client::recipe_book::BookFlags {
                open: flags.open,
                filtering: flags.filtering,
            },
        };
        let Some(data) = settings_codec::encode(&message) else {
            return false;
        };
        self.native_net.book_request = number;
        self.send(S::CustomPayload(CustomPayload {
            channel: CHANNEL.into(),
            data,
        }));
        true
    }

    pub(super) fn sync_server_book_write(
        &mut self,
        player: mod_api::PlayerRef,
        book: &mod_api::RecipeBookTypeKey,
        flags: horizon_client::recipe_book::BookFlags,
    ) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(guest) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == player.uuid() && g.placed)
        else {
            return;
        };
        let Some(authority) = self
            .native_net
            .book_connections
            .get_mut(&guest.conn)
            .and_then(|connection| connection.authority.as_mut())
        else {
            return;
        };
        if let Some(message) = authority.server_update(book.as_str(), flags)
            && let Some(data) = settings_codec::encode(&message)
        {
            guest.send(&C::CustomPayload(CustomPayload {
                channel: CHANNEL.into(),
                data,
            }));
        }
    }

    pub(super) fn begin_book_negotiation(&mut self) {
        self.reset_book_view();
        if self
            .remote
            .as_ref()
            .is_some_and(|r| r.custom_books && r.joined)
            && let Some(data) = settings_codec::encode(&Message::Hello)
        {
            self.send(S::CustomPayload(CustomPayload {
                channel: CHANNEL.into(),
                data,
            }));
        }
    }
    pub(super) fn guest_book_settings(&mut self, conn: u32, data: &[u8]) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if !host
            .hub
            .guest(conn)
            .is_some_and(|guest| guest.placed && guest.brand == "horizon")
        {
            return;
        }
        let connection = self.native_net.book_connections.entry(conn).or_default();
        if data.first() == Some(&settings_codec::PLACEMENT_TAG) {
            let frame = connection.receive_placement(data, std::time::Instant::now());
            if let Some(frame) = frame {
                self.guest_mod_book_placement(conn, frame);
            }
            return;
        }
        let changed = host
            .hub
            .guests
            .update_connection(conn, |guest| {
                let reply = connection.receive(
                    data,
                    &mut guest.recipe_book.custom,
                    registered,
                    || EPOCH.fetch_add(1, Ordering::Relaxed),
                    std::time::Instant::now(),
                )?;
                if let Some(data) = settings_codec::encode(&reply.message) {
                    guest.send(&C::CustomPayload(CustomPayload {
                        channel: CHANNEL.into(),
                        data,
                    }));
                }
                reply
                    .changed
                    .map(|(id, flags)| (guest.uuid, guest.name.clone(), id, flags))
            })
            .flatten();
        if let Some((uuid, name, id, flags)) = changed
            && let Some(key) = mod_api::RecipeBookTypeKey::registered()
                .into_iter()
                .find(|key| key.as_str() == id)
        {
            self.native_player_recipe_book_settings_change(
                uuid,
                &name,
                mod_api::RecipeBookIdentity::Custom(key),
                flags.open,
                flags.filtering,
            );
        }
    }
    pub(super) fn remote_book_settings(&mut self, data: &[u8]) {
        if !self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.custom_books && remote.joined)
        {
            return;
        }
        let Some(message) = settings_codec::decode(data) else {
            return;
        };
        if let Message::Placement(frame) = message {
            if self.native_net.book_ready && self.native_net.book_view.ready {
                self.remote_mod_book_placement(frame);
            }
            return;
        }
        if message == Message::Busy && self.native_net.book_ready {
            self.clear_mod_book_placement_screen();
            self.native_net.book_view.ready = false;
            self.native_net.book_retry_at.get_or_insert_with(|| {
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            });
            self.build_screen();
            return;
        }
        if message == Message::Ready {
            if self.native_net.book_ready {
                return;
            }
            self.native_net.book_ready = true;
            if let Some(data) =
                settings_codec::encode(&Message::Offer(registered().into_iter().collect()))
            {
                self.send(S::CustomPayload(CustomPayload {
                    channel: CHANNEL.into(),
                    data,
                }));
            }
            return;
        }
        if !self.native_net.book_ready
            || !matches!(message, Message::Snapshot { .. } | Message::Update { .. })
        {
            return;
        }
        let previous_epoch = self.native_net.book_view.epoch;
        if !self.native_net.book_view.receive(message, &registered())
            && !self.native_net.book_view.ready
            && let Some(data) = settings_codec::encode(&Message::Resnapshot)
        {
            self.send(S::CustomPayload(CustomPayload {
                channel: CHANNEL.into(),
                data,
            }));
        }
        if previous_epoch != self.native_net.book_view.epoch {
            self.native_net.placement.client = Default::default();
            self.native_net.placement.deadline = None;
            self.clear_mod_book_placement_screen();
        }
        // Both accepted settings and readiness loss change the visible panel.
        self.build_screen();
        if self.native_net.book_view.ready {
            self.native_net.book_retry_at = None;
            self.begin_mod_book_placement();
            self.ensure_mod_book_catalog();
        }
    }
}
