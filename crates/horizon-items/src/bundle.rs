//! `BundleContents.Mutable`: what a bundle takes in and gives back, by
//! weight. The menus (`menu`, `lan::menu`) run `BundleItem`'s click
//! overrides on it.
use crate::item_data::BundleSelected;
use crate::items::{ItemStack, Items};
use horizon_core::nbt::Tag;

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
/// insert at 1, 1; the drop of its contents is played at the player's block
/// like remove one.
pub const INSERT: &str = "minecraft:item.bundle.insert";
pub const INSERT_FAIL: &str = "minecraft:item.bundle.insert_fail";
pub const REMOVE_ONE: &str = "minecraft:item.bundle.remove_one";
pub const DROP_CONTENTS: &str = "minecraft:item.bundle.drop_contents";

pub(crate) fn max_stack(items: &Items, stack: &ItemStack) -> u32 {
    let info = items.get(&stack.item);
    if !stack.carries_component(info, "minecraft:max_stack_size") {
        return 1;
    }
    stack
        .data()
        .and_then(|data| data.other("minecraft:max_stack_size"))
        .and_then(crate::properties::component_int)
        .unwrap_or_else(|| info.map_or(1, |item| item.max_stack as i32))
        .max(1) as u32
}

/// `BundleItem.playInsertSound` and `playRemoveOneSound`: volume 0.8, pitch
/// `0.8 + level.getRandom().nextFloat() * 0.4`; `playInsertFailSound` 1, 1 and
/// no draw.
pub fn sound_volume_pitch(sound: &str, next_float: impl FnOnce() -> f32) -> (f32, f32) {
    if sound == INSERT_FAIL {
        (1.0, 1.0)
    } else {
        (0.8, 0.8 + next_float() * 0.4)
    }
}

/// Whether the stack's item is a bundle (`BundleItem`): its prototype carries
/// the component.
fn is_bundle_item(items: &Items, stack: &ItemStack) -> bool {
    items.get(&stack.item).is_some_and(|i| i.bundle)
}

/// `stack.get(BUNDLE_CONTENTS)`: the prototype's empty contents unless a
/// `!minecraft:bundle_contents` patch removed them, or the stack's own.
fn contents_of<'a>(items: &Items, stack: &'a ItemStack) -> Option<&'a [ItemStack]> {
    let inside = stack.data().map_or(&[][..], |d| &d.bundle_contents[..]);
    stack
        .carries_component(items.get(&stack.item), "minecraft:bundle_contents")
        .then_some(inside)
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

/// `BundleContents.computeContentWeight` returns an error when its fraction
/// arithmetic exceeds Java's integer bounds. The weight may exceed one.
pub(crate) fn valid_contents_weight(items: &Items, inside: &[ItemStack]) -> bool {
    fn checked(weight: Weight) -> Option<Weight> {
        (i32::try_from(weight.n).is_ok() && i32::try_from(weight.d).is_ok()).then_some(weight)
    }
    fn weigh(items: &Items, inside: &[ItemStack]) -> Option<Weight> {
        inside.iter().try_fold(Weight::ZERO, |total, stack| {
            let weight = if let Some(nested) = contents_of(items, stack) {
                checked(weigh(items, nested)?.add(Weight::new(1, 16)))?
            } else {
                item_weight(items, stack)
            };
            let weight = checked(weight.times(stack.count))?;
            checked(total.add(weight))
        })
    }
    weigh(items, inside).is_some()
}

/// `BundleItem.onDestroyed` (`ItemStack.onDestroyed` for a bundle): the stack
/// is emptied and every stack it held comes back as its own item copy, which
/// `ItemUtils.onContainerDestroyed` drops where the entity lay.
pub fn destroyed(stack: &mut ItemStack) -> Vec<ItemStack> {
    let Some(data) = stack.extra.as_deref_mut() else {
        return Vec::new();
    };
    if data.bundle_contents.is_empty() {
        return Vec::new();
    }
    // `set(BUNDLE_CONTENTS, BundleContents.EMPTY)`: no contents and no
    // selected index left.
    let spilled = std::mem::take(&mut data.bundle_contents);
    data.bundle_selected = BundleSelected(-1);
    stack.tidy();
    spilled
}

