use super::*;

fn run(item: &str, maxes: [Option<i32>; 2], damages: [u32; 2], remove: bool) -> Option<ItemStack> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources"),
    );
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
    let recipes = Recipes::load(
        &root,
        &items,
        &PackOverlay::NONE,
        &crate::test_resources::server_load(),
    )
    .unwrap();
    let ctx = Context {
        recipes: &recipes,
        context: crate::test_resources::server_loop_context(),
        map_scale: &|_| None,
    };
    let mut cells = [0, 1].map(|i| {
        let mut s = ItemStack::new(item, 1);
        let damage = damages[i];
        if let Some(max) = maxes[i] {
            s.data_mut()
                .set_other("minecraft:max_damage", Some(Tag::Int(max)));
        }
        s.damage = damage;
        Some(s)
    });
    if remove {
        cells[0].as_mut().unwrap().data_mut().set_other(
            "!minecraft:max_damage",
            Some(Tag::Compound(Default::default())),
        );
    }
    let input = Input::of(&cells, 2).unwrap();
    if !Special::RepairItem.matches(&ctx, &input) {
        return None;
    }
    Special::RepairItem.assemble(&ctx, &input)
}

#[test]
#[ignore = "daily"]
fn repair_overridden_durability() {
    let out = run(
        "minecraft:diamond_pickaxe",
        [Some(100), Some(100)],
        [60, 70],
        false,
    )
    .unwrap();
    assert_eq!(out.max_damage(None), 100);
    assert_eq!(out.damage, 25);
}

#[test]
#[ignore = "daily"]
fn repair_added_durability() {
    let out = run(
        "minecraft:totem_of_undying",
        [Some(10), Some(10)],
        [3, 3],
        false,
    )
    .unwrap();
    assert_eq!(&*out.item, "minecraft:totem_of_undying");
    assert_eq!(out.max_damage(None), 10);
    assert_eq!(out.damage, 0);
    // A totem has no default `damage`: the result keeps it at 0, so it stays damageable.
    assert!(out.data().is_some_and(|d| d.explicit_zero_damage));
}

#[test]
#[ignore = "daily"]
fn repair_removed_durability() {
    assert!(run("minecraft:diamond_pickaxe", [None, None], [0, 0], true).is_none());
}

#[test]
#[ignore = "daily"]
fn repair_plain_pickaxes() {
    let out = run(
        "minecraft:diamond_pickaxe",
        [None, None],
        [1000, 1200],
        false,
    )
    .unwrap();
    assert_eq!(out.max_damage(None), 1561);
    assert_eq!(out.damage, 561);
}

#[test]
#[ignore = "daily"]
fn repair_takes_the_larger_maximum() {
    // remaining = (100 - 60) + (200 - 150) + 200 * 5 / 100 = 40 + 50 + 10 = 100
    let out = run(
        "minecraft:diamond_pickaxe",
        [Some(100), Some(200)],
        [60, 150],
        false,
    )
    .unwrap();
    assert_eq!(out.max_damage(None), 200);
    assert_eq!(out.damage, 100);
}

#[test]
#[ignore = "daily"]
fn repair_clamps_damage_above_the_maximum() {
    // Both stacks claim damage 150 of 100: each counts as fully broken, so remaining = 0 + 0 + 100 * 5 / 100 = 5.
    let out = run(
        "minecraft:diamond_pickaxe",
        [Some(100), Some(100)],
        [150, 150],
        false,
    )
    .unwrap();
    assert_eq!(out.max_damage(None), 100);
    assert_eq!(out.damage, 95);
}
