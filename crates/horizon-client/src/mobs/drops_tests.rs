//! `LivingEntity.die`'s drops of the stacks a mob keeps: `Piglin`'s
//! inventory, `AbstractHorse`'s chest, and the saddle and body equipment
//! slots, which go out as the stacks they are (name and components kept).
use super::{Death, Drop, die};
use crate::items::ItemStack;
use crate::mobs::world::level_random;
use crate::mobs::{Mob, Species};
use crate::random_sequences::RandomSequences;
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;
use horizon_world::loot::{LootData, PackOverlay};
use std::sync::Arc;

fn mob(id: &str, species: Species) -> Mob {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let k = Arc::make_mut(&mut kind);
    k.id = id.into();
    k.species = species;
    Mob::new(1, kind, Vec3::ZERO, 0.0)
}

/// Loot data with no tables: only the drops `die` makes itself. `tag` keeps
/// the tests' folders apart.
fn empty_loot(tag: &str) -> LootData {
    let root =
        std::env::temp_dir().join(format!("horizon-drops-tests-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
    std::fs::write(root.join("items.json"), "{}").unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let loot = LootData::load(&root, &PackOverlay::default(), &defaults).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    loot
}

/// The vanilla data, for the enchantment effects a curse needs; `None`
/// without prepared resources.
fn vanilla_loot() -> Option<LootData> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("data").exists() {
        return None;
    }
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    Some(LootData::load(&root, &PackOverlay::default(), &defaults).unwrap())
}

fn named(item: &str, count: u32, name: &str) -> ItemStack {
    let mut stack = ItemStack::new(item, count);
    stack.data_mut().custom_name = Some(Tag::String(name.into()));
    stack
}

fn death(loot: &LootData) -> Death<'_> {
    Death {
        loot,
        damage: None,
        player: None,
        player_killed: false,
        luck: 0.0,
        vehicle: None,
        mob_drops: true,
        world_seed: 0,
    }
}

/// `Piglin.dropCustomDeathLoot`: `inventory.removeAllItems()` drops every
/// stack as it is, so a named stack of three ingots comes out named, thrice.
#[test]
fn a_piglins_inventory_drops_the_whole_stacks() {
    let loot = empty_loot("piglin");
    let mut m = mob("minecraft:piglin", Species::Piglin);
    m.extra
        .inventory
        .push(named("minecraft:gold_ingot", 3, "Fee"));
    let (out, _) = die(
        &mut m,
        &death(&loot),
        &mut RandomSequences::default(),
        &mut level_random(7),
    );
    let kept: Vec<&ItemStack> = out
        .iter()
        .filter_map(|d| match d {
            Drop::Stack(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(kept.len(), 1, "the inventory drops as one stack: {out:?}");
    assert_eq!((&*kept[0].item, kept[0].count), ("minecraft:gold_ingot", 3));
    assert_eq!(kept[0].custom_name().as_deref(), Some("Fee"));
    assert!(m.extra.inventory.is_empty(), "the inventory is emptied");

    // `dropCustomDeathLoot` is inside `shouldDropLoot`: with `doMobLoot` off
    // the inventory stays put.
    let mut m = mob("minecraft:piglin", Species::Piglin);
    m.extra
        .inventory
        .push(named("minecraft:gold_ingot", 3, "Fee"));
    let mut no_loot = death(&loot);
    no_loot.mob_drops = false;
    let (out, _) = die(
        &mut m,
        &no_loot,
        &mut RandomSequences::default(),
        &mut level_random(7),
    );
    assert!(
        !out.iter().any(|d| match d {
            Drop::Item(s) => s.item == "minecraft:gold_ingot",
            Drop::Stack(s) => &*s.item == "minecraft:gold_ingot",
            _ => false,
        }),
        "no inventory drop with doMobLoot off: {out:?}"
    );
}

/// `AbstractHorse.dropEquipment`: each chest slot drops as the stack it
/// holds, unless the stack's enchantments prevent equipment drops.
#[test]
fn a_donkeys_chest_drops_its_stacks_but_not_a_cursed_one() {
    let Some(loot) = vanilla_loot() else {
        return;
    };
    let mut m = mob("minecraft:donkey", Species::Donkey);
    m.extra.chested = true;
    m.extra.items = vec![
        Some(named("minecraft:diamond", 4, "Keep")),
        Some({
            let mut cursed = ItemStack::new("minecraft:emerald", 2);
            cursed.enchant("minecraft:vanishing_curse", 1);
            cursed
        }),
        None,
    ];
    let (out, _) = die(
        &mut m,
        &death(&loot),
        &mut RandomSequences::default(),
        &mut level_random(7),
    );
    let kept: Vec<&ItemStack> = out
        .iter()
        .filter_map(|d| match d {
            Drop::Stack(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(kept.len(), 1, "only the plain stack drops: {out:?}");
    assert_eq!((&*kept[0].item, kept[0].count), ("minecraft:diamond", 4));
    assert_eq!(kept[0].custom_name().as_deref(), Some("Keep"));
    assert!(
        !out.iter()
            .any(|d| matches!(d, Drop::Item(s) if s.item == "minecraft:emerald")),
        "the cursed stack stays with the donkey: {out:?}"
    );

    // `dropEquipment` is outside `shouldDropLoot`: the chest still empties
    // with `doMobLoot` off.
    let mut m = mob("minecraft:donkey", Species::Donkey);
    m.extra.chested = true;
    m.extra.items = vec![Some(named("minecraft:diamond", 4, "Keep"))];
    let mut no_loot = death(&loot);
    no_loot.mob_drops = false;
    let (out, _) = die(
        &mut m,
        &no_loot,
        &mut RandomSequences::default(),
        &mut level_random(7),
    );
    assert!(
        out.iter()
            .any(|d| matches!(d, Drop::Stack(s) if &*s.item == "minecraft:diamond")),
        "the chest drops with doMobLoot off: {out:?}"
    );
}

/// `Mob.dropCustomDeathLoot` spawns `getItemBySlot`'s stack: a named saddle
/// and a named wolf's armor drop with their names.
#[test]
fn a_named_saddle_and_wolfs_armor_drop_with_their_names() {
    let loot = empty_loot("equipment");
    let mut horse = mob("minecraft:horse", Species::Horse);
    horse.set_saddle(Some(named("minecraft:saddle", 1, "Seat")));
    let mut wolf = mob("minecraft:wolf", Species::Wolf);
    wolf.extra
        .set_body_stack(Some(named("minecraft:wolf_armor", 1, "Fang")));
    let mut names = Vec::new();
    for m in [&mut horse, &mut wolf] {
        let (out, _) = die(
            m,
            &death(&loot),
            &mut RandomSequences::default(),
            &mut level_random(7),
        );
        names.extend(out.iter().filter_map(|d| match d {
            Drop::Stack(s) => s.custom_name(),
            _ => None,
        }));
    }
    assert_eq!(names, ["Seat", "Fang"]);
}
