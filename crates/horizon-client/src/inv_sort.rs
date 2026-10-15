//! Horizon's sorting extra (#332, `docs/extras.md`): the left clicks that
//! sort a run of slots, starting and ending with an empty hand, as a player
//! could click them, so a vanilla server accepts them. Partial stacks of
//! one item merge first; then the stacks go in rank order (creative tab
//! order, then name), full stacks before the partial one, empty slots last.
use crate::items::ItemStack;

/// A stack's place in the order: its index in the creative tabs (items
/// outside them after), then its name.
pub type Rank = (usize, String);

/// A hand and slots under `AbstractContainerMenu.doClick`'s left `PICKUP`.
struct Sim<'a> {
    slots: Vec<Option<ItemStack>>,
    hand: Option<ItemStack>,
    max: &'a dyn Fn(&ItemStack) -> u32,
    clicks: Vec<usize>,
}
impl Sim<'_> {
    fn click(&mut self, i: usize) {
        self.clicks.push(i);
        let slot = self.slots[i].take();
        match (self.hand.take(), slot) {
            // Take the stack, or put the hand's down.
            (None, s) => self.hand = s,
            (Some(h), None) => self.slots[i] = Some(h),
            // The same item: as much as fits joins the slot.
            (Some(mut h), Some(mut s)) if s.same_item_same_components(&h) => {
                let n = (self.max)(&s).saturating_sub(s.count).min(h.count);
                s.count += n;
                h.count -= n;
                self.slots[i] = Some(s);
                self.hand = (h.count > 0).then_some(h);
            }
            // Another item: they swap.
            (Some(h), Some(s)) => {
                self.slots[i] = Some(h);
                self.hand = Some(s);
            }
        }
    }
    fn partial(&self, i: usize) -> Option<&ItemStack> {
        self.slots[i].as_ref().filter(|s| s.count < (self.max)(s))
    }
}

