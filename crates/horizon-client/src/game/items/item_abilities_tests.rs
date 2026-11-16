#![cfg(test)]
use super::Game;
use super::item_abilities::ability;
use horizon_client::items::ItemStack;
use horizon_client::mobs::Mob;
use horizon_core::geometry::{Direction, Vec3};
use horizon_core::use_animation::UseAnimation;
use mod_api::{CanPerformActionEvent, Hooks, Host, ItemPart, Registrar, RegistrationContext};
use std::sync::Arc;

/// A part of `item` that grants the abilities in `grants` and denies the ones in `denies`.
#[derive(Clone)]
struct Abilities {
    item: &'static str,
    grants: &'static [&'static str],
    denies: &'static [&'static str],
}

#[mod_api::horizon_part]
impl ItemPart for Abilities {
    fn can_perform_action(&self, _: &dyn Host, event: &mut CanPerformActionEvent<'_>) {
        if self.grants.contains(&event.ability) {
            event.can_perform = true;
        } else if self.denies.contains(&event.ability) {
            event.can_perform = false;
        }
    }
}

struct Mod(Vec<Abilities>);

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for part in &self.0 {
            let key = r.item_key(part.item);
            r.item_part(key, part.clone());
        }
    }
}

/// A serving level with one chunk of air and a pumpkin at (4, 64, 4).
fn field(game: &mut Game<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    game.world = horizon_client::world::World::new(game.res.facts().air, 1);
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    let pumpkin = game
        .pack
        .assets
        .registry
        .default_state("minecraft:pumpkin")
        .unwrap();
    game.world.set(4, 64, 4, pumpkin).unwrap();
}

fn block_name(game: &Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

fn add_sheep(game: &mut Game<'_>, x: f64) -> usize {
    let kind = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
    game.next_entity += 1;
    game.mobs.push(Mob::new(
        game.next_entity,
        kind,
        Vec3::new(x, 64.0, 0.5),
        0.0,
    ));
    game.mobs.len() - 1
}

/// Right click on the sheep at `index` holding `stack`.
fn click_sheep(game: &mut Game<'_>, index: usize, stack: &ItemStack) -> bool {
    game.inventory[game.selected] = Some(stack.clone());
    let id = game.mobs[index].id as i32;
    game.interact_mob(Some(crate::game::InteractTarget {
        id,
        location: [0.0; 3],
    }))
    .unwrap()
}

const SHEARING: &[&str] = &[ability::SHEARS_HARVEST, ability::SHEARS_CARVE];

/// Parts that grant a sword `shield_block`, a stick the shears' two abilities, and take
/// both away from the shears and `shield_block` from the shield.
#[test]
#[ignore = "daily"]
fn parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item() {
    crate::game::in_own_process(
        "game::item_abilities_tests::parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item",
        parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item_isolated,
    );
}

fn parts_grant_and_deny_the_abilities_vanilla_asks_of_an_item_isolated() {
    use crate::game::native_entity_refs::tests::with_game;
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "abilities",
        Box::new(Mod(vec![
            Abilities {
                item: "minecraft:diamond_sword",
                grants: &[ability::SHIELD_BLOCK],
                denies: &[],
            },
            Abilities {
                item: "minecraft:stick",
                grants: &[ability::SHEARS_CARVE],
                denies: &[],
            },
            Abilities {
                item: "minecraft:arrow",
                grants: &[ability::SHEARS_HARVEST],
                denies: &[ability::SHEARS_CARVE],
            },
            Abilities {
                item: "minecraft:shears",
                grants: &[],
                denies: SHEARING,
            },
            Abilities {
                item: "minecraft:shield",
                grants: &[],
                denies: &[ability::SHIELD_BLOCK],
            },
        ])),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            field(game);
            let sword = ItemStack::new("minecraft:diamond_sword", 1);
            let axe = ItemStack::new("minecraft:diamond_axe", 1);
            let shield = ItemStack::new("minecraft:shield", 1);

            // A sword the part lets block blocks as a bare `blocks_attacks` component does (no
            // block delay, one reduction of everything within 90 degrees), is held up for as
            // long as a shield, and shows the blocking pose.
            let granted = game
                .blocks_attacks(&sword)
                .expect("the part grants blocking");
            let again = game.blocks_attacks(&sword).unwrap();
            assert!(std::ptr::eq(granted.as_ref(), again.as_ref()));
            assert_eq!(granted.delay_seconds, 0.0);
            assert_eq!(granted.reductions.len(), 1);
            assert_eq!(granted.reductions[0].horizontal_angle, 90.0);
            drop(granted);
            assert_eq!(game.use_duration(&sword), 72_000);
            assert_eq!(game.item_use_animation(&sword), UseAnimation::Block);
            // An axe no part is attached to blocks nothing.
            assert!(game.blocks_attacks(&axe).is_none());
            assert_eq!(game.use_duration(&axe), 0);
            assert_eq!(game.item_use_animation(&axe), UseAnimation::None);
            // A shield whose part denies the ability no longer blocks nor is raised.
            assert!(game.blocks_attacks(&shield).is_none());
            assert_eq!(game.use_duration(&shield), 0);
            game.inventory[game.selected] = Some(shield.clone());
            assert!(!game.raise_shield(game.selected));
            game.inventory[game.selected] = Some(sword.clone());
            assert!(game.raise_shield(game.selected));

            // Shears carve a pumpkin only when the ability is not denied: a stick the part lets
            // carve does, and an item no part is attached to never does.
            let carve = |game: &mut Game<'_>, item: &str| {
                let held = ItemStack::new(item, 1);
                let carved = game
                    .use_pumpkin([4, 64, 4], [0, 1, 0], Some(&held))
                    .unwrap();
                (carved, block_name(game, [4, 64, 4]))
            };
            let pumpkin = "minecraft:pumpkin".to_string();
            assert_eq!(carve(game, "minecraft:arrow"), (false, pumpkin.clone()));
            assert_eq!(carve(game, "minecraft:shears"), (false, pumpkin.clone()));
            assert_eq!(carve(game, "minecraft:blaze_rod"), (false, pumpkin));
            assert_eq!(
                carve(game, "minecraft:stick"),
                (true, "minecraft:carved_pumpkin".to_string())
            );

            // A sheep, through a right click: the same.
            let stick = ItemStack::new("minecraft:arrow", 1);
            let shears = ItemStack::new("minecraft:shears", 1);
            let rod = ItemStack::new("minecraft:blaze_rod", 1);
            let sheep = add_sheep(game, 8.5);
            assert!(!click_sheep(game, sheep, &shears));
            assert!(!game.mobs[sheep].extra.sheared);
            assert!(!click_sheep(game, sheep, &rod));
            assert!(!game.mobs[sheep].extra.sheared);
            assert!(click_sheep(game, sheep, &stick));
            assert!(game.mobs[sheep].extra.sheared);

            // A dispenser at (7, 64, 0) facing a sheep at (8, 64, 0): the denied shears and the
            // rod are dropped instead of shearing it, the stick shears it.
            let sheep = add_sheep(game, 8.5);
            for held in [&shears, &rod] {
                game.dispense_item((7, 64, 0), Direction::East, held.clone())
                    .unwrap();
                assert!(!game.mobs[sheep].extra.sheared);
            }
            game.dispense_item((7, 64, 0), Direction::East, stick.clone())
                .unwrap();
            assert!(game.mobs[sheep].extra.sheared);
        });
    });
}

