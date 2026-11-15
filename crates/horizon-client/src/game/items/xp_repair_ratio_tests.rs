#![cfg(test)]
//! `ItemPart::xp_repair_ratio` in Mending's repair (`ExperienceOrb.repairPlayerItems`): the ratio a part
//! gets, the repair it makes, and the experience left for the player. The expected values are worked by
//! hand from vanilla's loop: `repair = min(floor(experience × ratio), damage)`, then
//! `experience − repair × experience / floor(experience × ratio)` goes round again. Each test installs
//! process-wide parts: run alone with the prepared resources.
use super::*;
use mod_api::{Hooks, ItemPart, ModEntry, Registrar, RegistrationContext, XpRepairRatioEvent};
use std::sync::{Arc, Mutex};

/// What a part answers, and the `(experience, ratio)` it was asked with.
#[derive(Clone, Default)]
struct Ratio {
    answer: Arc<Mutex<Option<f32>>>,
    asked: Arc<Mutex<Vec<(u32, f32)>>>,
}

#[mod_api::horizon_part]
impl ItemPart for Ratio {
    fn xp_repair_ratio(&self, _: &dyn mod_api::Host, event: &mut XpRepairRatioEvent<'_>) {
        self.asked
            .lock()
            .unwrap()
            .push((event.experience, event.ratio));
        if let Some(answer) = *self.answer.lock().unwrap() {
            event.ratio = answer;
        }
    }
}

impl Ratio {
    fn answer(&self, answer: Option<f32>) {
        *self.answer.lock().unwrap() = answer;
    }
    fn take_asked(&self) -> Vec<(u32, f32)> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

/// A part on the diamond pickaxe, and one on the diamond helmet.
struct Mod {
    pickaxe: Ratio,
    helmet: Ratio,
}
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:diamond_pickaxe");
        r.item_part(key, self.pickaxe.clone());
        let key = r.item_key("minecraft:diamond_helmet");
        r.item_part(key, self.helmet.clone());
    }
}

/// Runs `test` on a server game with the two parts, answering nothing.
fn with_parts(test: impl FnOnce(&mut Game<'_>, &Ratio, &Ratio)) {
    let (pickaxe, helmet) = (Ratio::default(), Ratio::default());
    let parts = Mod {
        pickaxe: pickaxe.clone(),
        helmet: helmet.clone(),
    };
    let mods: mod_api::Mods = vec![ModEntry::new("mending", Box::new(parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            test(game, &pickaxe, &helmet)
        });
    });
}

/// Holds a Mending diamond pickaxe with `damage`, and the helmet slot an
/// undamaged Mending diamond helmet.
fn hold_pickaxe(game: &mut Game<'_>, damage: u32) {
    game.selected = 0;
    let mut pickaxe = ItemStack::new("minecraft:diamond_pickaxe", 1);
    pickaxe.damage = damage;
    pickaxe.enchant("minecraft:mending", 1);
    game.inventory[0] = Some(pickaxe);
    game.offhand = None;
    let mut helmet = ItemStack::new("minecraft:diamond_helmet", 1);
    helmet.enchant("minecraft:mending", 1);
    game.armor = [None, None, None, Some(helmet)];
}

fn pickaxe_damage(game: &Game<'_>) -> u32 {
    game.inventory[0].as_ref().unwrap().damage
}

/// Mending with 5 points and a part's `answer`: the pickaxe's damage after,
/// and the experience left for the player.
fn mend(game: &mut Game<'_>, part: &Ratio, damage: u32, answer: Option<f32>) -> (u32, u32) {
    hold_pickaxe(game, damage);
    part.answer(answer);
    let rest = game.mending(5);
    (pickaxe_damage(game), rest)
}

