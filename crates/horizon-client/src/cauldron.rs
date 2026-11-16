//! `CauldronInteractions`: what an item does to an empty, water, lava or
//! powder snow cauldron. `tools/test_cauldron_parity.py` compares every item
//! with vanilla's `useItemOn`.
use crate::items::{ItemStack, Items};

/// `ItemTags.CAULDRON_CAN_REMOVE_DYE` membership.
pub fn removes_dye(items: &Items, item: &str) -> bool {
    items
        .item_tags
        .get("minecraft:cauldron_can_remove_dye")
        .is_some_and(|members| members.contains(item))
}
use horizon_world::chunk::BlockState;

/// The player side of an interaction.
pub trait Player {
    fn creative(&self) -> bool;
    /// The stack in the interacting hand.
    fn hand(&mut self) -> &mut Option<ItemStack>;
    /// `Inventory.add`; the leftover stays in `stack`.
    fn add(&mut self, stack: &mut ItemStack) -> bool;
    /// `Inventory.contains`.
    fn contains(&self, stack: &ItemStack) -> bool;
    fn drop_stack(&mut self, stack: ItemStack);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// `InteractionResult.SUCCESS`
    Success,
    /// `InteractionResult.CONSUME`
    Consume,
    /// `TRY_WITH_EMPTY_HAND`
    Pass,
}

#[derive(Debug)]
pub struct Interaction {
    pub outcome: Outcome,
    /// The cauldron's new state.
    pub block: Option<BlockState>,
    pub sound: Option<&'static str>,
    /// The custom statistic the player earns (`use_cauldron`, ...).
    pub stat: Option<&'static str>,
    /// `player.awardStat(Stats.ITEM_USED.get(itemInHand.getItem()))` after
    /// `stat`: the bottle and bucket interactions count the item as used.
    pub item_used: bool,
    /// `lowerFillLevel` posts BLOCK_CHANGE with the new state.
    pub lower_fill_level: bool,
    /// The `Level.gameEvent` the interaction ends with, after its sound:
    /// `FLUID_PICKUP` from `fillBucket` and the glass bottle, `FLUID_PLACE`
    /// from `emptyBucket` and a water potion. Washing posts none.
    pub fluid_event: Option<&'static str>,
}

fn state(name: &str, level: Option<u8>) -> BlockState {
    BlockState {
        name: name.to_owned(),
        properties: level
            .map(|l| ("level".to_owned(), l.to_string()))
            .into_iter()
            .collect(),
    }
}

fn pass() -> Interaction {
    Interaction {
        outcome: Outcome::Pass,
        block: None,
        sound: None,
        stat: None,
        item_used: false,
        lower_fill_level: false,
        fluid_event: None,
    }
}

/// `InteractionResult.CONSUME`: a lava or powder snow bucket under water.
fn consume() -> Interaction {
    Interaction {
        outcome: Outcome::Consume,
        ..pass()
    }
}

/// `ItemUtils.createFilledResult`: `new` for one item of the stack in `player`'s hand. In
/// creative (with `limit_creative`) the hand is kept and `new` added unless the inventory has
/// it; otherwise one item of the hand is used up, and `new` takes its place when it was the
/// last, else goes into the inventory, else is dropped.
pub fn create_filled_result(player: &mut dyn Player, new: ItemStack, limit_creative: bool) {
    let creative = player.creative();
    if limit_creative && creative {
        if !player.contains(&new) {
            let mut new = new;
            player.add(&mut new);
        }
        return;
    }
    let hand = player.hand();
    if !creative && let Some(held) = hand.as_mut() {
        held.count -= 1;
        if held.count == 0 {
            *hand = None;
        }
    }
    if hand.is_none() {
        *hand = Some(new);
        return;
    }
    let mut new = new;
    if !player.add(&mut new) {
        player.drop_stack(new);
    }
}

/// `LayeredCauldronBlock.handleEntityOnFireInside`: a burning entity in a
/// water cauldron lowers it a level; a powder snow cauldron becomes a water
/// cauldron of its level first, then lowers.
pub fn extinguished(cauldron: &BlockState) -> BlockState {
    if cauldron.name == "minecraft:powder_snow_cauldron" {
        let mut water = cauldron.clone();
        water.name = "minecraft:water_cauldron".into();
        lowered(&water)
    } else {
        lowered(cauldron)
    }
}

