//! Trading for LAN guests: the villager screen a guest opens runs the same
//! merchant menu as the host's (`horizon_client::menu`, `Kind::Merchant`),
//! with vanilla's packets: OpenScreen (`minecraft:merchant`), MerchantOffers,
//! the select trade packet, container clicks and the content sync after each.
use super::{
    Game, lan_actions::guest_within_entity_range, stations::Kind as StationKind,
    trading::MerchantState,
};
use horizon_client::{
    items::ItemStack,
    menu::{Env, Input, Kind, Menu},
    merchant,
};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A guest's open trading screen.
pub(super) struct GuestMerchant {
    pub(super) window: i32,
    state: i32,
    /// The payment slots and the result.
    slots: [Option<ItemStack>; 3],
    pub(super) merchant: Box<MerchantState>,
    /// The offers the client was last sent.
    sent: Vec<merchant::MerchantOffer>,
    sent_level: i32,
    sent_xp: i32,
}

/// A guest's UUID as gossip targets it.
fn uuid_words(uuid: u128) -> [i32; 4] {
    [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ]
}

impl Game<'_> {
    /// Development oracle: observe the actual open guest merchant, never a synthetic menu.
    pub(super) fn harness_merchant(
        &self,
        host: &super::lan::LanHost,
        conn: u32,
    ) -> Option<(i32, i32, serde_json::Value)> {
        use serde_json::json;
        let gm = host.play.get(&conn)?.merchant.as_ref()?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn && g.placed)?;
        let id = gm.merchant.mob?;
        self.mobs
            .iter()
            .find(|m| m.id == id && m.alive() && !m.removed)?;
        let stack = |s: &Option<ItemStack>| s.as_ref().map(|s| json!([s.item.as_ref(), s.count]));
        let session = &gm.merchant.session;
        Some((
            gm.window,
            gm.state,
            json!({"window":gm.window,"merchant":self.persist.uuids.get(&id)?,
            "player":uuid_words(guest.uuid),"slots":gm.slots.iter().map(stack).collect::<Vec<_>>(),
            "carried":stack(&guest.carried),"xp":session.xp,
            "sent_offers":gm.sent.iter().map(|o|json!({"uses":o.uses,
                "special":o.special_price_diff,"demand":o.demand,"cost":o.cost_a_count(self.items())})).collect::<Vec<_>>(),
            "offers":session.offers.iter().map(|o|json!({"uses":o.uses,
                "special":o.special_price_diff,"demand":o.demand,"cost":o.cost_a_count(self.items())})).collect::<Vec<_>>()}),
        ))
    }

    /// A trading screen the guest's interaction opened (it opened as a
    /// station while the guest acted): handed to the guest.
    pub(super) fn lan_open_merchant(&mut self, conn: u32, mut state: MerchantState) {
        let Some(uuid) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
            .map(|g| uuid_words(g.uuid))
        else {
            return;
        };
        if let Some(id) = state.mob
            && state.session.villager
        {
            // startTrading with the guest's reputation. The session already
            // captured their Hero effect while they acted as the player.
            state.session.reputation = self.villager_reputation_of(id, uuid);
            state.session.update_special_prices();
        }
        let title = self.merchant_display_name(&state);
        if !self.guest_may_open(conn, "minecraft:merchant", None) {
            self.merchant_closed(&mut state);
            return;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        let Some(window) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.open_merchant_window(conn, title))
        else {
            self.merchant_closed(&mut state);
            return;
        };
        let mut gm = GuestMerchant {
            window,
            state: 0,
            slots: [None, None, None],
            merchant: Box::new(state),
            sent: Vec::new(),
            sent_level: 0,
            sent_xp: 0,
        };
        self.lan_send_offers(conn, &mut gm, true);
        self.lan_send_merchant_content(conn, &mut gm);
        if let Some(host) = self.lan.as_mut() {
            host.play.entry(conn).or_default().merchant = Some(gm);
        }
        self.player_menu_opening(conn);
    }
    /// `Entity.getDisplayName` of the merchant, the screen's title: its name
    /// (custom, else the profession's) with the show_entity hover event and
    /// its UUID as insertion.
    fn merchant_display_name(&mut self, state: &MerchantState) -> horizon_core::nbt::Tag {
        use horizon_core::nbt::Tag;
        let s = &state.session;
        let (kind, key) = if s.villager {
            let p = s.profession.trim_start_matches("minecraft:");
            (
                "minecraft:villager",
                format!("entity.minecraft.villager.{p}"),
            )
        } else {
            (
                "minecraft:wandering_trader",
                "entity.minecraft.wandering_trader".to_owned(),
            )
        };
        let name = match &state.name {
            Some(n) => horizon_client::scoreboard::json_to_tag(&n.component()),
            None => Tag::compound([("translate", Tag::String(key.as_str().into()))]),
        };
        let uuid = state.mob.map_or([0; 4], |id| self.mob_uuid(id));
        let w = uuid.map(|v| v as u32);
        let text = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:04x}{:08x}",
            w[0],
            w[1] >> 16,
            w[1] & 0xffff,
            w[2] >> 16,
            w[2] & 0xffff,
            w[3]
        );
        let mut out = match name.clone() {
            Tag::Compound(c) => c,
            other => {
                let mut c = horizon_core::nbt::Compound::new();
                c.insert("text".into(), other);
                c
            }
        };
        out.insert(
            "hover_event".into(),
            Tag::compound([
                ("action", Tag::String("show_entity".into())),
                ("id", Tag::String(kind.into())),
                ("uuid", Tag::IntArray(uuid.to_vec())),
                ("name", name),
            ]),
        );
        out.insert("insertion".into(), Tag::String(text.as_str().into()));
        Tag::Compound(out)
    }
    /// The screen title: the custom name, else the profession (the client
    /// adds the level itself).
    /// `ServerPlayer.sendMerchantOffers`, when the offers changed.
    fn lan_send_offers(&mut self, conn: u32, gm: &mut GuestMerchant, force: bool) {
        let s = &gm.merchant.session;
        if !force && s.offers == gm.sent && s.level == gm.sent_level && s.xp == gm.sent_xp {
            return;
        }
        if let Some(host) = self.lan.as_mut() {
            let offers: Vec<play::NetOffer> = s
                .offers
                .iter()
                .filter_map(|o| {
                    let cost = |st: &ItemStack, hub: &mut horizon_client::lan::hub::Hub| {
                        hub.stack(&Some(st.clone())).map(play::NetCost::from)
                    };
                    Some(play::NetOffer {
                        cost_a: cost(&o.base_cost_a.stack, &mut host.hub)?,
                        result: host.hub.stack(&Some(o.result.clone()))?,
                        cost_b: match &o.cost_b {
                            Some(b) => Some(cost(&b.stack, &mut host.hub)?),
                            None => None,
                        },
                        out_of_stock: o.out_of_stock(),
                        uses: o.uses,
                        max_uses: o.max_uses,
                        xp: o.xp,
                        special_price: o.special_price_diff,
                        price_multiplier: o.price_multiplier,
                        demand: o.demand,
                    })
                })
                .collect();
            host.hub.send_to(
                conn,
                &C::MerchantOffers(play::MerchantOffers {
                    container: gm.window,
                    offers,
                    level: s.level,
                    xp: s.xp,
                    show_progress: s.villager,
                    can_restock: s.villager,
                }),
            );
            gm.sent = s.offers.clone();
            gm.sent_level = s.level;
            gm.sent_xp = s.xp;
        }
    }
    fn lan_send_merchant_content(&mut self, conn: u32, gm: &mut GuestMerchant) {
        if let Some(host) = self.lan.as_mut() {
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                return;
            };
            let mut items: Vec<Option<ItemStack>> = gm.slots.to_vec();
            let g = &host.hub.guests[gi];
            items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
            let carried = g.carried.clone();
            let net: Vec<_> = items.iter().map(|s| host.hub.stack(s)).collect();
            let carried = host.hub.stack(&carried);
            gm.state = (gm.state + 1) & 0x7FFF;
            host.hub.send_to(
                conn,
                &C::ContainerSetContent(play::ContainerSetContent {
                    container: gm.window,
                    state: gm.state,
                    items: net,
                    carried,
                }),
            );
        }
    }
    /// Runs `op` on the guest's merchant menu (their inventory copied in and
    /// back), then syncs the client. False when the guest has no such window.
    fn lan_merchant_op(
        &mut self,
        conn: u32,
        window: Option<i32>,
        refresh_offers: bool,
        op: impl FnOnce(&mut Menu, &Env),
    ) -> bool {
        let Some(mut gm) = self
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
            .and_then(|p| p.merchant.take())
        else {
            return false;
        };
        if window.is_some_and(|w| w != gm.window) {
            if let Some(host) = self.lan.as_mut() {
                host.play.entry(conn).or_default().merchant = Some(gm);
            }
            return false;
        }
        let Some(gi) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
        else {
            return false;
        };
        let mut session = gm.merchant.session.clone();
        self.merchant_sync_in(&mut session, gm.merchant.mob);
        let next_map = self.peek_map_id();
        let tag = |t: &str| self.structure_tag(t);
        let maps = super::trading::GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = merchant::TradeEnv {
            trades: &self.stations().trades,
            enchantments: &self.stations().enchantments,
            maps: Some(&maps),
        };
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let g = &host.hub.guests[gi];
        let (player_uuid, player_entity_id) = (g.uuid, g.entity_id);
        let env = Env {
            crafting_remainder: Some(self),
            player: Some(player_uuid),
            player_entity_id: Some(player_entity_id),
            loop_context: &self.loop_context,
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: &self.stations().recipes,
            enchantments: &self.stations().enchantments,
            names: &self.pack.names,
            trade: Some(&trade_env),
            maps: None,
            limited_crafting: host.hub.limited_crafting,
            can_equip: None,
            known_recipes: Some(&g.recipe_book),
            slot_rules: Some(self),
        };
        let mut menu = Menu::new(Kind::Merchant);
        menu.set_registry_id(Some("minecraft:merchant"));
        if let Some(host) = self.lan.as_ref() {
            let g = &host.hub.guests[gi];
            for i in 0..36 {
                menu.set_player_item(i, g.inventory[i].clone());
            }
            menu.set_player_item(40, g.offhand.clone());
            menu.selected = g.selected;
            menu.creative = g.creative();
            menu.carried = g.carried.clone();
        }
        for (i, s) in gm.slots.iter().enumerate() {
            menu.set_slot(&env, i, s.clone());
        }
        menu.merchant = Some(session);
        op(&mut menu, &env);
        let made = maps.made.take();
        self.guest_bundle_sounds(gi, std::mem::take(&mut menu.player_sounds));
        let session = menu.merchant.take().expect("attached");
        let dropped = std::mem::take(&mut menu.dropped);
        let crafted_counts = std::mem::take(&mut menu.crafted_counts);
        gm.slots = [
            menu.slot(0).cloned(),
            menu.slot(1).cloned(),
            menu.slot(2).cloned(),
        ];
        if let Some(host) = self.lan.as_mut() {
            host.hub.guests.update(gi, |g| {
                for i in 0..36 {
                    g.inventory[i] = menu.player_item(i).cloned();
                }
                g.offhand = menu.player_item(40).cloned();
                g.carried = menu.carried.take();
            });
        }
        self.register_exploration_maps(made);
        if !crafted_counts.is_empty() {
            self.as_guest(conn, |g| {
                for (item, count) in crafted_counts {
                    g.award_stat("crafted", &item, count);
                }
            });
        }
        let opaque = gm
            .merchant
            .mob
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .is_some_and(|m| {
                m.species() == horizon_client::mobs::Species::Villager
                    && m.extra.profession.index().is_none()
            });
        let offers_refreshed = session
            .events
            .iter()
            .any(|event| matches!(event, merchant::Event::OffersRefreshed));
        let (mut session, traded) = if opaque {
            (session, Vec::new())
        } else {
            self.merchant_apply(session, gm.merchant.mob)
        };
        if !traded.is_empty() {
            // The guest's own statistic and advancements.
            if let Some((uuid, name)) = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
                .map(|g| (g.uuid, g.name.clone()))
                && let Some(entity_id) = gm.merchant.mob
            {
                self.native_player_trade(uuid, &name, entity_id);
            }
            let mob = gm.merchant.mob;
            self.as_guest(conn, |g| {
                g.merchant_traded(mob, &mut session.offers, &traded)
            });
        }
        gm.merchant.session = session;
        self.drop_guest_items(gi, dropped);
        if refresh_offers || offers_refreshed {
            self.lan_send_offers(conn, &mut gm, true);
        }
        self.lan_send_merchant_content(conn, &mut gm);
        if let Some(host) = self.lan.as_mut() {
            host.play.entry(conn).or_default().merchant = Some(gm);
        }
        true
    }
    /// Close an open merchant whose profession became unresolved, returning inputs.
    pub(super) fn close_opaque_merchant(&mut self, id: u32) {
        if self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .is_some_and(|m| m.mob == Some(id))
        {
            self.close_container();
            self.set_screen(super::Screen::Playing);
            self.capture(true);
        }
        if let Some(conn) = self.lan_trade_conn(id) {
            let window = self
                .lan
                .as_ref()
                .and_then(|h| h.play.get(&conn))
                .and_then(|p| p.merchant.as_ref())
                .map(|m| m.window);
            if let Some(window) = window
                && self.lan_merchant_closed(conn, Some(window))
                && let Some(host) = self.lan.as_mut()
            {
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            }
        }
    }

    /// A container click on the guest's trading window.
    pub(super) fn lan_merchant_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let input = match p.input {
            0 => Input::Pickup,
            1 => Input::QuickMove,
            2 => Input::Swap,
            3 => Input::Clone,
            4 => Input::Throw,
            5 => Input::QuickCraft,
            _ => Input::PickupAll,
        };
        let (slot, button) = (i32::from(p.slot), i32::from(p.button));
        self.lan_merchant_op(conn, Some(p.container), false, |menu, env| {
            let _ = menu.click(env, slot, button, input);
        })
    }
    /// ServerboundSelectTradePacket: the selection hint, then tryMoveItems.
    pub(super) fn lan_select_trade(&mut self, conn: u32, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
            .map(|guest| (guest.uuid, guest.name.clone()))
        else {
            return;
        };
        if self.lan_merchant_op(conn, None, false, |menu, env| {
            menu.merchant_select(env, index)
        }) {
            let villager = self
                .lan
                .as_ref()
                .and_then(|host| host.play.get(&conn))
                .and_then(|p| p.merchant.as_ref())
                .and_then(|m| m.merchant.mob);
            self.native_trade_select(uuid, &name, villager, index);
        }
    }
    /// The guest closed the trading window (or it is closed for them):
    /// MerchantMenu.removed, then the villager gets its state back.
    pub(super) fn lan_merchant_closed(&mut self, conn: u32, window: Option<i32>) -> bool {
        if !self.lan_merchant_op(conn, window, false, |menu, env| menu.merchant_close(env)) {
            return false;
        }
        let Some(mut gm) = self
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
            .and_then(|p| p.merchant.take())
        else {
            return false;
        };
        self.merchant_closed(&mut gm.merchant);
        if window.is_some() {
            self.player_menu_closing(conn);
        }
        true
    }
    /// `Merchant.stillValid` for a trader: it is alive and the guest is within
    /// the entity interaction range plus 4 of its box.
    pub(super) fn lan_merchant_valid(
        &self,
        g: &horizon_client::lan::guest::Guest,
        id: u32,
    ) -> bool {
        self.mobs.iter().find(|m| m.id == id).is_some_and(|m| {
            m.alive()
                && (m.species() != horizon_client::mobs::Species::Villager
                    || m.extra.profession.index().is_some())
                && guest_within_entity_range(g, &m.bounds(), 4.0)
        })
    }
    /// `AbstractVillager.stillValid`: close invalid guest merchant screens.
    pub(super) fn lan_merchant_tick(&mut self) {
        // `AbstractVillager.stillValid`: the trade screen closes once the trader
        // is gone or dead, or the guest is more than 4 blocks past its reach.
        let far: Vec<(u32, i32)> = self
            .lan
            .as_ref()
            .map(|host| {
                host.play
                    .iter()
                    .filter_map(|(conn, p)| {
                        let gm = p.merchant.as_ref()?;
                        let id = gm.merchant.mob?;
                        let g = host
                            .hub
                            .guests
                            .iter()
                            .find(|g| g.conn == *conn && host.hub.here(g))?;
                        (!self.lan_merchant_valid(g, id)).then_some((*conn, gm.window))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (conn, window) in far {
            if self.lan_merchant_closed(conn, Some(window))
                && let Some(host) = self.lan.as_mut()
            {
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            }
        }
    }
    /// Refresh open guest prices after the actual villager reputation event.
    pub(super) fn lan_merchant_reputation(&mut self, id: u32) {
        let conns: Vec<u32> = self
            .lan
            .as_ref()
            .map(|host| {
                host.play
                    .iter()
                    .filter(|(_, p)| {
                        p.merchant.as_ref().is_some_and(|m| {
                            m.merchant.session.villager && m.merchant.mob == Some(id)
                        })
                    })
                    .map(|(c, _)| *c)
                    .collect()
            })
            .unwrap_or_default();
        for conn in conns {
            let Some(uuid) = self
                .lan
                .as_ref()
                .and_then(|host| {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.conn == conn && host.hub.here(g))
                })
                .map(|g| uuid_words(g.uuid))
            else {
                continue;
            };
            let reputation = self.villager_reputation_of(id, uuid);
            self.lan_merchant_op(conn, None, true, |menu, env| {
                if let Some(s) = menu.merchant.as_mut() {
                    s.reputation = reputation;
                    s.update_special_prices();
                }
                menu.merchant_update(env);
            });
        }
    }
    /// Takes a station a guest's action opened: a trading screen or a
    /// workstation menu goes to the guest (game/network/lan_stations.rs).
    pub(super) fn lan_station_opened(&mut self, conn: u32, station: super::stations::Station) {
        let mut station = station;
        if station.kind == StationKind::Merchant
            && let Some(m) = station.merchant.take()
        {
            self.lan_open_merchant(conn, *m);
            return;
        }
        self.lan_open_station(conn, &station);
    }
}

impl Game<'_> {
    /// The guest whose trading screen is open on mob `mob`.
    pub(super) fn lan_trade_conn(&self, mob: u32) -> Option<u32> {
        let host = self.lan.as_ref()?;
        host.play
            .iter()
            .find(|(_, p)| {
                p.merchant
                    .as_ref()
                    .is_some_and(|m| m.merchant.mob == Some(mob))
            })
            .map(|(conn, _)| *conn)
    }
    /// The live session of a guest's open trading screen, whose offers,
    /// experience and level are newer than the mob's until the screen closes.
    pub(super) fn lan_trade_session(&self, conn: u32) -> Option<&merchant::Session> {
        let gm = self.lan.as_ref()?.play.get(&conn)?.merchant.as_ref()?;
        Some(&gm.merchant.session)
    }
    /// The guest's player UUID.
    pub(super) fn lan_trade_player(&self, conn: u32) -> Option<u128> {
        Some(self.lan.as_ref()?.hub.guest(conn)?.uuid)
    }
    /// Changes the session of a guest's open trading screen and sends the
    /// client its offers and result slot again. The merchant's own random is
    /// not drawn: the update sound is held back.
    pub(super) fn lan_trade_edit(
        &mut self,
        conn: u32,
        edit: impl FnOnce(&mut merchant::Session),
    ) -> bool {
        let done = self.lan_merchant_op(conn, None, false, |menu, env| {
            let Some(session) = menu.merchant.as_mut() else {
                return;
            };
            edit(session);
            session.selection_hint = session
                .selection_hint
                .min(session.offers.len().saturating_sub(1));
            let sound_time = std::mem::replace(&mut session.ambient_sound_time, -80);
            menu.merchant_update(env);
            if let Some(session) = menu.merchant.as_mut() {
                session.ambient_sound_time = sound_time;
            }
        });
        if done {
            self.lan_trade_sequences(conn);
        }
        done
    }
    /// Lends the open session the world sequences of the levels above its
    /// level, and gives back the ones it no longer needs, so a level-up after
    /// the level was changed still rolls its trade set.
    fn lan_trade_sequences(&mut self, conn: u32) {
        let Some(session) = self.lan_trade_session(conn) else {
            return;
        };
        if !session.villager {
            return;
        }
        let needed = merchant::Session::sequences_needed(
            &self.stations().trades,
            &session.profession,
            session.level,
        );
        let Some(gm) = self
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
            .and_then(|p| p.merchant.as_mut())
        else {
            return;
        };
        let session = &mut gm.merchant.session;
        let stale: Vec<String> = session
            .sequences
            .keys()
            .filter(|id| !needed.contains(id))
            .cloned()
            .collect();
        for id in stale {
            if let Some(random) = session.sequences.remove(&id) {
                self.rand_sequences.give_back(id, random);
            }
        }
        for id in needed {
            if let std::collections::btree_map::Entry::Vacant(slot) = session.sequences.entry(id) {
                let random = self.rand_sequences.lend(slot.key(), self.level.seed);
                slot.insert(random);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_merchant_refs::tests::{offer, spawn};
    use super::*;
    use crate::game::lan::LanSettings;
    use horizon_client::{
        lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
        merchant::{MerchantOffer, Trader},
    };
    use horizon_protocol::packets::configuration::ClientInformation;
    use mod_api::ServerHost;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };

    /// A hub with one guest (conn 12) trading with a farmer villager (mob 40)
    /// at `level`; the guest's channel comes back with the villager.
    fn open_trade(
        game: &mut Game<'_>,
        level: i32,
    ) -> (
        mod_api::MerchantRef,
        std::sync::mpsc::Receiver<horizon_client::lan::Out>,
        MerchantOffer,
    ) {
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: game.dim.id().to_owned(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.level_shapes,
            level_seeds: game.levels.runtime.seeds(),
        };
        game.lan_host(settings, net, config).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 12,
                uuid: 12,
                name: "guest12".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            12,
            [3.0, 64.0, 5.0],
            10,
        );
        guest.placed = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);

        let villager = spawn(game, 40, "minecraft:villager");
        game.mobs[0].extra.profession =
            horizon_client::mobs::villager::ProfessionState::from_index(5);
        let profession = "minecraft:farmer".to_owned();
        let first = offer("minecraft:wheat", "minecraft:emerald");
        game.mobs[0].extra.trade = Some(Box::new(Trader {
            offers: vec![first.clone()],
            ..Trader::default()
        }));
        game.mobs[0].extra.flag2 = true;
        let mut session = merchant::Session::new(vec![first.clone()], true);
        session.level = level;
        session.profession = profession;
        session.xp = 0;
        session.variant = "minecraft:plains".into();
        for id in
            merchant::Session::sequences_needed(&game.stations().trades, &session.profession, level)
        {
            let random = game.rand_sequences.lend(&id, game.level.seed);
            session.sequences.insert(id, random);
        }
        let gm = GuestMerchant {
            window: 3,
            state: 0,
            slots: [None, None, None],
            merchant: Box::new(MerchantState {
                session: Box::new(session),
                scroll: 0,
                dragging: false,
                focused: None,
                mob: Some(40),
                name: None,
            }),
            sent: vec![first.clone()],
            sent_level: level,
            sent_xp: 0,
        };
        game.lan
            .as_mut()
            .unwrap()
            .play
            .entry(12)
            .or_default()
            .merchant = Some(gm);
        (villager, rx, first)
    }

    #[test]
    #[ignore = "daily"]
    fn changing_an_open_merchant_to_an_unknown_profession_closes_without_trading() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            let (_villager, rx, mut offer) = open_trade(game, 1);
            offer.uses = 3;
            {
                let gm = game
                    .lan
                    .as_mut()
                    .unwrap()
                    .play
                    .get_mut(&12)
                    .unwrap()
                    .merchant
                    .as_mut()
                    .unwrap();
                gm.merchant.session.offers[0] = offer.clone();
                gm.merchant.session.xp = 17;
                gm.slots[1] = Some(ItemStack::new("minecraft:coal", 2));
            }
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(ItemStack::new("minecraft:apple", 3))
            });
            let wheat = ItemStack::new("minecraft:wheat", 1);
            game.lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&12)
                .unwrap()
                .merchant
                .as_mut()
                .unwrap()
                .slots[0] = Some(wheat);
            let mut tag = game
                .command_target_nbt(super::super::commands::Target::Mob(40))
                .unwrap();
            if let Tag::Compound(data) = &mut tag {
                data.insert(
                    "VillagerData".into(),
                    Tag::compound([
                        ("type", Tag::String("minecraft:plains".into())),
                        ("profession", Tag::String("example:engineer".into())),
                        ("level", Tag::Int(4)),
                    ]),
                );
            }
            game.apply_entity_nbt(super::super::commands::Target::Mob(40), tag)
                .unwrap();
            assert!(game.lan.as_ref().unwrap().play[&12].merchant.is_none());
            game.lan_guest_packet(
                12,
                play::Serverbound::SelectTrade(play::SelectTrade { item: 0 }),
            );
            for input in [0, 1] {
                game.lan_guest_packet(
                    12,
                    play::Serverbound::ContainerClick(play::ContainerClick {
                        container: 3,
                        state: 0,
                        slot: 2,
                        button: 0,
                        input,
                        changed: vec![],
                        carried: None,
                    }),
                );
            }
            assert_eq!(game.mobs[0].extra.profession.id(), "example:engineer");
            assert_eq!(game.mobs[0].extra.villager_level, 4);
            assert_eq!(game.mobs[0].extra.trade.as_ref().unwrap().xp, 17);
            assert!(rx.try_iter().any(|out| match out {
                horizon_client::lan::Out::Packet(body) => matches!(
                    play::Clientbound::decode(body[0] as i32, &body[1..]),
                    Ok(play::Clientbound::ContainerClose(_))
                ),
                _ => false,
            }));
            assert_eq!(game.missing_professions.len(), 1);
            let tag = horizon_client::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                Some(game.pack.assets.registry),
                |_| None,
            );
            game.apply_entity_nbt(super::super::commands::Target::Mob(40), tag)
                .unwrap();
            assert_eq!(game.missing_professions.len(), 1);
            let second = spawn(game, 41, "minecraft:villager");
            let mut tag = horizon_client::entity_nbt::mob_to_nbt(
                &game.mobs[1],
                [0; 4],
                None,
                Some(game.pack.assets.registry),
                |_| None,
            );
            if let Tag::Compound(data) = &mut tag {
                data.insert(
                    "VillagerData".into(),
                    Tag::compound([
                        ("type", Tag::String("minecraft:plains".into())),
                        ("profession", Tag::String("example:engineer".into())),
                        ("level", Tag::Int(1)),
                    ]),
                );
            }
            game.apply_entity_nbt(
                super::super::commands::Target::Mob(second.entity().id()),
                tag.clone(),
            )
            .unwrap();
            assert_eq!(game.missing_professions.len(), 1);
            game.missing_professions.clear();
            game.apply_entity_nbt(
                super::super::commands::Target::Mob(second.entity().id()),
                tag,
            )
            .unwrap();
            assert_eq!(game.missing_professions.len(), 1);
            assert_eq!(
                game.mobs[0].extra.trade.as_ref().unwrap().offers,
                vec![offer]
            );
            let inventory = &game.lan.as_ref().unwrap().hub.guests[0].inventory;
            for (id, count) in [
                ("minecraft:wheat", 1),
                ("minecraft:coal", 2),
                ("minecraft:apple", 3),
            ] {
                assert_eq!(
                    inventory
                        .iter()
                        .flatten()
                        .filter(|s| &*s.item == id)
                        .map(|s| s.count)
                        .sum::<u32>(),
                    count
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn explicit_nbt_offers_and_xp_override_the_live_trade() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            let (_, _, _) = open_trade(game, 1);
            let session = &mut game
                .lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&12)
                .unwrap()
                .merchant
                .as_mut()
                .unwrap()
                .merchant
                .session;
            session.offers[0].uses = 3;
            session.xp = 17;
            let target = super::super::commands::Target::Mob(40);
            let old = game.command_target_nbt(target).unwrap();
            let replacement = offer("minecraft:coal", "minecraft:diamond");
            let patch = Tag::compound([
                (
                    "VillagerData",
                    Tag::compound([
                        ("profession", Tag::String("example:engineer".into())),
                        ("level", Tag::Int(4)),
                    ]),
                ),
                (
                    "Offers",
                    horizon_client::entity_nbt::offers_to_nbt(std::slice::from_ref(&replacement)),
                ),
                ("Xp", Tag::Int(9)),
            ]);
            game.apply_entity_nbt(target, horizon_client::data_cmd::merged(&old, &patch))
                .unwrap();
            let trade = game.mobs[0].extra.trade.as_ref().unwrap();
            assert_eq!(trade.offers, vec![replacement]);
            assert_eq!(trade.xp, 9);
            assert_eq!(game.mobs[0].extra.villager_level, 4);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_merchant_edit_reaches_the_guest_trading_with_it() {
        with_game(|game| {
            let (villager, rx, first) = open_trade(game, 1);

            // MerchantOffers packets (id 53) the guest was sent since last asked.
            let sent_offers = || {
                let mut n = 0;
                while let Ok(out) = rx.try_recv() {
                    let bodies = match out {
                        horizon_client::lan::Out::Packet(body) => vec![body],
                        horizon_client::lan::Out::Packets(bodies) => bodies,
                        _ => Vec::new(),
                    };
                    n += bodies.iter().filter(|b| b.first() == Some(&53)).count();
                }
                n
            };
            sent_offers();
            let player = villager.trading_player(game).unwrap();
            assert_eq!(player.uuid(), 12);
            let second = offer("minecraft:coal", "minecraft:diamond");
            assert!(
                villager
                    .add_offer(game, second.try_into_api().unwrap())
                    .unwrap()
            );
            assert_eq!(sent_offers(), 1);
            let mut changed = first.clone();
            changed.uses = 9;
            assert!(
                villager
                    .set_offer(game, 0, changed.try_into_api().unwrap())
                    .unwrap()
            );
            assert!(villager.set_xp(game, 40));
            assert!(villager.set_level(game, 3));
            // The profession cannot change under an open screen.
            assert!(!villager.set_profession(game, "minecraft:cleric"));
            assert!(
                !villager
                    .set_offer(game, 7, changed.try_into_api().unwrap())
                    .unwrap()
            );
            assert_eq!(sent_offers(), 3);
            // The mob still has the old offers; the screen has the new ones.
            assert_eq!(
                game.mobs[0].extra.trade.as_ref().unwrap().offers,
                vec![first]
            );
            let live = vec![changed.clone(), second.clone()];
            assert_eq!(villager.offers(game), Some(live.try_into_api().unwrap()));
            assert_eq!(villager.xp(game), Some(40));
            assert_eq!(villager.level(game), Some(3));
            {
                let gm = game.lan.as_ref().unwrap().play[&12]
                    .merchant
                    .as_ref()
                    .unwrap();
                assert_eq!(gm.sent, live);
                assert_eq!(gm.sent_level, 3);
                assert_eq!(gm.merchant.session.offers, live);
            }
            assert!(villager.remove_offer(game, 1));
            let gm = game.lan.as_ref().unwrap().play[&12]
                .merchant
                .as_ref()
                .unwrap();
            assert_eq!(gm.sent, vec![changed.clone()]);
            assert_eq!(sent_offers(), 1);

            // Closing the screen gives the mob the edited trades.
            let mut gm = game
                .lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&12)
                .unwrap()
                .merchant
                .take()
                .unwrap();
            game.merchant_closed(&mut gm.merchant);
            let trade = game.mobs[0].extra.trade.as_ref().unwrap();
            assert_eq!((trade.offers.clone(), trade.xp), (vec![changed], 40));
            assert_eq!(game.mobs[0].extra.villager_level, 3);
            assert_eq!(villager.trading_player(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_lowered_level_still_levels_up_with_the_next_trade_set() {
        with_game(|game| {
            let (villager, _rx, first) = open_trade(game, 3);
            let before = game.lan_trade_session(12).unwrap().sequences.len();
            assert_eq!(before, 2, "levels 4 and 5 were lent");
            assert!(villager.set_level(game, 1));
            assert!(villager.set_xp(game, 5));
            assert_eq!(game.lan_trade_session(12).unwrap().sequences.len(), 4);
            // The trade makes 12 experience: past level 1's 10, so level 2 is rolled.
            let mut gm = game
                .lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&12)
                .unwrap()
                .merchant
                .take()
                .unwrap();
            let items = game.items();
            let trade_env = merchant::TradeEnv {
                trades: &game.stations().trades,
                enchantments: &game.stations().enchantments,
                maps: None,
            };
            let session = &mut gm.merchant.session;
            session.origin = [0.0; 3];
            assert!(session.notify_trade(0, Some(&trade_env), items));
            assert_eq!(session.level, 2);
            assert!(session.offers.len() > 1, "level 2 offers were added");
            assert_eq!(session.offers[0].uses, first.uses + 1);
            game.lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&12)
                .unwrap()
                .merchant = Some(gm);
            // Level 7: nothing more to lend, and no further level-up.
            assert!(villager.set_level(game, 7));
            assert_eq!(villager.level(game), Some(7));
            assert!(game.lan_trade_session(12).unwrap().sequences.is_empty());
            assert!(villager.set_level(game, i32::MAX));
            assert_eq!(villager.level(game), Some(i32::MAX));
            assert!(game.lan_trade_session(12).unwrap().sequences.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_type_change_reaches_the_open_session_before_a_level_up() {
        with_game(|game| {
            let (villager, _rx, _) = open_trade(game, 4);
            assert!(villager.set_villager_type(game, "minecraft:swamp"));
            assert_eq!(
                game.lan_trade_session(12).unwrap().variant,
                "minecraft:swamp"
            );
            assert_eq!(game.mobs[0].extra.variant, 5);
            assert_eq!(
                villager.villager_type(game).as_deref(),
                Some("minecraft:swamp")
            );
        });
    }

    /// What the `trade_select` observer saw: the villager and offer index it
    /// was told, and what the merchant handle answered inside the hook.
    type Seen = Arc<std::sync::Mutex<Vec<(Option<u32>, usize, bool, bool)>>>;

    #[test]
    #[ignore = "daily"]
    fn a_trade_hook_fired_by_a_real_select_can_write_the_open_trade() {
        use mod_api::{self, Hooks, TradeSelectEvent};
        struct Observer(Seen);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn trade_select(&self, host: &mut dyn ServerHost, event: &mut TradeSelectEvent<'_>) {
                let merchant = event
                    .villager
                    .and_then(|id| host.entity(id))
                    .and_then(|entity| host.merchant(entity));
                let wrote = merchant.is_some_and(|m| {
                    m.add_offer(
                        host,
                        offer("minecraft:wheat", "minecraft:emerald")
                            .try_into_api()
                            .unwrap(),
                    )
                    .unwrap()
                });
                let player = merchant.is_some_and(|m| m.trading_player(host).is_some());
                self.0
                    .lock()
                    .unwrap()
                    .push((event.villager, event.index, wrote, player));
            }
        }
        let seen = Seen::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "merchant-observer",
            Box::new(Observer(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (villager, _rx, first) = open_trade(game, 1);
                let second = offer("minecraft:coal", "minecraft:diamond");
                assert!(
                    villager
                        .add_offer(game, second.try_into_api().unwrap())
                        .unwrap()
                );
                game.lan_select_trade(12, 1);
                // The selected menu is restored before the hook, so its live
                // session and trading player are readable and writable there.
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0], (Some(40), 1, true, true));
                assert_eq!(
                    villager.offers(game),
                    Some(
                        vec![first, second, offer("minecraft:wheat", "minecraft:emerald")]
                            .try_into_api()
                            .unwrap()
                    )
                );
            });
        });
    }
}
