//! `is_piglin_currency` answered by item Parts on vanilla items. A "coin", an emerald carrying a
//! `minecraft:custom_data` component, is currency and a plain emerald isn't, so the part's
//! answer depends on the stack's components: the coin must reach the off hand and the end of the
//! admiring whole, as vanilla's `holdInOffhand(stack.split(1))` keeps it. A gold ingot is refused
//! as currency: no longer taken from the hand, and once picked up from the ground (it stays
//! `#piglin_loved`) admired and kept instead of bartered. Expected values follow vanilla's
//! `PiglinAi` flow by hand; the world is ticked for real, `ADMIRE_DURATION` (119) ticks.
use super::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use horizon_client::first_person::Hand;
use horizon_client::items::ItemStack;
use horizon_client::mobs::{Mob, Species, piglin};
use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_core::position::ChunkPos;
use mod_api::{Hooks, Host, IsPiglinCurrencyEvent, ItemPart, Registrar, RegistrationContext};
use std::sync::Arc;

/// An emerald is currency when it carries `minecraft:custom_data`.
#[derive(Clone)]
struct Coin;

#[mod_api::horizon_part]
impl ItemPart for Coin {
    fn is_piglin_currency(&self, _: &dyn Host, event: &mut IsPiglinCurrencyEvent<'_>) {
        event.currency = event.stack.components().is_some_and(|c| {
            c.as_nbt()
                .contains_key(&JavaString::from("minecraft:custom_data"))
        });
    }
}

/// A gold ingot is not currency.
#[derive(Clone)]
struct NotCurrency;

#[mod_api::horizon_part]
impl ItemPart for NotCurrency {
    fn is_piglin_currency(&self, _: &dyn Host, event: &mut IsPiglinCurrencyEvent<'_>) {
        event.currency = false;
    }
}

struct CurrencyMod;

#[mod_api::__subscriptions]
impl Hooks for CurrencyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let emerald = r.item_key("minecraft:emerald");
        r.item_part(emerald, Coin);
        let gold = r.item_key("minecraft:gold_ingot");
        r.item_part(gold, NotCurrency);
    }
}

const EMERALD: &str = "minecraft:emerald";
const GOLD_INGOT: &str = "minecraft:gold_ingot";
const GOLDEN_SWORD: &str = "minecraft:golden_sword";

/// `count` emeralds carrying `minecraft:custom_data={coin:1b}`.
fn coins(count: u32) -> ItemStack {
    horizon_items::item_data::parse_stack(&format!(
        r#"{{id:"minecraft:emerald",count:{count},components:{{"minecraft:custom_data":{{coin:1b}}}}}}"#
    ))
    .unwrap()
    .unwrap()
}

/// The `minecraft:custom_data` the coins carry.
fn coin_data() -> Compound {
    Compound::from([(JavaString::from("coin"), Tag::Byte(1))])
}

