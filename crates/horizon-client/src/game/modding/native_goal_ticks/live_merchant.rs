//! Merchant handles for villagers temporarily lent to an AI tick.
use super::GoalCx;
use crate::mobs::{Mob, Species, render::VILLAGER_TYPES};
use mod_api::{MerchantOffer, MerchantRef, PlayerRef};

impl GoalCx<'_, '_> {
    fn lent_merchant(&self, merchant: MerchantRef) -> Option<&Mob> {
        let mob = self.entity_mob(merchant.entity())?;
        (self.w.goal_host?.merchant_is_live(mob)
            && matches!(mob.species(), Species::Villager | Species::WanderingTrader))
        .then_some(mob)
    }

    pub(super) fn r_merchant_offers(&self, merchant: MerchantRef) -> Option<Vec<MerchantOffer>> {
        self.w
            .goal_host?
            .merchant_offers_lent(self.lent_merchant(merchant)?)
    }

    pub(super) fn r_merchant_trading_player(&self, merchant: MerchantRef) -> Option<PlayerRef> {
        self.w
            .goal_host?
            .merchant_trading_player_lent(self.lent_merchant(merchant)?)
    }

    pub(super) fn r_merchant_profession(&self, merchant: MerchantRef) -> Option<String> {
        let mob = self.lent_merchant(merchant)?;
        (mob.species() == Species::Villager).then(|| mob.extra.profession.id())
    }

    pub(super) fn r_merchant_level(&self, merchant: MerchantRef) -> Option<i32> {
        let mob = self.lent_merchant(merchant)?;
        if mob.species() != Species::Villager {
            return None;
        }
        Some(self.w.goal_host?.merchant_trade_state_lent(mob).1.max(1))
    }

    pub(super) fn r_merchant_villager_type(&self, merchant: MerchantRef) -> Option<String> {
        let mob = self.lent_merchant(merchant)?;
        (mob.species() == Species::Villager).then(|| {
            let name = VILLAGER_TYPES[mob.extra.variant as usize % VILLAGER_TYPES.len()];
            format!("minecraft:{name}")
        })
    }

    pub(super) fn r_merchant_xp(&self, merchant: MerchantRef) -> Option<i32> {
        let mob = self.lent_merchant(merchant)?;
        if mob.species() != Species::Villager {
            return None;
        }
        Some(self.w.goal_host?.merchant_trade_state_lent(mob).0)
    }
}