/// The indices of `slots` to left-click, in order, to sort them. Empty
/// when they are sorted already, or when the clicks couldn't reach the
/// order (stacks over their maximum).
pub fn plan(
    slots: &[Option<ItemStack>],
    rank: &dyn Fn(&ItemStack) -> Rank,
    max: &dyn Fn(&ItemStack) -> u32,
) -> Vec<usize> {
    let n = slots.len();
    let mut sim = Sim {
        slots: slots.to_vec(),
        hand: None,
        max,
        clicks: Vec::new(),
    };
    // Merge: each later partial stack of the item tops up the first one;
    // what doesn't fit goes back.
    for i in 0..n {
        for j in i + 1..n {
            let Some(s) = sim.partial(i) else {
                break;
            };
            if !sim
                .partial(j)
                .is_some_and(|t| t.same_item_same_components(s))
            {
                continue;
            }
            sim.click(j);
            sim.click(i);
            if sim.hand.is_some() {
                sim.click(j);
            }
        }
    }
    let stacks: Vec<ItemStack> = sim.slots.iter().flatten().cloned().collect();
    let ranks: Vec<Rank> = stacks.iter().map(rank).collect();
    let mut order: Vec<usize> = (0..stacks.len()).collect();
    order.sort_by(|&a, &b| {
        ranks[a]
            .cmp(&ranks[b])
            .then(stacks[b].count.cmp(&stacks[a].count))
    });
    let target: Vec<Option<ItemStack>> = order
        .iter()
        .map(|&i| Some(stacks[i].clone()))
        .chain(std::iter::repeat(None))
        .take(n)
        .collect();
    // Order: each slot takes its stack from further on; what it held goes
    // where that stack was. A full stack clicked on the item's partial one
    // fills it and takes the rest, which comes to the same.
    for i in 0..n {
        if sim.slots[i] == target[i] {
            continue;
        }
        let Some(j) = (i + 1..n).find(|&j| sim.slots[j] == target[i]) else {
            return Vec::new();
        };
        sim.click(j);
        sim.click(i);
        if sim.hand.is_some() {
            sim.click(j);
        }
    }
    if sim.hand.is_some() || sim.slots != target {
        return Vec::new();
    }
    sim.clicks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{Input, Kind};

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        })
    }
    fn rank(order: &'static [&'static str]) -> impl Fn(&ItemStack) -> Rank {
        move |s| {
            let id = s.item.trim_start_matches("minecraft:");
            let i = order.iter().position(|o| *o == id).unwrap_or(usize::MAX);
            (i, id.to_owned())
        }
    }
    fn max(s: &ItemStack) -> u32 {
        match &*s.item {
            "minecraft:ender_pearl" => 16,
            "minecraft:diamond_sword" => 1,
            _ => 64,
        }
    }
    const ORDER: &[&str] = &["stone", "dirt", "diamond_sword", "ender_pearl"];

    #[test]
    fn merges_partial_stacks_then_orders_full_stacks_first() {
        let slots = vec![
            stack("ender_pearl", 3),
            None,
            stack("dirt", 40),
            stack("stone", 64),
            stack("dirt", 40),
            stack("ender_pearl", 15),
            stack("diamond_sword", 1),
            stack("zombie_head", 1),
        ];
        let clicks = plan(&slots, &rank(ORDER), &max);
        let mut sim = Sim {
            slots: slots.clone(),
            hand: None,
            max: &max,
            clicks: Vec::new(),
        };
        for i in clicks {
            sim.click(i);
        }
        assert_eq!(sim.hand, None);
        assert_eq!(
            sim.slots,
            vec![
                stack("stone", 64),
                stack("dirt", 64),
                stack("dirt", 16),
                stack("diamond_sword", 1),
                stack("ender_pearl", 16),
                stack("ender_pearl", 2),
                stack("zombie_head", 1),
                None,
            ]
        );
        // Sorted slots need no clicks.
        assert!(plan(&sim.slots, &rank(ORDER), &max).is_empty());
    }

    #[test]
    fn the_clicks_sort_a_chest_in_the_menu_engine() {
        // The planned clicks replayed as left PICKUPs on a ChestMenu: the
        // engine (checked against the server jar) ends in the planned order.
        crate::menu::tests::with_menu(Kind::Chest(3), |env, menu| {
            let max = |s: &ItemStack| env.items.get(&s.item).map_or(64, |i| i.max_stack);
            let names = ["stone", "dirt", "ender_pearl", "oak_log", "snowball"];
            let mut seed = 7u32;
            for round in 0..40 {
                let slots: Vec<Option<ItemStack>> = (0..27)
                    .map(|_| {
                        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                        let pick = (seed >> 16) as usize % 8;
                        let item = names.get(pick)?;
                        let top = if *item == "ender_pearl" || *item == "snowball" {
                            16
                        } else {
                            64
                        };
                        stack(item, 1 + (seed >> 8) % top)
                    })
                    .collect();
                for (i, s) in slots.iter().enumerate() {
                    menu.set_slot(env, i, s.clone());
                }
                let clicks = plan(&slots, &rank(&["oak_log", "stone", "dirt"]), &max);
                for i in &clicks {
                    menu.click(env, *i as i32, 0, Input::Pickup).unwrap();
                }
                assert_eq!(menu.carried, None, "round {round}");
                let after: Vec<Option<ItemStack>> =
                    (0..27).map(|i| menu.slot(i).cloned()).collect();
                let total = |v: &[Option<ItemStack>], item: &str| -> u32 {
                    v.iter()
                        .flatten()
                        .filter(|s| s.item.ends_with(item))
                        .map(|s| s.count)
                        .sum()
                };
                for item in names {
                    assert_eq!(total(&slots, item), total(&after, item), "round {round}");
                }
                assert!(plan(&after, &rank(&["oak_log", "stone", "dirt"]), &max).is_empty());
                let items: Vec<&str> = after.iter().flatten().map(|s| &*s.item).collect();
                let mut sorted = items.clone();
                sorted.sort_by_key(|id| {
                    rank(&["oak_log", "stone", "dirt"])(&stack(&id[10..], 1).unwrap())
                });
                assert_eq!(items, sorted, "round {round}");
            }
        });
    }
}
