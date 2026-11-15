//! Owned trading snapshots; the game keeps its own trading state and validates writes.
use crate::{Host, ItemStack};

/// An item payment, with an optional exact component predicate.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemCost {
    /// Item and minimum count required for this payment.
    pub stack: ItemStack,
}
impl ItemCost {
    /// Construct a payment from a nonempty item value.
    pub fn new(stack: ItemStack) -> Self {
        Self { stack }
    }
    /// Whether `stack` matches the payment's identity and explicit component predicate.
    pub fn test(&self, stack: &ItemStack) -> bool {
        stack.same_item(&self.stack)
            && (self.stack.components().is_none() || stack.same_components(&self.stack))
    }
}

/// A merchant offer copied at an API boundary, not a reference into the game's trade table.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct MerchantOffer {
    /// Undiscounted first payment.
    pub base_cost_a: ItemCost,
    /// Optional second payment.
    pub cost_b: Option<ItemCost>,
    /// Items received in exchange.
    pub result: ItemStack,
    /// Trades already performed since restocking.
    pub uses: i32,
    /// Trades allowed before the offer is out of stock.
    pub max_uses: i32,
    /// Whether completing this trade awards experience.
    pub reward_exp: bool,
    /// Player-specific price adjustment.
    pub special_price_diff: i32,
    /// Demand accumulated during restocking.
    pub demand: i32,
    /// Multiplier for demand and reputation price adjustments.
    pub price_multiplier: f32,
    /// Villager experience awarded by this trade.
    pub xp: i32,
}
impl MerchantOffer {
    /// Construct an unused offer with explicit payments, output, limits and experience.
    /// Discounts and demand begin at zero; completing it rewards experience.
    pub fn new(
        base_cost_a: ItemCost,
        cost_b: Option<ItemCost>,
        result: ItemStack,
        max_uses: i32,
        xp: i32,
        price_multiplier: f32,
    ) -> Self {
        Self {
            base_cost_a,
            cost_b,
            result,
            uses: 0,
            max_uses,
            reward_exp: true,
            special_price_diff: 0,
            demand: 0,
            price_multiplier,
            xp,
        }
    }
    /// The adjusted first payment count, or none if its item metadata is unavailable.
    pub fn cost_a_count(&self, host: &dyn Host) -> Option<u32> {
        let maximum = host.max_stack_size(self.base_cost_a.stack.item())?;
        Some(horizon_core::trading::modified_cost_count(
            self.base_cost_a.stack.count(),
            self.demand,
            self.price_multiplier,
            self.special_price_diff,
            maximum,
        ))
    }
    /// First payment with demand and discounts applied.
    pub fn cost_a(&self, host: &dyn Host) -> Option<ItemStack> {
        let mut result = self.base_cost_a.stack.clone();
        result.set_count(self.cost_a_count(host)?).ok()?;
        Some(result)
    }
    /// The optional second payment.
    pub fn cost_b(&self) -> Option<ItemStack> {
        self.cost_b.as_ref().map(|cost| cost.stack.clone())
    }
    /// Whether this offer has exhausted its uses.
    pub fn out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }
    /// Apply `MerchantOffer.updateDemand` to this snapshot.
    pub fn update_demand(&mut self) {
        self.demand = self.demand + self.uses - (self.max_uses - self.uses);
    }
    /// Whether both payment slots satisfy this snapshot's current price.
    pub fn satisfied_by(
        &self,
        host: &dyn Host,
        a: Option<&ItemStack>,
        b: Option<&ItemStack>,
    ) -> bool {
        let (Some(a), Some(count)) = (a, self.cost_a_count(host)) else {
            return false;
        };
        if !self.base_cost_a.test(a) || a.count() < count {
            return false;
        }
        match (&self.cost_b, b) {
            (None, None) => true,
            (Some(cost), Some(stack)) => cost.test(stack) && stack.count() >= cost.stack.count(),
            _ => false,
        }
    }
    /// Consume local payment values if satisfied. This does not change the game's trade state.
    pub fn take(
        &self,
        host: &dyn Host,
        a: &mut Option<ItemStack>,
        b: &mut Option<ItemStack>,
    ) -> bool {
        if !self.satisfied_by(host, a.as_ref(), b.as_ref()) {
            return false;
        }
        let Some(count) = self.cost_a_count(host) else {
            return false;
        };
        *a = a.take().and_then(|stack| stack.shrink(count));
        if let Some(cost) = &self.cost_b {
            *b = b.take().and_then(|stack| stack.shrink(cost.stack.count()));
        }
        true
    }
}