/// `BundleContents.Mutable`.
pub struct Contents {
    items: Vec<ItemStack>,
    weight: Weight,
    /// `selectedItem`: an index into `items`, -1 for none.
    selected: i32,
}

impl Contents {
    /// A component collection may belong to any item, including an item with
    /// no previous bundle_contents component. Reuse the same weight model.
    pub fn from_items(items: &Items, inside: Vec<ItemStack>) -> Self {
        let weight = contents_weight(items, &inside);
        Self {
            items: inside,
            weight,
            selected: -1,
        }
    }
    /// The bundle's contents; None when the stack is no bundle. `BundleItem`'s
    /// overrides (`overrideStackedOnOther`, `overrideOtherStackedOnMe`,
    /// `getTooltipImage`) need the item **and** the component: a bundle with
    /// it removed, or another item carrying it, is a plain item.
    pub fn of(items: &Items, stack: &ItemStack) -> Option<Self> {
        let mut contents = Self::from_stack(items, stack)?;
        // `BundleContents.Mutable` materializes each saved `ItemStackTemplate`.
        if crate::item_data::clean_stacks(&mut contents.items, &items.component_defaults) {
            contents.weight = contents_weight(items, &contents.items);
        }
        Some(contents)
    }

    fn from_stack(items: &Items, stack: &ItemStack) -> Option<Self> {
        if !is_bundle_item(items, stack) {
            return None;
        }
        let inside = contents_of(items, stack)?.to_vec();
        let weight = contents_weight(items, &inside);
        let selected = stack.data().map_or(-1, |d| d.bundle_selected.0);
        Some(Self {
            items: inside,
            weight,
            selected,
        })
    }

    /// Remaining capacity for `offered` in a stack's bundle component, without cloning contents.
    pub fn room_in_stack(items: &Items, bundle: &ItemStack, offered: &ItemStack) -> Option<u32> {
        let inside = contents_of(items, bundle)?;
        Some(
            Weight::ONE
                .sub(contents_weight(items, inside))
                .how_many(item_weight(items, offered)),
        )
    }

    /// `getMaxAmountToAdd` for `stack`'s kind of item.
    pub fn room_for(&self, items: &Items, stack: &ItemStack) -> u32 {
        Weight::ONE
            .sub(self.weight)
            .how_many(item_weight(items, stack))
    }

