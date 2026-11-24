#![cfg(test)]
use super::*;
use horizon_client::enchant::Enchantments;

fn table(id: &str, slots: &[&str], effect: bool) -> Enchantments {
    let effects = if effect {
        serde_json::json!({"minecraft:repair_with_xp": [{"effect": {"type": "minecraft:multiply", "factor": 2}}]})
    } else {
        serde_json::json!({})
    };
    Enchantments::default()
        .with_entries([(
            id,
            &serde_json::json!({
                "description": "Patch", "supported_items": "minecraft:diamond_pickaxe",
                "weight": 1, "max_level": 1, "anvil_cost": 1,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "slots": slots, "effects": effects
            }),
        )])
        .unwrap()
        .unwrap()
}

fn repair_with(
    id: &str,
    chest: bool,
    table: &Enchantments,
    mods: mod_api::Mods,
) -> (u32, u32) {
    mod_platform::testing::with_mods(mods, || {
        let mut result = None;
        crate::game::native_entity_refs::tests::with_game(|game| {
            let root = tempfile::tempdir().unwrap();
            let pack = root.path().join("datapacks/mending");
            std::fs::create_dir_all(&pack).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"Mending fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["file/mending"],"disabled":[]}"#,
            )
            .unwrap();
            game.world_root = root.path().to_path_buf();
            let packs = game.reload_datapacks(false, &[]).unwrap();
            let resources = game.options.resources.clone();
            game.world_tables = game
                .stage_world_tables(&packs, false, &resources)
                .unwrap()
                .0;
            game.world_tables.as_mut().unwrap().stations.enchantments =
                std::sync::Arc::new(table.clone());
            let mut stack = ItemStack::new(
                if chest {
                    "minecraft:diamond_chestplate"
                } else {
                    "minecraft:diamond_pickaxe"
                },
                1,
            );
            stack.damage = 20;
            stack.set_crafting_enchantments(vec![(id.into(), 1)]);
            game.inventory.fill(None);
            game.armor.fill(None);
            game.offhand = None;
            if chest {
                game.armor[1] = Some(stack);
            } else {
                game.inventory[game.selected] = Some(stack);
            }
            let remaining = game.mending(7);
            let damage = if chest {
                game.armor[1].as_ref()
            } else {
                game.inventory[game.selected].as_ref()
            }
            .unwrap()
            .damage;
            result = Some((damage, remaining));
        });
        result.unwrap()
    })
}

fn repair(id: &str, chest: bool, table: &Enchantments) -> (u32, u32) {
    repair_with(id, chest, table, Default::default())
}

#[test]
#[ignore = "daily"]
fn mending_pick_data_pack_effect() {
    assert_eq!(
        repair("test:patch", false, &table("test:patch", &["any"], true)),
        (6, 0)
    );
}

#[test]
#[ignore = "daily"]
fn mending_pick_redefined_mending_without_effect() {
    assert_eq!(
        repair(
            "minecraft:mending",
            false,
            &table("minecraft:mending", &["any"], false)
        ),
        (20, 7)
    );
}

#[test]
#[ignore = "daily"]
fn mending_pick_head_effect_excludes_chestplate() {
    assert_eq!(
        repair("test:patch", true, &table("test:patch", &["head"], true)),
        (20, 7)
    );
}

struct GrantMending;

#[mod_api::__subscriptions]
impl mod_api::Hooks for GrantMending {
    fn get_enchantment_level(&self, event: &mut mod_api::GetEnchantmentLevelEvent<'_>) {
        if mod_api::__private::item_key_id(event.stack.item()) != Some("minecraft:diamond_pickaxe")
        {
            return;
        }
        let mending = mod_api::Identifier::parse("minecraft:mending").unwrap();
        if event.target.is_none_or(|target| *target == mending) {
            event.enchantments.upgrade(mending, 1);
        }
    }
}

#[test]
#[ignore = "daily"]
fn mending_pick_uses_hook_granted_enchantment() {
    crate::game::api_values::install_stack_hooks();
    let _ = horizon_items::native_enchantment_hooks::install(
        crate::enchantment_levels::get_enchantment_level,
    );
    let mods = vec![mod_api::ModEntry::new(
        "grant_mending",
        Box::new(GrantMending),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.inventory.fill(None);
            game.armor.fill(None);
            game.offhand = None;
            let mut stack = ItemStack::new("minecraft:diamond_pickaxe", 1);
            stack.damage = 20;
            assert!(stack.enchantments().is_empty());
            game.inventory[game.selected] = Some(stack);
            assert_eq!(game.mending(7), 0);
            let current = game.inventory[game.selected].as_ref().unwrap();
            assert_eq!(current.damage, 6);
            assert!(current.enchantments().is_empty());
        });
    });
}

struct ZeroRepair;

#[mod_api::__subscriptions]
impl mod_api::Hooks for ZeroRepair {
    fn player_item_mend(&self, _: &mut dyn mod_api::ServerHost, event: &mut mod_api::PlayerItemMendEvent<'_>) {
        event.repair = 0;
    }
}

/// A listener that refuses the repair without spending XP must end the mending
/// walk, not retry the same stack forever (kacpherek on #18864).
#[test]
#[ignore = "daily"]
fn mending_stops_when_a_listener_zeroes_repair_without_spending_xp() {
    let mods = vec![mod_api::ModEntry::new("zero_repair", Box::new(ZeroRepair))].into();
    let table = table("test:patch", &["any"], true);
    assert_eq!(
        repair_with("test:patch", false, &table, mods),
        (20, 1),
        "damage must stay and the unspent XP must be returned"
    );
}
