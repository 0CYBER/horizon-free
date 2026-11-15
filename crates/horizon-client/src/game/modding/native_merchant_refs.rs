//! Merchant handles for native server mods: villagers and wandering traders.
use super::{Game, commands::Target};
use horizon_client::{
    merchant::{MerchantOffer, Trader},
    mobs::{Species, render::VILLAGER_TYPES, villager},
};
use horizon_core::identifier::Identifier;
use mod_api::{EntityRef, MerchantRef, PlayerRef};

/// The index of a `minecraft:` registry id in a table of bare names.
fn registry_index(id: &str, names: &[&str]) -> Option<u8> {
    let id = Identifier::parse(id).ok()?;
    if id.namespace() != "minecraft" {
        return None;
    }
    names.iter().position(|n| *n == id.path()).map(|i| i as u8)
}

impl Game<'_> {
    /// An item stack a trade can hold: a nonempty stack of an item the game
    /// has (a client would otherwise be sent air for it).
    fn merchant_real(&self, stack: &horizon_client::items::ItemStack) -> bool {
        stack.count > 0
            && &*stack.item != "minecraft:air"
            && self.items().get(&stack.item).is_some()
    }

    fn merchant_valid_offer(&self, offer: &MerchantOffer) -> bool {
        self.merchant_real(&offer.base_cost_a.stack)
            && offer
                .cost_b
                .as_ref()
                .is_none_or(|b| self.merchant_real(&b.stack))
            && self.merchant_real(&offer.result)
    }
    fn native_merchant_index(&self, merchant: MerchantRef) -> Option<usize> {
        let Target::Mob(id) = self.native_entity_target(merchant.entity())? else {
            return None;
        };
        self.mobs.iter().position(|m| {
            m.id == id
                && !m.removed
                && matches!(m.species(), Species::Villager | Species::WanderingTrader)
        })
    }

    fn native_villager_index(&self, merchant: MerchantRef) -> Option<usize> {
        let index = self.native_merchant_index(merchant)?;
        (self.mobs[index].species() == Species::Villager).then_some(index)
    }

    /// Reads the offers, experience and level (1 for a trader) of the mob at
    /// `index`: from a guest's open trading screen if there is one, whose copy
    /// is newer until it closes. A trader whose offers are not rolled yet has none.
    fn merchant_read<T>(
        &self,
        index: usize,
        read: impl FnOnce(&[MerchantOffer], i32, i32) -> T,
    ) -> T {
        self.merchant_read_lent(&self.mobs[index], read)
    }

    /// The same read while an AI tick lends this mob out of `self.mobs`.
    pub(super) fn merchant_read_lent<T>(
        &self,
        mob: &horizon_client::mobs::Mob,
        read: impl FnOnce(&[MerchantOffer], i32, i32) -> T,
    ) -> T {
        if mob.species() == Species::Villager
            && self
                .stations()
                .professions
                .state_index(&mob.extra.profession)
                .is_none()
        {
            return read(
                &[],
                mob.extra.trade.as_deref().map_or(0, |t| t.xp),
                mob.extra.villager_level.max(1),
            );
        }
        if let Some(session) = self
            .lan_trade_conn(mob.id)
            .and_then(|conn| self.lan_trade_session(conn))
        {
            return read(&session.offers, session.xp, session.level);
        }
        let level = mob.extra.villager_level.max(1);
        match mob.extra.trade.as_deref() {
            Some(t) if !t.unset => read(&t.offers, t.xp, level),
            Some(t) => read(&[], t.xp, level),
            None => read(&[], 0, level),
        }
    }

    /// Changes the offers, experience and level of the mob at `index`: on a
    /// guest's open trading screen if there is one (and resent to them), else on
    /// the mob. False when a trade is open somewhere this cannot reach (the host
    /// window, or inside a hook of that trade's own click), or `edit` refused.
    fn merchant_write(
        &mut self,
        index: usize,
        edit: impl FnOnce(&mut Vec<MerchantOffer>, &mut i32, &mut i32) -> bool,
    ) -> bool {
        let mob = &self.mobs[index];
        if mob.species() == Species::Villager
            && self
                .stations()
                .professions
                .state_index(&mob.extra.profession)
                .is_none()
        {
            return false;
        }
        let id = mob.id;
        if let Some(conn) = self.lan_trade_conn(id) {
            let mut done = false;
            let sent = self.lan_trade_edit(conn, |session| {
                done = edit(&mut session.offers, &mut session.xp, &mut session.level);
                if done {
                    session
                        .events
                        .push(horizon_client::merchant::Event::OffersRefreshed);
                }
            });
            return sent && done;
        }
        let mob = &mut self.mobs[index];
        if mob.extra.flag2 {
            return false;
        }
        let trader = mob.extra.trade.get_or_insert_with(|| {
            Box::new(Trader {
                unset: true,
                ..Trader::default()
            })
        });
        let mut level = mob.extra.villager_level.max(1);
        let done = edit(&mut trader.offers, &mut trader.xp, &mut level);
        if done && mob.species() == Species::Villager {
            mob.extra.villager_level = level.max(1);
        }
        done
    }

    /// Narrows an entity to a villager or wandering trader, or None for
    /// another kind, a stale handle, or a client.
    pub(super) fn merchant(&self, entity: EntityRef) -> Option<MerchantRef> {
        let merchant = MerchantRef::new(entity);
        self.native_merchant_index(merchant).map(|_| merchant)
    }

    /// `AbstractVillager.getOffers`, as they are now (a copy). Offers not
    /// rolled yet read as empty: they are rolled when a player first trades,
    /// or by `add_offer`; reading never draws a random.
    pub(super) fn merchant_offers(&self, merchant: MerchantRef) -> Option<Vec<MerchantOffer>> {
        let index = self.native_merchant_index(merchant)?;
        Some(self.merchant_read(index, |offers, _, _| offers.to_vec()))
    }

    /// Replaces offer `index` (a player trading with it is sent the new
    /// offers). False for an index past the end, a stale handle, or an offer
    /// without a real item in `base_cost_a` or `result` (or `cost_b` when set).
    pub(super) fn merchant_set_offer(
        &mut self,
        merchant: MerchantRef,
        index: u32,
        offer: MerchantOffer,
    ) -> bool {
        let Some(mob) = self.native_merchant_index(merchant) else {
            return false;
        };
        if !self.merchant_valid_offer(&offer) {
            return false;
        }
        self.merchant_write(mob, |offers, _, _| {
            let Some(slot) = offers.get_mut(index as usize) else {
                return false;
            };
            *slot = offer;
            true
        })
    }

    /// Appends an offer, as `getOffers().add(offer)` does. Offers not rolled
    /// yet are rolled first, as `getOffers` does for a player's first trade
    /// (this draws the trade set's world random sequence, as that does).
    /// False for a stale handle or an invalid offer (see `set_offer`).
    pub(super) fn merchant_add_offer(
        &mut self,
        merchant: MerchantRef,
        offer: MerchantOffer,
    ) -> bool {
        let Some(mob) = self.native_merchant_index(merchant) else {
            return false;
        };
        if !self.merchant_valid_offer(&offer) {
            return false;
        }
        if self.lan_trade_conn(self.mobs[mob].id).is_none() && !self.mobs[mob].extra.flag2 {
            self.ensure_offers(mob);
        }
        self.merchant_write(mob, |offers, _, _| {
            offers.push(offer);
            true
        })
    }

    /// Removes offer `index`, the ones after it moving up. False for an index
    /// past the end or a stale handle.
    pub(super) fn merchant_remove_offer(&mut self, merchant: MerchantRef, index: u32) -> bool {
        let Some(mob) = self.native_merchant_index(merchant) else {
            return false;
        };
        self.merchant_write(mob, |offers, _, _| {
            if index as usize >= offers.len() {
                return false;
            }
            offers.remove(index as usize);
            true
        })
    }

    /// `Merchant.getTradingPlayer`: the player whose trading screen is open on
    /// it. None when nobody trades, or the screen is the host window's.
    pub(super) fn merchant_trading_player(&self, merchant: MerchantRef) -> Option<PlayerRef> {
        let index = self.native_merchant_index(merchant)?;
        let conn = self.lan_trade_conn(self.mobs[index].id)?;
        self.player(self.lan_trade_player(conn)?)
    }

    /// `VillagerData.profession`, a registry id (`minecraft:farmer`; none is
    /// `minecraft:none`). None for a wandering trader.
    pub(super) fn merchant_profession(&self, merchant: MerchantRef) -> Option<String> {
        let mob = &self.mobs[self.native_villager_index(merchant)?];
        Some(mob.extra.profession.id())
    }

    /// `Villager.setVillagerData` with a new profession: a different one
    /// forgets the offers, which roll again on the next trade. Clients see the
    /// new profession through entity data. False for an unknown id, a wandering
    /// trader, or a villager with a trading screen open.
    pub(super) fn merchant_set_profession(
        &mut self,
        merchant: MerchantRef,
        profession: &str,
    ) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        let Some(profession) = Identifier::parse(profession)
            .ok()
            .and_then(|id| self.stations().professions.resolve(&id.to_string()))
        else {
            return false;
        };
        let id = self.mobs[index].id;
        if self
            .stations()
            .professions
            .state_index(&self.mobs[index].extra.profession)
            .is_none()
            || self.mobs[index].extra.flag2
            || self.lan_trade_conn(id).is_some()
        {
            return false;
        }
        let table = self.stations().professions.clone();
        villager::set_profession(&mut self.mobs[index], &table, profession);
        true
    }

    /// `VillagerData.level`, at least 1.
    pub(super) fn merchant_level(&self, merchant: MerchantRef) -> Option<i32> {
        let index = self.native_villager_index(merchant)?;
        Some(self.merchant_read(index, |_, _, level| level.max(1)))
    }

    /// `VillagerData.withLevel`: `Math.max(1, level)`, with no upper limit in
    /// vanilla. The offers stay as they are; the next level-up adds its trades
    /// (a level past 5 can't level up). A trading guest's session follows,
    /// and gets the sequences a level-up from the new level needs.
    pub(super) fn merchant_set_level(&mut self, merchant: MerchantRef, level: i32) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        let level = level.max(1);
        self.merchant_write(index, |_, _, current| {
            *current = level;
            true
        })
    }

    /// `VillagerData.type`, a registry id (`minecraft:plains`).
    pub(super) fn merchant_villager_type(&self, merchant: MerchantRef) -> Option<String> {
        let mob = &self.mobs[self.native_villager_index(merchant)?];
        let name = VILLAGER_TYPES[mob.extra.variant as usize % VILLAGER_TYPES.len()];
        Some(format!("minecraft:{name}"))
    }

    /// `VillagerData.withType`: also the type a trading guest's session
    /// rolls its next level-up offers with. False for an unknown id, a
    /// wandering trader, or a trade open where this can't reach.
    pub(super) fn merchant_set_villager_type(&mut self, merchant: MerchantRef, kind: &str) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        let Some(kind) = registry_index(kind, &VILLAGER_TYPES) else {
            return false;
        };
        let id = self.mobs[index].id;
        if let Some(conn) = self.lan_trade_conn(id) {
            let variant = format!("minecraft:{}", VILLAGER_TYPES[kind as usize]);
            if !self.lan_trade_edit(conn, |session| session.variant = variant) {
                return false;
            }
        } else if self.mobs[index].extra.flag2 {
            return false;
        }
        self.mobs[index].extra.variant = kind;
        true
    }

    /// `Villager.getVillagerXp`.
    pub(super) fn merchant_xp(&self, merchant: MerchantRef) -> Option<i32> {
        let index = self.native_villager_index(merchant)?;
        Some(self.merchant_read(index, |_, xp, _| xp))
    }

    /// `Villager.setVillagerXp`: assigned as given, as vanilla does.
    pub(super) fn merchant_set_xp(&mut self, merchant: MerchantRef, xp: i32) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        self.merchant_write(index, |_, current, _| {
            *current = xp;
            true
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::api_values::{StackReadError, TryIntoApi, TryIntoEngine};
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::{items::ItemStack, merchant::ItemCost, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use mod_api::WorldEpoch;

    pub(in crate::game) fn offer(cost: &str, result: &str) -> MerchantOffer {
        MerchantOffer {
            base_cost_a: ItemCost {
                stack: ItemStack::new(cost, 20),
            },
            cost_b: None,
            result: ItemStack::new(result, 1),
            uses: 1,
            max_uses: 12,
            reward_exp: true,
            special_price_diff: -2,
            demand: 3,
            price_multiplier: 0.05,
            xp: 7,
        }
    }

    pub(in crate::game) fn spawn(game: &mut Game<'_>, id: u32, kind: &str) -> MerchantRef {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
        let entity = game.entity(id).unwrap();
        game.merchant(entity).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn unresolved_profession_preserves_offers_but_cannot_trade_them() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let villager = spawn(game, 40, "minecraft:villager");
            game.mobs[0].extra.profession =
                horizon_client::mobs::villager::ProfessionState::from_id("example:engineer");
            game.mobs[0].extra.trade = Some(Box::new(Trader {
                offers: vec![offer("minecraft:wheat", "minecraft:emerald")],
                ..Trader::default()
            }));
            assert_eq!(villager.offers(game), Some(Vec::new()));
            assert!(!villager.set_profession(game, "minecraft:farmer"));
            assert!(game.interact_trader(0, "minecraft:air").unwrap());
            assert!(game.station.is_none());
            assert_eq!(game.mobs[0].extra.trade.as_ref().unwrap().offers.len(), 1);
            assert_eq!(
                villager.profession(game).as_deref(),
                Some("example:engineer")
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn merchant_handles_edit_offers_and_villager_data() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let villager = spawn(game, 40, "minecraft:villager");
            let trader = spawn(game, 41, "minecraft:wandering_trader");
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, zombie, Vec3::ZERO, 0.0));
            assert!(game.merchant(game.entity(42).unwrap()).is_none());

            // Nothing is rolled yet; reading and a refused write draw nothing.
            assert_eq!(villager.offers(game), Some(Vec::new()));
            assert!(!villager.remove_offer(game, 0));
            assert_eq!(
                villager.set_offer(
                    game,
                    0,
                    offer("minecraft:wheat", "minecraft:emerald")
                        .try_into_api()
                        .unwrap()
                ),
                Ok(false)
            );
            assert!(game.mobs[0].extra.trade.as_ref().is_none_or(|t| t.unset));

            // A wandering trader has offers but no villager data.
            assert_eq!(
                trader.add_offer(
                    game,
                    offer("minecraft:emerald", "minecraft:diamond")
                        .try_into_api()
                        .unwrap()
                ),
                Ok(true)
            );
            assert!(trader.offers(game).unwrap().len() > 1);
            assert_eq!(trader.profession(game), None);
            assert!(!trader.set_level(game, 2));
            assert!(!trader.set_xp(game, 5));
            assert_eq!(trader.villager_type(game), None);

            // Put in known offers, as a rolled villager has them.
            game.mobs[0].extra.trade = Some(Box::new(Trader {
                offers: vec![
                    offer("minecraft:wheat", "minecraft:emerald"),
                    offer("minecraft:coal", "minecraft:emerald"),
                ],
                ..Trader::default()
            }));
            let mut changed = offer("minecraft:carrot", "minecraft:diamond");
            changed.uses = 5;
            assert_eq!(
                villager.set_offer(game, 1, changed.try_into_api().unwrap()),
                Ok(true)
            );
            assert_eq!(
                villager.offers(game).unwrap()[1],
                changed.try_into_api().unwrap()
            );
            assert_eq!(
                villager.set_offer(game, 2, changed.try_into_api().unwrap()),
                Ok(false)
            );
            // Empty and unknown engine stacks are refused before an API offer exists.
            let mut bad = changed.clone();
            bad.result.count = 0;
            assert_eq!(bad.try_into_api(), Err(StackReadError::Empty));
            for (a, b, r) in [
                ("minecraft:not_an_item", None, "minecraft:emerald"),
                (
                    "minecraft:wheat",
                    Some("minecraft:not_an_item"),
                    "minecraft:emerald",
                ),
                ("minecraft:wheat", None, "minecraft:not_an_item"),
            ] {
                let mut bad = changed.clone();
                bad.base_cost_a.stack = ItemStack::new(a, 1);
                bad.cost_b = b.map(|b| ItemCost {
                    stack: ItemStack::new(b, 1),
                });
                bad.result = ItemStack::new(r, 1);
                assert_eq!(bad.try_into_api(), Err(StackReadError::UnknownItem));
            }
            assert_eq!(
                villager.add_offer(game, changed.try_into_api().unwrap()),
                Ok(true)
            );
            assert_eq!(villager.offers(game).unwrap().len(), 3);
            assert!(villager.remove_offer(game, 0));
            assert!(!villager.remove_offer(game, 5));
            assert_eq!(
                villager.offers(game).unwrap(),
                vec![changed.clone(), changed].try_into_api().unwrap()
            );
            assert_eq!(villager.trading_player(game), None);

            assert_eq!(villager.profession(game).as_deref(), Some("minecraft:none"));
            assert!(!villager.set_profession(game, "minecraft:wizard"));
            assert!(!villager.set_profession(game, "farmer_x"));
            assert!(villager.set_profession(game, "minecraft:farmer"));
            assert_eq!(
                villager.profession(game).as_deref(),
                Some("minecraft:farmer")
            );
            // A new profession forgets the offers; the same one keeps them.
            assert_eq!(villager.offers(game), Some(Vec::new()));
            game.mobs[0].extra.trade.as_mut().unwrap().unset = false;
            game.mobs[0].extra.trade.as_mut().unwrap().offers =
                vec![offer("minecraft:wheat", "minecraft:emerald")];
            assert!(villager.set_profession(game, "minecraft:farmer"));
            assert_eq!(villager.offers(game).unwrap().len(), 1);

            assert_eq!(villager.level(game), Some(1));
            assert!(villager.set_level(game, 4));
            assert_eq!(villager.level(game), Some(4));
            assert_eq!(game.mobs[0].extra.villager_level, 4);
            assert!(villager.set_level(game, 123456));
            assert_eq!(villager.level(game), Some(123456));
            assert_eq!(game.mobs[0].extra.villager_level, 123456);
            // Vanilla: `Math.max(1, level)`, no upper limit.
            assert!(villager.set_level(game, 0));
            assert_eq!(villager.level(game), Some(1));
            assert!(villager.set_level(game, 6));
            assert_eq!(villager.level(game), Some(6));
            assert!(villager.set_level(game, 1000));
            assert_eq!(villager.level(game), Some(1000));
            assert!(villager.set_level(game, 4));
            assert_eq!(
                villager.villager_type(game).as_deref(),
                Some("minecraft:plains")
            );
            assert!(villager.set_villager_type(game, "minecraft:desert"));
            assert_eq!(
                villager.villager_type(game).as_deref(),
                Some("minecraft:desert")
            );
            assert!(!villager.set_villager_type(game, "minecraft:mars"));
            assert_eq!(villager.xp(game), Some(0));
            assert!(villager.set_xp(game, 80));
            assert_eq!(villager.xp(game), Some(80));
            assert!(villager.set_xp(game, -1));
            assert_eq!(villager.xp(game), Some(-1));
            assert_eq!(villager.offers(game).unwrap().len(), 1);

            // Saved with the entity (`Offers`) and read back unchanged.
            let saved = villager
                .offers(game)
                .unwrap()
                .try_into_engine(game.items())
                .unwrap();
            let tag = horizon_client::entity_nbt::offers_to_nbt(&saved);
            assert_eq!(horizon_client::entity_nbt::offers_from_nbt(&tag), saved);

            let stale = MerchantRef::new(EntityRef::new(
                WorldEpoch::new(villager.entity().epoch().get() + 1),
                villager.entity().id(),
            ));
            assert_eq!(stale.offers(game), None);
            assert!(!stale.set_xp(game, 1));
            assert_eq!(
                stale.add_offer(
                    game,
                    offer("minecraft:wheat", "minecraft:emerald")
                        .try_into_api()
                        .unwrap()
                ),
                Ok(false)
            );
            let taken = game.mobs.lift();
            assert_eq!(villager.level(game), None);
            game.mobs = taken;
        });
    }
}