/// `LayeredCauldronBlock.lowerFillLevel`.
fn lowered(cauldron: &BlockState) -> BlockState {
    let level: u8 = cauldron
        .properties
        .get("level")
        .and_then(|l| l.parse().ok())
        .unwrap_or(1);
    if level <= 1 {
        state("minecraft:cauldron", None)
    } else {
        let mut next = cauldron.clone();
        next.properties
            .insert("level".into(), (level - 1).to_string());
        next
    }
}

fn changed(block: BlockState, sound: Option<&'static str>, stat: &'static str) -> Interaction {
    Interaction {
        outcome: Outcome::Success,
        block: Some(block),
        sound,
        stat: Some(stat),
        item_used: false,
        lower_fill_level: false,
        fluid_event: None,
    }
}

fn changed_lowered(
    cauldron: &BlockState,
    sound: Option<&'static str>,
    stat: &'static str,
) -> Interaction {
    let mut result = changed(lowered(cauldron), sound, stat);
    result.lower_fill_level = true;
    result
}

impl Interaction {
    /// The same answer, ending with the interaction's `Level.gameEvent`.
    fn with_event(mut self, event: &'static str) -> Self {
        self.fluid_event = Some(event);
        self
    }

    /// The same answer, also counting the held item as used.
    fn using_item(mut self) -> Self {
        self.item_used = true;
        self
    }
}

const COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

#[derive(Clone, Copy)]
pub enum Action {
    EmptyLava,
    EmptyWater,
    EmptySnow,
    FillWater,
    FillLava,
    FillSnow,
    EmptyPotion,
    FillBottle,
    TopUpPotion,
    CleanBanner,
    CleanShulker,
}

pub mod interactions;

/// The cauldron interaction of the held item (`useItemOn`); the hand stack is
/// changed through `player`. `removes_dye` is whether the held item is in the
/// `cauldron_can_remove_dye` tag.
pub fn interact(
    cauldron: &BlockState,
    under_water: bool,
    removes_dye: bool,
    player: &mut dyn Player,
) -> Interaction {
    let entry = player.hand().as_ref().and_then(|held| {
        interactions::Dispatcher::vanilla()
            .select(&cauldron.name, &held.item, removes_dye, |_| false)
    });
    interact_selected(cauldron, under_water, entry.as_ref(), player)
}

