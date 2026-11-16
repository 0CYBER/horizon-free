#![cfg(test)]
//! `ItemPart::is_primary_item_for`, `apply_enchantments` and `is_not_replaceable_by_pick_action`
//! through the game: the answers each part gets, starting from vanilla's, and what the game does
//! with what it leaves. Each test installs process-wide parts: run alone with the prepared
//! resources (`HORIZON_TEST_RESOURCES`).
use super::*;
use mod_api::{
    ApplyEnchantmentsEvent, Hooks, IsNotReplaceableByPickActionEvent, IsPrimaryItemForEvent,
    ItemPart, ModEntry, Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};

const SHARPNESS: &str = "minecraft:sharpness";

/// What the parts answer, and what they were asked.
#[derive(Clone, Default)]
struct Answers {
    /// `is_primary_item_for`: the item and the answer for sharpness.
    primary: Arc<Mutex<Vec<(String, bool)>>>,
    /// `is_primary_item_for`: how many times a part was asked.
    primary_asked: Arc<Mutex<usize>>,
    /// `apply_enchantments`: the damage and the count the part sets on the result.
    applied: Arc<Mutex<Option<(u32, u32)>>>,
    /// `apply_enchantments`: the list it was given and the result as it arrived.
    applied_asked: Arc<Mutex<Vec<(Vec<(String, u32)>, mod_api::ItemStack)>>>,
    /// `is_not_replaceable_by_pick_action`: the item and the answer.
    keeps: Arc<Mutex<Vec<(String, bool)>>>,
    /// `is_not_replaceable_by_pick_action`: the slot and the answer as it arrived.
    keeps_asked: Arc<Mutex<Vec<(u32, bool)>>>,
}

#[derive(Clone)]
struct Part {
    item: &'static str,
    answers: Answers,
}

#[mod_api::horizon_part]
impl ItemPart for Part {
    fn is_primary_item_for(&self, _: &dyn mod_api::Host, event: &mut IsPrimaryItemForEvent<'_>) {
        *self.answers.primary_asked.lock().unwrap() += 1;
        if event.enchantment != SHARPNESS {
            return;
        }
        let answers = self.answers.primary.lock().unwrap();
        if let Some((_, answer)) = answers.iter().find(|(item, _)| item == self.item) {
            event.primary = *answer;
        }
    }
    fn apply_enchantments(&self, _: &dyn mod_api::Host, event: &mut ApplyEnchantmentsEvent<'_>) {
        let list = event
            .enchantments
            .iter()
            .map(|(id, level)| (id.to_string(), *level))
            .collect();
        self.answers
            .applied_asked
            .lock()
            .unwrap()
            .push((list, event.result.clone()));
        if let Some((damage, count)) = *self.answers.applied.lock().unwrap() {
            event.result.set_damage(damage);
            let _ = event.result.set_count(count);
        }
    }
    fn is_not_replaceable_by_pick_action(
        &self,
        _: &dyn mod_api::Host,
        event: &mut IsNotReplaceableByPickActionEvent<'_>,
    ) {
        self.answers
            .keeps_asked
            .lock()
            .unwrap()
            .push((event.slot, event.not_replaceable));
        let answers = self.answers.keeps.lock().unwrap();
        if let Some((_, answer)) = answers.iter().find(|(item, _)| item == self.item) {
            event.not_replaceable = *answer;
        }
    }
}

struct Mod(Vec<Part>);

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for part in &self.0 {
            let key = r.item_key(part.item);
            r.item_part(key, part.clone());
        }
    }
}

/// Runs `test` on an integrated-server game with a part on the stick, the diamond sword and
/// the stone, sharing one set of answers, after the parts resolved.
fn with_parts(test: impl FnOnce(&mut Game<'_>, &Answers)) {
    let answers = Answers::default();
    let parts = [
        "minecraft:stick",
        "minecraft:diamond_sword",
        "minecraft:stone",
    ]
    .map(|item| Part {
        item,
        answers: answers.clone(),
    })
    .to_vec();
    let mods: mod_api::Mods = vec![ModEntry::new("enchant_pick", Box::new(Mod(parts)))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.resolve_parts().unwrap();
            test(game, &answers);
            // The resources may be shared with the next test's game: retire the answers.
            game.clear_world_parts();
        });
    });
}