/// With no part attached to any item, vanilla's answers stand: a shield blocks after its
/// quarter second, shears carve, shear and are dispensed as shears, nothing else does.
#[test]
#[ignore = "daily"]
fn without_parts_the_vanilla_item_checks_stand() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        field(game);
        let shield = ItemStack::new("minecraft:shield", 1);
        let granted = game.blocks_attacks(&shield).expect("a shield blocks");
        assert_eq!(granted.delay_seconds, 0.25);
        drop(granted);
        assert_eq!(game.use_duration(&shield), 72_000);
        let sword = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(game.blocks_attacks(&sword).is_none());
        assert_eq!(game.use_duration(&sword), 0);

        let stick = ItemStack::new("minecraft:stick", 1);
        let shears = ItemStack::new("minecraft:shears", 1);
        assert!(
            game.use_pumpkin([4, 64, 4], [0, 1, 0], Some(&stick))
                .is_ok_and(|carved| !carved)
        );
        assert_eq!(block_name(game, [4, 64, 4]), "minecraft:pumpkin");
        assert!(
            game.use_pumpkin([4, 64, 4], [0, 1, 0], Some(&shears))
                .unwrap()
        );
        assert_eq!(block_name(game, [4, 64, 4]), "minecraft:carved_pumpkin");

        let sheep = add_sheep(game, 8.5);
        assert!(!click_sheep(game, sheep, &stick));
        assert!(!game.mobs[sheep].extra.sheared);
        assert!(click_sheep(game, sheep, &shears));
        assert!(game.mobs[sheep].extra.sheared);
        let sheep = add_sheep(game, 8.5);
        game.dispense_item((7, 64, 0), Direction::East, stick)
            .unwrap();
        assert!(!game.mobs[sheep].extra.sheared);
        game.dispense_item((7, 64, 0), Direction::East, shears)
            .unwrap();
        assert!(game.mobs[sheep].extra.sheared);
    });
}
