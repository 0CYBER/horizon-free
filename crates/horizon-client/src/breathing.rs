//! Breathing for native mods: `LivingEntity.baseTick`'s air supply, the part
//! that raises or lowers it. A player (`motion/environment.rs`) and a mob
//! (`mobs/ai.rs`) work out what vanilla does this tick, `breathe` lets the
//! mods change it (`native_living_breathe`, NeoForge's `LivingBreatheEvent`),
//! and the caller applies the answer. Servers only: the callers keep it off a
//! client's prediction.

use horizon_world::native_load_context::LoopContext;

/// One tick of breathing: whether the entity can breathe, how much air it
/// loses when it cannot and how much it gains when it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Breath {
    pub can_breathe: bool,
    /// `decreaseAirSupply`: 1, or 0 by the oxygen bonus's chance, for an
    /// entity that drowns; 1 for one that would not.
    pub consume: i32,
    /// `increaseAirSupply`: 4 for one that can breathe, the whole maximum for a
    /// dolphin (whatever its air, hence less it), 0 for one that drowns or is
    /// held under by the nautilus' breath.
    pub refill: i32,
}

/// An entity's breathing this tick (`LivingBreatheEvent`, posted from
/// `LivingEntity.baseTick` every tick for a living entity with the generic
/// air logic): `entity` is its ID and `kind` its type (`minecraft:player`,
/// `minecraft:zombie`). `can_breathe`, `consume` and `refill` come in as
/// Horizon does it today (a mod-free game changes nothing); set them to
/// change this tick. With `can_breathe` the air rises by `refill`, up to its
/// maximum (300; 4800 for a dolphin); without, it falls by `consume`, and at -20
/// it is set to 0 and the entity takes 2 drowning damage. Negative amounts count
/// as 0.
/// Asked on servers for players and mobs. Water animals, axolotls and
/// nautiluses come in with their own air logic's answer ([`own_air`]).
#[inline(never)]
pub fn native_living_breathe(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    can_breathe: &mut bool,
    consume: &mut i32,
    refill: &mut i32,
) {
    crate::native_client_hooks::living_breathe(context, entity, kind, can_breathe, consume, refill);
}

/// `vanilla`'s breathing after the mods have had their say.
pub fn breathe(context: &LoopContext, entity: u32, kind: &str, vanilla: Breath) -> Breath {
    let mut now = vanilla;
    native_living_breathe(
        context,
        entity,
        kind,
        &mut now.can_breathe,
        &mut now.consume,
        &mut now.refill,
    );
    Breath {
        can_breathe: now.can_breathe,
        consume: now.consume.max(0),
        refill: now.refill.max(0),
    }
}

/// An entity that can't breathe and has no air (`LivingDrownEvent`, posted
/// from `LivingEntity.baseTick`), with vanilla's answers as the defaults: it
/// drowns at -20 air, for 2 damage and 8 bubbles. Eight bubbles are the
/// vanilla entity event; another positive count is sent as that many bubble
/// particles, and none sends nothing. Asked on servers, for the entities
/// `native_living_breathe` is; water animals, axolotls and nautiluses show no
/// bubbles, as in vanilla.
#[inline(never)]
pub fn native_living_drown(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    is_drowning: &mut bool,
    damage_amount: &mut f32,
    bubble_count: &mut i32,
    cancel: &mut bool,
) {
    crate::native_client_hooks::living_drown(
        context,
        entity,
        kind,
        is_drowning,
        damage_amount,
        bubble_count,
        cancel,
    );
}

/// The most bubbles a mod's `bubble_count` draws and sends: a larger count
/// is clamped to it, a negative one to none.
pub const MAX_DROWN_BUBBLES: i32 = 64;

/// A drowning tick as the mods left it.
pub struct Drowning {
    pub damage: Option<f32>,
    /// `LivingDrownEvent.getBubbleCount`: 8 unless a mod changed it, within
    /// 0 to [`MAX_DROWN_BUBBLES`].
    pub bubbles: i32,
}

/// `breath` applied to `air` up to `max` (`getMaxAirSupply`: 300, 4800 for a
/// dolphin) (`apply`), then `LivingEntity.baseTick`'s drowning, which the mods
/// may change when `server` (`native_living_drown`). An entity that drowns is set
/// to 0 air.
pub fn drown(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    air: &mut i32,
    max: i32,
    breath: Breath,
    server: bool,
) -> Option<Drowning> {
    let (next, drowned) = apply(*air, max, breath);
    *air = if drowned {
        air.saturating_sub(breath.consume)
    } else {
        next
    };
    let (mut drowning, mut damage, mut bubbles, mut cancel) = (drowned, 2.0, 8, false);
    if server && !breath.can_breathe && *air <= 0 {
        native_living_drown(
            context,
            entity,
            kind,
            &mut drowning,
            &mut damage,
            &mut bubbles,
            &mut cancel,
        );
    }
    if cancel || !drowning {
        return None;
    }
    *air = 0;
    Some(Drowning {
        damage: (damage > 0.0).then_some(damage),
        bubbles: bubbles.clamp(0, MAX_DROWN_BUBBLES),
    })
}