/// `available_results` at value 30 over `[sharpness]` for `item`.
fn sharpness_at_30(game: &Game<'_>, item: &str) -> Vec<(String, u32)> {
    game.stations()
        .enchantments
        .available_results(game.items(), 30, item, &[SHARPNESS.into()])
        .into_iter()
        .map(|(id, level)| (id.to_string(), level))
        .collect()
}

#[test]
#[ignore = "daily"]
fn primary_answers_are_asked_at_install_and_read_by_selection() {
    with_parts(|game, answers| {
        let registered = game.stations().enchantments.list.len();
        assert!(registered > 0);
        // No answer: vanilla. A stick has no sharpness; a diamond sword gets level 3, the
        // highest whose window (23 to 43) holds 30.
        let level_3 = vec![(SHARPNESS.to_owned(), 3)];
        assert!(sharpness_at_30(game, "minecraft:stick").is_empty());
        assert_eq!(sharpness_at_30(game, "minecraft:diamond_sword"), level_3);
        // Asked once per registered enchantment for each of the three items, at install.
        assert_eq!(*answers.primary_asked.lock().unwrap(), 3 * registered);
        *answers.primary.lock().unwrap() = vec![
            ("minecraft:stick".into(), true),
            ("minecraft:diamond_sword".into(), false),
        ];
        // Read from the table: no new answer until the parts resolve again.
        assert!(sharpness_at_30(game, "minecraft:stick").is_empty());
        game.resolve_parts().unwrap();
        assert_eq!(*answers.primary_asked.lock().unwrap(), 6 * registered);
        assert_eq!(sharpness_at_30(game, "minecraft:stick"), level_3);
        assert!(sharpness_at_30(game, "minecraft:diamond_sword").is_empty());
        // Selection asks no part: the count stays.
        for _ in 0..5 {
            sharpness_at_30(game, "minecraft:stick");
        }
        assert_eq!(*answers.primary_asked.lock().unwrap(), 6 * registered);
        // A book is offered every enchantment whatever the parts say.
        assert_eq!(sharpness_at_30(game, "minecraft:book"), level_3);
        // The answers are this server's snapshot's: its stations and its loot items read them.
        let items = &game.items().enchantments;
        assert_eq!(
            items.primary_answers().answer("minecraft:stick", SHARPNESS),
            Some(true)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_client_refresh_keeps_and_a_world_close_retires_the_servers_answers() {
    use crate::game::execution_role::ExecutionRole;
    with_parts(|game, answers| {
        *answers.primary.lock().unwrap() = vec![("minecraft:stick".into(), true)];
        game.resolve_parts().unwrap();
        let level_3 = vec![(SHARPNESS.to_owned(), 3)];
        assert_eq!(sharpness_at_30(game, "minecraft:stick"), level_3);
        let asked = *answers.primary_asked.lock().unwrap();
        // A logical client's tag install (a client refresh on an integrated server, which may
        // share this snapshot) neither asks nor clears: the server's answers stay.
        game.options.execution_role = ExecutionRole::Client;
        *answers.primary.lock().unwrap() = Vec::new();
        game.resolve_primary_items();
        assert_eq!(*answers.primary_asked.lock().unwrap(), asked);
        assert_eq!(sharpness_at_30(game, "minecraft:stick"), level_3);
        game.options.execution_role = ExecutionRole::IntegratedServer;
        // The server's world closes with its parts: the answers are retired.
        game.clear_world_parts();
        assert!(sharpness_at_30(game, "minecraft:stick").is_empty());
        assert!(game.stations().enchantments.primary_answers().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn apply_enchantments_starts_from_vanilla_and_refuses_another_count() {
    with_parts(|game, answers| {
        let sword = ItemStack::new("minecraft:diamond_sword", 1);
        let vanilla = horizon_client::stations::apply_enchantment(
            &game.stations().enchantments,
            game.items(),
            7,
            2,
            30,
            &sword,
        )
        .unwrap();
        let list = horizon_client::stations::enchantment_list(
            &game.stations().enchantments,
            game.items(),
            7,
            2,
            30,
            &sword,
        );
        // No answer: vanilla's result, and the part got the list and that result.
        assert_eq!(
            game.table_enchant(7, 2, 30, &sword).as_ref(),
            Some(&vanilla)
        );
        let asked = std::mem::take(&mut *answers.applied_asked.lock().unwrap());
        assert_eq!(asked.len(), 1);
        let want: Vec<_> = list.iter().map(|(id, l)| (id.to_string(), *l)).collect();
        assert_eq!(asked[0].0, want);
        assert_eq!(asked[0].1.count(), 1);
        assert_eq!(asked[0].1.damage(), 0);
        // A changed result is the table's.
        *answers.applied.lock().unwrap() = Some((7, 1));
        let mut changed = vanilla.clone();
        changed.damage = 7;
        assert_eq!(game.table_enchant(7, 2, 30, &sword), Some(changed));
        // A count other than the input's is refused: vanilla's result.
        *answers.applied.lock().unwrap() = Some((7, 2));
        assert_eq!(game.table_enchant(7, 2, 30, &sword), Some(vanilla));
        // A stick isn't enchantable: no result, and its part isn't asked.
        answers.applied_asked.lock().unwrap().clear();
        let stick = ItemStack::new("minecraft:stick", 1);
        assert_eq!(game.table_enchant(7, 2, 30, &stick), None);
        assert!(answers.applied_asked.lock().unwrap().is_empty());
    });
}

/// Enchanted diamond sword, stone, then dirt × 7 in the hotbar, oak planks in main slot 20,
/// slot 0 selected: picks the planks, and gives the selected slot and what went to slot 20.
fn pick_planks(game: &mut Game<'_>) -> (usize, String) {
    game.inventory = vec![None; game.inventory.len()];
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    sword.enchant(SHARPNESS, 1);
    game.inventory[0] = Some(sword);
    game.inventory[1] = Some(ItemStack::new("minecraft:stone", 5));
    for slot in 2..9 {
        game.inventory[slot] = Some(ItemStack::new("minecraft:dirt", 5));
    }
    game.inventory[20] = Some(ItemStack::new("minecraft:oak_planks", 5));
    game.selected = 0;
    assert!(game.pick_into_inventory(&ItemStack::new("minecraft:oak_planks", 1), false));
    let moved = game.inventory[20].as_ref().unwrap().item.to_string();
    (game.selected, moved)
}

#[test]
#[ignore = "daily"]
fn pick_asks_each_hotbar_stack_until_one_may_be_replaced() {
    with_parts(|game, answers| {
        // Vanilla: the sword stays (enchanted), the stone goes.
        assert_eq!(pick_planks(game), (1, "minecraft:stone".into()));
        // The sword's part and the stone's, with vanilla's answers.
        assert_eq!(
            std::mem::take(&mut *answers.keeps_asked.lock().unwrap()),
            [(0, true), (1, false)]
        );
        *answers.keeps.lock().unwrap() = vec![("minecraft:stone".into(), true)];
        assert_eq!(pick_planks(game), (2, "minecraft:dirt".into()));
        *answers.keeps.lock().unwrap() = vec![("minecraft:diamond_sword".into(), false)];
        assert_eq!(pick_planks(game), (0, "minecraft:diamond_sword".into()));
        // A hotbar with an empty slot asks nobody.
        answers.keeps_asked.lock().unwrap().clear();
        game.inventory = vec![None; game.inventory.len()];
        game.inventory[0] = Some(ItemStack::new("minecraft:stone", 5));
        game.inventory[20] = Some(ItemStack::new("minecraft:oak_planks", 5));
        game.selected = 0;
        assert!(game.pick_into_inventory(&ItemStack::new("minecraft:oak_planks", 1), false));
        assert_eq!(game.selected, 1);
        assert!(answers.keeps_asked.lock().unwrap().is_empty());
    });
}
