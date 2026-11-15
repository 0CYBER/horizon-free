//! Engine trading values and payment operations.
use crate::items::{ItemStack, Items};
/// An `ItemCost`: item, count and the required exact component subset,
/// kept as the stack's component patch.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ItemCost {
    pub stack: ItemStack,
}
impl ItemCost {
    pub fn test(&self, s: &ItemStack) -> bool {
        if s.item != self.stack.item {
            return false;
        }
        // DataComponentExactPredicate checks required values, allowing unrelated components.
        let Some(horizon_core::nbt::Tag::Compound(required)) =
            crate::item_data::components_tag(self.stack.damage, self.stack.data())
        else {
            return true;
        };
        let actual = crate::item_data::components_tag(s.damage, s.data());
        required.iter().all(|(key, value)| {
            matches!(&actual, Some(horizon_core::nbt::Tag::Compound(c)) if c.get(key) == Some(value))
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MerchantOffer {
    pub base_cost_a: ItemCost,
    pub cost_b: Option<ItemCost>,
    pub result: ItemStack,
    pub uses: i32,
    pub max_uses: i32,
    pub reward_exp: bool,
    pub special_price_diff: i32,
    pub demand: i32,
    pub price_multiplier: f32,
    pub xp: i32,
}
impl MerchantOffer {
    fn max_stack(items: &Items, s: &ItemStack) -> u32 {
        items.get(&s.item).map_or(64, |i| i.max_stack)
    }
    /// `getModifiedCostCount` for cost A.
    pub fn cost_a_count(&self, items: &Items) -> u32 {
        horizon_core::trading::modified_cost_count(
            self.base_cost_a.stack.count,
            self.demand,
            self.price_multiplier,
            self.special_price_diff,
            Self::max_stack(items, &self.base_cost_a.stack),
        )
    }
    pub fn cost_a(&self, items: &Items) -> ItemStack {
        ItemStack {
            count: self.cost_a_count(items),
            ..self.base_cost_a.stack.clone()
        }
    }
    pub fn cost_b(&self) -> Option<ItemStack> {
        self.cost_b.as_ref().map(|c| c.stack.clone())
    }
    pub fn out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }
    /// `MerchantOffer.updateDemand` (on restock).
    pub fn update_demand(&mut self) {
        self.demand = self.demand + self.uses - (self.max_uses - self.uses);
    }
    /// `satisfiedBy(buyA, buyB)`.
    pub fn satisfied_by(
        &self,
        items: &Items,
        a: Option<&ItemStack>,
        b: Option<&ItemStack>,
    ) -> bool {
        let Some(a) = a else { return false };
        if !self.base_cost_a.test(a) || a.count < self.cost_a_count(items) {
            return false;
        }
        match (&self.cost_b, b) {
            (None, b) => b.is_none(),
            (Some(c), Some(b)) => c.test(b) && b.count >= c.stack.count,
            (Some(_), None) => false,
        }
    }
    /// `MerchantOffer.take`: consume the payment; false when not satisfied.
    pub fn take(
        &self,
        items: &Items,
        a: &mut Option<ItemStack>,
        b: &mut Option<ItemStack>,
    ) -> bool {
        if !self.satisfied_by(items, a.as_ref(), b.as_ref()) {
            return false;
        }
        let n = self.cost_a_count(items);
        if let Some(s) = a.as_mut() {
            s.count -= n;
            if s.count == 0 {
                *a = None;
            }
        }
        if let (Some(c), Some(s)) = (&self.cost_b, b.as_mut()) {
            s.count -= c.stack.count;
            if s.count == 0 {
                *b = None;
            }
        }
        true
    }
}