/// Apply an already selected vanilla entry; mod callbacks are handled by the action owner.
pub fn interact_selected<H>(
    cauldron: &BlockState,
    under_water: bool,
    entry: Option<&interactions::Entry<H>>,
    player: &mut dyn Player,
) -> Interaction {
    let Some(held) = player.hand().clone() else {
        return pass();
    };
    let level: u8 = cauldron
        .properties
        .get("level")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let water_potion = held.data().and_then(|d| d.potion.as_deref()) == Some("minecraft:water");
    // Dispatcher.get: tags first, then the item map.
    if matches!(entry, Some(interactions::Entry::Dye)) {
        if held.data().and_then(|d| d.dyed_color).is_none() {
            return pass();
        }
        if let Some(stack) = player.hand().as_mut() {
            stack.data_mut().dyed_color = None;
            stack.tidy();
        }
        return changed_lowered(cauldron, None, "clean_armor");
    }
    let fill_bucket = |player: &mut dyn Player, new: &str, can_fill: bool, sound| {
        if !can_fill {
            return pass();
        }
        create_filled_result(player, ItemStack::new(new, 1), true);
        changed(
            state("minecraft:cauldron", None),
            Some(sound),
            "use_cauldron",
        )
        .using_item()
        .with_event("minecraft:fluid_pickup")
    };
    let empty_bucket = |player: &mut dyn Player, block: BlockState, sound| {
        create_filled_result(player, ItemStack::new("minecraft:bucket", 1), true);
        changed(block, Some(sound), "fill_cauldron")
            .using_item()
            .with_event("minecraft:fluid_place")
    };
    let Some(interactions::Entry::Vanilla(action)) = entry else {
        return pass();
    };
    match action {
        Action::EmptyLava => {
            if under_water {
                consume()
            } else {
                empty_bucket(
                    player,
                    state("minecraft:lava_cauldron", None),
                    "minecraft:item.bucket.empty_lava",
                )
            }
        }
        Action::EmptyWater => empty_bucket(
            player,
            state("minecraft:water_cauldron", Some(3)),
            "minecraft:item.bucket.empty",
        ),
        Action::EmptySnow => {
            if under_water {
                consume()
            } else {
                empty_bucket(
                    player,
                    state("minecraft:powder_snow_cauldron", Some(3)),
                    "minecraft:item.bucket.empty_powder_snow",
                )
            }
        }
        Action::EmptyPotion => {
            if !water_potion {
                return pass();
            }
            create_filled_result(player, ItemStack::new("minecraft:glass_bottle", 1), true);
            changed(
                state("minecraft:water_cauldron", Some(1)),
                Some("minecraft:item.bottle.empty"),
                "use_cauldron",
            )
            .using_item()
            .with_event("minecraft:fluid_place")
        }
        Action::FillWater => fill_bucket(
            player,
            "minecraft:water_bucket",
            level == 3,
            "minecraft:item.bucket.fill",
        ),
        Action::FillBottle => {
            create_filled_result(
                player,
                ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"),
                true,
            );
            changed_lowered(cauldron, Some("minecraft:item.bottle.fill"), "use_cauldron")
                .using_item()
                .with_event("minecraft:fluid_pickup")
        }
        Action::TopUpPotion => {
            if level == 3 || !water_potion {
                return pass();
            }
            create_filled_result(player, ItemStack::new("minecraft:glass_bottle", 1), true);
            let mut next = cauldron.clone();
            // BlockState.cycle on a 1..=3 property
            next.properties
                .insert("level".into(), (level % 3 + 1).to_string());
            changed(next, Some("minecraft:item.bottle.empty"), "use_cauldron")
                .using_item()
                .with_event("minecraft:fluid_place")
        }
        Action::CleanBanner => {
            let layers = held.data().map_or(0, |d| d.banner_patterns.len());
            if layers == 0 {
                return pass();
            }
            let mut cleaned = held.clone();
            cleaned.count = 1;
            cleaned.data_mut().banner_patterns.pop();
            cleaned.tidy();
            create_filled_result(player, cleaned, false);
            changed_lowered(cauldron, None, "clean_banner")
        }
        Action::CleanShulker => {
            create_filled_result(player, held.transmute("minecraft:shulker_box", 1), false);
            changed_lowered(cauldron, None, "clean_shulker_box")
        }
        Action::FillLava => fill_bucket(
            player,
            "minecraft:lava_bucket",
            true,
            "minecraft:item.bucket.fill_lava",
        ),
        Action::FillSnow => fill_bucket(
            player,
            "minecraft:powder_snow_bucket",
            level == 3,
            "minecraft:item.bucket.fill_powder_snow",
        ),
    }
}