    /// `tryInsert`: what fits moves to the front, merged with its kind. `fits` is the stack's
    /// `canFitInsideContainerItems` answer, which `canItemBeInBundle` asks and the caller
    /// gives, since the parts of the stack's item may change it; an empty stack never goes in.
    pub fn try_insert(&mut self, items: &Items, stack: &mut ItemStack, fits: bool) -> u32 {
        if stack.count == 0 || !fits {
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

    /// `removeOne`: the selected stack, else the first; nothing is selected after.
    pub fn remove_one(&mut self, items: &Items) -> Option<ItemStack> {
        if self.items.is_empty() {
            return None;
        }
        let at =
            usize::try_from(self.selected).map_or(0, |i| if i < self.items.len() { i } else { 0 });
        let stack = self.items.remove(at);
        self.weight = self
            .weight
            .sub(item_weight(items, &stack).times(stack.count));
        self.selected = -1;
        Some(stack)
    }

    /// `getSelectedItemIndex`.
    pub fn selected(&self) -> i32 {
        self.selected
    }

    /// The stacks inside, in display order (the first is the next out).
    pub fn items(&self) -> &[ItemStack] {
        &self.items
    }

    /// `getNumberOfItemsToShow`: a full grid shows 11 and a `+N`, so the last
    /// row keeps its fourth cell for the count.
    pub fn shown_count(&self) -> usize {
        let count = self.items.len();
        let available: usize = if count > 12 { 11 } else { 12 };
        let on_last_row = count % 4;
        let empty = if on_last_row == 0 { 0 } else { 4 - on_last_row };
        count.min(available.saturating_sub(empty))
    }

    /// The items the tooltip shows, and how many sit beyond them.
    pub fn shown(&self) -> (&[ItemStack], u32) {
        let n = self.shown_count();
        let hidden = self.items[n..].iter().map(|s| s.count).sum();
        (&self.items[..n], hidden)
    }

    /// The `Fraction` weight as the float the progress bar reads.
    pub fn weight_fraction(&self) -> f32 {
        self.weight.n as f32 / self.weight.d as f32
    }

    /// `Fraction.ONE`: a full bundle draws the full bar and says so.
    pub fn is_full(&self) -> bool {
        self.weight.n >= self.weight.d
    }

    /// `BundleItem.isBarVisible`, `getBarWidth` and `getBarColor`: the fullness bar
    /// (width, RGB), blue until the bundle is full, then red.
    pub fn bar(&self) -> Option<(i32, u32)> {
        if self.weight.n <= 0 {
            return None;
        }
        let width = (1 + self.weight.n * 12 / self.weight.d).min(13) as i32;
        let color = if self.is_full() { 0xFF5454 } else { 0x7087FF };
        Some((width, color))
    }

    /// `toImmutable`, written back into the bundle's component.
    pub fn store(self, bundle: &mut ItemStack) {
        let data = bundle.data_mut();
        data.bundle_contents = self.items;
        data.bundle_selected = BundleSelected(self.selected);
        bundle.tidy();
    }
}

/// `BundleItem.toggleSelectedItem`: select `index`, or nothing when it is the
/// selected one already or outside the contents.
pub fn toggle_selected(bundle: &mut ItemStack, index: i32) {
    let Some(data) = bundle.extra.as_deref_mut() else {
        return;
    };
    let inside = (0..data.bundle_contents.len() as i32).contains(&index);
    let selected = if data.bundle_selected.0 != index && inside {
        index
    } else {
        -1
    };
    data.bundle_selected = BundleSelected(selected);
}

#[cfg(test)]
#[path = "bundle_component_tests.rs"]
mod component_tests;

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

// ------------------------------------------------------------- the tooltip

/// `ClientBundleTooltip`'s geometry, relative to the tooltip box's top-left:
/// the 96 pixel column of a 4 wide slot grid and the progress bar under it.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipLayout {
    /// The component's own width (96) and height, the text's excluded.
    pub width: f32,
    pub height: f32,
    /// Each slot's top-left and the stack in it, in drawing order.
    pub slots: Vec<([f32; 2], ItemStack)>,
    /// The position in `slots` of the selected item, which is drawn highlighted.
    pub selected: Option<usize>,
    /// The `+N` of the items past the grid: its top-left and the count.
    pub overflow: Option<([f32; 2], u32)>,
    /// The progress bar's top-left, its fill's width out of 94, and whether
    /// the weight fills it (the full sprite).
    pub bar: [f32; 2],
    pub fill: f32,
    pub full: bool,
    /// `item.minecraft.bundle.empty` / `.full`, centred over the bar.
    pub label: Option<&'static str>,
    /// An empty bundle's description, wrapped at `width` from here.
    pub empty_description: Option<[f32; 2]>,
}

/// The tooltip of a bundle's contents (`ClientBundleTooltip`), or None when
/// the stack is not a bundle. `x`/`y` are the box's top-left, `box_width` the
/// tooltip's width, which the component is centred in.
pub fn tooltip_layout(
    items: &Items,
    stack: &ItemStack,
    x: f32,
    y: f32,
    box_width: f32,
    description_lines: usize,
) -> Option<TooltipLayout> {
    let contents = Contents::from_stack(items, stack)?;
    let offset = ((box_width - 96.0) / 2.0).floor();
    if contents.items().is_empty() {
        // The description takes the top; the bar follows it. Its lines are
        // the caller's font's (`getEmptyBundleDescriptionTextHeight`).
        let description_height = 9.0 * description_lines as f32;
        return Some(TooltipLayout {
            width: 96.0,
            height: description_height + 13.0 + 8.0,
            slots: Vec::new(),
            selected: None,
            overflow: None,
            bar: [x + offset, y + description_height + 4.0],
            fill: 0.0,
            full: false,
            label: Some("item.minecraft.bundle.empty"),
            empty_description: Some([x + offset, y]),
        });
    }
    let (shown, hidden) = contents.shown();
    // `gridSizeY` counts `slotCount()` (at most twelve), not the slots the
    // overflow count leaves room for.
    let rows = contents.items().len().min(12).div_ceil(4).max(1);
    let grid_height = rows as f32 * 24.0;
    let x_start = x + offset + 96.0;
    let y_start = y + grid_height;
    let mut slots = Vec::with_capacity(shown.len());
    let mut selected = None;
    let mut overflow = None;
    // Rows fill bottom-up and columns right-to-left, as vanilla draws them.
    let mut slot = 0;
    for row in 1..=rows {
        for column in 1..=4 {
            let at = [x_start - column as f32 * 24.0, y_start - row as f32 * 24.0];
            if hidden > 0 && row == 1 && column == 1 {
                overflow = Some((at, hidden));
            } else if slot < shown.len() {
                // `extractSlot`: the cell of slot number n holds item `size - n`.
                let index = shown.len() - 1 - slot;
                if usize::try_from(contents.selected()) == Ok(index) {
                    selected = Some(slots.len());
                }
                slots.push((at, shown[index].clone()));
                slot += 1;
            }
        }
    }
    let weight = contents.weight_fraction();
    Some(TooltipLayout {
        width: 96.0,
        height: grid_height + 13.0 + 8.0,
        slots,
        selected,
        overflow,
        bar: [x + offset, y + grid_height + 4.0],
        // `Mth.mulAndTruncate(weight, 94)`, kept within the bar.
        fill: (weight * 94.0).trunc().clamp(0.0, 94.0),
        full: contents.is_full(),
        label: if contents.is_full() {
            Some("item.minecraft.bundle.full")
        } else {
            None
        },
        empty_description: None,
    })
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;

    /// The prepared resources, when they are on this machine: the tooltip
    /// asks the item registry whether a stack is a bundle.
    fn items() -> Option<Items> {
        crate::items::prepared_items()
    }

    fn bundle_with(counts: &[(&str, u32)]) -> ItemStack {
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = counts
            .iter()
            .map(|(item, n)| ItemStack::new(item, *n))
            .collect();
        bundle
    }

    #[test]
    fn a_grid_shows_twelve_stacks_and_counts_the_rest() {
        let Some(items) = items() else { return };
        let twelve: Vec<(&str, u32)> = (0..12).map(|_| ("minecraft:stone", 1)).collect();
        let layout = tooltip_layout(&items, &bundle_with(&twelve), 0.0, 0.0, 96.0, 2).unwrap();
        assert_eq!(layout.slots.len(), 12);
        assert_eq!(layout.overflow, None);
        assert_eq!(layout.height, 3.0 * 24.0 + 13.0 + 8.0);
        // Rows fill bottom-up and columns right-to-left: the twelfth item is in
        // the bottom-right cell, the first in the top-left.
        assert_eq!(layout.slots[0].0, [96.0 - 24.0, 3.0 * 24.0 - 24.0]);
        assert_eq!(&*layout.slots[0].1.item, "minecraft:stone");
        assert_eq!(layout.slots[11].0, [0.0, 0.0]);
        // A wider tooltip centres the 96 pixel column in it.
        let centred = tooltip_layout(&items, &bundle_with(&twelve), 0.0, 0.0, 120.0, 2).unwrap();
        assert_eq!(centred.slots[0].0[0], 12.0 + 96.0 - 24.0);
        // Thirteen stacks: `getNumberOfItemsToShow` leaves the last row room
        // for the count, so 8 slots show and `+6` (four stones and two dirt)
        // takes the first cell.
        let mut counts = twelve;
        counts.push(("minecraft:dirt", 2));
        let layout = tooltip_layout(&items, &bundle_with(&counts), 0.0, 0.0, 96.0, 2).unwrap();
        assert_eq!(layout.slots.len(), 8);
        assert_eq!(layout.overflow, Some(([96.0 - 24.0, 48.0], 6)));
    }

    #[test]
    fn the_bar_fills_with_the_weight_and_an_empty_bundle_explains_itself() {
        let Some(items) = items() else { return };
        let layout = tooltip_layout(&items, &bundle_with(&[]), 4.0, 8.0, 96.0, 2).unwrap();
        assert!(layout.slots.is_empty());
        assert_eq!(layout.fill, 0.0);
        assert_eq!(layout.label, Some("item.minecraft.bundle.empty"));
        assert_eq!(layout.empty_description, Some([4.0, 8.0]));
        assert_eq!(layout.bar, [4.0, 8.0 + 18.0 + 4.0]);
        // A bundle holding one 64 stack is full: the whole bar, the full label.
        let layout = tooltip_layout(
            &items,
            &bundle_with(&[("minecraft:stone", 64)]),
            0.0,
            0.0,
            96.0,
            2,
        )
        .unwrap();
        assert!(layout.full);
        assert_eq!(layout.fill, 94.0);
        assert_eq!(layout.label, Some("item.minecraft.bundle.full"));
        // Something that is not a bundle has no component.
        let dirt = ItemStack::new("minecraft:dirt", 1);
        assert!(tooltip_layout(&items, &dirt, 0.0, 0.0, 96.0, 2).is_none());
    }
    /// `getEmptyBundleBackgroundHeight`: the description's wrapped height (9 a
    /// line) plus the bar's 13 and the tooltip's 8, not a fixed two lines.
    #[test]
    fn an_empty_bundles_height_follows_its_description_lines() {
        let Some(items) = items() else { return };
        let layout = tooltip_layout(&items, &bundle_with(&[]), 4.0, 8.0, 96.0, 3).unwrap();
        assert_eq!(layout.height, 3.0 * 9.0 + 13.0 + 8.0);
        assert_eq!(layout.bar, [4.0, 8.0 + 3.0 * 9.0 + 4.0]);
        assert_eq!(layout.empty_description, Some([4.0, 8.0]));
    }

    #[test]
    fn bundle_removes_selected_item_or_defaults_to_last_inserted() {
        let Some(items) = items() else { return };
        let bundle = bundle_with(&[
            ("minecraft:dirt", 5),
            ("minecraft:stone", 10),
            ("minecraft:apple", 2),
        ]);
        // Initially, selected is -1. remove_one takes index 0 (dirt, first stack).
        let mut contents = Contents::of(&items, &bundle).unwrap();
        assert_eq!(contents.selected(), -1);
        let removed = contents.remove_one(&items).unwrap();
        assert_eq!(&*removed.item, "minecraft:dirt");
        assert_eq!(removed.count, 5);

        // When an item is explicitly selected (e.g. index 2: apple):
        let mut bundle = bundle_with(&[
            ("minecraft:dirt", 5),
            ("minecraft:stone", 10),
            ("minecraft:apple", 2),
        ]);
        toggle_selected(&mut bundle, 2);
        let mut contents = Contents::of(&items, &bundle).unwrap();
        assert_eq!(contents.selected(), 2);
        let removed = contents.remove_one(&items).unwrap();
        assert_eq!(&*removed.item, "minecraft:apple");
        assert_eq!(removed.count, 2);
        assert_eq!(contents.selected(), -1);
    }
}

#[cfg(test)]
mod stack_size_tests {
    use super::*;

