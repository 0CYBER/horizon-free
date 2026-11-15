//! `living_shield_block` and `living_use_totem` heard by a trait mod through the game's own
//! hooks and the hit's level host: when each fires in a hit, and what a mod may change.
use super::entity::damage_level_of;
use super::native_entity_refs::tests::with_game;
use horizon_client::mobs::{Damage, DamageKind, Mob};
use horizon_core::{geometry::Vec3, random::RandomSource};
use mod_api::{
    self, Hand, Hooks, LevelRandomHost, LivingDamagePostEvent, LivingShieldBlockEvent,
    LivingUseTotemEvent,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Heard {
    /// Each hook in the order the hit fired it.
    order: Vec<&'static str>,
    /// The shield hook's entity, damage and whether vanilla blocked.
    shield: Vec<(i32, f32, bool)>,
    totem: Vec<(i32, Hand, String)>,
    /// What the shield hook sets: `blocked`, `blocked_damage`.
    set_block: Option<(bool, f32)>,
    cancel_totem: bool,
    random_draw: bool,
}

struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_shield_block(
        &self,
        level: &mut dyn LevelRandomHost,
        event: &mut LivingShieldBlockEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard.order.push("shield");
        heard
            .shield
            .push((event.entity, event.damage, event.original_blocked));
        if let Some((blocked, damage)) = heard.set_block {
            event.block.blocked = blocked;
            event.block.blocked_damage = damage;
        }
        if heard.random_draw {
            level.with_random(|random| random.next_int());
        }
    }
    fn living_damage_post(&self, _: &mut dyn LevelRandomHost, _: &mut LivingDamagePostEvent<'_>) {
        self.0.lock().unwrap().order.push("post");
    }
    fn living_use_totem(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingUseTotemEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.order.push("totem");
        heard
            .totem
            .push((event.entity, event.hand, event.item.to_string()));
        if heard.cancel_totem {
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}

fn heard(setup: Heard, test: impl FnOnce(&mut super::Game<'_>)) -> Heard {
    let log = Arc::new(Mutex::new(setup));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(test));
    std::mem::take(&mut *log.lock().unwrap())
}

fn zombie(game: &super::Game<'_>, id: u32) -> Mob {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

#[test]
#[ignore = "daily"]
fn the_shield_hook_sees_the_hit_before_armor_and_enchantment_reductions() {
    let heard = heard(Heard::default(), |game| {
        let mut hit = zombie(game, 226001);
        // A mob's attack: its armor takes a share of the 10 later, in `actuallyHurt`.
        assert!(hit.damage(
            damage_level_of!(game),
            10.0,
            Damage::mob(226002, [2.0, 0.0, 2.0])
        ));
        assert!(hit.health < 20.0);
    });
    assert_eq!(heard.shield, [(226001, 10.0, false)]);
}

#[test]
#[ignore = "daily"]
fn a_blocked_damage_above_the_hit_is_clamped_to_it() {
    let setup = Heard {
        set_block: Some((true, 100.0)),
        ..Heard::default()
    };
    let heard = heard(setup, |game| {
        let mut hit = zombie(game, 226003);
        let before = hit.health;
        hit.damage(damage_level_of!(game), 6.0, Damage::of(DamageKind::Generic));
        // All 6 are stopped, no more: the zombie lost nothing.
        assert_eq!(hit.health, before);
    });
    assert_eq!(heard.shield, [(226003, 6.0, false)]);
}

#[test]
#[ignore = "daily"]
fn setting_blocked_false_blocks_nothing() {
    let setup = Heard {
        set_block: Some((false, 6.0)),
        ..Heard::default()
    };
    let heard = heard(setup, |game| {
        let mut hit = zombie(game, 226004);
        let before = hit.health;
        hit.damage(damage_level_of!(game), 6.0, Damage::of(DamageKind::Generic));
        assert!(hit.health < before);
    });
    assert_eq!(heard.shield.len(), 1);
}

#[test]
#[ignore = "daily"]
fn the_totem_hook_fires_after_the_health_is_gone_and_a_cancel_lets_the_mob_die() {
    let setup = Heard {
        cancel_totem: true,
        ..Heard::default()
    };
    let heard = heard(setup, |game| {
        let mut hit = zombie(game, 226005);
        hit.extra
            .set_main_hand(Some("minecraft:totem_of_undying".into()));
        hit.damage(
            damage_level_of!(game),
            1000.0,
            Damage::of(DamageKind::Generic),
        );
        assert_eq!(hit.health, 0.0);
    });
    assert_eq!(
        heard.totem,
        [(226005, Hand::Main, "minecraft:totem_of_undying".to_string())]
    );
    // The totem is asked first, and Post closes `hurtServer`.
    assert_eq!(heard.order, ["shield", "totem", "post"]);
}

#[test]
#[ignore = "daily"]
fn an_uncanceled_totem_saves_the_mob() {
    let heard = heard(Heard::default(), |game| {
        let mut hit = zombie(game, 226006);
        hit.extra
            .set_main_hand(Some("minecraft:totem_of_undying".into()));
        hit.damage(
            damage_level_of!(game),
            1000.0,
            Damage::of(DamageKind::Generic),
        );
        assert_eq!(hit.health, 1.0);
        assert!(hit.extra.main_hand_stack().is_none());
    });
    assert_eq!(heard.totem.len(), 1);
}

/// The hit's level is the one the hit draws from: a mod's draw moves the world's random on by
/// exactly one place.
#[test]
#[ignore = "daily"]
fn the_shield_hook_draws_from_the_hits_random() {
    let after = |random_draw| {
        let next = Mutex::new(0);
        heard(
            Heard {
                random_draw,
                ..Heard::default()
            },
            |game| {
                *game.world.random = horizon_core::random::LegacyRandom::new(5);
                let mut hit = zombie(game, 226007);
                hit.damage(damage_level_of!(game), 1.0, Damage::of(DamageKind::Generic));
                *next.lock().unwrap() = game.world.random.next_int();
            },
        );
        next.into_inner().unwrap()
    };
    // The world's random is one stream: the mod's draw puts the value after the hit one
    // place further on than it is without the mod.
    let without = after(false);
    let mut stream = horizon_core::random::LegacyRandom::new(5);
    let place = (0..64).find(|_| stream.next_int() == without).unwrap();
    assert_eq!(after(true), stream.next_int(), "after {place} draws");
}