/// A serving level, frozen and stepped by hand, with a stone floor at y 99 and survival rules.
fn field(game: &mut super::Game<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
    for command in [
        "tick freeze",
        "forceload add -32 -32 31 31",
        "fill -31 99 -31 31 99 31 stone",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game.creative = false;
}

fn step(game: &mut super::Game<'_>, ticks: usize) {
    for _ in 0..ticks {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
}

/// An adult (or `baby`) piglin at `(x, 100, z)` that never zombifies, holding `main` in its main
/// hand; its id.
fn add_piglin(game: &mut super::Game<'_>, x: f64, z: f64, main: Option<&str>, baby: bool) -> u32 {
    let before: Vec<u32> = game.mobs.iter().map(|m| m.id).collect();
    let baby = if baby { ",IsBaby:1b" } else { "" };
    let command = format!(
        "summon minecraft:piglin {x} 100 {z} {{PersistenceRequired:1b,IsImmuneToZombification:1b{baby}}}"
    );
    assert!(game.run_command(&command), "{command}");
    let m = game
        .mobs
        .iter_mut()
        .find(|m| m.species() == Species::Piglin && !before.contains(&m.id))
        .unwrap();
    // `finalizeSpawn` may arm it: the test chooses what it holds.
    m.extra.set_main_hand(main.map(Into::into));
    m.id
}

fn mob<'a>(game: &'a super::Game<'_>, id: u32) -> &'a Mob {
    game.mobs.iter().find(|m| m.id == id).unwrap()
}

fn index(game: &super::Game<'_>, id: u32) -> usize {
    game.mobs.iter().position(|m| m.id == id).unwrap()
}

/// Right click on the piglin `id` holding `held`; whether the click was taken.
fn click(game: &mut super::Game<'_>, id: u32, held: ItemStack) -> bool {
    game.inventory[game.selected] = Some(held);
    game.interact_mob(Some(crate::game::InteractTarget {
        id: id as i32,
        location: [0.0; 3],
    }))
    .unwrap()
}

/// A player's hit stops the admiring: `ai::after_hurt` and `piglin_hurt` both stop it, and the
/// game applies what they raised with the next tick's events.
fn hit(game: &mut super::Game<'_>, id: u32) {
    let i = index(game, id);
    piglin::stop_admiring(&mut game.mobs[i]);
    piglin::stop_admiring(&mut game.mobs[i]);
    step(game, 1);
}

/// Drop one `stack` right beside the piglin `id`, ready to be picked up, and tick once.
fn drop_beside(game: &mut super::Game<'_>, id: u32, stack: ItemStack) {
    let at = mob(game, id).position;
    game.spawn_item(stack, [at.x + 0.5, at.y, at.z], [0.0; 3], 0);
    step(game, 1);
}

fn stored(m: &Mob, item: &str) -> u32 {
    m.extra
        .inventory
        .iter()
        .filter(|s| &*s.item == item)
        .map(|s| s.count)
        .sum()
}

fn off_hand(game: &super::Game<'_>, id: u32) -> Option<ItemStack> {
    horizon_client::mobs::finalize::item_stack(
        mob(game, id),
        horizon_client::mobs::finalize::OFFHAND,
    )
}

/// Thrown barter items (no other items are on the ground in these tests).
fn thrown(game: &super::Game<'_>) -> usize {
    game.dropped.len()
}

#[test]
#[ignore = "daily"]
fn a_part_makes_a_coin_currency_and_refuses_the_gold_ingot() {
    crate::game::in_own_process(
        "game::native_is_piglin_currency_tests::a_part_makes_a_coin_currency_and_refuses_the_gold_ingot",
        a_part_makes_a_coin_currency_and_refuses_the_gold_ingot_isolated,
    );
}

fn a_part_makes_a_coin_currency_and_refuses_the_gold_ingot_isolated() {
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("currency", Box::new(CurrencyMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            field(game);

            // A plain emerald isn't currency: the click passes and the player keeps 3.
            let p = add_piglin(game, -24.5, -24.5, None, false);
            assert!(!click(game, p, ItemStack::new(EMERALD, 3)));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 3);
            assert_eq!(off_hand(game, p), None);

            // Hand-over of a coin: the click is taken, one coin, data and all, leaves the hand for
            // the off hand. It is admired for 119 ticks, then bartered: gone, barter thrown.
            assert!(click(game, p, coins(3)));
            let held = game.inventory[game.selected].as_ref().unwrap();
            assert_eq!((&*held.item, held.count), (EMERALD, 2));
            let admired = off_hand(game, p).unwrap();
            assert_eq!((&*admired.item, admired.count), (EMERALD, 1));
            assert_eq!(admired.custom_data(), Some(&coin_data()));
            step(game, 118);
            assert_eq!(
                off_hand(game, p).map(|s| s.custom_data().cloned()),
                Some(Some(coin_data()))
            );
            assert_eq!(thrown(game), 0);
            step(game, 1);
            assert_eq!(off_hand(game, p), None);
            assert_eq!(mob(game, p).extra.main_hand_id(), None);
            assert_eq!(stored(mob(game, p), EMERALD), 0);
            assert!(thrown(game) > 0, "the barter is thrown at the 119th tick");
            game.dropped.clear();

            // A hit: an adult admiring a coin loses it, nothing thrown or stored. Were the coin
            // asked about without its data, it would be kept instead.
            let p = add_piglin(game, 24.5, -24.5, None, false);
            assert!(click(game, p, coins(1)));
            hit(game, p);
            assert_eq!(off_hand(game, p), None);
            assert_eq!(mob(game, p).extra.main_hand_id(), None);
            assert_eq!(stored(mob(game, p), EMERALD), 0);
            assert_eq!(thrown(game), 0);

            // The client's prediction: an adult that isn't admiring takes the coin, not a plain
            // emerald; a baby takes neither.
            let adult = add_piglin(game, -24.5, 24.5, None, false);
            let baby = add_piglin(game, -20.5, 24.5, None, true);
            game.inventory[game.selected] = Some(coins(1));
            assert!(game.remote_mob_would_take(index(game, adult), Hand::Main));
            assert!(!game.remote_mob_would_take(index(game, baby), Hand::Main));
            game.inventory[game.selected] = Some(ItemStack::new(EMERALD, 1));
            assert!(!game.remote_mob_would_take(index(game, adult), Hand::Main));

            // Refusing gold: a piglin with a golden sword doesn't take the ingot from the hand.
            let p = add_piglin(game, 24.5, 24.5, Some(GOLDEN_SWORD), false);
            assert!(!click(game, p, ItemStack::new(GOLD_INGOT, 1)));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 1);
            assert_eq!(off_hand(game, p), None);
            // A dropped ingot is still picked up and admired (`#piglin_loved`), then kept: the
            // sword stays in the main hand, the ingot goes to the inventory, nothing bartered.
            drop_beside(game, p, ItemStack::new(GOLD_INGOT, 1));
            assert_eq!(off_hand(game, p).map(|s| s.item), Some(GOLD_INGOT.into()));
            step(game, 119);
            assert_eq!(off_hand(game, p), None);
            assert_eq!(mob(game, p).extra.main_hand_id(), Some(GOLDEN_SWORD));
            assert_eq!(stored(mob(game, p), GOLD_INGOT), 1);
            assert_eq!(thrown(game), 0, "no barter");

            // Ground currency needs `#piglin_loved`: a coin is picked up without admiring, and
            // a wanted sword isn't replaced by it, so it goes to the inventory, data and all.
            let p = add_piglin(game, 0.5, 24.5, Some(GOLDEN_SWORD), false);
            drop_beside(game, p, coins(1));
            assert_eq!(off_hand(game, p), None);
            assert_eq!(mob(game, p).extra.main_hand_id(), Some(GOLDEN_SWORD));
            let kept = &mob(game, p).extra.inventory;
            assert_eq!(kept.len(), 1);
            assert_eq!((&*kept[0].item, kept[0].count), (EMERALD, 1));
            assert_eq!(kept[0].custom_data(), Some(&coin_data()));
            assert_eq!(thrown(game), 0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn without_parts_only_the_gold_ingot_is_currency() {
    crate::game::in_own_process(
        "game::native_is_piglin_currency_tests::without_parts_only_the_gold_ingot_is_currency",
        without_parts_only_the_gold_ingot_is_currency_isolated,
    );
}

fn without_parts_only_the_gold_ingot_is_currency_isolated() {
    with_game(|game| {
        field(game);
        // A coin is just an emerald: the click passes and the player keeps 3.
        let p = add_piglin(game, -24.5, -24.5, None, false);
        assert!(!click(game, p, coins(3)));
        assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 3);
        assert_eq!(off_hand(game, p), None);
        game.inventory[game.selected] = Some(coins(1));
        assert!(!game.remote_mob_would_take(index(game, p), Hand::Main));

        // A gold ingot is taken, and lost to a hit: an adult keeps nothing.
        let p = add_piglin(game, 24.5, -24.5, None, false);
        assert!(click(game, p, ItemStack::new(GOLD_INGOT, 1)));
        assert_eq!(game.inventory[game.selected], None);
        hit(game, p);
        assert_eq!(off_hand(game, p), None);
        assert_eq!(mob(game, p).extra.main_hand_id(), None);
        assert_eq!(stored(mob(game, p), GOLD_INGOT), 0);
        assert_eq!(thrown(game), 0);

        // An admired golden sword picked up from the ground keeps its components, and a hit
        // that stops the admiring puts that same sword in the empty main hand.
        let p = add_piglin(game, -24.5, 24.5, None, false);
        let blade = horizon_items::item_data::parse_stack(
            r#"{id:"minecraft:golden_sword",count:1,components:{"minecraft:custom_data":{coin:1b}}}"#,
        )
        .unwrap()
        .unwrap();
        drop_beside(game, p, blade);
        let admired = off_hand(game, p).unwrap();
        assert_eq!(&*admired.item, GOLDEN_SWORD);
        assert_eq!(admired.custom_data(), Some(&coin_data()));
        hit(game, p);
        assert_eq!(off_hand(game, p), None);
        let i = index(game, p);
        let main = horizon_client::mobs::finalize::item_stack(&game.mobs[i], 0).unwrap();
        assert_eq!(&*main.item, GOLDEN_SWORD);
        assert_eq!(main.custom_data(), Some(&coin_data()));
        assert_eq!(thrown(game), 0);

        // A baby admiring a picked-up ingot keeps it when hit (`stopHoldingOffHandItem`'s
        // baby branch): it holds it in its main hand.
        let p = add_piglin(game, 24.5, 24.5, None, true);
        drop_beside(game, p, ItemStack::new(GOLD_INGOT, 1));
        assert_eq!(off_hand(game, p).map(|s| s.item), Some(GOLD_INGOT.into()));
        hit(game, p);
        assert_eq!(off_hand(game, p), None);
        assert_eq!(mob(game, p).extra.main_hand_id(), Some(GOLD_INGOT));
    });
}

/// A content mod whose only file adds `minecraft:emerald` to `#minecraft:piglin_loved`; the
/// resource root it merges into the prepared resources.
fn loved_emerald_root(scratch: &std::path::Path) -> std::path::PathBuf {
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let dir = scratch.join("loved");
    for (name, text) in [
        (
            "horizon-mod.toml",
            "id = \"loved\"\nname = \"loved\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\nnamespaces = []\n",
        ),
        (
            "data/minecraft/tags/item/piglin_loved.json",
            r#"{"values": ["minecraft:emerald"]}"#,
        ),
    ] {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let package = horizon_mod_package::Package::read(&dir).unwrap();
    let loved = horizon_client::mod_content::ContentMod {
        id: package.manifest.id.clone(),
        content: horizon_mod_package::content::Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    let inventory =
        horizon_mod_package::enabled::EnabledMods::new([("loved".to_owned(), mod_api::Side::Both)]);
    horizon_client::mod_content::resource_root(
        &base,
        std::path::Path::new(""),
        &[loved],
        &scratch.join("cache"),
        &inventory,
    )
    .unwrap()
}

#[test]
#[ignore = "daily"]
fn a_loved_coin_from_the_ground_is_admired_and_bartered() {
    crate::game::in_own_process(
        "game::native_is_piglin_currency_tests::a_loved_coin_from_the_ground_is_admired_and_bartered",
        a_loved_coin_from_the_ground_is_admired_and_bartered_isolated,
    );
}

fn a_loved_coin_from_the_ground_is_admired_and_bartered_isolated() {
    let scratch = std::env::temp_dir().join(format!("horizon-loved-coin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let root = loved_emerald_root(&scratch);
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("currency", Box::new(CurrencyMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        super::native_entity_refs::tests::with_game_at(root, |game| {
            field(game);
            // With the emerald `#piglin_loved` through the content mod's tag file, a coin from
            // the ground is admired, data and all, and bartered after 119 ticks; the wanted
            // sword stays in the main hand.
            let p = add_piglin(game, 0.5, 0.5, Some(GOLDEN_SWORD), false);
            drop_beside(game, p, coins(1));
            let admired = off_hand(game, p).unwrap();
            assert_eq!((&*admired.item, admired.count), (EMERALD, 1));
            assert_eq!(admired.custom_data(), Some(&coin_data()));
            step(game, 118);
            assert!(off_hand(game, p).is_some());
            step(game, 1);
            assert_eq!(off_hand(game, p), None);
            assert_eq!(mob(game, p).extra.main_hand_id(), Some(GOLDEN_SWORD));
            assert_eq!(stored(mob(game, p), EMERALD), 0);
            assert!(thrown(game) > 0, "the barter is thrown");
        });
    });
    let _ = std::fs::remove_dir_all(&scratch);
}
