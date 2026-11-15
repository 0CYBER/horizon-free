//! The held item's say on a player's blow and swing: `ItemPart::on_left_click_entity` and
//! `ItemPart::on_entity_swing` heard by a trait mod through the game's own paths.
use super::creatures::SpawnSite;
use super::native_entity_refs::tests::with_game;
use super::native_guest_turn_tests::with_guests;
use horizon_client::{
    first_person::{Hand, SwingAnimation},
    items::ItemStack,
    motion::PlayerMotion,
};
use horizon_core::geometry::Vec3;
use mod_api::{
    self, Flow, Hooks, HostSide, HurtEnemyEvent, ItemPart, OnEntitySwingEvent,
    OnLeftClickEntityEvent, Registrar, RegistrationContext, ServerHost,
};
use std::sync::{Arc, Mutex};

/// What the Parts heard, and what they answer.
#[derive(Default)]
struct Heard {
    cancel_attack: bool,
    cancel_swing: bool,
    /// Each blow asked about: the player, the target, its kind and the item held.
    attacks: Vec<(u128, i32, String, String)>,
    /// Each hit that landed (`hurt_enemy`): the target and its kind.
    hits: Vec<(u32, String)>,
    /// Each swing asked about: the swinging entity, the hand and the item in it.
    swings: Vec<(Option<i32>, Hand, String)>,
}

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Heard>>);

fn item(stack: &mod_api::ItemStack) -> String {
    mod_api::__private::item_key_id(stack.item())
        .unwrap()
        .to_owned()
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for item in ["minecraft:iron_sword", "minecraft:stick"] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
}
#[mod_api::horizon_part]
impl ItemPart for Probe {
    fn on_left_click_entity(
        &self,
        _: &mut dyn ServerHost,
        event: &mut OnLeftClickEntityEvent<'_>,
    ) -> Flow {
        let mut heard = self.0.lock().unwrap();
        heard.attacks.push((
            event.player,
            event.entity,
            event.kind.to_owned(),
            item(event.stack),
        ));
        if heard.cancel_attack {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn hurt_enemy(&self, _: &mut dyn ServerHost, event: &mut HurtEnemyEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .hits
            .push((event.entity, event.kind.to_owned()));
    }
    fn on_entity_swing(&self, _: HostSide<'_>, event: &mut OnEntitySwingEvent<'_>) -> Flow {
        let mut heard = self.0.lock().unwrap();
        heard
            .swings
            .push((event.entity, event.hand, item(event.stack)));
        if heard.cancel_swing {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn with_probe(probe: &Probe, test: impl FnOnce()) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "attack_probe",
        Box::new(probe.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, test);
}

/// Guest 12's held stack.
fn held(game: &super::Game<'_>) -> ItemStack {
    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
    guest.inventory[guest.selected].clone().unwrap()
}

fn hold(game: &mut super::Game<'_>, item: &str) {
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest.inventory[guest.selected] = Some(ItemStack::new(item, 1));
        })
        .unwrap();
}

fn health(game: &super::Game<'_>, id: u32) -> f32 {
    game.mobs.iter().find(|m| m.id == id).unwrap().health
}

/// A guest's blow on a mob goes `guest_attack` → `attack_entity` → the held item's Part: a
/// cancel leaves the mob unhurt, `hurt_enemy` silent and the sword unworn, while an item
/// without the Part, or a Part that lets the blow through, hits as before.
#[test]
#[ignore = "daily"]
fn a_part_cancels_the_blow_before_it_lands() {
    let probe = Probe::default();
    with_probe(&probe, || {
        with_guests(|game| {
            let zombie = game
                .spawn_mob("minecraft:zombie", [3.5, 64.0, 6.5], true, SpawnSite::Raw)
                .unwrap();
            let other = game
                .spawn_mob("minecraft:zombie", [2.0, 64.0, 6.0], true, SpawnSite::Raw)
                .unwrap();
            let gi = game.guest_index(12).unwrap();
            let full = health(game, zombie);

            // The Part cancels: the target keeps its health, no hit lands and the
            // sword takes no wear.
            probe.0.lock().unwrap().cancel_attack = true;
            hold(game, "minecraft:iron_sword");
            assert!(!game.guest_attack(12, gi, zombie as i32).unwrap());
            assert_eq!(health(game, zombie), full);
            assert_eq!(held(game).damage, 0);
            {
                let heard = probe.0.lock().unwrap();
                assert_eq!(
                    heard.attacks,
                    [(
                        12,
                        zombie as i32,
                        "minecraft:zombie".to_owned(),
                        "minecraft:iron_sword".to_owned()
                    )]
                );
                assert!(heard.hits.is_empty(), "heard {:?}", heard.hits);
            }

            // An item without the Part hits as before, and the Part is not asked.
            hold(game, "minecraft:stone_sword");
            assert!(game.guest_attack(12, gi, other as i32).unwrap());
            assert!(health(game, other) < full);
            assert_eq!(probe.0.lock().unwrap().attacks.len(), 1);

            // The Part lets the blow through: the hit lands, `hurt_enemy` hears it
            // and the sword wears.
            probe.0.lock().unwrap().cancel_attack = false;
            hold(game, "minecraft:iron_sword");
            assert!(game.guest_attack(12, gi, zombie as i32).unwrap());
            assert!(health(game, zombie) < full);
            assert_eq!(held(game).damage, 1);
            let heard = probe.0.lock().unwrap();
            assert_eq!(heard.attacks.len(), 2);
            assert_eq!(heard.hits, [(zombie, "minecraft:zombie".to_owned())]);
        });
    });
}

/// `swing_hand` asks the item in that hand first: a cancel starts no swing, a pass swings as
/// before, and a hand holding an item without the Part is not asked.
#[test]
#[ignore = "daily"]
fn a_part_keeps_the_swing_from_starting() {
    let probe = Probe::default();
    with_probe(&probe, || {
        with_game(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
            game.offhand = Some(ItemStack::new("minecraft:stone", 1));

            // Not canceled: the swing starts as before.
            game.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
            assert_eq!(game.swings_started, 1);
            assert!(game.swing.current.is_some());
            assert_eq!(
                probe.0.lock().unwrap().swings,
                [(Some(7), Hand::Main, "minecraft:stick".to_owned())]
            );

            // Canceled: no swing starts.
            game.swing = Default::default();
            game.swings_started = 0;
            probe.0.lock().unwrap().cancel_swing = true;
            game.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
            assert_eq!(game.swings_started, 0);
            assert!(game.swing.current.is_none());
            assert_eq!(probe.0.lock().unwrap().swings.len(), 2);

            // The off hand holds an item without the Part: it swings, unasked.
            game.swing_hand(Hand::Off, SwingAnimation::DEFAULT);
            assert_eq!(game.swings_started, 1);
            assert_eq!(probe.0.lock().unwrap().swings.len(), 2);
        });
    });
}
