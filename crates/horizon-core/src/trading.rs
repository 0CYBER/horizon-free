//! Pure trading arithmetic shared by engine offers and API snapshots.

/// `MerchantOffer.getModifiedCostCount`: demand and special prices, limited by the item.
/// `maximum` is the effective positive stack limit supplied by the owning item registry.
pub fn modified_cost_count(
    base: u32,
    demand: i32,
    multiplier: f32,
    special_price: i32,
    maximum: u32,
) -> u32 {
    let base = base as i32;
    let demand_diff = ((base.wrapping_mul(demand) as f32 * multiplier).floor() as i32).max(0);
    (base + demand_diff + special_price).clamp(1, maximum as i32) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prices_respect_demand_discounts_and_stack_limits() {
        assert_eq!(modified_cost_count(10, 3, 0.2, 0, 64), 16);
        assert_eq!(modified_cost_count(10, -3, 0.2, 0, 64), 10);
        assert_eq!(modified_cost_count(10, 3, 0.2, -30, 64), 1);
        assert_eq!(modified_cost_count(10, 3, 0.2, 50, 16), 16);
    }
}
