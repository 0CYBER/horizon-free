//! `BundleContents.Mutable`: what a bundle takes in and gives back, by
//! weight. The menus (`menu`, `lan::menu`) run `BundleItem`'s click
//! overrides on it.
use crate::items::{ItemStack, Items};
use horizon_world::nbt::Tag;

/// A bundle weight: vanilla's `Fraction`, exact and kept reduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Weight {
    n: i128,
    d: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

impl Weight {
    const ZERO: Self = Self { n: 0, d: 1 };
    const ONE: Self = Self { n: 1, d: 1 };

    fn new(n: i128, d: i128) -> Self {
        let g = gcd(n, d);
        Self { n: n / g, d: d / g }
    }

    fn add(self, o: Self) -> Self {
        Self::new(self.n * o.d + o.n * self.d, self.d * o.d)
    }

    fn sub(self, o: Self) -> Self {
        self.add(Self { n: -o.n, d: o.d })
    }

    fn times(self, count: u32) -> Self {
        Self::new(self.n * i128::from(count), self.d)
    }

    /// `this.divideBy(item).intValue()` (truncated), at least 0.
    fn how_many(self, item: Self) -> u32 {
        let q = (self.n * item.d) / (self.d * item.n);
        u32::try_from(q.max(0)).unwrap_or(u32::MAX)
    }
}

/// The sounds of `BundleItem` (`Entity.playSound` at the player): insert
/// and remove one at volume 0.8 and pitch `0.8 + random * 0.4`, a failed
/// insert at 1, 1.
pub const INSERT: &str = "minecraft:item.bundle.insert";
pub const INSERT_FAIL: &str = "minecraft:item.bundle.insert_fail";
pub const REMOVE_ONE: &str = "minecraft:item.bundle.remove_one";

fn max_stack(items: &Items, stack: &ItemStack) -> u32 {
    items.get(&stack.item).map_or(64, |i| i.max_stack).max(1)
}

/// The stack's `bundle_contents`, when it has the component.
fn contents_of<'a>(items: &Items, stack: &'a ItemStack) -> Option<&'a [ItemStack]> {
    let inside = stack.data().map_or(&[][..], |d| &d.bundle_contents[..]);
    let bundle = items.get(&stack.item).is_some_and(|i| i.bundle);
    (bundle || !inside.is_empty()).then_some(inside)
}

/// `BundleContents.getWeight`: a nested bundle weighs its contents plus
/// 1/16, a beehive with bees 1, the rest 1 / max stack size.
fn item_weight(items: &Items, stack: &ItemStack) -> Weight {
    if let Some(inside) = contents_of(items, stack) {
        return contents_weight(items, inside).add(Weight::new(1, 16));
    }
    let bees = stack
        .data()
        .and_then(|d| d.other("minecraft:bees"))
        .is_some_and(|b| matches!(b, Tag::List(l) if !l.is_empty()));
    if bees {
        Weight::ONE
    } else {
        Weight::new(1, i128::from(max_stack(items, stack)))
    }
}

fn contents_weight(items: &Items, inside: &[ItemStack]) -> Weight {
    inside.iter().fold(Weight::ZERO, |w, s| {
        w.add(item_weight(items, s).times(s.count))
    })
}

/// `BundleContents.canItemBeInBundle`: shulker boxes don't fit.
pub fn can_be_in_bundle(stack: &ItemStack) -> bool {
    stack.count > 0 && !stack.item.ends_with("shulker_box")
}

/// `BundleContents.Mutable`.
pub struct Contents {
    items: Vec<ItemStack>,
    weight: Weight,
}

impl Contents {
    /// The bundle's contents; None when the stack is no bundle.
    pub fn of(items: &Items, stack: &ItemStack) -> Option<Self> {
        let inside = contents_of(items, stack)?.to_vec();
        let weight = contents_weight(items, &inside);
        Some(Self {
            items: inside,
            weight,
        })
    }

    /// `getMaxAmountToAdd` for `stack`'s kind of item.
    pub fn room_for(&self, items: &Items, stack: &ItemStack) -> u32 {
        Weight::ONE
            .sub(self.weight)
            .how_many(item_weight(items, stack))
    }

    /// `tryInsert`: what fits moves to the front, merged with its kind.
    pub fn try_insert(&mut self, items: &Items, stack: &mut ItemStack) -> u32 {
        if !can_be_in_bundle(stack) {
            return 0;
        }
        let weight = item_weight(items, stack);
        let amount = stack
            .count
            .min(Weight::ONE.sub(self.weight).how_many(weight));
        if amount == 0 {
            return 0;
        }
        self.weight = self.weight.add(weight.times(amount));
        // ItemStack.isStackable: a damaged stack is damageable.
        let stackable = max_stack(items, stack) > 1 && stack.damage == 0;
        let same = self
            .items
            .iter()
            .position(|s| stackable && s.same_item_same_components(stack));
        let added = match same {
            Some(i) => {
                let mut merged = self.items.remove(i);
                merged.count += amount;
                stack.count -= amount;
                merged
            }
            None => {
                let mut part = stack.clone();
                part.count = amount;
                stack.count -= amount;
                part
            }
        };
        self.items.insert(0, added);
        amount
    }

    /// `removeOne`: the first stack (Horizon keeps no selected item).
    pub fn remove_one(&mut self, items: &Items) -> Option<ItemStack> {
        if self.items.is_empty() {
            return None;
        }
        let stack = self.items.remove(0);
        self.weight = self
            .weight
            .sub(item_weight(items, &stack).times(stack.count));
        Some(stack)
    }

    /// `toImmutable`, written back into the bundle's component.
    pub fn store(self, bundle: &mut ItemStack) {
        bundle.data_mut().bundle_contents = self.items;
        bundle.tidy();
    }
}

#[cfg(test)]
mod tests {
    use super::Weight;

    #[test]
    fn weights_add_up_exactly() {
        let w = Weight::new(1, 64)
            .times(32)
            .add(Weight::new(1, 16).times(4));
        assert_eq!(w, Weight::new(3, 4));
        assert_eq!(Weight::ONE.sub(w).how_many(Weight::new(1, 64)), 16);
        assert_eq!(Weight::ONE.sub(Weight::ONE).how_many(Weight::ONE), 0);
        // An over-full bundle (made by a command) has no room.
        let over = Weight::ONE.sub(Weight::new(5, 4));
        assert_eq!(over.how_many(Weight::new(1, 64)), 0);
    }
}