/// `WaterAnimal`, `Axolotl` and `Nautilus.handleAirSupply` on a server, which
/// replace `LivingEntity.baseTick`'s air: where the entity `breathes` (water,
/// or rain for an axolotl) its air is full (`max`), elsewhere it loses 1, and
/// at -20 it is set to 0 and takes 2 damage (the caller's type: drowning, or
/// drying out). The mods are asked as for any entity (`native_living_breathe`
/// with those answers, then `native_living_drown`); the damage they leave
/// comes back. These entities show no drowning bubbles, so a `bubble_count`
/// a mod sets for them is not used. All of them drown at -20 air or less
/// (`shouldTakeDrowningDamage`). Air out of range (a refill from below 0) ends
/// as vanilla leaves it while the mods keep vanilla's answer.
pub fn own_air(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    air: &mut i32,
    breathes: bool,
    max: i32,
) -> Option<f32> {
    let vanilla = Breath {
        can_breathe: breathes,
        consume: 1,
        refill: max,
    };
    let breath = breathe(context, entity, kind, vanilla);
    if breath.can_breathe && breath.refill == max {
        // `setAirSupply(getMaxAirSupply())`: full whatever the air was.
        *air = max;
        return None;
    }
    drown(context, entity, kind, air, max, breath, true)?.damage
}

/// The air supply after `breath`: raised up to `max` when it can breathe,
/// lowered otherwise. The second is whether the entity now drowns
/// (`LivingEntity.shouldTakeDrowningDamage`: -20 or less), and its air is
/// then 0. A mod's amounts saturate rather than overflow.
pub fn apply(air: i32, max: i32, breath: Breath) -> (i32, bool) {
    if breath.can_breathe {
        return (air.max(air.saturating_add(breath.refill).min(max)), false);
    }
    let air = air.saturating_sub(breath.consume);
    if air <= -20 { (0, true) } else { (air, false) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA: Breath = Breath {
        can_breathe: true,
        consume: 1,
        refill: 4,
    };

    #[test]
    fn air_refills_up_to_the_maximum() {
        assert_eq!(apply(100, 300, VANILLA), (104, false));
        assert_eq!(apply(299, 300, VANILLA), (300, false));
        assert_eq!(apply(300, 300, VANILLA), (300, false));
        // A refill of 0 leaves it, as does one that would pass the maximum.
        let held = Breath {
            refill: 0,
            ..VANILLA
        };
        assert_eq!(apply(120, 300, held), (120, false));
    }

    #[test]
    fn air_runs_down_and_drowns_at_minus_twenty() {
        let drowning = Breath {
            can_breathe: false,
            consume: 1,
            refill: 0,
        };
        assert_eq!(apply(300, 300, drowning), (299, false));
        assert_eq!(apply(-18, 300, drowning), (-19, false));
        assert_eq!(apply(-19, 300, drowning), (0, true));
        // The oxygen bonus can spare a tick, and a mod can take more.
        let spared = Breath {
            consume: 0,
            ..drowning
        };
        assert_eq!(apply(-19, 300, spared), (-19, false));
        let gas = Breath {
            consume: 30,
            ..drowning
        };
        assert_eq!(apply(20, 300, gas), (-10, false));
        assert_eq!(apply(10, 300, gas), (0, true));
    }

    #[test]
    fn nothing_changes_without_a_mod() {
        let context = crate::test_resources::server_loop_context();
        assert_eq!(breathe(context, 1, "minecraft:zombie", VANILLA), VANILLA);
        let negative = Breath {
            can_breathe: false,
            consume: -3,
            refill: -2,
        };
        // Mods only ever answer with their own values, so what comes back is clamped.
        assert_eq!(breathe(context, 1, "minecraft:player", negative).consume, 0);
        assert_eq!(breathe(context, 1, "minecraft:player", negative).refill, 0);
    }

    #[test]
    fn own_air_fills_where_it_breathes_and_runs_out_elsewhere() {
        let context = crate::test_resources::server_loop_context();
        let mut air = 5;
        assert_eq!(
            own_air(context, 1, "minecraft:cod", &mut air, true, 300),
            None
        );
        assert_eq!(air, 300);
        assert_eq!(
            own_air(context, 1, "minecraft:cod", &mut air, false, 300),
            None
        );
        assert_eq!(air, 299);
        air = -18;
        assert_eq!(
            own_air(context, 1, "minecraft:cod", &mut air, false, 300),
            None
        );
        assert_eq!(air, -19);
        // At -20 it is set to 0 and takes 2 damage.
        assert_eq!(
            own_air(context, 1, "minecraft:cod", &mut air, false, 300),
            Some(2.0)
        );
        assert_eq!(air, 0);
        // A drowning player shows vanilla's eight bubbles without a mod.
        let mut player_air = -19;
        let breath = Breath {
            can_breathe: false,
            consume: 1,
            refill: 0,
        };
        let d = drown(
            context,
            3,
            "minecraft:player",
            &mut player_air,
            300,
            breath,
            true,
        )
        .unwrap();
        assert_eq!((d.damage, d.bubbles, player_air), (Some(2.0), 8, 0));
        // An axolotl's full air is its own.
        assert_eq!(
            own_air(context, 2, "minecraft:axolotl", &mut air, true, 6000),
            None
        );
        assert_eq!(air, 6000);
        // Out of range, without a mod, as before: entering water from -19
        // fills up, and a water animal or an axolotl below -20 still drowns or
        // dries out (`getAirSupply() <= -20`).
        air = -19;
        own_air(context, 1, "minecraft:cod", &mut air, true, 300);
        assert_eq!(air, 300);
        air = -19;
        own_air(context, 2, "minecraft:axolotl", &mut air, true, 6000);
        assert_eq!(air, 6000);
        air = -25;
        assert_eq!(
            own_air(context, 1, "minecraft:cod", &mut air, false, 300),
            Some(2.0)
        );
        assert_eq!(air, 0);
        air = -25;
        assert_eq!(
            own_air(context, 2, "minecraft:axolotl", &mut air, false, 6000),
            Some(2.0)
        );
        assert_eq!(air, 0);
    }
}