/// What `interact` would answer for `held`, changing nothing: a client's
/// `useItemOn`, where every interaction checks the same conditions and acts
/// only `if (!level.isClientSide())`.
pub fn outcome(
    cauldron: &BlockState,
    under_water: bool,
    removes_dye: bool,
    held: &ItemStack,
) -> Outcome {
    struct Dry(Option<ItemStack>);
    impl Player for Dry {
        fn creative(&self) -> bool {
            false
        }
        fn hand(&mut self) -> &mut Option<ItemStack> {
            &mut self.0
        }
        fn add(&mut self, _: &mut ItemStack) -> bool {
            true
        }
        fn contains(&self, _: &ItemStack) -> bool {
            false
        }
        fn drop_stack(&mut self, _: ItemStack) {}
    }
    interact(
        cauldron,
        under_water,
        removes_dye,
        &mut Dry(Some(held.clone())),
    )
    .outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player whose inventory is full: `Inventory.add` takes nothing, and in creative clears
    /// what it can't place, as vanilla's does.
    struct Full {
        creative: bool,
        hand: Option<ItemStack>,
        dropped: Vec<ItemStack>,
    }
    impl Player for Full {
        fn creative(&self) -> bool {
            self.creative
        }
        fn hand(&mut self) -> &mut Option<ItemStack> {
            &mut self.hand
        }
        fn add(&mut self, stack: &mut ItemStack) -> bool {
            if self.creative {
                stack.count = 0;
                return true;
            }
            false
        }
        fn contains(&self, _: &ItemStack) -> bool {
            false
        }
        fn drop_stack(&mut self, stack: ItemStack) {
            self.dropped.push(stack);
        }
    }

    #[test]
    fn a_filled_result_with_a_full_inventory_drops_only_in_survival() {
        let buckets = |count| Some(ItemStack::new("minecraft:bucket", count));
        for filled in ["minecraft:water_bucket", "minecraft:lava_bucket"] {
            let new = ItemStack::new(filled, 1);
            // Creative: the hand is kept, and the filled bucket the inventory can't take is
            // discarded, not dropped.
            let mut p = Full {
                creative: true,
                hand: buckets(2),
                dropped: Vec::new(),
            };
            create_filled_result(&mut p, new.clone(), true);
            assert_eq!((p.hand, p.dropped), (buckets(2), vec![]));
            // Survival: one bucket is used up, and the filled one is dropped.
            let mut p = Full {
                creative: false,
                hand: buckets(2),
                dropped: Vec::new(),
            };
            create_filled_result(&mut p, new.clone(), true);
            assert_eq!((p.hand, p.dropped), (buckets(1), vec![new.clone()]));
            // The last bucket becomes the filled one in the hand.
            let mut p = Full {
                creative: false,
                hand: buckets(1),
                dropped: Vec::new(),
            };
            create_filled_result(&mut p, new.clone(), true);
            assert_eq!((p.hand, p.dropped), (Some(new), vec![]));
        }
    }

    #[test]
    fn a_client_sees_the_bucket_taken_without_acting() {
        let empty = state("minecraft:cauldron", None);
        let water = |l| state("minecraft:water_cauldron", Some(l));
        let stack = |id| ItemStack::new(id, 1);
        let outcome = |c: &BlockState, under_water, id| outcome(c, under_water, false, &stack(id));
        assert_eq!(
            outcome(&empty, false, "minecraft:water_bucket"),
            Outcome::Success
        );
        assert_eq!(
            outcome(&empty, false, "minecraft:lava_bucket"),
            Outcome::Success
        );
        // fillLavaInteraction under water: CONSUME, the bucket kept.
        assert_eq!(
            outcome(&empty, true, "minecraft:lava_bucket"),
            Outcome::Consume
        );
        assert_eq!(
            outcome(&water(3), false, "minecraft:bucket"),
            Outcome::Success
        );
        // fillBucket needs a full cauldron.
        assert_eq!(outcome(&water(2), false, "minecraft:bucket"), Outcome::Pass);
        assert_eq!(outcome(&empty, false, "minecraft:bucket"), Outcome::Pass);
        assert_eq!(outcome(&empty, false, "minecraft:stone"), Outcome::Pass);
    }

    #[test]
    fn table_preserves_tag_priority_and_cauldron_guards() {
        let water = state("minecraft:water_cauldron", Some(3));
        let lava = state("minecraft:lava_cauldron", None);
        let bucket = ItemStack::new("minecraft:water_bucket", 1);
        // A matching dye tag is terminal even when the item is also in the item table.
        assert_eq!(outcome(&water, false, true, &bucket), Outcome::Pass);
        assert_eq!(outcome(&water, false, false, &bucket), Outcome::Success);
        assert_eq!(outcome(&lava, false, true, &bucket), Outcome::Success);
        assert_eq!(
            outcome(
                &lava,
                true,
                false,
                &ItemStack::new("minecraft:lava_bucket", 1)
            ),
            Outcome::Consume
        );
        assert_eq!(
            outcome(
                &water,
                false,
                false,
                &ItemStack::new("minecraft:red_banner", 1)
            ),
            Outcome::Pass
        );
        assert_eq!(
            outcome(
                &water,
                false,
                false,
                &ItemStack::new("minecraft:red_shulker_box", 1)
            ),
            Outcome::Success
        );
        assert_eq!(
            outcome(
                &water,
                false,
                false,
                &ItemStack::new("example:red_shulker_box", 1)
            ),
            Outcome::Pass
        );
    }

    #[test]
    fn a_burning_entity_lowers_water_and_melts_powder_snow() {
        let water = |l| state("minecraft:water_cauldron", Some(l));
        assert_eq!(extinguished(&water(3)), water(2));
        assert_eq!(extinguished(&water(1)), state("minecraft:cauldron", None));
        // Powder snow turns to water of its level, then lowers.
        assert_eq!(
            extinguished(&state("minecraft:powder_snow_cauldron", Some(3))),
            water(2)
        );
        assert_eq!(
            extinguished(&state("minecraft:powder_snow_cauldron", Some(1))),
            state("minecraft:cauldron", None)
        );
    }
}

#[cfg(test)]
#[path = "cauldron/interaction_tests.rs"]
mod interaction_tests;