#[test]
#[ignore = "daily"]
fn the_ratio_a_part_answers_scales_the_repair() {
    with_parts(|game, pickaxe, helmet| {
        // No answer is vanilla: 5 points repair 10.
        assert_eq!(mend(game, pickaxe, 100, None), (90, 0));
        // The part is asked once, with vanilla Mending's 2.0 per point.
        assert_eq!(pickaxe.take_asked(), [(5, 2.0)]);
        // 4.0 repairs 20; 0.5 repairs floor(2.5) = 2 and spends 5 − 2·5/2 = 0.
        assert_eq!(mend(game, pickaxe, 100, Some(4.0)), (80, 0));
        assert_eq!(mend(game, pickaxe, 100, Some(0.5)), (98, 0));
        // 0 takes the positive vanilla repair to nothing: the 5 points stay
        // the player's.
        assert_eq!(mend(game, pickaxe, 100, Some(0.0)), (100, 5));
        // Damage 3 at 2.0: 3 repaired of 10, 5 − 3·5/10 = 4 left. The second
        // pass finds nothing damaged.
        assert_eq!(mend(game, pickaxe, 3, Some(2.0)), (0, 4));
        // At 4.0, 3 repaired of 20 spends 3·5/20 = 0 points: all 5 are left.
        assert_eq!(mend(game, pickaxe, 3, Some(4.0)), (0, 5));
        pickaxe.take_asked();
        // Only the picked stack's part: the undamaged helmet's is never asked.
        assert!(helmet.take_asked().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn bad_ratios_never_break_the_repair() {
    with_parts(|game, pickaxe, _| {
        // Not finite: refused, vanilla's repair of 10.
        assert_eq!(mend(game, pickaxe, 100, Some(f32::NAN)), (90, 0));
        assert_eq!(mend(game, pickaxe, 100, Some(f32::INFINITY)), (90, 0));
        assert_eq!(mend(game, pickaxe, 100, Some(f32::NEG_INFINITY)), (90, 0));
        // Negative counts as 0: no damage added, the points left unspent.
        assert_eq!(mend(game, pickaxe, 100, Some(-1.0)), (100, 5));
        // Huge: i32::MAX, capped at the damage. 100·5/i32::MAX spends 0 points.
        assert_eq!(mend(game, pickaxe, 100, Some(1e30)), (0, 5));
        assert_eq!(mend(game, pickaxe, 100, Some(f32::MAX)), (0, 5));
    });
}

#[test]
#[ignore = "daily"]
fn give_experience_points_repairs_through_the_part() {
    with_parts(|game, pickaxe, _| {
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::ZERO,
            )
            .unwrap(),
        );
        crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        let player = game.player(123).unwrap();
        for (damage, answer, after, experience) in [
            (100, None, 90, 0),
            (100, Some(4.0), 80, 0),
            (100, Some(0.0), 100, 5),
            (3, Some(2.0), 0, 4),
        ] {
            hold_pickaxe(game, damage);
            pickaxe.answer(answer);
            game.experience = Default::default();
            assert!(game.player_give_experience_points(player, 5));
            assert_eq!(pickaxe_damage(game), after);
            assert_eq!(game.experience.total, experience);
        }
        assert_eq!(pickaxe.take_asked()[0], (5, 2.0));
    });
}

/// A data pack's Mending reaches the part as its arriving ratio: multiplying
/// by 0.3, 7 points repair floor(2.1) = 2, so the ratio arrives as 2/7, and a
/// part that writes it back unchanged still repairs exactly 2.
#[test]
#[ignore = "daily"]
fn a_data_packs_mending_arrives_as_the_ratio() {
    with_parts(|game, pickaxe, _| {
        let root = std::env::temp_dir().join(format!("horizon-xp-repair-{}", std::process::id()));
        let pack = root.join("datapacks/mending");
        let definitions = pack.join("data/minecraft/enchantment");
        std::fs::create_dir_all(&definitions).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"xp_repair_ratio fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["file/mending"],"disabled":[]}"#,
        )
        .unwrap();
        let mending = serde_json::json!({
            "anvil_cost": 4,
            "description": {"translate": "enchantment.minecraft.mending"},
            "effects": {"minecraft:repair_with_xp": [
                {"effect": {"type": "minecraft:multiply", "factor": 0.3}}
            ]},
            "max_cost": {"base": 75, "per_level_above_first": 25},
            "max_level": 1,
            "min_cost": {"base": 25, "per_level_above_first": 25},
            "slots": ["any"],
            "supported_items": "#minecraft:enchantable/durability",
            "weight": 2
        });
        std::fs::write(
            definitions.join("mending.json"),
            serde_json::to_vec(&mending).unwrap(),
        )
        .unwrap();
        game.world_root = root.clone();
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let resources = game.options.resources.clone();
        game.world_tables = game
            .stage_world_tables(&stack, false, &resources)
            .unwrap()
            .0;

        for answer in [None, Some(2.0_f32 / 7.0)] {
            hold_pickaxe(game, 100);
            pickaxe.answer(answer);
            game.mending(7);
            assert_eq!(pickaxe_damage(game), 98);
            assert_eq!(pickaxe.take_asked(), [(7, 2.0 / 7.0)]);
        }
        let _ = std::fs::remove_dir_all(root);
    });
}