    /// Stone with its own `max_stack_size`, or with the component removed
    /// (`!minecraft:max_stack_size`).
    fn stone(size: Option<i32>, count: u32) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:stone", count);
        let data = stack.data_mut();
        match size {
            Some(n) => data.set_other("minecraft:max_stack_size", Some(Tag::Int(n))),
            None => data.set_other(
                "!minecraft:max_stack_size",
                Some(Tag::Compound(Default::default())),
            ),
        };
        stack.tidy();
        stack
    }

    /// `BundleContents.getWeight` weighs a stack by
    /// `1 / ItemStack.getMaxStackSize()`: four stones with a `max_stack_size`
    /// of 4 fill a bundle, and one whose component is removed is vanilla's
    /// `getOrDefault` default of 1, which fills it on its own.
    #[test]
    fn a_bundle_weighs_a_stack_by_its_own_max_stack_size() {
        let Some(items) = crate::items::prepared_items() else {
            return;
        };
        let bundle = ItemStack::new("minecraft:bundle", 1);
        let mut four = stone(Some(4), 8);
        let mut contents = Contents::of(&items, &bundle).expect("a bundle");
        assert_eq!(contents.room_for(&items, &four), 4);
        assert_eq!(contents.try_insert(&items, &mut four, true), 4);
        assert_eq!(four.count, 4);
        assert!(contents.is_full());
        let mut one = stone(None, 2);
        let mut contents = Contents::of(&items, &bundle).expect("a bundle");
        assert_eq!(contents.room_for(&items, &one), 1);
        assert_eq!(contents.try_insert(&items, &mut one, true), 1);
        assert_eq!(one.count, 1);
        assert!(contents.is_full());
    }
}
