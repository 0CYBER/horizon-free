use super::*;
struct Hand {
    stack: Option<ItemStack>,
    dropped: Vec<ItemStack>,
}
impl Player for Hand {
    fn creative(&self) -> bool {
        false
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        &mut self.stack
    }
    fn add(&mut self, _: &mut ItemStack) -> bool {
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
fn cauldron_no_mod_water_bucket_and_bottle_keep_vanilla_values() {
    let mut hand = Hand {
        stack: Some(ItemStack::new("minecraft:water_bucket", 1)),
        dropped: vec![],
    };
    let result = interact(&state("minecraft:cauldron", None), false, false, &mut hand);
    assert_eq!(result.outcome, Outcome::Success);
    assert_eq!(
        result.block,
        Some(state("minecraft:water_cauldron", Some(3)))
    );
    assert_eq!(hand.stack, Some(ItemStack::new("minecraft:bucket", 1)));
    assert_eq!(result.sound, Some("minecraft:item.bucket.empty"));
    assert_eq!(result.fluid_event, Some("minecraft:fluid_place"));
    assert!(hand.dropped.is_empty());
    hand.stack = Some(ItemStack::new("minecraft:glass_bottle", 1));
    let result = interact(
        &state("minecraft:water_cauldron", Some(3)),
        false,
        false,
        &mut hand,
    );
    assert_eq!(result.outcome, Outcome::Success);
    assert_eq!(
        result.block,
        Some(state("minecraft:water_cauldron", Some(2)))
    );
    assert_eq!(
        hand.stack,
        Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"))
    );
    assert_eq!(result.sound, Some("minecraft:item.bottle.fill"));
    assert_eq!(result.fluid_event, Some("minecraft:fluid_pickup"));
    assert!(hand.dropped.is_empty());
}
#[test]
fn cauldron_prebound_entries_refuse_replacement_and_keep_tag_priority() {
    use interactions::{Dispatcher, Entry};
    let mut table = Dispatcher::<&str>::default();
    assert!(matches!(
        table.register(
            "minecraft:cauldron".into(),
            "minecraft:water_bucket".into(),
            "mod"
        ),
        Err(Entry::Vanilla(Action::EmptyWater))
    ));
    assert!(
        table
            .register(
                "minecraft:water_cauldron".into(),
                "minecraft:leather_helmet".into(),
                "first"
            )
            .is_ok()
    );
    assert!(matches!(
        table.register(
            "minecraft:water_cauldron".into(),
            "minecraft:leather_helmet".into(),
            "second"
        ),
        Err(Entry::Mod(owner)) if *owner == "first"
    ));
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:leather_helmet",
            true,
            |_| false
        ),
        Some(Entry::Dye)
    ));
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:leather_helmet",
            false,
            |_| false
        ),
        Some(Entry::Mod(owner)) if *owner == "first"
    ));
}
#[test]
fn cauldron_tag_entries_follow_exact_ones_in_registration_order() {
    use interactions::{Dispatcher, Entry};
    let mut table = Dispatcher::<&str>::default();
    let water = || "minecraft:water_cauldron".to_owned();
    assert!(
        table
            .register(water(), "minecraft:milk_bucket".into(), "exact")
            .is_ok()
    );
    assert!(
        table
            .register_tag(water(), "c:buckets/milk".into(), "milk")
            .is_ok()
    );
    assert!(
        table
            .register_tag(water(), "c:buckets".into(), "buckets")
            .is_ok()
    );
    assert!(matches!(
        table.register_tag(water(), "c:buckets/milk".into(), "again"),
        Err(owner) if *owner == "milk"
    ));
    // Membership is asked at use time: the tags hold whatever is loaded now.
    let milk = |tag: &str| matches!(tag, "c:buckets/milk" | "c:buckets");
    let pick = |item| match table.select("minecraft:water_cauldron", item, false, milk) {
        Some(Entry::Mod(owner)) => Some(*owner),
        _ => None,
    };
    assert_eq!(pick("minecraft:milk_bucket"), Some("exact"));
    assert_eq!(pick("othermod:goat_milk_bucket"), Some("milk"));
    assert!(
        table
            .select(
                "minecraft:water_cauldron",
                "othermod:goat_milk_bucket",
                false,
                |_| false
            )
            .is_none()
    );
    // A tag entry on a custom block declares that block.
    assert!(
        table
            .register_tag("test:syrup_cauldron".into(), "c:buckets".into(), "custom")
            .is_ok()
    );
    assert!(table.contains_block("test:syrup_cauldron"));
    assert!(matches!(
        table.select("test:syrup_cauldron", "othermod:goat_milk_bucket", false, milk),
        Some(Entry::Mod(owner)) if *owner == "custom"
    ));
}
