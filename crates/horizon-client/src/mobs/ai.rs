//! The per-tick mob update in vanilla order (`Entity.baseTick`,
//! `LivingEntity.baseTick/tick/aiStep`, `Mob.serverAiStep`) and the
//! dispatch to each species family's goals and special behaviour.
pub(crate) use freezing::in_powder_snow;
pub use freezing::{can_freeze, wears_gaze_disguise};
mod feeding;
mod freezing;
#[cfg(test)]
mod sink_tests;

pub use feeding::takes_food;

use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who, aquatic,
    control::{BodyRotation, LookControl, MoveControl},
    farm,
    goals::{Cx, GoalSelector},
    hostile,
    nav::Navigation,
    path::{PATH_TYPES, Path},
    push::{Cap, Verdict, root_vehicle},
    undead,
};
use crate::blocks::Fluid;
use crate::breathing::{Breath, breathe, drown, own_air};
use crate::first_person::SwingAnimation;
use crate::health::DamageLevel;
use crate::powder_snow::particle_velocity as powder_snow_particle_velocity;
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    random::RandomSource,
};
use horizon_world::native_load_context::LoopContext;

#[cfg(test)]
mod inside_walk_tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    /// What the test's fire does, as a part's callback would: it queues a push
    /// and a wither effect, and can kill or panic on later contacts. The
    /// mod_api dispatch hop this stands for is the player's, the item's and the
    /// the projectile's tests' to prove; the mob tests here prove the walk, its
    /// arms, the flush and the containment.
    /// The module's fire reads process-wide statics: one test at a time.
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static CONTACTS: Mutex<Vec<[i32; 3]>> = Mutex::new(Vec::new());
    static KILL_AFTER: AtomicUsize = AtomicUsize::new(usize::MAX);
    static PANIC_AFTER: AtomicUsize = AtomicUsize::new(usize::MAX);
    static ENABLED: AtomicBool = AtomicBool::new(false);

    fn reset(kill: usize, panic: usize, enabled: bool) {
        mod_api::__private::resolve_effects(["minecraft:wither"], |_| Some(0)).unwrap();
        CONTACTS.lock().unwrap().clear();
        KILL_AFTER.store(kill, Ordering::SeqCst);
        PANIC_AFTER.store(panic, Ordering::SeqCst);
        ENABLED.store(enabled, Ordering::SeqCst);
    }

    /// What the fire's callback does, as a part's would.
    fn fire_like_a_part(at: horizon_core::position::BlockPos, staged: &mut mod_api::InsideEffects) {
        let mut contacts = CONTACTS.lock().unwrap();
        contacts.push([at.x, at.y, at.z]);
        let seen = contacts.len();
        drop(contacts);
        if seen > PANIC_AFTER.load(Ordering::SeqCst) {
            panic!("an entity_inside part panics");
        }
        if seen > KILL_AFTER.load(Ordering::SeqCst) {
            staged.run_after(
                mod_api::InsideEffect::Freeze,
                mod_api::EntityWrite::Hurt {
                    source: None,
                    amount: 1000.0,
                },
            );
        }
        staged.run_after(
            mod_api::InsideEffect::Freeze,
            mod_api::EntityWrite::Push([0.5, 0.0, 0.0]),
        );
        staged.run_after(
            mod_api::InsideEffect::Freeze,
            mod_api::EntityWrite::AddEffect {
                effect: mod_api::__private::effect_key("minecraft:wither")
                    .expect("an effect key resolves"),
                duration: 40,
                amplifier: 0,
            },
        );
    }

    fn install_walk_hooks() {
        let _ = crate::block_contact::HOOKS.set(crate::block_contact::Hooks {
            step_on: |_, _| {},
            any_inside: || ENABLED.load(Ordering::SeqCst),
            inside_listens: |_| ENABLED.load(Ordering::SeqCst),
            pass_listens: |_| false,
            fall_listens: |_| false,
            bounce_listens: |_| false,
            fall_on: |_, _, _, _, _, _| {},
            bounce_on: |_, _, _, _, _| {},
            // The production binding's own shape: the whole callback staged,
            // a panic contained with it.
            inside_fire: |_level, _dimension, contact, _precise, _swept, staged, _writes| {
                let fired = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fire_like_a_part(contact.position, staged);
                }));
                let _ = fired;
            },
        });
    }

    /// The tests' mob: a zombie whose move crosses two stone blocks.
    fn mob_at_move(types: &super::super::MobTypes) -> Mob {
        let kind = Arc::new(types.types["minecraft:zombie"].clone());
        let mut mob = Mob::new(1, kind, Vec3::new(2.5, 64.0, 1.5), 0.0);
        mob.previous = Vec3::new(0.5, 64.0, 1.5);
        mob
    }

    /// The fire's pushes and its wither effect reach the mob in its own tick,
    /// one contact a block, in the shared walk's order.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_a_mob_once_a_block_in_walk_order() {
        let _test = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset(usize::MAX, usize::MAX, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = mob.velocity.x;
                crate::block_contact::in_level(Some("minecraft:overworld"), || {
                    effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                });
                assert_eq!(
                    *CONTACTS.lock().unwrap(),
                    vec![[1, 64, 1], [2, 64, 1]],
                    "one contact a block, in the walk's order"
                );
                assert!(
                    (mob.velocity.x - before - 1.0).abs() < 1e-9,
                    "both pushes reached the mob"
                );
                assert!(
                    mob.extra
                        .effects
                        .iter()
                        .any(|effect| effect.id == "minecraft:wither" && effect.duration == 40),
                    "the wither effect reached the mob's own list"
                );
                assert!(crate::block_contact::take_pending().is_empty());
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// A killing write stops the mob's flush: the walk still reaches both
    /// blocks, the writes after the death never apply.
    #[test]
    #[ignore = "daily"]
    fn a_killing_write_stops_the_mobs_flush() {
        let _test = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset(0, usize::MAX, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = mob.velocity.x;
                crate::block_contact::in_level(Some("minecraft:overworld"), || {
                    effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                });
                assert_eq!(CONTACTS.lock().unwrap().len(), 2);
                assert!(!mob.alive(), "the killing write applied");
                assert!(
                    (mob.velocity.x - before).abs() < 1e-9,
                    "the writes after the death never apply"
                );
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// A panicking part is contained mid-walk: the mob's tick answers, its
    /// state keeps what it had, and the level host still answers after (the
    /// lifted rows themselves are arismoko/horizon#2285's restoration, its own tests'; the
    /// item batch test proves the lifted pair end to end).
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_mobs_tick_restored() {
        let _test = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset(usize::MAX, 0, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = mob.velocity.x;
                crate::block_contact::in_level(Some("minecraft:overworld"), || {
                    effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                });
                assert_eq!(
                    CONTACTS.lock().unwrap().len(),
                    2,
                    "both contacts reached the fire; each panic was contained"
                );
                assert!(mob.alive(), "the mob's tick answered");
                assert!(
                    (mob.velocity.x - before).abs() < 1e-9,
                    "the panicking callback's staged writes never applied"
                );
                assert!(
                    level.level().random().next_int_bound(2).is_ok(),
                    "the level host still answers"
                );
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// Without a listening part the walk runs its vanilla moments only:
    /// nothing of the part plumbing applies, exactly what main did with no
    /// listener.
    #[test]
    #[ignore = "daily"]
    fn a_mob_without_a_part_keeps_vanilla_behavior() {
        let _test = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset(usize::MAX, usize::MAX, false);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = (mob.velocity.x, mob.health, mob.extra.effects.len());
                crate::block_contact::in_level(Some("minecraft:overworld"), || {
                    effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                });
                assert_eq!(
                    (mob.velocity.x, mob.health, mob.extra.effects.len()),
                    before,
                    "no writes without a part"
                );
                assert!(CONTACTS.lock().unwrap().is_empty());
                assert!(crate::block_contact::take_pending().is_empty());
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// The walk's block order on a diagonal move is vanilla's own sweep: the
    /// x cells of the move come before the z cells the box enters.
    #[test]
    #[ignore = "daily"]
    fn a_diagonal_move_walks_blocks_in_vanillas_order() {
        let _test = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset(usize::MAX, usize::MAX, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                // A diagonal path: +x crosses [1] and [2], then +z crosses the
                // cell ahead.
                world.set(1, 64, 1, stone).unwrap();
                world.set(2, 64, 1, stone).unwrap();
                world.set(2, 64, 2, stone).unwrap();
                install_walk_hooks();
            },
            |w, types| {
                let kind = Arc::new(types.types["minecraft:zombie"].clone());
                let mut mob = Mob::new(1, kind, Vec3::new(2.5, 64.0, 2.5), 0.0);
                mob.previous = Vec3::new(0.5, 64.0, 0.5);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                crate::block_contact::in_level(Some("minecraft:overworld"), || {
                    effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                });
                let contacts = CONTACTS.lock().unwrap().clone();
                let x_first: Vec<_> = contacts.iter().filter(|p| p[2] == 1).collect();
                let z_last: Vec<_> = contacts.iter().filter(|p| p[2] == 2).collect();
                assert_eq!(
                    x_first.len(),
                    2,
                    "the x cells of the move come first: {contacts:?}"
                );
                assert!(
                    contacts.iter().position(|p| p[2] == 2)
                        > contacts.iter().rposition(|p| p[2] == 1),
                    "the z cell the box enters comes after the x cells: {contacts:?}"
                );
                let _ = z_last;
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }
}
#[cfg(test)]
#[path = "fall_flying_tests.rs"]
mod fall_flying_tests;
#[path = "ai_glide.rs"]
mod glide;
use glide::update_fall_flying;
#[cfg(test)]
#[path = "cramming_tests.rs"]
mod cramming_tests;
#[cfg(test)]
#[path = "wet_fire_tests.rs"]
mod wet_fire_tests;

mod step;
pub(super) use step::inherited_step_sound;

/// The inside-block effects of fire and lava (`InsideBlockEffectType`
/// FIRE_IGNITE and LAVA_IGNITE): ignite, then hurt.
fn block_contact_damage(
    m: &mut Mob,
    _w: &MobWorld,
    level: &mut DamageLevel<'_>,
    fire: f32,
    lava: bool,
) {
    if fire > 0.0 && !m.kind.flags.fire_immune {
        let multiplier = m.burning_time();
        m.fire_ticks = crate::motion::environment::ignite_contact(
            m.fire_ticks,
            false,
            &mut m.random,
            multiplier,
        );
        m.damage(level, fire, Damage::of(DamageKind::InFire));
    }
    if lava && !m.kind.flags.fire_immune {
        m.ignite_for_ticks(300);
        m.damage(level, 4.0, Damage::of(DamageKind::Lava));
    }
}

#[derive(Clone, Debug, Default)]
pub struct Ai {
    /// `Mob.homePosition` and `homeRadius`: where a restricted mob stays
    /// (a lead's holder, a happy ghast's float area).
    pub home: Option<([i32; 3], i32)>,
    /// Mob keeps homePosition even when clearHome makes homeRadius -1.
    /// Active homes still use `home`; this only retains its hidden position.
    last_home_position: [i32; 3],
    pub goals: GoalSelector,
    pub targets: GoalSelector,
    pub nav: Navigation,
    pub(super) navigation_redirect: Option<Box<super::nav::Redirect>>,
    pub mov: MoveControl,
    pub look: LookControl,
    pub body: BodyRotation,
    /// JumpControl.jump (requested this tick).
    pub jump: bool,
    /// LivingEntity.jumping
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub xxa: f32,
    pub yya: f32,
    pub zza: f32,
    pub speed: f32,
    pub target: Option<Who>,
    pub last_hurt_by: Option<Who>,
    pub last_hurt_by_time: i32,
    /// The bare UUID of an unresolved `lastHurtByMob` (see `hurt_by`).
    pub last_hurt_by_ref: Option<[i32; 4]>,
    /// Ticks left during which kills count as player kills.
    pub last_hurt_by_player: i32,
    pub no_action_time: i32,
    pub ai_paused: bool, // A mod paused serverAiStep (`MobRef::set_ai_paused`): mobs/ai_pause.rs.
    pub dab_turns: u32,
    pub aggressive: bool,
    pub panicking: bool,
    pub malus: [Option<f32>; PATH_TYPES],
    /// A path computed in canUse and consumed by start.
    pub pending_path: Option<Path>,
    /// Sensing: line-of-sight results cached for one tick.
    pub seen_cache: Vec<(Who, bool)>,
    /// Swing animation progress (attackAnim) and ticks.
    pub swing_time: i32,
    pub swinging: bool,
    /// The current swing's `SwingDescription.durationTicks`; 0 is the default 6.
    pub swing_duration: i32,
    /// Walked distance and the next footstep.
    pub steps: super::living::Steps,
}
impl Ai {
    /// `Mob.getHomePosition`, including the position retained by clearHome.
    pub fn home_position(&self) -> [i32; 3] {
        self.home.map_or(self.last_home_position, |(pos, _)| pos)
    }
    /// Adopt only the two persistent home fields after loading Mob data.
    pub fn restore_home(&mut self, pos: [i32; 3], radius: i32) {
        self.last_home_position = pos;
        self.home = (radius != -1).then_some((pos, radius));
    }
    /// `Mob.clearHome`: clear the radius, retaining the position.
    pub fn clear_home(&mut self) {
        self.last_home_position = self.home_position();
        self.home = None;
    }
    /// `Mob.isWithinRestriction(pos)`: an unrestricted mob is within
    /// everywhere.
    pub fn is_within_home(&self, p: [i32; 3]) -> bool {
        self.home
            .is_none_or(|(c, r)| super::goals::within_home(c, r, p))
    }
}
impl Mob {
    /// `LivingEntity.getCurrentSwingDuration`: 6 ticks unless a swing set its own.
    pub fn swing_duration(&self) -> i32 {
        if self.ai.swing_duration > 0 {
            self.ai.swing_duration
        } else {
            6
        }
    }
    /// `SwingState.startIfAble`: a swing inside the first half of the current
    /// one is refused. `swing_time` counts one behind `SwingState.ticks` (a
    /// swing starts at -1 and `tick_swing` counts it up at the end of the
    /// mob's tick, where vanilla's `SwingState.tick` runs at the start of
    /// `baseTick`), so vanilla's `ticks > 0 && ticks <= durationTicks / 2` is
    /// `swing_time >= 0 && swing_time < durationTicks / 2`.
    fn start_swing(&mut self, animation: SwingAnimation, duration: i32) -> bool {
        let half = self.swing_duration() / 2;
        let ticks = self.ai.swing_time + 1;
        if self.ai.swinging && ticks > 0 && ticks <= half {
            return false;
        }
        self.ai.swing_time = -1;
        self.ai.swinging = true;
        self.ai.swing_duration = duration.max(1);
        self.anim.swing_animation = animation;
        true
    }
    /// Whether the current swing is the off hand's (`swing_hand`'s arm
    /// inverted); a swing that named no hand is the main hand's.
    pub fn swing_is_off_hand(&self) -> bool {
        self.extra
            .swing_right_arm
            .is_some_and(|right| right == self.main_arm_left())
    }
    /// LivingEntity.swing
    pub fn swing(&mut self) {
        if self.start_swing(SwingAnimation::DEFAULT, 6) {
            self.extra.swing_right_arm = None;
            self.extra.swing_stab = false;
        }
    }
    /// `LivingEntity.swing(hand, animation, false)` for a swing of `duration`
    /// ticks (`getModifiedSwingDuration`): the arm is `hand.asArm(mainArm)`.
    pub fn swing_hand(&mut self, off_hand: bool, duration: i32) {
        if self.start_swing(SwingAnimation::DEFAULT, duration) {
            self.extra.swing_right_arm = Some(!off_hand != self.main_arm_left());
            self.extra.swing_stab = false;
        }
    }
    /// `swing_hand` for a swing of `animation`, lengthened or shortened by
    /// this mob's haste, conduit power or mining fatigue
    /// (`LivingEntity.getModifiedSwingDuration`). Whether the swing started
    /// (`SwingState.startIfAble`).
    pub fn swing_hand_modified(&mut self, off_hand: bool, animation: SwingAnimation) -> bool {
        let amplifier = |id: &str| {
            self.extra
                .effects
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.amplifier)
        };
        let dig_speed = amplifier("minecraft:haste").max(amplifier("minecraft:conduit_power"));
        let fatigue = amplifier("minecraft:mining_fatigue");
        let duration = crate::first_person::swing_duration(animation, dig_speed, fatigue);
        if self.start_swing(animation, duration) {
            self.extra.swing_right_arm = Some(!off_hand != self.main_arm_left());
            self.extra.swing_stab = animation.kind == 2;
            return true;
        }
        false
    }
}
/// The module implementing a species' behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Farm,
    Undead,
    Hostile,
    Aquatic,
    Generic,
    /// No goals at all (Giant).
    Inert,
    Witch,
    Enderman,
    Tame,
    /// Blazes and ghasts.
    Nether,
    Golem,
    Illager,
    /// Piglins, brutes, hoglins and zoglins.
    Piglin,
    Equine,
    Phantom,
    /// Slimes and magma cubes.
    Cube,
    Guardian,
    Shulker,
    Vex,
    Ravager,
    Wither,
    Allay,
    Villager,
    Trader,
    Breeze,
    Sniffer,
    HappyGhast,
    Warden,
    Creaking,
    /// Nautili and zombie nautili.
    Nautilus,
    Strider,
    Bat,
    Bee,
    Panda,
    PolarBear,
}
pub fn family(s: Species) -> Family {
    use Species as S;
    match s {
        S::Pig | S::Cow | S::Mooshroom | S::Sheep | S::Chicken => Family::Farm,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::Parched
        | S::WitherSkeleton
        | S::ZombifiedPiglin => Family::Undead,
        S::Creeper | S::Spider | S::CaveSpider | S::Silverfish | S::Endermite => Family::Hostile,
        S::Squid
        | S::GlowSquid
        | S::Cod
        | S::Tadpole
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Dolphin
        | S::Axolotl => Family::Aquatic,
        S::Giant => Family::Inert,
        S::Witch => Family::Witch,
        S::Wolf | S::Cat => Family::Tame,
        S::Blaze | S::Ghast => Family::Nether,
        S::IronGolem | S::SnowGolem | S::CopperGolem => Family::Golem,
        S::Pillager | S::Vindicator | S::Evoker | S::Illusioner => Family::Illager,
        S::Vex => Family::Vex,
        S::Ravager => Family::Ravager,
        S::Wither => Family::Wither,
        S::Allay => Family::Allay,
        S::Creaking => Family::Creaking,
        S::Villager => Family::Villager,
        S::WanderingTrader => Family::Trader,
        S::Breeze => Family::Breeze,
        S::Sniffer => Family::Sniffer,
        S::HappyGhast => Family::HappyGhast,
        S::Piglin | S::PiglinBrute | S::Hoglin | S::Zoglin => Family::Piglin,
        S::Horse
        | S::Donkey
        | S::Mule
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk
        | S::Llama
        | S::TraderLlama => Family::Equine,
        S::Nautilus | S::ZombieNautilus => Family::Nautilus,
        S::Phantom => Family::Phantom,
        S::Enderman => Family::Enderman,
        S::Slime | S::MagmaCube | S::SulfurCube => Family::Cube,
        S::Guardian | S::ElderGuardian => Family::Guardian,
        S::Warden => Family::Warden,
        S::Shulker => Family::Shulker,
        S::Strider => Family::Strider,
        S::Bat => Family::Bat,
        S::Bee => Family::Bee,
        S::Panda => Family::Panda,
        S::PolarBear => Family::PolarBear,
        _ => Family::Generic,
    }
}
/// The `setPathfindingMalus` calls of the vanilla constructors, base classes
/// first: the species' own register may still change them.
fn constructor_maluses(m: &mut Mob) {
    use super::path::PathType as P;
    use Species as S;
    let s = m.species();
    // `Animal()`: no walking through fire, and a wide berth around it.
    let animal = matches!(
        s,
        S::Armadillo
            | S::Axolotl
            | S::Bee
            | S::Camel
            | S::CamelHusk
            | S::Cat
            | S::Chicken
            | S::Cow
            | S::Donkey
            | S::Fox
            | S::Frog
            | S::Goat
            | S::HappyGhast
            | S::Hoglin
            | S::Horse
            | S::Llama
            | S::Mule
            | S::Mooshroom
            | S::Nautilus
            | S::Ocelot
            | S::Panda
            | S::Parrot
            | S::Pig
            | S::PolarBear
            | S::Rabbit
            | S::Sheep
            | S::SkeletonHorse
            | S::Sniffer
            | S::Strider
            | S::TraderLlama
            | S::Turtle
            | S::Wolf
            | S::ZombieHorse
            | S::ZombieNautilus
    );
    // `AbstractVillager()`, `AbstractPiglin()` and `CopperGolem()` say the same.
    if animal
        || matches!(
            s,
            S::Villager | S::WanderingTrader | S::Piglin | S::PiglinBrute | S::CopperGolem
        )
    {
        m.set_malus(P::FireInNeighbor, 16.0);
        m.set_malus(P::Fire, -1.0);
    }
    match s {
        S::Fox => {
            m.set_malus(P::DamagingInNeighbor, 0.0);
            m.set_malus(P::Damaging, 0.0);
        }
        S::Parrot => {
            m.set_malus(P::FireInNeighbor, -1.0);
            m.set_malus(P::Fire, -1.0);
            m.set_malus(P::Cocoa, -1.0);
        }
        S::Frog => {
            m.set_malus(P::Water, 4.0);
            m.set_malus(P::Trapdoor, -1.0);
        }
        S::Goat | S::Wolf => {
            m.set_malus(P::PowderSnow, -1.0);
            m.set_malus(P::OnTopOfPowderSnow, -1.0);
        }
        S::CopperGolem => m.set_malus(P::DamagingInNeighbor, 16.0),
        S::Blaze => {
            m.set_malus(P::Water, -1.0);
            m.set_malus(P::Lava, 8.0);
            m.set_malus(P::FireInNeighbor, 0.0);
            m.set_malus(P::Fire, 0.0);
        }
        S::ZombifiedPiglin | S::WitherSkeleton => m.set_malus(P::Lava, 8.0),
        _ => {}
    }
}
/// Brain-backed vanilla `getTarget` overrides. Piglins and hoglins use
/// Horizon's `ai.target` as the stand-in for their brain memory.
pub fn brain_target(mob: &Mob) -> Option<Option<Who>> {
    Some(match mob.species() {
        Species::Warden => super::warden::brain_target(mob),
        Species::Breeze | Species::Creaking => super::monster_brain::attack_target(mob),
        Species::Axolotl => mob
            .extra
            .animal_brain
            .as_ref()
            .and_then(|b| b.mem.attack_target.value()),
        Species::Frog => super::frog::attack_target(mob),
        Species::Piglin | Species::PiglinBrute | Species::Hoglin | Species::Zoglin => mob.ai.target,
        _ => return None,
    })
}

/// Whether `Brain.setMemory(ATTACK_TARGET, ...)` reaches what the mob's
/// brain acts on (arismoko/horizon#934): every species [`brain_target`] reads but the frog,
/// whose target belongs to its running tongue attack.
pub fn writes_brain_target(mob: &Mob) -> bool {
    match mob.species() {
        Species::Frog => false,
        Species::Axolotl => mob.extra.animal_brain.is_some(),
        _ => brain_target(mob).is_some(),
    }
}

/// `Brain.setMemory(ATTACK_TARGET, target)` / `eraseMemory` (arismoko/horizon#934): the
/// memory each brain keeps it in, non-expiring, with `ai.target` mirrored
/// where the brain mirrors it. No species setter side effects. False when
/// [`writes_brain_target`] is false.
pub fn set_brain_target(mob: &mut Mob, target: Option<Who>) -> bool {
    if !writes_brain_target(mob) {
        return false;
    }
    match mob.species() {
        Species::Warden => super::warden_brain::mem(mob).attack_target.set_opt(target),
        Species::Breeze | Species::Creaking => {
            super::monster_brain::set_attack_target(mob, target);
        }
        Species::Axolotl => {
            super::animal_brain::mem(mob).attack_target.set_opt(target);
            mob.ai.target = target;
        }
        // The piglin, piglin brute, hoglin and zoglin brains keep it in the
        // mob's own target slot (what `brain_target` reads).
        _ => mob.ai.target = target,
    }
    true
}

pub fn register(m: &mut Mob) {
    constructor_maluses(m);
    // Raider subclasses call super.registerGoals() first.
    super::raider::register(m);
    match family(m.species()) {
        Family::Farm => farm::register(m),
        Family::Undead => undead::register(m),
        Family::Hostile => hostile::register(m),
        Family::Aquatic => aquatic::register(m),
        // Turtle.registerGoals has its own list, none of the generic goals.
        Family::Generic if m.species() == Species::Turtle => super::turtle::register(m),
        Family::Generic if m.species() == Species::Armadillo => super::armadillo::register(m),
        Family::Generic if m.species() == Species::Frog => super::frog::register(m),
        Family::Generic if m.species() == Species::Fox => super::fox::register(m),
        Family::Generic if m.species() == Species::Parrot => super::parrot::register(m),
        Family::Generic if m.species() == Species::Rabbit => super::rabbit::register(m),
        Family::Generic if m.species() == Species::Ocelot => super::tame::register_ocelot(m),
        Family::Generic => {
            generic(m);
            if m.species() == Species::Frog {
                // `Frog()`: SmoothSwimmingMoveControl(85, 10, 0.02, 0.1, true).
                // The speed attribute is 1.0, so the controller's scaling is
                // what keeps a frog at a walking pace on land.
                m.ai.mov.kind = super::control::MoveKind::SmoothSwimming {
                    max_turn_x: 85.0,
                    max_turn_y: 10.0,
                    in_water: 0.02,
                    outside: 0.1,
                    gravity: true,
                };
            }
            if m.species() == Species::Goat {
                super::goat::register(m);
            }
        }
        Family::Inert => {}
        Family::Witch => super::witch::register(m),
        Family::Tame if m.species() == Species::Cat => super::tame::register_cat(m),
        Family::Tame => super::tame::register(m),
        Family::Nether if m.species() == Species::Ghast => super::nether::register_ghast(m),
        Family::Nether => super::nether::register_blaze(m),
        Family::Golem if m.species() == Species::IronGolem => super::golem::register_iron(m),
        Family::Golem if m.species() == Species::CopperGolem => super::copper_golem::register(m),
        Family::Golem => super::golem::register_snow(m),
        Family::Illager if m.species() == Species::Pillager => super::illager::register_pillager(m),
        Family::Illager if m.species() == Species::Evoker => super::illager::register_evoker(m),
        Family::Illager if m.species() == Species::Illusioner => super::illusioner::register(m),
        Family::Illager => super::illager::register_vindicator(m),
        Family::Equine => super::equine::register(m),
        Family::Phantom => super::phantom::register(m),
        Family::Piglin => match m.species() {
            Species::Piglin => super::piglin::register_piglin(m),
            Species::PiglinBrute => super::piglin::register_brute(m),
            Species::Hoglin => super::piglin::register_hoglin(m),
            _ => super::piglin::register_zoglin(m),
        },
        Family::Enderman => super::enderman::register(m),
        Family::Cube => super::cube::register(m),
        Family::Guardian => super::guardian::register(m),
        Family::Shulker => super::shulker::register(m),
        Family::Vex => super::vex::register(m),
        Family::Ravager => super::ravager::register(m),
        Family::Wither => super::wither::register(m),
        Family::Allay => super::allay::register(m),
        Family::Villager => super::villager::register(m),
        Family::Trader => super::trader::register(m),
        Family::Breeze => super::breeze::register(m),
        Family::Sniffer => super::sniffer::register(m),
        Family::HappyGhast => super::happy_ghast::register(m),
        Family::Warden => super::warden::register(m),
        Family::Creaking => super::creaking::register(m),
        Family::Nautilus => super::nautilus::register(m),
        Family::Strider => super::strider::register(m),
        Family::Bat => super::bat::register(m),
        Family::Bee => super::bee::register(m),
        Family::Panda => super::panda::register(m),
        Family::PolarBear => super::polar_bear::register(m),
    }
    // FloatGoal's constructor lets the navigation float, as do the
    // constructors of these brain-driven and flying mobs. A floater rises
    // 0.04 a tick in Mob.jumpInLiquid, the others 0.3 (the bounce of #542).
    if m.ai.goals.has("Float")
        || matches!(
            m.species(),
            Species::Allay
                | Species::Armadillo
                | Species::Camel
                | Species::Goat
                | Species::Parrot
                | Species::Wither
        )
    {
        m.ai.nav.can_float = true;
    }
}
/// Mob.tick additions that run before the entity tick (cube/creeper fuses, puffing).
fn pre_tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::SulfurCube {
        // SulfurCube.tick calls tickFuse before super.tick, outside serverAiStep.
        super::sulfur_cube::tick_fuse(m, events);
    }
    if m.species() == Species::Wither && m.alive() {
        super::wither::steer(m);
    }
    if m.species() == Species::Breeze {
        super::breeze::tick(m, w, events);
    }
    match family(m.species()) {
        Family::Hostile => {
            if m.species() == Species::Creeper {
                hostile::creeper_tick(m, events)
            }
        }
        Family::Aquatic => aquatic::tick(m, events),
        _ => {}
    }
}
/// Move controls implemented by the species (MoveKind::Custom).
pub fn custom_move_control(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Drowned {
        return undead::drowned_move_control(m, w);
    }
    if m.species() == Species::Ghast {
        return super::nether::ghast_move_control(m, w);
    }
    if m.species() == Species::HappyGhast {
        return super::happy_ghast::move_control(m, w);
    }
    if m.species() == Species::Phantom {
        return super::phantom::move_control(m);
    }
    if family(m.species()) == Family::Guardian {
        return super::guardian::move_control(m);
    }
    if m.species() == Species::Vex {
        return super::vex::move_control(m);
    }
    match family(m.species()) {
        Family::Aquatic => aquatic::fish_move_control(m),
        Family::Cube => super::cube::move_control(m, events),
        _ => {}
    }
}
/// A fallback for species without bespoke goals yet: animals stroll and
/// panic, monsters hunt the player in melee.
fn generic(m: &mut Mob) {
    use super::goals::*;
    let hostile = m.hostile();
    let g = &mut m.ai.goals;
    if hostile {
        g.add(1, Float);
        g.add(2, MeleeAttack::new(1.0, false));
        g.add(5, Stroll::avoiding_water(0.8));
        g.add(6, LookAtPlayer::new(8.0));
        g.add(6, RandomLookAround::default());
        m.ai.targets.add(1, HurtByTarget::new());
        m.ai.targets.add(2, NearestTarget::player(true));
    } else {
        if m.kind.species == Species::Goat {
            // GoatAi's idle and core behaviours (the ram and long jump
            // activities are not here): panic 2.0, AnimalMakeLove, tempting
            // with goat_food and baby follow at 1.25, then stroll.
            g.add(0, Float);
            g.add(1, AnimalPanic::new(2.0));
            g.add(2, Breed::new(1.0));
            g.add(3, Tempt::new(1.25, m.kind.food.clone(), false));
            g.add(4, FollowParent::new(1.25));
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(6.0));
            g.add(7, RandomLookAround::default());
            return;
        }
        g.add(0, Float);
        g.add(1, Panic::new(1.25));
        g.add(5, Stroll::avoiding_water(1.0));
        g.add(6, LookAtPlayer::new(6.0));
        g.add(7, RandomLookAround::default());
    }
}
/// Subclasses of `net.minecraft.world.entity.monster.Monster`.
pub fn is_monster(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Zombie
            | S::Husk
            | S::Drowned
            | S::ZombieVillager
            | S::ZombifiedPiglin
            | S::Giant
            | S::Skeleton
            | S::Stray
            | S::Bogged
            | S::Parched
            | S::WitherSkeleton
            | S::Creeper
            | S::Spider
            | S::CaveSpider
            | S::Enderman
            | S::Endermite
            | S::Silverfish
            | S::Witch
            | S::Pillager
            | S::Vindicator
            | S::Evoker
            | S::Illusioner
            | S::Ravager
            | S::Vex
            | S::Piglin
            | S::PiglinBrute
            | S::Zoglin
            | S::Blaze
            | S::Guardian
            | S::ElderGuardian
            | S::Breeze
            | S::Creaking
            | S::Warden
            | S::Wither
    )
}
/// `Entity.getSoundSource` for mobs: `Monster`'s own override is hostile, and
/// so are a few mobs that are not `Monster`s (`Hoglin`, `Ghast`, `Phantom`,
/// `Shulker` and the slimes). A killer bunny is hostile too; `Mob::sound_source`
/// adds it from the variant.
pub fn hostile_species(s: Species) -> bool {
    is_monster(s)
        || matches!(
            s,
            Species::Hoglin
                | Species::Ghast
                | Species::Phantom
                | Species::Shulker
                | Species::Slime
                | Species::MagmaCube
        )
}
/// One full entity tick. Returns false when the mob is gone.
/// `Mob.getTarget` / `Mob.setTarget` (`asValidTarget`): a player in creative
/// or spectator mode or in Peaceful is never a mob's target, however the target was set (an
/// alert from another mob included). A `NeutralMob` angry at that player stops
/// being angry (`NeutralMob.updatePersistentAnger`).
fn drop_invalid_player_target(m: &mut Mob, w: &MobWorld) {
    if m.ai.target == Some(Who::Player)
        && w.selected_player()
            .is_some_and(|p| p.creative || p.spectator || w.difficulty.base == 0)
    {
        m.set_ai_target(w, None);
        if matches!(m.species(), Species::ZombifiedPiglin | Species::IronGolem) {
            super::neutral_anger::stop(m);
            m.ai.forget_hurt_by(m.ai.last_hurt_by_time);
        }
    }
}
/// A tick either completed or paused at the fox's inline consumption boundary.
#[derive(Debug)]
pub enum TickProgress {
    Complete(bool),
    Consumption {
        decision: super::fox::Consumption,
        /// The base-tick snapshot, before contact ignition or listener writes.
        was_on_fire: bool,
    },
}

/// Run a whole mob tick when no owning-host pause is needed by the caller.
pub fn tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> bool {
    match tick_until_consumption(m, w, level, events) {
        TickProgress::Complete(kept) => kept,
        TickProgress::Consumption {
            decision,
            was_on_fire,
        } => {
            super::fox::apply_consumption(&decision, w, m, level, events);
            resume_after_consumption(m, w, level, was_on_fire, events)
        }
    }
}

/// Stop before a fox's effect draws so its owner can provide a mutable host.
pub fn tick_until_consumption(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    // ServerLevel.tick checks despawning before ticking the entity.
    if !check_despawn(m, w) {
        m.removed = true;
        return TickProgress::Complete(false);
    }
    // Entity.commonTick: `invulnerableTime` counts down before setOldPosAndRot.
    if m.invulnerable_time > 0 {
        m.invulnerable_time -= 1;
    }
    // `Entity.computeSpeed`: what it moved in the last tick.
    let known_speed = m.position.subtract(m.previous);
    m.previous = m.position;
    m.previous_yaw = m.yaw;
    m.previous_body_yaw = m.body_yaw;
    m.previous_head_yaw = m.head_yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_swell = m.swell;
    m.anim.begin_tick();
    super::camel::tick_age(m, w.game_time);
    m.ai.seen_cache.clear();
    if m.ai.last_hurt_by_player > 0 {
        m.ai.last_hurt_by_player -= 1;
    }
    drop_invalid_player_target(m, w);
    // Mob.tick pre-update hooks (Creeper.tick swells before super.tick).
    pre_tick(m, w, events);
    if m.removed {
        return TickProgress::Complete(false);
    }
    if m.species() == Species::ZombieVillager {
        // ZombieVillager.tick counts the cure down before super.tick, with
        // or without AI (NoAI only skips serverAiStep).
        super::zombie_villager::step(m, &mut Cx { w, level, events });
    }
    super::warden::pre_tick(m, w, events);
    if m.species() == Species::Strider {
        super::strider::pre_tick(m, w, events);
    }
    let was_on_fire = base_tick(m, w, level, events);
    if m.impulse_grace_time > 0 {
        m.impulse_grace_time -= 1;
    }
    if m.species() == Species::Rabbit {
        super::rabbit::base_tick(m);
    }
    if !m.alive() && m.death_time >= super::creaking::death_limit(m) {
        // Creaking.tickDeath: a heart's creaking crumbles away.
        if super::creaking::is_creaking(m) && super::creaking::tearing_down(m) {
            super::creaking::tear_down(m, events);
        }
        return TickProgress::Complete(false);
    }
    // LivingEntity.tick: updatingUsingItem, the stab of a spear in use.
    if m.alive() {
        super::spear_use::update_using_item(m, w, known_speed, events);
    }
    super::sulfur_cube::adopt_body(m);
    let sleep_answer = m
        .extra
        .villager
        .as_deref_mut()
        .and_then(|e| e.may_continue_sleeping.take());
    // LivingEntity.tick checks the bed even for mobs without AI.
    if super::villager::sleeping(m)
        && !sleep_answer.unwrap_or_else(|| {
            super::villager::sleeping_pos(m)
                .is_some_and(|p| w.block_name(p[0], p[1], p[2]).ends_with("_bed"))
        })
    {
        super::villager::stop_sleeping(m, w, events);
        super::villager_brain::woken(m, w.game_time);
    }
    // Fox.aiStep consumes before its super.aiStep, so the hook and its RNG
    // precede goals, travel and block effects at the resumed position.
    if m.species() == Species::Fox
        && !m.no_ai
        && let Some(decision) = super::fox::ai_step_until_consumption(m, w, events)
    {
        return TickProgress::Consumption {
            decision,
            was_on_fire,
        };
    }
    TickProgress::Complete(ai_and_finish_tick(m, w, level, was_on_fire, events))
}

/// Resume a fox after its item effect, before the living AI/movement step.
pub fn resume_after_consumption(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    was_on_fire: bool,
    events: &mut Vec<MobEvent>,
) -> bool {
    if m.removed {
        return false;
    }
    super::fox::before_living_ai_step(m, w);
    ai_and_finish_tick(m, w, level, was_on_fire, events)
}

fn ai_and_finish_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    was_on_fire: bool,
    events: &mut Vec<MobEvent>,
) -> bool {
    if m.no_ai {
        // Witch.aiStep picks and drinks its potions before super.aiStep():
        // LivingEntity.tick calls aiStep without AI too, so a NoAI witch
        // on fire still drinks fire resistance.
        witch_ai_step(m, w, events);
        update_fall_flying(m, w, events);
        reset_effect_fall_distance(m);
        // Mob.aiStep still runs without AI: undead burn, endermites age.
        burn_undead(m, w, events);
        if m.species() == Species::Endermite {
            super::hostile::endermite_lifetime(m);
        }
        // AbstractHorse.aiStep (it overrides LivingEntity.aiStep, which NoAI
        // doesn't skip) and AbstractHorse.tick: a NoAI horse still swishes
        // its tail before super.aiStep, then heals, grazes and runs its
        // stand counter and animations.
        let horse = super::equine::is_equine(m.species())
            && !super::camel::is_camel(m.species())
            && !super::llama::is_llama(m.species());
        if horse {
            super::equine::tail_tick(m);
        }
        // LivingEntity.aiStep: without AI there is no serverAiStep and no
        // travel, but the motion below 0.003 is still dropped and
        // pushEntities still runs, so a NoAI mob pushes what it overlaps
        // (and is pushed back by its own push).
        drop_tiny_motion(m);
        push_entities(m, w, level, events);
        ageable_step(m, w, level, events);
        animal_step(m, events);
        if m.species() == Species::Chicken {
            farm::chicken_egg_step(m, events);
        }
        if m.species() == Species::Allay {
            super::allay::passive_heal(m, w.loop_context);
            super::allay::update_duplication_cooldown(m);
        }
        if horse {
            super::equine::ai_step(m, w, events);
        }
        // LivingEntity.aiStep ends in the water's hurt with or without AI;
        // a NoAI enderman's or shulker's hurts still teleport it (hurtServer).
        hurt_by_water(m, w, level, events);
        hurt_teleports(m, w, events);
        // Ravager.aiStep runs outside serverAiStep: NoAI still advances
        // attack/stun/roar, including the stun-to-roar transition.
        if m.species() == Species::Ravager {
            super::ravager::ai_step(m, w, events);
        }
        if family(m.species()) == Family::Aquatic {
            aquatic::no_ai_step(m, w, events);
        }
        super::cube::no_ai_tick(m, w, events); // AbstractCubeMob.tick
        // Shulker.tick after its living tick: a NoAI shell still keeps a
        // face and eases its lid, pushing what the lid opens into.
        if m.species() == Species::Shulker {
            super::shulker::shell_tick(m, w, events);
        }
        // TraderLlama.aiStep still calls maybeDespawn after its super step.
        // Keep unrelated llama healing and its random draw in the AI path.
        if m.species() == Species::TraderLlama {
            super::caravan::trader_llama_tick(m, w);
        }
    } else {
        ai_step(m, w, level, events);
    }
    if m.no_ai && m.species() == Species::Fox {
        super::fox::finish_ai_step(m, w, events);
    }
    finish_tick(m, w, level, was_on_fire, events)
}

/// Resume the living tick after species AI, before block effects and animation.
/// Kept as one continuation so an inline action can regain its owning host and
/// then run the same remaining work, in the original order.
#[inline]
pub fn finish_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    was_on_fire: bool,
    events: &mut Vec<MobEvent>,
) -> bool {
    count_fall_fly_ticks(m);
    super::finalize::refresh_dirty_attributes(m); // LivingEntity.tick: refreshDirtyAttributes
    effects_from_blocks(m, w, level, events);
    if m.alive() {
        freeze_tick(m, w, level);
    }
    extinguish_if_wet(m, w, was_on_fire, events);
    m.reset_unlit_fire();
    if m.species() == Species::SulfurCube && m.alive() {
        // SulfurCube.doPush applies the body's contact damage even with NoAI.
        // Player contact keeps its existing player_push route.
        super::sulfur_cube::mob_contacts(m, w, events);
    }
    if m.species() == Species::Rabbit {
        super::rabbit::post_tick(m);
    }
    // Mob.tickHeadTurn -> BodyRotationControl.clientTick (a frozen creaking's waits).
    if m.alive()
        && (!super::creaking::is_creaking(m) || super::creaking::can_move(m))
        && !super::armadillo::scared(m)
    {
        m.body_rotation_tick();
    }
    events.append(&mut m.outbox);
    m.update_walk_animation();
    super::axolotl::tick_animation(m);
    m.anim.end_tick(m.ai.swinging);
    tick_swing(m);
    flower_contact_effects(m, w);
    // The species' own tick after the living tick, with or without AI.
    match m.species() {
        // CopperGolem.tick: updateWeathering.
        Species::CopperGolem => {
            super::copper_golem::update_weathering(m, w, level.random(), events)
        }
        Species::Strider => super::strider::float(m, w),
        Species::Bat => super::bat::tick(m),
        Species::Bee => super::bee::tick(m, level.random()),
        Species::Allay => super::allay::tick(m, w, events),
        Species::Panda => super::panda::tick(m, w, events),
        Species::Villager => super::villager::tick(m, w.game_time),
        // Vex.tick: gravity off on every tick, AI or not.
        Species::Vex => m.no_gravity = true,
        _ => {}
    }
    aquatic::school_tick(m, w, level.random());
    // Dolphin.tick: after super.tick, NoAI restores the species maximum.
    if m.no_ai && m.species() == Species::Dolphin {
        m.air = m.max_air_supply();
    }
    if m.species() == Species::Mannequin {
        m.extra.static_env_stamp = if m.is_static_mannequin() {
            let at = m.position;
            let [bx, by, bz] = m.block_on_legacy(w);
            let below = w.world.block(bx, by, bz);
            let mut inert = is_inert_static_mannequin_below(&w.facts.get(below).name);
            if inert {
                let body = m.bounds_at(at).deflate(Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
                let lo = body.min_position();
                let hi = body.max_position();
                'body_scan: for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                    for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                        for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                            let state = w.world.block(x, y, z);
                            if !is_inert_static_mannequin_inside(&w.facts.get(state).name) {
                                inert = false;
                                break 'body_scan;
                            }
                        }
                    }
                }
            }
            if inert && mob_in_wall(m, w) {
                inert = false;
            }
            inert.then_some((
                w.world.revision,
                [at.x.to_bits(), at.y.to_bits(), at.z.to_bits()],
                [m.width().to_bits(), m.height().to_bits()],
            ))
        } else {
            None
        };
    }
    // TraderLlama.maybeDespawn discards the llama in its aiStep: it leaves the
    // level's entity list in this tick, after the rest of its tick ran.
    !(m.removed && m.species() == Species::TraderLlama)
}

/// `Entity.isInWall`: whether a suffocating block collision shape intersects the mob's eye box.
fn mob_in_wall(m: &Mob, w: &MobWorld) -> bool {
    let mut scene = crate::collisions::SceneCollisions {
        world: w.world,
        binding: w.collision,
        subject: None,
    };
    in_wall(
        m.width(),
        [m.position.x, m.eye_y(), m.position.z],
        |x, y, z, eye_bounds| {
            let p = [x, y, z];
            let state = w.world.block(x, y, z);
            if !w
                .suffocation
                .is_suffocating(&mut scene, state, p)
                .unwrap_or(false)
            {
                return false;
            }
            w.collision
                .collision_shape(&mut scene, state, p)
                .ok()
                .is_some_and(|shape| {
                    let Ok(eye_shape) = VoxelShape::from_box(*eye_bounds) else {
                        return false;
                    };
                    shape
                        .moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)))
                        .join_is_not_empty(&eye_shape, ShapeOp::AND)
                        .unwrap_or(false)
                })
        },
    )
}

/// Returns `true` when the supporting block under a static mannequin is a vanilla block with no
/// `step_on` hazard or mod callback that requires running the full tick.
fn is_inert_static_mannequin_below(name: &str) -> bool {
    name.starts_with("minecraft:")
        && !matches!(
            name,
            "minecraft:turtle_egg"
                | "minecraft:magma_block"
                | "minecraft:cactus"
                | "minecraft:sweet_berry_bush"
                | "minecraft:campfire"
                | "minecraft:soul_campfire"
                | "minecraft:pointed_dripstone"
                | "minecraft:wither_rose"
        )
}

/// Returns `true` when a block overlapping a static mannequin's bounding box is a vanilla block
/// with no `entity_inside` hazard or mod callback that requires running the full tick.
fn is_inert_static_mannequin_inside(name: &str) -> bool {
    name.starts_with("minecraft:")
        && !matches!(
            name,
            "minecraft:cactus"
                | "minecraft:sweet_berry_bush"
                | "minecraft:wither_rose"
                | "minecraft:fire"
                | "minecraft:soul_fire"
                | "minecraft:powder_snow"
                | "minecraft:cobweb"
                | "minecraft:honey_block"
        )
}

/// Lightweight tick for an immovable, invulnerable mannequin verified stationary in an inert
/// environment at `(world_revision, m.position, [width, height])`. Falls back to a full tick
/// automatically on any world edit (`world_revision` change), any teleport (position bit change
/// even when `previous` matches `position`), any pose/scale box change, or any flag, velocity,
/// or effect change.
pub fn tick_static_mannequin(m: &mut Mob, world_revision: u64, bottom_y: i32) -> bool {
    let stamp = (
        world_revision,
        [
            m.position.x.to_bits(),
            m.position.y.to_bits(),
            m.position.z.to_bits(),
        ],
        [m.width().to_bits(), m.height().to_bits()],
    );
    if m.extra.static_env_stamp != Some(stamp)
        || !m.is_static_mannequin()
        || m.position.y < f64::from(bottom_y - 64)
    {
        return false;
    }
    m.prepare_tick();
    m.age += 1;
    m.previous = m.position;
    m.previous_yaw = m.yaw;
    m.previous_body_yaw = m.body_yaw;
    m.previous_head_yaw = m.head_yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_swell = m.swell;
    m.anim.begin_tick();
    super::finalize::refresh_dirty_attributes(m);
    m.body_rotation_tick();
    m.update_walk_animation();
    m.anim.end_tick(false);
    tick_swing(m);
    true
}
/// `LivingEntity.aiStep` -> `Entity.applyEffectsFromBlocks` for the blocks
/// that hurt, hold or slide a mob: the cactus (`CactusBlock.entityInside`, 1),
/// the cobweb (stuck at 0.25, 0.05, 0.25), the sweet berry bush
/// (`SweetBerryBushBlock.entityInside`: stuck at 0.8, 0.75, 0.8, and a grown bush pricks for 1 when the mob moved at least 0.003 on X
/// or Z this tick) and the honey block (`honey_slide`). The blocks are those
/// vanilla's own sweep reaches along the mob's move this tick, on the shared
/// inside walk; a part's contact fires mid-walk there.
pub(crate) fn effects_from_blocks(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    // Entity.applyEffectsFromBlocks fixes the tick's movements before stepOn:
    // a hurt below that teleports an enderman or a shulker leaves the scan on
    // the path it had. A stepOn teleport (snapTo) has rewritten the mob's
    // live fields by the time the walk below reads them, so the walk reads
    // these.
    let (tick_from, tick_to) = (m.previous, m.position);
    // Block.stepOn for the block under a mob on the ground (`getOnPosLegacy`):
    // MagmaBlock.stepOn's hot floor, 1 damage (a mob never sneaks).
    if m.on_ground && m.alive() {
        // 0.2 below the feet, or the supporting block when the centre is over
        // its edge.
        let [x, y, z] = m.block_on_legacy(w);
        let state = w.world.block(x, y, z);
        let name: &str = &w.facts.get(state).name;
        crate::block_contact::step_on(
            name,
            crate::block_contact::Contact {
                entity: m.id as i32,
                kind: &m.kind.id,
                state,
                position: horizon_core::position::BlockPos::new(x, y, z),
            },
        );
        if name == "minecraft:magma_block" {
            m.damage(level, 1.0, Damage::of(DamageKind::HotFloor));
            // Enderman.hurtServer finishes before stepOn returns; the scan
            // below still uses the movements captured before that hurt.
            hurt_teleports(m, w, events);
        }
        // TurtleEggBlock.stepOn: a mob never steps carefully, so an egg under
        // it breaks one time in a hundred (`destroyEgg`), unless it is a turtle
        // or a bat or mob griefing is off (`canDestroyEgg`); the roll is the
        // level's and comes only after those checks.
        if name == "minecraft:turtle_egg"
            && !matches!(m.species(), Species::Turtle | Species::Bat)
            && w.can_grief(m.id)
            && level.random().next_int_bound(100).unwrap_or(1) == 0
        {
            events.push(MobEvent::CrackTurtleEgg { pos: [x, y, z] });
        }
    }
    // `Entity.checkInsideBlocks` itself, the shared walk: each axis of the
    // move in vanilla's order, a 16-iteration budget, one visited set, the
    // alive check. The mob's own per-block moments run mid-walk where vanilla
    // runs them, and the part's contact fires there too, staged. (This is the
    // parity change: the 0.2-block sampling over every box cell this replaced
    // visited blocks in another order; the walk's own commit carries it.)
    let mut visited = crate::fast_hash::FxHashSet::default();
    let mut effects = mod_api::InsideEffects::default();
    // `Entity.applyEffectsFromBlocks` walks the tick's own movements; a hurt
    // below that teleports an enderman or a shulker has left the old position
    // as the path's start (#9607), so the walk begins where the tick began:
    // the box's own cells at the start are visited first, as the tick's own
    // first movement would, then the move itself sweeps the path (the shared
    // visited set keeps each block once).
    let start = tick_from;
    let movements = [horizon_world::entity_move::MovementRecord {
        from: start,
        to: tick_to,
        original: None,
    }];
    // The box's own cells at the start, as the tick's first movement would
    // visit them (a hurt that teleports leaves the old position as the
    // path's start, #9607).
    let start_box = m.bounds_at(start).deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let mut inside = MobInside {
        m,
        w,
        level,
        events: &mut *events,
    };
    let lo = start_box.min_position();
    let hi = start_box.max_position();
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                let at = horizon_core::position::BlockPos::new(x, y, z);
                let state = w.world.block(x, y, z);
                let solid = w.registry.state(state).is_none_or(|d| {
                    !matches!(
                        d.name.as_str(),
                        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                    )
                });
                if solid
                    && visited.insert(at.pack())
                    && let Ok(()) = crate::block_contact::InsideWalk::entity_inside(
                        &mut inside,
                        at,
                        state,
                        0,
                        false,
                        &mut effects,
                    )
                {}
            }
        }
    }
    let _ =
        crate::block_contact::sweep_movements(&mut inside, &movements, &mut visited, &mut effects);
    let MobInside { m, level, .. } = inside;
    // `applyAndClear` for the part's queued writes, in the collector's order,
    // in this mob's tick, until it dies.
    for planned in effects.take_planned() {
        if !m.alive() {
            break;
        }
        if let mod_api::PlannedInside::Write(write) = planned {
            apply_entity_write(m, level, write);
        }
    }
}

/// A typed entity write an `entity_inside` part queued for a mob, applied in
/// its own tick. `Hurt` uses the generic damage kind: the block's own hurts
/// keep their named kinds in the walk's arms above.
fn apply_entity_write(m: &mut Mob, level: &mut DamageLevel<'_>, write: mod_api::EntityWrite) {
    let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
    match write {
        mod_api::EntityWrite::Push(v) => {
            m.velocity = m.velocity.add_vector(triple(v));
        }
        mod_api::EntityWrite::SetMovement(v) => m.velocity = triple(v),
        mod_api::EntityWrite::StuckIn(v) => {
            m.fall_distance = 0.0;
            m.stuck_multiplier = triple(v);
        }
        mod_api::EntityWrite::Hurt { amount, .. } => {
            m.damage(level, amount, Damage::of(DamageKind::Generic));
        }
        mod_api::EntityWrite::ResetFallDistance => m.fall_distance = 0.0,
        mod_api::EntityWrite::SetFireTicks(ticks) => m.fire_ticks = ticks,
        mod_api::EntityWrite::AddEffect {
            effect,
            duration,
            amplifier,
        } => {
            if let Some(id) = mod_api::__private::effect_key_id(effect) {
                m.extra.effects.push(crate::mobs::StoredEffect {
                    id,
                    duration,
                    amplifier: i32::from(amplifier),
                    registry_index: None,
                    show_icon: true,
                });
            }
        }
    }
}

/// The mob's view of the shared inside walk: its box, its life, the level's
/// blocks, its own `entityInside` moments (kept as their own code, run
/// mid-walk where vanilla runs them), and the part's contact, fired mid-walk
/// over the level random host the mob's tick already holds.
struct MobInside<'a, 'w, 'l> {
    m: &'a mut Mob,
    w: &'a MobWorld<'w>,
    level: &'a mut DamageLevel<'l>,
    events: &'a mut Vec<MobEvent>,
}

impl crate::block_contact::InsideWalk for MobInside<'_, '_, '_> {
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> horizon_core::geometry::Aabb {
        self.m.bounds_at(at)
    }
    fn is_alive(&self) -> bool {
        self.m.alive()
    }
    fn block(&self, at: horizon_core::position::BlockPos) -> horizon_world::registry::BlockStateId {
        self.w.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: horizon_world::registry::BlockStateId) -> bool {
        self.w.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        _state: horizon_world::registry::BlockStateId,
        _at: horizon_core::position::BlockPos,
    ) -> crate::block_contact::InsideShape {
        // `getEntityInsideCollisionShape` is the whole block for every block a
        // mob meets, as the mob's own walk always treated it.
        crate::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        at: horizon_core::position::BlockPos,
        state: horizon_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let (x, y, z) = (at.x, at.y, at.z);
        let p = [x, y, z];
        // The part's contact, mid-walk, staged: its writes join this step only
        // when it answered, and a panic is contained.
        if let Some(hooks) = crate::block_contact::HOOKS.get()
            && let Some(dimension) = crate::block_contact::current_level()
            && (hooks.inside_listens)(state)
        {
            let mut staged = mod_api::InsideEffects::default();
            let mut writes = Vec::new();
            let m = &*self.m;
            (hooks.inside_fire)(
                self.level,
                dimension,
                crate::block_contact::Contact {
                    entity: m.id as i32,
                    kind: &m.kind.id,
                    state,
                    position: at,
                },
                precise,
                &mod_api::SweptEntity::new(
                    m.position,
                    m.previous,
                    m.velocity,
                    m.extra.avatar_pose == crate::pose::Pose::Crouching,
                    m.extra.crouching,
                    m.fall_distance,
                    true,
                ),
                &mut staged,
                &mut writes,
            );
            effects.absorb_step(staged);
            crate::block_contact::inside_written(state, at, writes);
        }
        // PowderSnowBlock.entityInside: a burning mob destroys the
        // block (`canEntityGrief` for the mob), then EXTINGUISH puts it out.
        if self.w.block_name(x, y, z) == "minecraft:powder_snow" {
            if self.m.is_on_fire() && self.w.can_grief(self.m.id) {
                self.events.push(MobEvent::BreakBlock {
                    pos: p,
                    drop: false,
                    by: None,
                    breaker: None,
                });
            }
            self.m.extinguish_fire();
        }
        match self.w.block_name(x, y, z) {
            "minecraft:cactus" => {
                self.m
                    .damage(self.level, 1.0, Damage::of(DamageKind::Cactus));
            }
            // CampfireBlock.entityInside: a lit campfire hurts a
            // living entity by its fire damage (1, soul 2).
            name @ ("minecraft:campfire" | "minecraft:soul_campfire")
                if self.w.prop(x, y, z, "lit") == Some("true") =>
            {
                let damage = if name == "minecraft:campfire" {
                    1.0
                } else {
                    2.0
                };
                self.m
                    .damage(self.level, damage, Damage::of(DamageKind::Campfire));
            }
            // WebBlock.entityInside -> makeStuckInBlock, which a
            // spider ignores for cobwebs and a wither always.
            "minecraft:cobweb"
                if !matches!(
                    self.m.species(),
                    Species::Spider | Species::CaveSpider | Species::Wither
                ) =>
            {
                self.m.fall_distance = 0.0;
                self.m.stuck_multiplier = if self.m.has_effect("minecraft:weaving") {
                    Vec3::new(0.5, 0.25, 0.5)
                } else {
                    Vec3::new(0.25, f64::from(0.05_f32), 0.25)
                };
            }
            "minecraft:sweet_berry_bush"
                if !matches!(self.m.species(), Species::Fox | Species::Bee) =>
            {
                if self.m.species() != Species::Wither {
                    self.m.fall_distance = 0.0;
                    self.m.stuck_multiplier =
                        Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32));
                }
                let moved = self.m.previous.subtract(self.m.position);
                if self.w.prop(x, y, z, "age") != Some("0")
                    && moved.horizontal_distance_squared() > 0.0
                    && (moved.x.abs() >= f64::from(0.003_f32)
                        || moved.z.abs() >= f64::from(0.003_f32))
                {
                    self.m
                        .damage(self.level, 1.0, Damage::of(DamageKind::SweetBerryBush));
                }
            }
            // PowderSnowBlock.entityInside: a living entity is stuck
            // (and forgets its fall) only while the block at its
            // own feet is powder snow.
            "minecraft:powder_snow"
                if self.w.block_name(
                    self.m.position.x.floor() as i32,
                    self.m.position.y.floor() as i32,
                    self.m.position.z.floor() as i32,
                ) == "minecraft:powder_snow" =>
            {
                if self.m.species() != Species::Wither {
                    self.m.fall_distance = 0.0;
                    self.m.stuck_multiplier =
                        Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32));
                }
                let known_movement = self
                    .w
                    .controlling_player_movement
                    .unwrap_or(self.m.velocity);
                if let Some(velocity) =
                    powder_snow_particle_velocity(known_movement, self.level.random())
                {
                    self.events.push(MobEvent::PowderSnowParticle {
                        at: [self.m.position.x, f64::from(y + 1), self.m.position.z],
                        velocity,
                    });
                }
            }
            "minecraft:honey_block" => honey_slide(self.m, p, self.level, self.events),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod powder_snow_particle_tests {
    use super::*;

    /// A ridden mount uses the controlling player's known movement for the
    /// powder-snow roll, even when the mount's own delta is zero.
    #[test]
    #[ignore = "daily"]
    fn a_controlled_mount_uses_its_riders_known_movement() {
        let seed = (0..1000)
            .find(|seed| {
                let mut random = horizon_core::random::LegacyRandom::new(*seed);
                random.next_boolean()
            })
            .unwrap();
        let player_movement = Vec3::new(0.2, 0.0, 0.0);
        let mut expected_random = horizon_core::random::LegacyRandom::new(seed);
        let expected =
            powder_snow_particle_velocity(player_movement, &mut expected_random).unwrap();

        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                for cx in -1..1 {
                    for cz in -1..1 {
                        world.chunks.insert(
                            (cx, cz),
                            std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                        );
                    }
                }
                world
                    .set(
                        0,
                        64,
                        0,
                        registry.default_state("minecraft:powder_snow").unwrap(),
                    )
                    .unwrap();
            },
            |w, types| {
                let mut w = w.clone();
                w.controlling_player_movement = Some(player_movement);
                let kind = std::sync::Arc::new(types.types["minecraft:horse"].clone());
                let mut horse = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                horse.extra.set_saddled(true);
                horse.rider = Some(crate::mobs::equine::Rider::default());
                let mut level = crate::health::TestLevel::new(seed);
                let mut events = Vec::new();

                effects_from_blocks(&mut horse, &w, &mut level.level(), &mut events);

                assert!(events.iter().any(|event| matches!(
                    event,
                    MobEvent::PowderSnowParticle { velocity, .. } if *velocity == expected
                )));
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }
}

/// `HoneyBlock.entityInside` for a mob: a falling mob beside the honey block at
/// `p` slides down it (`isSlidingDown`, `doSlideMovement`, `maybeDoSlideEffects`).
fn honey_slide(m: &mut Mob, p: [i32; 3], level: &mut DamageLevel<'_>, events: &mut Vec<MobEvent>) {
    let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
    let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
    let [x, y, z] = p.map(f64::from);
    let pos = m.position;
    if m.on_ground || pos.y > y + 0.9375 - 1.0e-7 || old_delta_y(m.velocity.y) >= -0.08 {
        return;
    }
    let dx = (x + 0.5 - pos.x).abs();
    let dz = (z + 0.5 - pos.z).abs();
    let overlap = 0.4375 + f64::from(m.width()) / 2.0;
    if dx + 1.0e-7 <= overlap && dz + 1.0e-7 <= overlap {
        return;
    }
    // doSlideMovement
    let v = m.velocity;
    let old = old_delta_y(v.y);
    m.velocity = if old < -0.13 {
        let factor = -0.05 / old;
        Vec3::new(v.x * factor, new_delta_y(-0.05), v.z * factor)
    } else {
        Vec3::new(v.x, new_delta_y(-0.05), v.z)
    };
    m.fall_distance = 0.0;
    // maybeDoSlideEffects: the slide sound (one time in five, unless silent),
    // then the server's entity event 53 (one time in five).
    let sound = level.random().next_int_bound(5).unwrap_or(1) == 0;
    if sound && !m.silent {
        events.push(MobEvent::Sound(
            "block.honey_block.slide",
            [pos.x, pos.y, pos.z],
            1.0,
            1.0,
        ));
    }
    if level.random().next_int_bound(5).unwrap_or(1) == 0 {
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 53,
            at: [pos.x, pos.y, pos.z],
        });
    }
}
/// LivingEntity.updateSwingTime.
pub fn tick_swing(m: &mut Mob) {
    let duration = m.swing_duration();
    if m.ai.swinging {
        m.ai.swing_time += 1;
        if m.ai.swing_time >= duration {
            m.ai.swing_time = 0;
            m.ai.swinging = false;
            m.ai.swing_duration = 0;
            m.extra.swing_right_arm = None;
            m.extra.swing_stab = false;
        }
    } else {
        m.ai.swing_time = 0;
    }
    m.anim.attack_anim = m.ai.swing_time as f32 / duration as f32;
}
/// `Entity.isInRain`: rain at the block position, or at the top of the box.
pub(super) fn in_rain(m: &Mob, w: &MobWorld) -> bool {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    let top = (m.position.y + f64::from(m.height())).floor() as i32;
    w.raining_at(x, y, z) || w.raining_at(x, top, z)
}
/// `LivingEntity.baseTick` refills a breather's air by 4, then
/// `Axolotl.handleAirSupply` (unless the mob has no AI): water or rain fills
/// it, anywhere else it runs down from the pre-tick value, and at -20 the
/// axolotl dries out.
fn axolotl_air_supply(m: &mut Mob, w: &MobWorld, level: &mut DamageLevel<'_>) {
    if m.no_ai {
        m.air = (m.air + 4).min(super::axolotl::MAX_AIR);
        return;
    }
    let breathes = m.fluids.in_water || in_rain(m, w);
    let (cx, max) = (w.loop_context, super::axolotl::MAX_AIR);
    if let Some(damage) = own_air(cx, m.id, &m.kind.id, &mut m.air, breathes, max) {
        m.damage(level, damage, Damage::of(DamageKind::DryOut));
    }
}
/// `Nautilus.baseTick`: `LivingEntity.baseTick` refills the air of this
/// water breather, then, unless it has no AI, `Nautilus.handleAirSupply`
/// takes it back out of water, where the nautilus dries out.
fn nautilus_air_supply(m: &mut Mob, w: &MobWorld, level: &mut DamageLevel<'_>) {
    const MAX_AIR: i32 = 300;
    if m.no_ai {
        if m.alive() {
            m.air = (m.air + 4).min(MAX_AIR);
        }
        return;
    }
    if !m.alive() {
        m.air = MAX_AIR;
        return;
    }
    let (cx, wet) = (w.loop_context, m.fluids.in_water);
    if let Some(damage) = own_air(cx, m.id, &m.kind.id, &mut m.air, wet, MAX_AIR) {
        m.damage(level, damage, Damage::of(DamageKind::DryOut));
    }
}
/// `Entity.isInWall` (minus the noPhysics early-out, which the caller applies):
/// visit every cell covered by the thin eye box and ask whether a suffocating
/// collision shape actually intersects it. Width multiplication is a float.
fn in_wall(
    width: f32,
    eye: [f64; 3],
    mut intersects: impl FnMut(i32, i32, i32, &Aabb) -> bool,
) -> bool {
    let side = f64::from(width * 0.8_f32);
    let half = [side / 2.0, 1.0e-6 / 2.0, side / 2.0];
    let lo = [0, 1, 2].map(|i| eye[i] - half[i]);
    let hi = [0, 1, 2].map(|i| eye[i] + half[i]);
    let bounds = Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    );
    let c0 = lo.map(|v| v.floor() as i32);
    let c1 = hi.map(|v| v.floor() as i32);
    (c0[0]..=c1[0])
        .any(|x| (c0[1]..=c1[1]).any(|y| (c0[2]..=c1[2]).any(|z| intersects(x, y, z, &bounds))))
}
fn extinguish_if_wet(m: &mut Mob, w: &MobWorld, was_on_fire: bool, events: &mut Vec<MobEvent>) {
    // Entity.applyEffectsFromBlocks snapshots burning before contact ignition.
    // Rain also clears newly ignited fire, but only a previously burning mob
    // plays the hiss and consumes its two pitch draws. This includes NoAI mobs.
    if m.fire_ticks > 0 && (m.fluids.in_water || in_rain(m, w)) {
        m.fire_ticks = 0;
        if was_on_fire {
            events.push(MobEvent::Sound(
                "entity.generic.extinguish_fire",
                [m.position.x, m.position.y, m.position.z],
                0.7,
                1.6 + (m.random.next_float() - m.random.next_float()) * 0.4,
            ));
        }
    }
}

/// `LivingEntity.canBreatheUnderwater`: the `can_breathe_under_water` tag, which
/// `MobType.flags.breathes_water` holds, and the two overrides — a baby happy ghast
/// and a sulfur cube with a block in its body breathe under water too.
fn can_breathe_under_water(m: &Mob) -> bool {
    m.kind.flags.breathes_water
        || m.species() == Species::HappyGhast && m.is_baby()
        || m.species() == Species::SulfurCube && crate::mobs::sulfur_cube::has_body(m)
}

/// `IronGolem.decreaseAirSupply` hands back the air it is given (`iload_1;
/// ireturn`), so a golem under water is in `baseTick`'s drowning branch with a
/// consume of 0: it never loses a bubble, never reaches the -20 that hurts, and
/// never refills either, though the tag does not list its type.
fn keeps_its_air(m: &Mob) -> bool {
    m.species() == Species::IronGolem
}

/// Entity.baseTick + LivingEntity.baseTick + Mob.baseTick.
fn base_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> bool {
    if m.first_tick {
        super::villager::refresh_sleeping_position(m, w);
    }
    // Entity.canSpawnSprintParticle / spawnSprintParticle (a dashing cat or
    // ocelot): a server level drops the particle, but its spot is drawn
    // from the mob's random first.
    // `IronGolem.canSpawnSprintParticle`: moving, and a one in five draw, sprinting or not.
    let spawn = if m.species() == Species::IronGolem {
        let horizontal = m.velocity.x * m.velocity.x + m.velocity.z * m.velocity.z;
        horizontal > f64::from(2.500_000_3e-7_f32) && m.random.next_int_bound(5).unwrap_or(1) == 0
    } else {
        m.extra.sprinting && !m.fluids.in_water && !m.fluids.in_lava && m.alive()
    };
    if spawn {
        let at = m.block_on_legacy(w);
        let state = w.world.block(at[0], at[1], at[2]);
        if w.facts.get(state).render_shape != crate::blocks::RenderShape::Invisible {
            let _ = (m.random.next_double(), m.random.next_double());
        }
    }
    m.was_eye_in_water = m.fluids.eye_in_water;
    m.update_fluids(w, events);
    if m.species() == Species::Drowned {
        super::undead::drowned_update_swimming(m, w);
    }
    let fire = crate::motion::environment::fire_contact(w.world, w.registry, m.bounds());
    let lava = m.fluids.in_lava;
    let contact_at = m.position;
    if fire > 0.0 {
        m.ticks_frozen = 0;
    }
    // Entity.baseTick counts burn damage and the timer before a wet mob is put out.
    if m.fire_ticks > 0 {
        if m.kind.flags.fire_immune {
            m.fire_ticks = 0;
        } else {
            if m.fire_ticks % 20 == 0 && !m.fluids.in_lava {
                m.damage(level, 1.0, Damage::of(DamageKind::OnFire));
                // Entity.baseTick's hurt teleports before later contact is
                // checked, so the old spot's fire cannot ignite the landing.
                hurt_teleports(m, w, events);
            }
            m.fire_ticks -= 1;
        }
    }
    // Entity.checkBelowWorld: LivingEntity.onBelowWorld hurts 4 a tick.
    if m.position.y < f64::from(w.world.bottom_y() - 64) {
        m.damage(level, 4.0, Damage::of(DamageKind::OutOfWorld));
    }
    m.first_tick = false;
    if m.alive() {
        // LivingEntity.baseTick: a suffocating shape intersects the eye. Entity.isInWall
        // is false under noPhysics, which Vex.tick holds while it ticks.
        if m.species() != Species::Vex && mob_in_wall(m, w) {
            m.damage(level, 1.0, Damage::of(DamageKind::InWall));
        }
        // Air supply and drowning (WaterAnimal.handleAirSupply for aquatic mobs).
        // Nautilus.baseTick: NoAI skips only the custom handler; the
        // superclass Breath/drown path must still process its air.
        if m.species() == Species::Axolotl {
            axolotl_air_supply(m, w, level);
        } else if m.species() == Species::Nautilus {
            nautilus_air_supply(m, w, level);
        } else if m.kind.flags.needs_water {
            let (cx, wet) = (w.loop_context, m.fluids.in_water);
            if let Some(damage) = own_air(cx, m.id, &m.kind.id, &mut m.air, wet, 300) {
                m.damage(level, damage, Damage::of(DamageKind::Drown));
            }
        } else {
            // LivingEntity.baseTick / MobEffectUtil: bubble columns and breathing
            // effects protect submerged mobs. Nautilus alone holds air rather
            // than refilling it; Water Breathing or Conduit Power still refill.
            let submerged = m.fluids.eye_in_water
                && w.block_name(
                    m.position.x.floor() as i32,
                    m.eye_y().floor() as i32,
                    m.position.z.floor() as i32,
                ) != "minecraft:bubble_column";
            let water_breathing = m.has_effect("minecraft:water_breathing")
                || m.has_effect("minecraft:conduit_power");
            let nautilus = m.has_effect("minecraft:breath_of_the_nautilus");
            let drowns = submerged && !can_breathe_under_water(m) && !water_breathing && !nautilus;
            let held = submerged && nautilus && !water_breathing;
            let max = m.max_air_supply();
            let vanilla = Breath {
                can_breathe: !drowns,
                consume: if keeps_its_air(m) { 0 } else { 1 },
                // `increaseAirSupply`: four a tick, except the dolphin's own, which
                // hands back the whole 4800 maximum however little air it has.
                refill: if drowns || held {
                    0
                } else if m.species() == Species::Dolphin {
                    max.saturating_sub(m.air)
                } else {
                    4
                },
            };
            let breath = breathe(w.loop_context, m.id, &m.kind.id, vanilla);
            if let Some(drowning) = drown(
                w.loop_context,
                m.id,
                &m.kind.id,
                &mut m.air,
                max,
                breath,
                true,
            ) {
                events.push(MobEvent::Drown {
                    mob: m.id,
                    at: [m.position.x, m.position.y, m.position.z],
                    bubbles: drowning.bubbles,
                });
                if let Some(damage) = drowning.damage {
                    m.damage(level, damage, Damage::of(DamageKind::Drown));
                }
            }
        }
    }
    if m.species() == Species::Illusioner {
        super::illusioner::tick(m, events);
    }
    if matches!(m.species(), Species::Evoker | Species::Illusioner) {
        super::illager::cast_particles(m, events);
    }
    super::hurt_by::base_tick(m, w);
    // LivingEntity.baseTick runs tickEffects after the timers count down.
    // Enderman.hurtServer can teleport even when LivingEntity rejects the
    // damage. Settle each attempt before the next effect uses its position or RNG.
    tick_effects(m, w.loop_context, level, w.mod_effects, |m| {
        hurt_teleports(m, w, events);
    });
    // Entity.applyEffectsFromBlocks runs in LivingEntity.aiStep, after
    // baseTick has counted the fire and the hurt timers down: a fire block or
    // lava ignites to the full 160 / 300 ticks and its hit lands after the
    // invulnerable time of that tick has already dropped.
    let was_on_fire = m.fire_ticks > 0;
    let (fire, lava) = if m.position == contact_at {
        (fire, lava)
    } else {
        (
            crate::motion::environment::fire_contact(w.world, w.registry, m.bounds()),
            touches_lava(m, w),
        )
    };
    // LavaCauldronBlock.entityInside: LAVA_IGNITE and lavaHurt, as in lava.
    let lava = lava || crate::entities::lava_cauldron_contacts(w.world, w.registry, m.bounds()) > 0;
    block_contact_damage(m, w, level, fire, lava);
    // Enderman.hurtServer and Shulker.hurtServer teleport inside the hurt
    // (fire, lava, a block, an effect).
    hurt_teleports(m, w, events);
    if !m.alive() {
        m.death_time += 1;
    }
    // Hurt sounds of the damage above come before the ambient sound.
    events.append(&mut m.outbox);
    // Mob.baseTick: ambient sounds.
    if m.alive() {
        let roll = m.random.next_int_bound(1000).unwrap_or(999);
        let t = m.ambient_timer;
        m.ambient_timer += 1;
        if roll < t {
            m.ambient_timer = -m.ambient_interval();
            super::entity_sounds::ambient(m, w, events);
        }
    }
    // Entity.baseTick ends with Leashable.tickLeash.
    super::leash::tick(m, w, events);
    was_on_fire
}
/// Freezing runs after movement and block effects.
fn freeze_tick(m: &mut Mob, w: &MobWorld, level: &mut DamageLevel<'_>) {
    let in_snow = in_powder_snow(m, w);
    m.in_powder_snow = in_snow;
    if !in_snow && m.ticks_frozen == 0 {
        return;
    }
    let can_freeze = can_freeze(m, w);
    if in_snow && can_freeze {
        m.ticks_frozen =
            (m.ticks_frozen + 1).min(crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
    } else {
        m.ticks_frozen = (m.ticks_frozen - 2).max(0);
    }
    if m.age.is_multiple_of(40) && m.is_fully_frozen() && can_freeze {
        m.damage(level, 1.0, Damage::of(DamageKind::Freeze));
    }
}
/// Whether the mob's box (shrunk by `1.0E-5`) touches a lava block: the
/// LAVA_IGNITE contact read where a teleport landed, whose own fluid check
/// (`Enderman.canRandomlyTeleportTo`, a shulker's empty spot) keeps lava
/// from the feet.
fn touches_lava(m: &Mob, w: &MobWorld) -> bool {
    let b = m.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let (min, max) = (b.min_position(), b.max_position());
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for y in min.y.floor() as i32..=max.y.floor() as i32 {
            for z in min.z.floor() as i32..=max.z.floor() as i32 {
                if &*w.facts.get(w.world.block(x, y, z)).name == "minecraft:lava" {
                    return true;
                }
            }
        }
    }
    false
}
/// `Mob.playAmbientSound` on demand: a spawn egg or a mob bucket just placed `m`.
pub fn ambient_sound(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    super::entity_sounds::ambient(m, w, events);
}
pub(super) fn play_ambient(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if super::axolotl::is_playing_dead(m) {
        return;
    }
    let sound = match m.species() {
        // Warden.getAmbientSound: by anger, silent while roaring or digging.
        Species::Warden => match super::warden::ambient_sound(m) {
            Some(sound) => sound,
            None => return,
        },
        Species::Zombie | Species::Husk if m.is_baby() => m.kind.sounds[0],
        Species::Drowned if m.fluids.in_water => "entity.drowned.ambient_water",
        // Nautilus/ZombieNautilus.getAmbientSound: by age, under water or on land.
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::sounds(m)[0],
        Species::Enderman if m.ai.target.is_some() => "entity.enderman.scream",
        // Wolf.isAngry reads the persistent anger timer, not the current target.
        Species::Wolf if super::tame::angry(m) => {
            super::finalize::variant_sound(m, "growl").unwrap_or("entity.wolf.growl")
        }
        Species::Wolf if m.random.next_int_bound(3).unwrap_or(1) == 0 => {
            // Wolf.getAmbientSound: under 20 health, whatever the max health.
            let hurt = m.extra.tame && m.health < 20.0;
            let kind = if hurt { "whine" } else { "pant" };
            super::finalize::variant_sound(m, kind).unwrap_or("entity.wolf.pant")
        }
        Species::Parrot => super::parrot::ambient(m, w),
        // Cat.getAmbientSound: purr in love, purreow one time in four, else ambient.
        Species::Cat if m.extra.tame && m.extra.in_love > 0 => {
            super::finalize::variant_sound(m, "purr").unwrap_or("entity.cat.purr")
        }
        Species::Cat if m.extra.tame && m.random.next_int_bound(4).unwrap_or(1) == 0 => {
            super::finalize::variant_sound(m, "purreow").unwrap_or("entity.cat.purreow")
        }
        Species::Cat if !m.extra.tame => {
            super::finalize::variant_sound(m, "stray_ambient").unwrap_or("entity.cat.stray_ambient")
        }
        // Fox.getAmbientSound: a sleeping fox makes its sleep sound.
        Species::Fox if super::fox::flag(m, super::fox::SLEEPING) => "entity.fox.sleep",
        // Fox.playAmbientSound: the screech at volume 2. Anything else goes to
        // Mob.playAmbientSound, which asks getAmbientSound again (a second draw).
        Species::Fox => {
            if super::fox::awake_ambient(m, w) == super::fox::SCREECH {
                let pitch = m.voice_pitch();
                let at = [m.position.x, m.position.y, m.position.z];
                events.push(MobEvent::Sound(super::fox::SCREECH, at, 2.0, pitch));
                return;
            }
            super::fox::awake_ambient(m, w)
        }
        // Turtle.getAmbientSound: the land ambient only on land and grown up, else silent.
        Species::Turtle if m.fluids.in_water || !m.on_ground || m.is_baby() => return,
        Species::PolarBear if m.is_baby() => "entity.polar_bear.ambient_baby",
        // Turtle.getAmbientSound: only an adult on land makes a sound.
        Species::Turtle if turtle_ambient_silent(m) => return,
        Species::Dolphin if !m.fluids.in_water => "entity.dolphin.ambient",
        Species::Guardian if !m.fluids.in_water => "entity.guardian.ambient_land",
        Species::ElderGuardian if !m.fluids.in_water => "entity.elder_guardian.ambient_land",
        Species::Axolotl if !m.fluids.in_water => "entity.axolotl.idle_air",
        // Villager.getAmbientSound: silent while asleep, the trade sound
        // while a player has its menu open.
        Species::Villager if super::villager::sleeping(m) => return,
        Species::Villager if m.extra.flag2 => "entity.villager.trade",
        // WanderingTrader.getAmbientSound: only the trade branch, it never sleeps.
        Species::WanderingTrader if m.extra.flag2 => "entity.wandering_trader.trade",
        Species::Bat if m.extra.flag && m.random.next_int_bound(4).unwrap_or(1) != 0 => return,
        Species::Allay if m.extra.main_hand_id().is_some() => "entity.allay.ambient_with_item",
        // Strider.getAmbientSound: quiet while panicking or tempted.
        Species::Strider if m.ai.panicking || m.ai.goals.is_running("Tempt") => return,
        // HappyGhast: its own voice; `Mob::voice` plays it at the feet
        // (`Entity.playSound`), never for a Silent ghast, at pitch 1.0.
        Species::HappyGhast => super::happy_ghast::sounds(m)[0],
        // Sniffer.getAmbientSound.
        Species::Sniffer => match super::sniffer::ambient(m) {
            Some(s) => s,
            None => return,
        },
        // Breeze.playAmbientSound: a local sound, nothing drawn.
        Species::Breeze => {
            if let Some(s) = super::breeze::ambient(m) {
                let at = [m.position.x, m.position.y, m.position.z];
                events.push(MobEvent::LocalSound(s, at, 1.0, 1.0));
            }
            return;
        }
        // Creaking.getAmbientSound: silent while awake.
        Species::Creaking if m.extra.flag => return,
        // Shulker.playAmbientSound: silent while closed.
        Species::Shulker if m.extra.counter == 0 => return,
        Species::ZombifiedPiglin if m.ai.target.is_some() => "entity.zombified_piglin.angry",
        Species::Piglin => super::piglin::piglin_ambient(m, w),
        Species::Hoglin => super::piglin::hoglin_ambient(m, w),
        // Zoglin.getAmbientSound: with an attack target, the angry sound.
        Species::Zoglin if m.ai.target.is_some() => "entity.zoglin.angry",
        _ => super::entity_sounds::voices(m)[0],
    };
    let _ = w;
    if !sound.is_empty() {
        m.voice(sound, events);
    }
}
/// `Turtle.getAmbientSound`: `entity.turtle.ambient_land` only when the turtle
/// is out of water, on the ground and not a baby; otherwise `null` (silent).
fn turtle_ambient_silent(m: &Mob) -> bool {
    m.fluids.in_water || !m.on_ground || m.is_baby()
}
/// `Animal.aiStep`: age-based love clearing and countdown, following the age step.
/// Both the normal and NoAI paths run it after the age step.
fn animal_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.alive() {
        if m.extra.age != 0 {
            m.extra.in_love = 0;
        }
        if m.extra.in_love > 0 {
            m.extra.in_love -= 1;
            if m.extra.in_love % 10 == 0 {
                // The heart's speed and getRandomX/Y/Z, drawn even on the server.
                let (w, h) = (f64::from(m.width()), f64::from(m.height()));
                let r = &mut m.random;
                let velocity = [
                    r.next_gaussian() * 0.02,
                    r.next_gaussian() * 0.02,
                    r.next_gaussian() * 0.02,
                ];
                let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
                let y = m.position.y + h * r.next_double() + 0.5;
                let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
                events.push(MobEvent::ParticleAt {
                    kind: "heart",
                    at: [x, y, z],
                    velocity,
                });
            }
        }
    }
}
/// `AgeableMob.aiStep` runs even when `Mob.serverAiStep` is disabled.
fn ageable_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    // AgeableMob.aiStep: a forced age-up's particles (`sendParticles`, so the
    // players are sent them), then the age.
    if m.extra.forced_age_timer > 0 {
        if m.extra.forced_age_timer % 4 == 0 {
            let (w, h) = (f64::from(m.width()), f64::from(m.height()));
            let r = &mut m.random;
            let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
            let y = m.position.y + h * r.next_double() + 0.5;
            let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
            // sendParticles(HAPPY_VILLAGER, x, y, z, 1, 0, 0, 0, 0): exactly there, at rest.
            events.push(MobEvent::LevelParticles {
                kind: "happy_villager",
                at: [x, y, z],
                spread: [0.0; 3],
                speed: 0.0,
                count: 1,
            });
        }
        m.extra.forced_age_timer -= 1;
    }
    // AgeableMob.aiStep: an age counts only for its subclasses (a tadpole
    // counts its own age up, a zombie's baby age never moves). `canAgeUp`:
    // a baby grows unless an age lock holds it; a breeding pause counts down.
    if m.alive() && super::finalize::ageable(m.species()) {
        if can_age_up(m) {
            let (bw, bh, _) = m.dims();
            // AgeableMob.setAge(getAge() + 1): the crossing is where a species'
            // ageBoundaryReached runs (a goat's and a hoglin's attack damage, a
            // sulfur cube's size), which the age write used to skip.
            let next = m.extra.age + 1;
            super::finalize::set_age(m, next, Some(w));
            // setAge at the age boundary: refreshDimensions.
            if m.extra.age >= 0 {
                m.fudge_position_after_size_change(w, bw, bh);
                if m.species() == Species::Villager {
                    super::villager_brain::age_boundary_reached(m, w, level, events);
                }
                // Turtle.ageBoundaryReached: a grown turtle drops a scute.
                if m.species() == Species::Turtle {
                    events.push(MobEvent::GiftLoot {
                        mob: m.id,
                        table: "minecraft:gameplay/turtle_grow",
                        at: [m.position.x, m.position.y, m.position.z],
                    });
                }
            }
        } else if m.extra.age > 0 {
            m.extra.age -= 1;
        }
    }
    if super::finalize::ageable(m.species()) {
        make_age_locked_particle(m, events);
    }
}

/// `LivingEntity.tick` after `aiStep`: the glide's tick count.
fn count_fall_fly_ticks(m: &mut Mob) {
    m.extra.fall_fly_ticks = if m.extra.fall_flying {
        m.extra.fall_fly_ticks + 1
    } else {
        0
    };
}

/// `LivingEntity.aiStep`: both flight effects discard accumulated fall before
/// travel, even when NoAI skips the server AI step.
fn reset_effect_fall_distance(m: &mut Mob) {
    if m.has_effect("minecraft:slow_falling") || m.has_effect("minecraft:levitation") {
        m.fall_distance = 0.0;
    }
}

/// `Witch.aiStep` before `super.aiStep()`, with or without AI (NoAI only
/// skips `serverAiStep` and travel inside `LivingEntity.aiStep`).
fn witch_ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Witch {
        let far = m
            .target_info(w)
            .is_some_and(|t| t.position.distance_squared(m.position) > 121.0);
        super::witch::ai_step(m, w.loop_context, events, far);
    }
}

/// LivingEntity.aiStep (and Mob/AgeableMob/Animal additions).
fn ai_step(m: &mut Mob, w: &MobWorld, level: &mut DamageLevel<'_>, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Parrot {
        super::parrot::ai_step(m, w, level.random(), events);
    }
    witch_ai_step(m, w, events);
    if m.species() == Species::Fox {
        super::fox::pre_ai_step(m);
    }
    // Blaze.aiStep slows the fall before super.aiStep(), so before travel.
    if m.species() == Species::Blaze {
        super::nether::blaze_ai_step(m);
    }
    // AbstractHorse.aiStep: the tail swishes now and then (moveTail).
    if super::equine::is_equine(m.species()) {
        super::equine::tail_tick(m);
    }
    // Monster.aiStep: updateNoActionTime (Raider: always +2).
    if is_monster(m.species())
        && (super::raider::is_raider(m.species()) || super::hostile::magic_value(m, w) > 0.5)
    {
        // A Java `int`: past `i32::MAX` it wraps, as there.
        m.ai.no_action_time = m.ai.no_action_time.wrapping_add(2);
    }
    if super::creaking::is_creaking(m) {
        super::creaking::update(m, w, events);
    }
    if m.ai.no_jump_delay > 0 {
        m.ai.no_jump_delay -= 1;
    }
    drop_tiny_motion(m);
    m.ai.xxa *= 0.98;
    m.ai.zza *= 0.98;
    // LivingEntity.isImmobile, no serverAiStep or input: a stunned ravager, a rearing horse, a paused mob.
    let immobile = super::ai_pause::hold(m, w)
        || (m.species() == Species::Ravager && super::ravager::immobile(m))
        || super::equine::immobile(m)
        || m.species() == Species::Mannequin;
    if matches!(m.species(), Species::Piglin | Species::Hoglin) && m.alive() {
        super::piglin::before_goals(m, w, level.random(), events);
    }
    if m.alive() && !immobile {
        super::dab::step(m, w, level, events);
    } else {
        m.ai.jumping = false;
        m.ai.xxa = 0.0;
        m.ai.zza = 0.0;
    }
    m.handle_jumping(w);
    let input = Vec3::new(
        f64::from(m.ai.xxa),
        f64::from(m.ai.yya),
        f64::from(m.ai.zza),
    );
    update_fall_flying(m, w, events);
    reset_effect_fall_distance(m);
    if super::equine::moved_by_client(m) {
        // LivingEntity.travelRidden: the rider's client moves it
        // (handleMoveVehicle), so the server only stops it.
        m.velocity = Vec3::ZERO;
    } else if !(m.species() == Species::Mannequin && m.extra.immovable) {
        // Mannequin.isEffectiveAi: an immovable one never travels.
        m.travel(w, level, input, events);
        // A fall's hurt (checkFallDamage) teleports at once.
        hurt_teleports(m, w, events);
    }
    // LivingEntity.aiStep: applyEffectsFromBlocks follows travel.
    m.apply_effects_from_blocks(w, events);
    push_entities(m, w, level, events);
    hurt_by_water(m, w, level, events);
    // Spider.tick: climbing while pressed against a wall.
    if m.kind.flags.climber {
        m.climbing = m.horizontal_collision;
    }
    ageable_step(m, w, level, events);
    animal_step(m, events);
    burn_undead(m, w, events);
    species_ai_step(m, w, level, events);
}
/// The end of `LivingEntity.aiStep`: a mob that water hurts (an enderman, a
/// blaze, a snow golem) takes 1 drowning damage in water or rain
/// (`isSensitiveToWater`, `isInWaterOrRain`). An enderman's teleport is part of
/// that hurt (`Enderman.hurtServer`), so it happens now, not next tick.
fn hurt_by_water(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    if m.kind.flags.water_sensitive && (m.fluids.in_water || in_rain(m, w)) {
        m.damage(level, 1.0, Damage::of(DamageKind::Drown));
        hurt_teleports(m, w, events);
    }
}
/// A hurt the mob's own tick deals (`Entity.baseTick`, `tickEffects`, a
/// block's touch, cramming), with the teleport `Enderman.hurtServer` or
/// `Shulker.hurtServer` makes inside it: a second hurt of the same tick draws
/// after the first one's teleport and sees where it landed.
#[cfg(test)]
fn hurt(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>, amount: f32, source: Damage) {
    m.damage(
        &mut crate::health::TestLevel::new(0).level(),
        amount,
        source,
    );
    hurt_teleports(m, w, events);
}
/// The teleport `Enderman.hurtServer` or `Shulker.hurtServer` makes inside a
/// hurt, after the hurt's sound; with or without AI.
fn hurt_teleports(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Enderman && m.extra.counter2 > 0 {
        events.append(&mut m.outbox);
        super::enderman::run_pending_teleports(m, w, events);
    }
    if m.species() == Species::Shulker && m.extra.flag {
        events.append(&mut m.outbox);
        super::shulker::hurt_teleport(m, w, events);
    }
}
impl Mob {
    /// LivingEntity.isPushable (and the push it takes from `Entity.push(Entity)`):
    /// bats and shulkers are never moved by others, nor is a mob on a climbable
    /// block (`onClimbable`, cached by the last tick) unless its own
    /// `isPushable` says otherwise (`Parrot.isPushable` is a plain true),
    /// nor a ridden horse (`AbstractHorse.isPushable` is `!isVehicle`).
    pub fn pushable(&self) -> bool {
        self.alive()
            && !matches!(self.species(), Species::Bat | Species::Shulker)
            && (!self.on_climbable || self.species() == Species::Parrot)
            && (self.species() != Species::Creaking || super::creaking::can_move(self))
            && (self.species() != Species::Warden || !super::warden::digging_or_emerging(self))
            && !(super::equine::is_equine(self.species()) && super::equine::is_vehicle(self))
    }
}
/// LivingEntity.onClimbable for the pusher (Spider.onClimbable: climbing a wall).
fn on_climbable(m: &Mob, w: &MobWorld) -> bool {
    m.on_climbable(w)
}
/// `LivingEntity.aiStep`, with or without AI: a mob's motion below 0.003 on an
/// axis stops.
fn drop_tiny_motion(m: &mut Mob) {
    let v = m.velocity;
    m.velocity = Vec3::new(
        if v.x.abs() < 0.003 { 0.0 } else { v.x },
        if v.y.abs() < 0.003 { 0.0 } else { v.y },
        if v.z.abs() < 0.003 { 0.0 } else { v.z },
    );
}
/// LivingEntity.pushEntities: every pushable entity overlapping the box is
/// pushed apart from this one (`doPush(e)` = `e.push(this)`); pushes of other
/// mobs are `Push` events their tick picks up.
pub(super) fn push_entities(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    m.on_climbable = on_climbable(m, w);
    if m.species() == Species::Bat {
        return;
    }
    let bounds = m.bounds();
    // The pusher's own isPushable and isVehicle, checked by `e.push(this)` before moving it.
    let self_pushable = m.pushable() && !super::equine::is_vehicle(m);
    // Each passenger with whether it shares this one's root vehicle.
    let mut targets = super::world::take_push_targets();
    if let Some(p) = w.selected_player()
        // Entity.push: passengers of the same vehicle never push each other.
        && m.rider.is_none()
        && p.alive
        && !p.spectator
        // EntitySelector.pushableBy asks isPushable: a climbing player is left
        // out, and `doPush` then gives neither of the pair a push.
        && !p.on_climbable
        && p.bounds.intersects(bounds)
    {
        targets.push((Who::Player, p.position, p.passenger, false, false));
    }
    let root = root_vehicle(w, m.id, m.extra.mount);
    for o in w.others_near(bounds) {
        if o.id != m.id && o.alive && o.pushable && o.bounds.intersects(bounds) {
            let same = root_vehicle(w, o.id, o.vehicle) == root;
            targets.push((Who::Mob(o.id), o.position, o.passenger, same, o.is_vehicle));
        }
    }
    // EntitySelector.pushableBy(this): the teams' collision rules.
    let own = w.teams.of(Who::Mob(m.id));
    targets.retain(|(who, ..)| crate::scoreboard::pushable_by(own, w.teams.of(*who), true));
    // Cramming: a quarter of the ticks spent with `maxEntityCramming` or more
    // others around that are not riding anything hurts it by 6.
    let limit = w.max_entity_cramming;
    if limit > 0
        && targets.len() as i64 > i64::from(limit) - 1
        && m.random.next_int_bound(4).unwrap_or(1) == 0
        && targets.iter().filter(|t| !t.2).count() as i64 > i64::from(limit) - 1
    {
        m.damage(level, 6.0, Damage::of(DamageKind::Cramming));
    }
    // The server's collision cap, after the cramming check above, which counts every entity.
    let mut cap = Cap::new(m, w);
    for &(who, at, _, same, vehicle) in &targets {
        match cap.check(who, w) {
            Verdict::Push => {}
            Verdict::Skip => continue,
            Verdict::Stop => break,
        }
        // Warden.doPush: touching it angers it, then the push itself.
        if m.species() == Species::Warden {
            super::warden::do_push(m, w, who, events);
        }
        // IronGolem.doPush: a monster that is not a creeper becomes its target
        // one time in 20, and the draw is made for those alone, then the push.
        if m.species() == Species::IronGolem
            && let Who::Mob(id) = who
            && w.seen(id).is_some_and(super::golem::golem_prey)
            && m.random.next_int_bound(20).unwrap_or(1) == 0
        {
            m.set_ai_target(w, Some(who));
        }
        // doPush calls the other living entity's push; sleeping vetoes both impulses.
        let sleeping = match who {
            Who::Player => w.selected_player().is_some_and(|p| p.sleeping),
            Who::Mob(id) => w.seen(id).is_some_and(|o| o.sleeping),
        };
        if sleeping {
            continue;
        }
        // Entity.push: passengers of the same vehicle (a rider and its mount
        // among them) never push each other.
        if same {
            continue;
        }
        // Entity.push(Entity) with `this` the other entity.
        let (mut xa, mut za) = (m.position.x - at.x, m.position.z - at.z);
        let mut dd = xa.abs().max(za.abs());
        if dd < f64::from(0.01_f32) {
            continue;
        }
        dd = dd.sqrt();
        xa /= dd;
        za /= dd;
        let pow = (1.0 / dd).min(1.0);
        xa *= pow;
        za *= pow;
        xa *= f64::from(0.05_f32);
        za *= f64::from(0.05_f32);
        // A guest's server-side push never reaches its client (ServerPlayer
        // motion is synced only when hurt): its client pushes it itself.
        if !vehicle && (who != Who::Player || w.selected_player().is_none_or(|p| !p.remote)) {
            events.push(w.route_player_event(MobEvent::Push {
                target: who,
                velocity: [-xa, 0.0, -za],
            }));
        }
        if self_pushable {
            m.velocity = Vec3::new(m.velocity.x + xa, m.velocity.y, m.velocity.z + za);
        }
        cap.record(m, who);
    }
    super::world::give_push_targets(targets);
}
/// `Mob.aiStep` -> `Mob.burnUndead`: undead burn in daylight. It runs with
/// or without AI (`NoAI` only skips `serverAiStep`).
fn burn_undead(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.kind.flags.burns_in_day && m.alive() && sun_burn_tick(m, w) {
        if m.item_by_slot(m.sun_protection_slot()).is_some() {
            events.push(MobEvent::SunProtectionWear { mob: m.id });
        } else {
            m.ignite_for_ticks(160);
        }
    }
}
fn sun_burn_tick(m: &mut Mob, w: &MobWorld) -> bool {
    // Mob.isSunBurnTick: the monsters_burn attribute, not the sky darkness.
    if !w.monsters_burn {
        return false;
    }
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.eye_y().floor() as i32,
        m.position.z.floor() as i32,
    );
    let br = w.brightness(x, y, z);
    let wet = m.fluids.in_water || in_rain(m, w) || in_powder_snow(m, w) || m.in_powder_snow;
    br > 0.5 && m.random.next_float() * 30.0 < (br - 0.4) * 2.0 && !wet && w.can_see_sky(x, y, z)
}
// Keep the actual selectors (including partially updated goal state) attached to
// their mob even when a goal callback unwinds. Drop does not invoke goal code.
struct BorrowedGoalSelectors<'a> {
    mob: &'a mut Mob,
    targets: GoalSelector,
    goals: GoalSelector,
}

impl<'a> BorrowedGoalSelectors<'a> {
    fn new(mob: &'a mut Mob) -> Self {
        Self {
            targets: std::mem::take(&mut mob.ai.targets),
            goals: std::mem::take(&mut mob.ai.goals),
            mob,
        }
    }
}

impl Drop for BorrowedGoalSelectors<'_> {
    fn drop(&mut self) {
        self.mob.ai.targets = std::mem::take(&mut self.targets);
        self.mob.ai.goals = std::mem::take(&mut self.goals);
    }
}

#[cfg(test)]
mod selector_restore_tests {
    use super::super::goals::Goal;
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct TrackedGoal {
        progress: i32,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for TrackedGoal {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Goal for TrackedGoal {
        fn flags(&self) -> u8 {
            0
        }
        fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
            false
        }
        fn box_clone(&self) -> Box<dyn Goal> {
            panic!("selectors must not clone goals")
        }
        fn set_start_delay(&mut self, ticks: i32) {
            self.progress = ticks;
        }
        fn name(&self) -> &'static str {
            if self.progress == 7 {
                "progress"
            } else {
                "initial"
            }
        }
    }

    // Exercise the production acquisition site, not just the restoration helper.
    // This is an engine Goal callback, not native Game integration coverage.
    #[test]
    #[ignore = "daily"]
    fn server_ai_step_restores_live_selectors_after_goal_unwind() {
        use crate::mobs::world::with_resource_test_world;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::sync::{Mutex, atomic::AtomicBool};
        #[derive(Debug)]
        struct Probe {
            calls: Arc<Mutex<Vec<(usize, u32)>>>,
            panic: Arc<AtomicBool>,
            ticks: u32,
        }
        impl Goal for Probe {
            fn flags(&self) -> u8 {
                0
            }
            fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
                true
            }
            fn every_tick(&self) -> bool {
                true
            }
            fn box_clone(&self) -> Box<dyn Goal> {
                panic!("live goals must not clone")
            }
            fn tick(&mut self, _: &mut Mob, _: &mut Cx) {
                self.ticks += 1;
                self.calls
                    .lock()
                    .unwrap()
                    .push((self as *const Self as usize, self.ticks));
                if self.panic.load(Ordering::SeqCst) {
                    std::panic::resume_unwind(Box::new(73_u32));
                }
            }
        }
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
        );
        assert!(
            with_resource_test_world(
                &root,
                &[],
                true,
                true,
                |_, _| {},
                |world, _| {
                    for panic_at in [None, Some(0), Some(1)] {
                        // Full selector tick and the alternate running-only production path.
                        for age in [0, 2] {
                            let kind = crate::mobs::sensor_scan_tests::kind();
                            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
                            mob.age = 0;
                            mob.ai.targets.clear();
                            mob.ai.goals.clear();
                            let calls = [
                                Arc::new(Mutex::new(Vec::new())),
                                Arc::new(Mutex::new(Vec::new())),
                            ];
                            let panics = [
                                Arc::new(AtomicBool::new(false)),
                                Arc::new(AtomicBool::new(false)),
                            ];
                            let mut identities = [0; 2];
                            for (i, selector) in [&mut mob.ai.targets, &mut mob.ai.goals]
                                .into_iter()
                                .enumerate()
                            {
                                let goal = Box::new(Probe {
                                    calls: calls[i].clone(),
                                    panic: panics[i].clone(),
                                    ticks: 0,
                                });
                                identities[i] = &*goal as *const Probe as usize;
                                selector.add_boxed(1, goal);
                            }
                            let mut level = crate::health::TestLevel::new(0);
                            let mut events = Vec::new();
                            // Start both live goals before testing either production branch.
                            server_ai_step(&mut mob, world, &mut level.level(), &mut events);
                            mob.age = age;
                            if let Some(i) = panic_at {
                                panics[i].store(true, Ordering::SeqCst);
                            }
                            let result = catch_unwind(AssertUnwindSafe(|| {
                                server_ai_step(&mut mob, world, &mut level.level(), &mut events);
                            }));
                            if panic_at.is_some() {
                                assert_eq!(*result.unwrap_err().downcast::<u32>().unwrap(), 73);
                            } else {
                                assert!(result.is_ok());
                            }
                            assert_eq!(
                                mob.ai.targets.entries().count(),
                                1,
                                "actual target selector lost"
                            );
                            assert_eq!(
                                mob.ai.goals.entries().count(),
                                1,
                                "actual action selector lost"
                            );
                            for panic in &panics {
                                panic.store(false, Ordering::SeqCst);
                            }
                            server_ai_step(&mut mob, world, &mut level.level(), &mut events);
                            for i in 0..2 {
                                let expected_ticks =
                                    if panic_at == Some(0) && i == 1 { 2 } else { 3 };
                                let expected: Vec<_> = (1..=expected_ticks)
                                    .map(|tick| (identities[i], tick))
                                    .collect();
                                assert_eq!(
                                    *calls[i].lock().unwrap(),
                                    expected,
                                    "live goal identity/progress {i}"
                                );
                            }
                        }
                    }
                }
            ),
            "production-site proof requires prepared resources"
        );
    }

    // Lower-layer ownership test, not a native callback integration test.
    #[test]
    fn selector_ownership_survives_normal_return_and_unwind() {
        for unwind in [false, true] {
            let mut mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
            mob.ai.targets.clear();
            mob.ai.goals.clear();
            let drops = Arc::new(AtomicUsize::new(0));
            for selector in [&mut mob.ai.targets, &mut mob.ai.goals] {
                selector.add(
                    1,
                    TrackedGoal {
                        progress: 0,
                        drops: drops.clone(),
                    },
                );
            }
            let payload = Arc::new(17);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut borrowed = BorrowedGoalSelectors::new(&mut mob);
                borrowed.targets.set_start_delay("initial", 7);
                borrowed.goals.set_start_delay("initial", 7);
                borrowed.targets.disabled = 3;
                borrowed.goals.disabled = 5;
                if unwind {
                    std::panic::resume_unwind(Box::new(payload.clone()));
                }
            }));
            if unwind {
                let caught = result.unwrap_err().downcast::<Arc<i32>>().unwrap();
                assert!(Arc::ptr_eq(&caught, &payload));
            } else {
                assert!(result.is_ok());
            }
            assert!(mob.ai.targets.has("progress"));
            assert!(mob.ai.goals.has("progress"));
            assert_eq!(mob.ai.targets.disabled, 3);
            assert_eq!(mob.ai.goals.disabled, 5);
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(mob);
            assert_eq!(drops.load(Ordering::SeqCst), 2);
        }
    }
}

/// Mob.serverAiStep
pub(super) fn server_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    // A Java `int`: past `i32::MAX` it wraps, as there.
    m.ai.no_action_time = m.ai.no_action_time.wrapping_add(1);
    // A happy ghast set up for its age (ageBoundaryReached runs at once).
    if m.species() == Species::HappyGhast {
        super::happy_ghast::ensure_setup(m, Some(w));
    }
    // A saddled mount follows its rider instead of its goals.
    if super::equine::controlled(m) {
        super::equine::ridden_step(m, w.loop_context);
        return;
    }
    super::riders::drive(m, w);
    let tick_count = super::dab::selector_tick(m);
    let id_based = tick_count + i64::from(m.id);
    {
        let BorrowedGoalSelectors {
            mob,
            targets,
            goals,
        } = &mut BorrowedGoalSelectors::new(m);
        let mut cx = Cx { w, level, events };
        if id_based % 2 != 0 && tick_count > 1 {
            targets.tick_running(mob, &mut cx, false);
            goals.tick_running(mob, &mut cx, false);
        } else {
            targets.tick(mob, &mut cx);
            goals.tick(mob, &mut cx);
        }
    }
    // A creaking looked at stands still: its navigation, move, look and jump
    // controls wait (CreakingPathNavigation and the Creaking*Control).
    let frozen = super::creaking::is_creaking(m) && !super::creaking::can_move(m);
    // The bee's navigation waits while it pollinates.
    if !frozen && !super::bee::pollinating(m) {
        m.nav_tick(w);
    }
    if super::monster_brain::has_brain(m.species()) {
        // Breeze/Creaking.customServerAiStep: the brain, then updateActivity.
        super::monster_brain::tick(m, w, level, events);
    }
    if m.species() == Species::Villager {
        // Villager.customServerAiStep: the brain, then the raid sound roll.
        super::villager::lobotomy::brain_tick(m, w, level, events);
        if let Some(player) = super::villager::ext_mut(m).last_traded_player.take() {
            events.push(MobEvent::VillagerTrade { mob: m.id, player });
        }
        let _ = m.random.next_int_bound(100);
    }
    if m.species() == Species::Warden {
        // Warden.customServerAiStep: the brain and its bookkeeping.
        super::warden::server_ai_step(m, w, level, events);
    }
    // Sniffer and HappyGhast.customServerAiStep: the brain (a happy ghast's
    // only while it is a ghastling), then updateActivity.
    if m.species() == Species::Sniffer
        || m.species() == Species::HappyGhast && m.is_baby()
        || m.species() == Species::CopperGolem
        || m.species() == Species::Axolotl
        || m.species() == Species::Tadpole
    {
        super::animal_brain::tick(m, w, level, events);
    }
    {
        let mut cx = Cx { w, level, events };
        custom_server_ai_step(m, &mut cx);
    }
    if frozen {
        m.ai.jumping = false;
        return;
    }
    if !super::axolotl::is_playing_dead(m) {
        m.move_control_tick(w, events);
        // BeeLookControl: still while angry.
        if !(m.species() == Species::Bee && super::bee::angry(m, w)) {
            m.look_control_tick();
        }
    }
    m.jump_control_tick();
}
/// `Armadillo.pickNextScuteDropTime`: 5 to 10 minutes.
pub(super) fn next_scute_time(m: &mut Mob) -> i32 {
    m.random.next_int_bound(20 * 60 * 5).unwrap_or(0) + 20 * 60 * 5
}
/// `Armadillo.customServerAiStep`: the scute timer keeps counting down and is
/// only reset once `shouldDropLoot` (not a baby, `mob_drops` on) lets it shed.
fn armadillo_server_ai_step(m: &mut Mob, mob_drops: bool, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    m.extra.egg_time -= 1;
    if m.extra.egg_time <= 0 && !m.is_baby() && mob_drops {
        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
        events.push(MobEvent::ShedScute {
            mob: m.id,
            at: [m.position.x, m.position.y, m.position.z],
            pitch,
        });
        m.extra.egg_time = next_scute_time(m);
    }
}
fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    match m.species() {
        Species::Rabbit => {
            let target = m.target_info(cx.w).map(|t| t.position);
            super::rabbit::server_ai_step(m, target)
        }
        Species::Cat | Species::Ocelot => super::tame::feline_server_ai_step(m),
        // Armadillo.customServerAiStep: the brain and its activity, then the scute.
        Species::Armadillo => {
            super::armadillo::server_ai_step(m, cx);
            armadillo_server_ai_step(m, cx.w.mob_drops, cx.events);
        }
        Species::Sheep => {}
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::Skeleton
        | Species::Stray
        | Species::Bogged
        | Species::Parched
        | Species::WitherSkeleton
        | Species::ZombifiedPiglin => undead::custom_server_ai_step(m, cx),
        Species::Vindicator => super::illager::vindicator_server_ai_step(m),
        Species::Enderman => super::enderman::custom_server_ai_step(m, cx),
        Species::Blaze => super::nether::blaze_server_ai_step(m, cx),
        Species::ElderGuardian => super::guardian::elder_server_ai_step(m, cx),
        Species::Wither if m.alive() => super::wither::custom_server_ai_step(m, cx.w, cx.events),
        // SpellcasterIllager.customServerAiStep.
        Species::Evoker | Species::Illusioner if m.extra.counter > 0 => m.extra.counter -= 1,
        Species::Bat => super::bat::server_ai_step(m, cx.w, cx.events),
        Species::Bee => super::bee::server_ai_step(m, cx),
        Species::Frog => super::frog::server_ai_step(m, cx.w),
        // Hoglin and Piglin.customServerAiStep: the brain, then updateActivity.
        Species::Piglin | Species::Hoglin => super::piglin::brain_step(m, cx),
        Species::Zoglin => super::piglin::zoglin_step(m, cx),
        _ => {}
    }
}
fn species_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    if family(m.species()) == Family::Aquatic {
        aquatic::ai_step(m, w, level, events);
    }
    match m.species() {
        Species::Enderman => super::enderman::ai_step(m, w, events),
        // Endermite.aiStep: `counter` is the life; an endermite not required
        // to persist is discarded at 2400.
        Species::Endermite => super::hostile::endermite_lifetime(m),
        Species::Wolf => super::tame::ai_step(m, w, events),
        Species::Cat => super::tame::cat_ai_step(m, events),
        Species::IronGolem => super::golem::iron_ai_step(m, w),
        Species::Piglin | Species::PiglinBrute | Species::Hoglin => {
            super::piglin::ai_step(m, w, level.random(), events)
        }
        Species::Zoglin if m.extra.anim_ticks > 0 => m.extra.anim_ticks -= 1,
        s if super::equine::is_equine(s) => super::equine::ai_step(m, w, events),
        Species::Phantom => super::phantom::flap_sound(m, events),
        Species::Warden => super::warden::ai_step(m, events),
        Species::Guardian | Species::ElderGuardian => super::guardian::ai_step(m, w, events),
        Species::Vex => super::vex::ai_step(m, w, level),
        Species::Ravager => super::ravager::ai_step(m, w, events),
        Species::Wither => super::wither::ai_step(m, w, level.random(), events),
        Species::Allay => super::allay::ai_step(m, w),
        Species::Bee => super::bee::ai_step(m, w),
        Species::PolarBear => super::polar_bear::ai_step(m, w),
        Species::Breeze => super::breeze::ai_step(m),
        Species::Sniffer => super::sniffer::ai_step(m, events),
        Species::HappyGhast => super::happy_ghast::ai_step(m, w),
        Species::Villager => super::villager::ai_step(m, w, events),
        Species::WanderingTrader => {
            super::villager::ai_step(m, w, events);
            super::trader::ai_step(m, w);
        }
        Species::Shulker => {
            super::shulker::tick(m, w, events);
            super::shulker::ai_step(m, w, level.random(), events);
        }
        Species::SnowGolem => super::golem::snow_ai_step(m, w, events),
        Species::CopperGolem => super::copper_golem::ai_step(m, w, events),
        Species::Slime | Species::MagmaCube => super::cube::ai_step(m, w, events),
        Species::SulfurCube => {
            super::cube::ai_step(m, w, events);
            super::sulfur_cube::ai_step(m, w, events);
        }
        Species::Nautilus | Species::ZombieNautilus => {
            super::nautilus::ai_step(m, w, level.random(), events)
        }
        Species::Chicken => farm::chicken_ai_step(m, w, events),
        Species::Parrot => super::parrot::calculate_flapping(m),
        Species::Armadillo => super::armadillo::tick(m),
        Species::Frog => super::frog::tick(m),
        Species::Goat => super::goat::tick(m),
        Species::Fox => super::fox::finish_ai_step(m, w, events),
        Species::Turtle => super::turtle::ai_step(m, w, events),
        Species::Sheep if m.extra.anim_ticks > 0 => {
            m.extra.anim_ticks -= 1;
        }
        _ => {}
    }
}
/// `Entity.checkDespawn` alone, for a mob outside the entity-ticking
/// chunks. Returns false when the mob is gone.
pub fn despawn_check(m: &mut Mob, w: &MobWorld) -> bool {
    if m.removed {
        return false;
    }
    if !check_despawn(m, w) {
        m.removed = true;
        return false;
    }
    true
}
/// `Mob.requiresCustomPersistence` and its overrides: a passenger or a leashed
/// mob, a raider in a raid, an enderman carrying a block, a tame nautilus, a
/// sulfur cube holding a block or let out of a bucket, a fish or axolotl let
/// out of a bucket and every tadpole (`AbstractFish`/`Axolotl`: `fromBucket()`).
/// The spawn caps do not count these mobs either (`NaturalSpawner.createState`).
pub fn requires_custom_persistence(m: &Mob) -> bool {
    super::riders::keeps_alive(m)
        || super::aquatic::from_bucket(m)
        || m.extra.leash.is_some()
        || super::raider::state(m).is_some_and(|s| s.raid_id.is_some())
        || m.species() == Species::Enderman && m.extra.carried.is_some()
        || super::nautilus::is_nautilus(m.species()) && m.extra.tame
        || m.species() == Species::SulfurCube
            && (super::sulfur_cube::has_body(m) || m.extra.scratch[3] != 0)
}
/// Mob.checkDespawn, before the tick (players are the only despawn
/// reference). Returns false when the mob is discarded.
fn check_despawn(m: &mut Mob, w: &MobWorld) -> bool {
    // A mannequin is a LivingEntity, which never despawns.
    if m.species() == Species::Mannequin {
        return true;
    }
    // `MobDespawnEvent` comes first in NeoForge's `Mob.checkDespawn`, so an answer
    // other than `Default` skips vanilla's checks and their `nextInt(800)` draw.
    match std::mem::take(&mut m.despawn) {
        mod_api::Despawn::Allow => return false,
        mod_api::Despawn::Deny => return true,
        _ => {}
    }
    // Mob.checkDespawn discards disallowed types even when persistent or NoAI.
    if w.difficulty.effective == 0.0 && !super::spawn_rules::allowed_in_peaceful(m.species()) {
        return false;
    }
    let Some(p) = w.nearest_player else {
        return true;
    };
    let d = m.distance_sqr(p);
    let persistent = m.persistent || requires_custom_persistence(m);
    // removeWhenFarAway: animals, villagers, golems, patrols and raiders,
    // zombie villagers being cured and ridden mobs stay, bar the two felines.
    let feline_goes = feline_removed_when_far(m.species(), m.extra.tame || m.extra.trusting, m.age);
    let removable = !(never_removed_when_far(m.species())
        || super::aquatic::named_keeps_it(m)
        || m.kind.category == "creature" && m.kind.flags.animal && !m.extra.jockey && !feline_goes
        || super::raider::keeps_alive(m, d)
        || super::zombie_villager::keeps_alive(m)
        || super::riders::keeps_alive(m));
    // MobCategory.getDespawnDistance / getNoDespawnDistance
    let despawn = if m.kind.category == "water_ambient" {
        64.0
    } else {
        128.0
    };
    if !persistent && d > despawn * despawn && removable {
        return false;
    }
    if !persistent
        && m.ai.no_action_time > 600
        && m.random.next_int_bound(800).unwrap_or(1) == 0
        && d > 32.0 * 32.0
        && removable
    {
        return false;
    }
    if d < 32.0 * 32.0 {
        m.ai.no_action_time = 0;
    }
    true
}
/// `Cat.removeWhenFarAway` and `Ocelot.removeWhenFarAway`: an untamed cat or an
/// untrusting ocelot is removed once it is older than 2400 ticks. That is
/// `Entity.tickCount`, not `AgeableMob.age`, which only counts a baby up to
/// adulthood. `Animal.removeWhenFarAway` keeps every other animal.
fn feline_removed_when_far(s: Species, tamed: bool, ticks: u32) -> bool {
    matches!(s, Species::Cat | Species::Ocelot) && !tamed && ticks > 2400
}
/// The types whose `removeWhenFarAway` is always false (`Villager`,
/// `WanderingTrader`, `AbstractGolem`: iron, snow and copper golems and
/// shulkers; `Allay`, `Warden`), and the wither, whose `checkDespawn` never
/// removes it for distance.
fn never_removed_when_far(s: Species) -> bool {
    matches!(
        s,
        Species::Villager
            | Species::WanderingTrader
            | Species::IronGolem
            | Species::SnowGolem
            | Species::CopperGolem
            | Species::Shulker
            | Species::Allay
            | Species::Warden
            | Species::Wither
    )
}
/// The movement speed multiplier of the mob's transient `minecraft:baby`
/// modifier (`Zombie.SPEED_MODIFIER_BABY` +0.5, `Piglin.SPEED_MODIFIER_BABY`
/// +0.2, both `ADD_MULTIPLIED_BASE`): the same modifier `/attribute` reads, so
/// one taken away with `modifier remove` no longer speeds the baby up.
pub fn speed_bonus(m: &Mob) -> f64 {
    match super::finalize::transient(m, super::finalize::Attr::MovementSpeed) {
        Some((amount, super::finalize::Op::AddMultipliedBase)) => 1.0 + amount,
        _ => 1.0,
    }
}
/// The ambient, hurt and death sounds of species that pick them by state
/// (`getHurtSound` / `getDeathSound` overrides); the type's otherwise.
/// `Goat.getAmbientSound`, `Goat.getHurtSound`, and `Goat.getDeathSound`
/// select the screaming voices from the goat's saved flag.
pub(super) fn inherited_voice_sounds(m: &Mob) -> [&'static str; 3] {
    match m.species() {
        Species::Goat if m.extra.screaming => [
            "entity.goat.screaming.ambient",
            "entity.goat.screaming.hurt",
            "entity.goat.screaming.death",
        ],
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::sounds(m),
        Species::SulfurCube => super::sulfur_cube::sounds(m),
        Species::Armadillo => super::armadillo::sounds(m),
        // HappyGhast.getHurtSound and getDeathSound: the ghastling's by age.
        Species::HappyGhast => super::happy_ghast::sounds(m),
        // Panda.getAmbientSound.
        Species::Panda => {
            let ambient = if super::panda::is_aggressive(m) {
                "entity.panda.aggressive_ambient"
            } else if super::panda::is_worried(m) {
                "entity.panda.worried_ambient"
            } else {
                m.kind.sounds[0]
            };
            [ambient, m.kind.sounds[1], m.kind.sounds[2]]
        }
        // Horse.getAmbientSound, getHurtSound and getDeathSound by age.
        Species::Horse if m.is_baby() => [
            "entity.baby_horse.ambient",
            "entity.baby_horse.hurt",
            "entity.baby_horse.death",
        ],
        // Drowned, Guardian and ElderGuardian: the hurt and death sounds by
        // water or land.
        Species::Drowned if m.fluids.in_water => [
            "entity.drowned.ambient_water",
            "entity.drowned.hurt_water",
            "entity.drowned.death_water",
        ],
        Species::Guardian if !m.fluids.in_water => [
            m.kind.sounds[0],
            "entity.guardian.hurt_land",
            "entity.guardian.death_land",
        ],
        Species::ElderGuardian if !m.fluids.in_water => [
            m.kind.sounds[0],
            "entity.elder_guardian.hurt_land",
            "entity.elder_guardian.death_land",
        ],
        // Slime and MagmaCube.getHurtSound and getDeathSound: tiny ones.
        Species::Slime if m.extra.size <= 1 => [
            m.kind.sounds[0],
            "entity.slime.hurt_small",
            "entity.slime.death_small",
        ],
        Species::MagmaCube if m.extra.size <= 1 => [
            m.kind.sounds[0],
            "entity.magma_cube.hurt_small",
            "entity.magma_cube.death_small",
        ],
        // Shulker.getHurtSound: a closed shell.
        Species::Shulker if m.extra.counter == 0 => [
            m.kind.sounds[0],
            "entity.shulker.hurt_closed",
            m.kind.sounds[2],
        ],
        // Creaking.getHurtSound: bound to a heart it sways, unbound it hands
        // the blow to `Monster.getHurtSound`. `Monster`'s own sounds are the
        // hostile pair, which the giant keeps too.
        Species::Creaking => [
            m.kind.sounds[0],
            if super::creaking::heart_bound(m) {
                "entity.creaking.sway"
            } else {
                "entity.hostile.hurt"
            },
            m.kind.sounds[2],
        ],
        Species::Giant => [
            m.kind.sounds[0],
            "entity.hostile.hurt",
            "entity.hostile.death",
        ],
        // Turtle.getHurtSound and getDeathSound by age.
        Species::Turtle if m.is_baby() => [
            m.kind.sounds[0],
            "entity.turtle.hurt_baby",
            "entity.turtle.death_baby",
        ],
        // SkeletonHorse.getAmbientSound: its own under water.
        Species::SkeletonHorse if m.fluids.eye_in_water => [
            "entity.skeleton_horse.ambient_water",
            m.kind.sounds[1],
            m.kind.sounds[2],
        ],
        // The mob's sound variant set, the baby one for a baby.
        Species::Pig | Species::Cow | Species::Chicken | Species::Wolf | Species::Cat => {
            use super::finalize::variant_sound as v;
            match (v(m, "ambient"), v(m, "hurt"), v(m, "death")) {
                (Some(a), Some(h), Some(d)) => [a, h, d],
                _ => m.kind.sounds,
            }
        }
        _ => m.kind.sounds,
    }
}
/// `Animal.mobInteract` with its food in hand: an adult that can fall in
/// love does, a baby grows up by a tenth, and either plays its eating sound
/// (`Strider.mobInteract` plays its own afterwards). `None` when the animal
/// does not take the food; otherwise the sound to play, if any.
pub fn feed_animal(
    m: &mut Mob,
    level: &mut impl RandomSource,
    love_cause: Option<[i32; 4]>,
) -> Option<Option<(&'static str, f32, f32)>> {
    feed_animal_with_love_decision(m, level, love_cause, true)
}
/// Apply the ordinary feed, with the server's decision about entering love mode.
/// A refused entry still accepts the food and plays the eating sound.
pub fn feed_animal_with_love_decision(
    m: &mut Mob,
    level: &mut impl RandomSource,
    love_cause: Option<[i32; 4]>,
    enter_love: bool,
) -> Option<Option<(&'static str, f32, f32)>> {
    if !takes_food(m) {
        return None;
    }
    if m.extra.age != 0 {
        // AgeableMob.ageUp(getSpeedUpSecondsWhenFeeding(-age), true): a tenth.
        let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
        age_up(m, secs, true);
    } else if enter_love {
        m.extra.in_love = 600;
        m.extra.love_player = true;
        // Animal.setInLove: the player who fed it, for a fox baby to trust.
        m.extra.love_cause = love_cause;
    }
    let mut sound = eating_sound(m, level);
    if m.species() == Species::Strider && !m.silent {
        let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
        sound = Some(("entity.strider.eat", 1.0, pitch));
    }
    Some(sound)
}
/// `AgeableMob.canAgeUp`: a baby that no age lock holds back.
pub fn can_age_up(m: &Mob) -> bool {
    m.is_baby() && !m.extra.age_locked
}
/// `AgeableMob.canUseGoldenDandelion`: a golden dandelion in hand, a baby
/// whose particle timer has run out, and a type outside
/// `#minecraft:cannot_be_age_locked`.
pub fn can_use_golden_dandelion(
    item: &str,
    baby: bool,
    cooldown: i32,
    cannot_be_age_locked: bool,
) -> bool {
    item == "minecraft:golden_dandelion" && baby && cooldown == 0 && !cannot_be_age_locked
}
/// `AgeableMob.setAgeLockedData`: a golden dandelion flips the lock, puts the
/// age back to a baby's starting age (`getBabyStartAge`) and starts the
/// particle timer (40 ticks).
pub fn set_age_locked_data(m: &mut Mob) {
    m.extra.age_locked = !m.extra.age_locked;
    let start = super::finalize::baby_start_age(m.species());
    super::finalize::set_age(m, start, None);
    m.extra.age_lock_particle_timer = 40;
}
/// `AgeableMob.makeAgeLockedParticle`: while the golden dandelion's timer
/// runs, every other tick one pause (locked) or reset (unlocked) growth
/// particle rises above the mob (`sendParticles`, so the players are sent it).
pub fn make_age_locked_particle(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.extra.age_lock_particle_timer <= 0 {
        return;
    }
    if m.extra.age_lock_particle_timer % 2 == 0 {
        let locked = m.extra.age_locked;
        let (w, h) = (f64::from(m.width()), f64::from(m.height()));
        let r = &mut m.random;
        // getRandomX(1.0), getRandomY(0.2) + getBbHeight() + 0.2F or 0, getRandomZ(1.0).
        let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
        let y = m.position.y
            + h * ((2.0 * r.next_double() - 1.0) * 0.2)
            + h
            + if locked { f64::from(0.2_f32) } else { 0.0 };
        let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
        events.push(MobEvent::LevelParticles {
            kind: if locked {
                "pause_mob_growth"
            } else {
                "reset_mob_growth"
            },
            at: [x, y, z],
            spread: [0.0; 3],
            speed: 0.0,
            count: 1,
        });
    }
    m.extra.age_lock_particle_timer -= 1;
}
/// `AgeableMob.ageUp(seconds, forced)`: both writes go through `setAge`, whose
/// crossing is where a species' `ageBoundaryReached` runs (a goat's and a
/// hoglin's attack damage, a sulfur cube's size); a field write skipped it.
pub fn age_up(m: &mut Mob, seconds: i32, forced: bool) {
    let age = m.extra.age;
    let grown = (age + seconds * 20).min(0);
    super::finalize::set_age(m, grown, None);
    if forced {
        m.extra.forced_age += grown - age;
        if m.extra.forced_age_timer == 0 {
            m.extra.forced_age_timer = 40;
        }
    }
    if m.extra.age == 0 {
        let forced_age = m.extra.forced_age;
        super::finalize::set_age(m, forced_age, None);
    }
}
/// `Animal.playEatingSound` (empty) and its overrides, as (sound, volume,
/// pitch); `level` is the level's random. Pigs and cats play their sound
/// variant's set.
pub fn eating_sound(
    m: &mut Mob,
    level: &mut impl RandomSource,
) -> Option<(&'static str, f32, f32)> {
    // Mth.randomBetween(level.getRandom(), 0.8, 1.2).
    let mut between = || level.next_float() * 0.4 + 0.8;
    let (sound, volume, pitch, entity_sound) = match m.species() {
        // makeSound: getSoundVolume, getVoicePitch.
        Species::Armadillo | Species::Pig | Species::SulfurCube => {
            let sound = match m.species() {
                Species::Armadillo => "entity.armadillo.eat",
                Species::Pig => {
                    super::finalize::variant_sound(m, "eat").unwrap_or("entity.pig.eat")
                }
                _ => "entity.small_sulfur_cube.eat",
            };
            let pitch = m.voice_pitch();
            (sound, sound_volume(m), pitch, true)
        }
        // playSound(sound, 1, 1).
        Species::Cat => (
            super::finalize::variant_sound(m, "eat").unwrap_or("entity.cat.eat"),
            1.0,
            1.0,
            true,
        ),
        Species::Fox => ("entity.fox.eat", 1.0, 1.0, true),
        // level().playSound(null, this, sound, NEUTRAL, volume, pitch).
        Species::Frog => ("entity.frog.eat", 2.0, 1.0, false),
        Species::Goat if m.extra.screaming => ("entity.goat.screaming.eat", 1.0, between(), false),
        Species::Goat => ("entity.goat.eat", 1.0, between(), false),
        Species::Sniffer => ("entity.sniffer.eat", 1.0, between(), false),
        _ => return None,
    };
    // Entity.playSound is skipped for a silent entity.
    if entity_sound && m.silent {
        return None;
    }
    Some((sound, volume, pitch))
}
/// Whether `playEatingSound` binds the sound to the animal: the frog's, the
/// goat's and the sniffer's are `level().playSound(null, this, ..., NEUTRAL,
/// ...)`, the others `Entity.playSound` at its position.
pub fn eating_sound_is_bound(species: Species) -> bool {
    matches!(species, Species::Frog | Species::Goat | Species::Sniffer)
}
pub fn sound_volume(m: &Mob) -> f32 {
    match m.species() {
        Species::Ghast => 5.0,
        Species::HappyGhast => super::happy_ghast::volume(m),
        Species::Warden => 4.0,
        Species::Slime | Species::MagmaCube | Species::SulfurCube => {
            0.4 * m.extra.size.max(1) as f32
        }
        Species::Bat => 0.1,
        Species::Squid
        | Species::GlowSquid
        | Species::Cow
        | Species::Mooshroom
        | Species::Bee
        | Species::Allay
        | Species::Wolf => 0.4,
        s if super::equine::is_equine(s) => 0.8,
        _ => 1.0,
    }
}
/// PathfinderMob.getWalkTargetValue (Animal and Monster overrides).
pub fn walk_target_value(m: &Mob, w: &MobWorld, p: [i32; 3]) -> f32 {
    let light = || w.brightness(p[0], p[1], p[2]) - 0.5;
    // Guardian.getWalkTargetValue: water first.
    if family(m.species()) == Family::Guardian && w.fluid(p[0], p[1], p[2]) == Fluid::Water {
        return 10.0 + light();
    }
    // HappyGhast.getWalkTargetValue: open air, best just above the ground.
    if m.species() == Species::HappyGhast {
        let air = |y: i32| w.facts.is_air(w.world.block(p[0], y, p[2]));
        return if !air(p[1]) {
            0.0
        } else if air(p[1] - 1) && !air(p[1] - 2) {
            10.0
        } else {
            5.0
        };
    }
    if m.species() == Species::Strider {
        return super::strider::walk_target_value(m, w, p);
    }
    if m.species() == Species::Turtle {
        return super::turtle::walk_target_value(m, w, p, light());
    }
    // Hoglin.getWalkTargetValue: nowhere near a repellent, crimson nylium best.
    if m.species() == Species::Hoglin {
        let near = m.extra.repellent.is_some_and(|r| {
            let d = |i: usize| i64::from(r[i] - p[i]);
            d(0) * d(0) + d(1) * d(1) + d(2) * d(2) < 64
        });
        return if near {
            -1.0
        } else if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:crimson_nylium" {
            10.0
        } else {
            0.0
        };
    }
    // Bee.getWalkTargetValue: any air block.
    if m.species() == Species::Bee {
        return if w.facts.is_air(w.world.block(p[0], p[1], p[2])) {
            10.0
        } else {
            0.0
        };
    }
    // AbstractNautilus, Axolotl and Creaking.getWalkTargetValue: everywhere alike.
    if matches!(
        m.species(),
        Species::Nautilus | Species::ZombieNautilus | Species::Axolotl | Species::Creaking
    ) {
        return 0.0;
    }
    // Silverfish.getWalkTargetValue: a block sitting on an infested host is
    // the best place to walk to, so it goes back into the stone.
    if m.species() == Species::Silverfish
        && super::hostile::is_infested_host(w.block_name(p[0], p[1] - 1, p[2]))
    {
        return 10.0;
    }
    // Enderman.getWalkTargetValue, Pillager's and Warden's: every position scores
    // the same, so their strolls pick one of the ten at random.
    if matches!(
        m.species(),
        Species::Enderman | Species::Pillager | Species::Warden
    ) {
        return 0.0;
    }
    if m.hostile()
        && !matches!(
            m.species(),
            Species::Slime | Species::MagmaCube | Species::Ghast | Species::Phantom
        )
    {
        return -light();
    }
    // MushroomCow.getWalkTargetValue replaces Animal's: mycelium is the best
    // ground and any other is scored by the light, grass or not.
    if m.species() == Species::Mooshroom {
        return if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:mycelium" {
            10.0
        } else {
            light()
        };
    }
    if m.kind.flags.animal {
        if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:grass_block" {
            return 10.0;
        }
        return light();
    }
    0.0
}
/// Species damage filters (LivingEntity.hurtServer overrides).
pub fn accepts_damage(
    m: &mut Mob,
    source: &Damage,
    _amount: f32,
    attack: &crate::health::Attack,
) -> bool {
    match m.species() {
        Species::Breeze => super::breeze::accepts_damage(m, source),
        Species::Allay => !attack
            .player_uuid
            .is_some_and(|uuid| super::allay::is_liked_player(m, source.attacker, uuid)),
        Species::Creaking => super::creaking::accepts_damage(m, source),
        Species::Wither => super::wither::accepts(m, source, attack, false),
        Species::Warden => super::warden::accepts_damage(m, source),
        Species::Shulker => super::shulker::accepts(m, source, attack),
        // Bat.hurtServer: a hit wakes it.
        Species::Bat => {
            super::bat::hurt(m);
            true
        }
        Species::Bee => {
            super::bee::hurt(m);
            true
        }
        // Wolf.hurtServer, Parrot.hurtServer: a hit it isn't invulnerable to
        // stands it up before LivingEntity.hurtServer, even one the hurt
        // frames then turn away.
        Species::Wolf | Species::Parrot => {
            super::tame::stand_when_hurt(m);
            true
        }
        // Panda.hurtServer: up first, whatever the damage.
        Species::Panda => {
            super::panda::hurt(m);
            true
        }
        Species::Silverfish => {
            super::hostile::silverfish_hurt(m, source);
            true
        }
        Species::Enderman => super::enderman::on_damage(m, source),
        Species::WitherSkeleton | Species::Blaze | Species::MagmaCube | Species::Strider
            if source.is_fire() =>
        {
            false
        }
        _ => true,
    }
}
/// Reactions after taking damage (sounds, anger).
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    m.ai.no_action_time = 0;
    match m.species() {
        Species::Armadillo => super::armadillo::after_hurt(m, source),
        Species::Warden => super::warden::hurt(m, source),
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::after_hurt(m, source),
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin => super::undead::after_hurt(m, source),
        // PiglinAi.wasHurtBy: drop what is being admired, no admiring for
        // a while after a player's hit, and anger.
        Species::Piglin if source.attacker == Some(Who::Player) => {
            super::piglin::stop_admiring(m);
            m.extra.egg_time = 400;
            m.extra.anger = 600;
        }
        _ => {}
    }
}
/// Mob.doHurtTarget: attack damage (+ species extras) to the target.
pub use super::combat::do_hurt_target;
/// Mob.ate: `gameEvent(GameEvent.EAT)`, then (Sheep.ate) the wool grows back and a baby
/// grows.
pub fn ate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::GameEvent {
        event: "minecraft:eat",
        at: [m.position.x, m.position.y, m.position.z],
        source: m.id,
        state: horizon_world::registry::BlockStateId(0),
    });
    if m.species() == Species::Sheep {
        m.extra.sheared = false;
        // Sheep.ate: only a baby `canAgeUp` (no age lock holds it) grows.
        if can_age_up(m) {
            // AgeableMob.ageUp(60)
            age_up(m, 60, false);
        }
    }
}
/// `NeutralMob.playerDied` for a mob that is neutral: the anger at the
/// player who died ends. The caller checks `forgive_dead_players`.
pub fn forgives_dead_player(m: &Mob) -> bool {
    match m.species() {
        Species::Bee => super::bee::get(m).and_then(|bee| bee.angry_at) == Some(Who::Player),
        Species::PolarBear => m.extra.flag2,
        Species::Enderman | Species::ZombifiedPiglin => {
            m.ai.target == Some(Who::Player) || m.extra.angry_at == Some(Who::Player)
        }
        _ => m.extra.angry_at == Some(Who::Player),
    }
}

/// Apply forgiveness after the caller has dispatched the target change. Anger
/// bookkeeping still occurs if a listener cancels the target change.
pub fn forgive_dead_player_with_change(m: &mut Mob, change: super::TargetChange) {
    if !forgives_dead_player(m) {
        return;
    }
    let previous = m.ai.target;
    forgive_dead_player(m);
    match change {
        super::TargetChange::Keep => m.ai.target = previous,
        super::TargetChange::Set(target) => m.set_target(target),
    }
}

pub fn forgive_dead_player(m: &mut Mob) {
    match m.species() {
        Species::Bee => super::bee::forgive_dead_player(m),
        Species::PolarBear => super::polar_bear::forgive_dead_player(m),
        // The enderman's and zombified piglin's anger is the target they hold
        // (the stare goal, the hurt-by path) and the anger target and timer
        // that outlast it: vanilla's `NeutralMob.playerDied` ends all of it
        // through `stopBeingAngry` when it is the player who died, the last
        // hurt by with it.
        Species::Enderman | Species::ZombifiedPiglin => {
            if forgives_dead_player(m) {
                m.ai.forget_hurt_by(m.age as i32);
                m.ai.target = None;
                super::neutral_anger::stop(m);
            }
        }
        // The iron golem keeps its anger's target as a wolf does
        // (`tame::update_persistent_anger`): `playerDied` ends the anger at
        // that player whether or not it is the golem's target now.
        _ => super::tame::forgive_dead_player(m),
    }
}
/// Species-specific travel (fish, squid, dolphins swim with their own rules).
pub fn custom_travel(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    input: Vec3,
    events: &mut Vec<MobEvent>,
) -> bool {
    if m.species() == Species::Drowned {
        return undead::drowned_travel(m, w, level, input, events);
    }
    if m.species() == Species::Turtle {
        return super::turtle::travel(m, w, level, input, events);
    }
    if m.species() == Species::Frog {
        return super::frog::travel(m, w, level, input, events);
    }
    match family(m.species()) {
        Family::Aquatic => aquatic::travel(m, w, level, input, events),
        Family::Guardian => super::guardian::travel(m, w, level, input, events),
        // Shulkers never move on their own.
        Family::Shulker => true,
        Family::Allay => super::allay::travel(m, w, level, input, events),
        Family::Vex => super::vex::travel(m, w),
        Family::Nautilus => super::nautilus::travel(m, w, level, input, events),
        _ => false,
    }
}

/// `Entity.applyEffectsFromBlocks` for the flowers that give an effect:
/// `WitherRoseBlock.entityInside` (wither for 40 ticks on a living mob that
/// can take it, not in peaceful) and `EyeblossomBlock.entityInside` (poison
/// for 25 ticks on a bee, when the open flower attracts bees). The blocks are
/// the ones the mob's box passes through along this tick's move, as
/// `Entity.checkInsideBlocks` sweeps it, so a mob falling past a flower is
/// inside it on the way.
fn flower_contact_effects(m: &mut Mob, w: &MobWorld) {
    if !m.alive()
        || ((w.difficulty.base == 0 || m.invulnerable_flag) && m.species() != Species::Bee)
    {
        return;
    }
    let inset = f64::from(1.0e-5_f32);
    let box_at_end = m
        .bounds_at(m.position)
        .deflate(Vec3::new(inset, inset, inset));
    let _ = horizon_core::geometry::sweep_intersected_blocks(
        m.previous,
        m.position,
        box_at_end,
        1 << 22,
        |p, _| {
            match w.block_name(p.x, p.y, p.z) {
                "minecraft:wither_rose" => {
                    // World difficulty, not the regional one, and
                    // isInvulnerableTo(wither) (the invulnerable
                    // flag, not the hit timer); a wither and a wither
                    // skeleton refuse the effect in `addEffect`.
                    if w.difficulty.base != 0 && !m.invulnerable_flag {
                        m.add_effect("minecraft:wither", 40, 0);
                    }
                }
                "minecraft:open_eyeblossom"
                    if m.species() == Species::Bee
                        && super::bee::attracts_bees(w, [p.x, p.y, p.z])
                        && !m.has_effect("minecraft:poison") =>
                {
                    m.add_effect("minecraft:poison", 25, 0);
                }
                _ => {}
            }
            // `checkInsideBlocks` stops the sweep once the entity is dead.
            m.alive()
        },
    );
}

/// `LivingEntity.tickEffects`: tick active effects in place, including the
/// consequences of each effect's hurt before starting the next effect.
pub(crate) fn tick_effects(
    m: &mut Mob,
    context: &LoopContext,
    level: &mut DamageLevel<'_>,
    mods: Option<&dyn super::ModEffectTicks>,
    mut after_hurt: impl FnMut(&mut Mob),
) {
    // LivingEntity.tickEffects ticks each active effect with the mob's list
    // still in place: the damage a Poison or Wither tick deals reads the
    // list it finds (Resistance softens the hit, Infested spawns silverfish,
    // a totem adds its effects to it). An effect a tick appends ticks from
    // the next tick on.
    let first = m.extra.effects.len();
    let old_max_absorption = super::armor::max_absorption(m);
    let old_boost_amp = m
        .extra
        .effects
        .iter()
        .find(|e| e.id == "minecraft:health_boost")
        .map(|e| e.amplifier);
    let mut i = 0;
    while i < m.extra.effects.len() && i < first {
        let (effect, amp) = (m.extra.effects[i].id, m.extra.effects[i].amplifier);
        // MobEffectInstance.tickServer: an infinite (-1) effect times its
        // ticks by the mob's tickCount and never counts down.
        let infinite = m.extra.effects[i].duration == -1;
        // MobEffectInstance.tickServer: hasRemainingDuration is false at 0, so the
        // instance ticks nothing; LivingEntity.tickEffects drops it below.
        if !infinite && m.extra.effects[i].duration == 0 {
            i += 1;
            continue;
        }
        let count = if infinite {
            m.age as i32
        } else {
            m.extra.effects[i].duration
        };
        let interval = |base: i32| base.wrapping_shr(amp as u32).max(1);
        // A refused mod effect is left at 0 ticks for the retain below.
        let mut expired = false;
        match effect {
            "minecraft:poison"
                if count % interval(25) == 0 && !m.kind.flags.undead && m.health > 1.0 =>
            {
                m.damage(level, 1.0, Damage::of(DamageKind::Magic));
                after_hurt(m);
            }
            "minecraft:wither" if count % interval(40) == 0 => {
                // WitherMobEffect.applyEffectTick: damageSources().wither().
                m.damage(level, 1.0, Damage::of(DamageKind::Wither));
                after_hurt(m);
            }
            // InstantenousMobEffect: applied on every tick it has time left
            // (`/effect give` keeps the effect listed for its few ticks; the hit
            // frames let a harming one through once).
            "minecraft:instant_damage" | "minecraft:instant_health" if count >= 1 => {
                m.apply_heal_or_harm(
                    context,
                    level,
                    effect == "minecraft:instant_damage",
                    m.kind.flags.undead,
                    amp,
                    1.0,
                );
                after_hurt(m);
            }
            "minecraft:regeneration"
                if count % interval(50) == 0 && !m.kind.flags.undead && m.alive() =>
            {
                m.heal(context, 1.0);
            }
            // AbsorptionMobEffect.applyEffectTick (asked on every tick): false
            // once the absorption is used up, so tickServer drops the effect
            // before it counts down.
            "minecraft:absorption" if m.absorption <= 0.0 => {
                expired = true;
            }
            id if horizon_client_core::potions::is_mod_effect(id) => {
                // tickServer: one with no ticks left goes unasked.
                if !infinite && m.extra.effects[i].duration <= 0 {
                    i += 1;
                    continue;
                }
                if let Some(mods) = mods
                    && let Some(index) = m.extra.effects[i].registry_index
                {
                    // Removed before it counts down.
                    expired = !mods.tick_mod_effect(level, index, m, count, amp);
                    after_hurt(m);
                }
            }
            _ => {}
        }
        if m.extra.effects.get(i).is_some_and(|e| e.id == effect) {
            if !expired {
                if let Some(hidden) = m.extra.effect_chains.iter_mut().find(|e| e.id == effect) {
                    hidden.tick_down();
                }
                if !infinite {
                    m.extra.effects[i].duration -= 1;
                }
            } else {
                m.extra.effects[i].duration = 0;
            }
            if !expired
                && m.extra.effects[i].duration == 0
                && let Some(chain) = m.extra.effect_chains.iter().position(|e| e.id == effect)
            {
                let mut hidden = m.extra.effect_chains.remove(chain);
                if effect == "minecraft:absorption" {
                    m.absorption = m.absorption.min(4.0 * (hidden.amplifier + 1) as f32);
                }
                m.extra.effects[i] = super::StoredEffect::from_instance(effect, &hidden);
                if effect == "minecraft:health_boost" {
                    super::finalize::health_boost_changed(m);
                }
                m.extra.ambient_effects.retain(|e| *e != effect);
                if hidden.ambient {
                    m.extra.ambient_effects.push(effect);
                }
                m.extra.hidden_effects.retain(|e| *e != effect);
                if !hidden.visible {
                    m.extra.hidden_effects.push(effect);
                }
                if let Some(next) = hidden.hidden.take() {
                    m.extra.effect_chains.push(*next);
                }
                m.effects_dirty = true;
            }
        }
        // LivingEntity.tickEffects refreshes a surviving effect every 600 ticks.
        if m.extra
            .effects
            .get(i)
            .is_some_and(|e| e.id == effect && e.duration > 0 && e.duration % 600 == 0)
        {
            m.effects_dirty = true;
        }
        i += 1;
    }
    let mob_id = m.id;
    m.retain_effects(|e| {
        if e.duration > 0 || e.duration == -1 {
            return true;
        }
        // `MobEffectEvent.Expired`: canceling the expiry keeps the effect.
        let mut cancel = false;
        if let Some(mods) = mods
            && let Some(index) = e.registry_index
        {
            mods.effect_expired(mob_id, index, e.amplifier, &mut cancel);
        }
        cancel
    });
    if m.effects_dirty {
        let invisible = m.has_effect("minecraft:invisibility");
        m.needs_sync |= m.invisible != invisible;
        m.invisible = invisible;
        m.effects_dirty = false;
    }

    let max_absorption = super::armor::max_absorption(m);
    if max_absorption != old_max_absorption {
        m.absorption = m.absorption.min(max_absorption);
    }

    if old_boost_amp
        != m.extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:health_boost")
            .map(|e| e.amplifier)
    {
        super::finalize::health_boost_changed(m);
    }
}

/// `Entity.getMovementEmission` is not NONE: only a shulker moves without
/// emitting anything at all, neither events nor sounds.
pub fn emits_movement(m: &Mob) -> bool {
    !matches!(m.species(), Species::Shulker)
}
/// `Entity.getMovementEmission` includes sounds (`MovementEmission.ALL`): a
/// silverfish, an endermite, a bat, a squid and a glow squid, a guardian, an
/// elder guardian and a breeze answer `EVENTS`, which keeps their footfalls and
/// swim strokes quiet while `Entity.vibrationAndSoundEffectsFromBlock` still
/// sends the step game event. Every other type is `ALL`.
pub fn movement_sounds(m: &Mob) -> bool {
    !matches!(
        m.species(),
        Species::Silverfish
            | Species::Endermite
            | Species::Bat
            | Species::Squid
            | Species::GlowSquid
            | Species::Guardian
            | Species::ElderGuardian
            | Species::Breeze
    )
}
/// `getSwimSound`. A skeleton horse's own override counts the steps it takes
/// through the water, so it needs the mob, not just its species.
pub fn swim_sound(m: &mut Mob) -> &'static str {
    if m.species() == Species::SkeletonHorse {
        return super::equine::skeleton_horse_swim_sound(m);
    }
    match m.species() {
        Species::Drowned => "entity.drowned.swim",
        Species::Axolotl => "entity.axolotl.swim",
        Species::Dolphin => "entity.dolphin.swim",
        Species::Cod
        | Species::Salmon
        | Species::TropicalFish
        | Species::Pufferfish
        | Species::Tadpole => "entity.fish.swim",
        Species::Nautilus if m.is_baby() => "entity.baby_nautilus.swim",
        Species::Nautilus => "entity.nautilus.swim",
        Species::ZombieNautilus => "entity.zombie_nautilus.swim",
        Species::Turtle => "entity.turtle.swim",
        // Monster.getSwimSound, and Hoglin's own override of it.
        s if s == Species::Hoglin || is_monster(s) => "entity.hostile.swim",
        _ => "entity.generic.swim",
    }
}

#[cfg(test)]
mod bucket_axolotl_tests {
    use super::*;
    #[test]
    fn bucket_axolotl_uses_aquatic_not_generic_land_ai() {
        assert_eq!(family(Species::Axolotl), Family::Aquatic);
    }
}
#[cfg(test)]
mod despawn_tests {
    use super::*;
    /// Village villagers are placed without PersistenceRequired (unlike the
    /// village's iron golem), so they must never despawn for distance.
    #[test]
    fn villagers_traders_and_golems_never_despawn_for_distance() {
        for s in [
            Species::Villager,
            Species::WanderingTrader,
            Species::IronGolem,
            Species::SnowGolem,
            Species::CopperGolem,
            Species::Shulker,
        ] {
            assert!(never_removed_when_far(s), "{s:?}");
        }
        for s in [Species::Zombie, Species::ZombieVillager, Species::Pillager] {
            assert!(!never_removed_when_far(s), "{s:?}");
        }
    }

    /// `Cat.removeWhenFarAway` (`!isTame() && tickCount > 2400`) and
    /// `Ocelot.removeWhenFarAway` (`!isTrusting() && tickCount > 2400`): the two
    /// animals vanilla lets go, on `Entity.tickCount` and never on
    /// `AgeableMob.age`, which an adult keeps at 0.
    #[test]
    fn an_untamed_feline_goes_once_it_is_old() {
        for s in [Species::Cat, Species::Ocelot] {
            assert!(!feline_removed_when_far(s, false, 2400), "{s:?}");
            assert!(feline_removed_when_far(s, false, 2401), "{s:?}");
            assert!(!feline_removed_when_far(s, true, 12000), "{s:?}");
        }
        // Animal.removeWhenFarAway: every other animal stays at any age.
        for s in [Species::Cow, Species::Sheep, Species::Wolf] {
            assert!(!feline_removed_when_far(s, false, 12000), "{s:?}");
        }
    }
    /// `AbstractFish.removeWhenFarAway` and `Axolotl.removeWhenFarAway`: a fish or
    /// an axolotl with a custom name is kept however far it swims, while the name
    /// saves no other mob — a monster still goes.
    #[test]
    #[ignore = "daily"]
    fn a_named_fish_or_axolotl_never_despawns() {
        crate::mobs::world::with_test_world(&[], |w, types| {
            let far = MobWorld {
                // 200 blocks from the player: past the 128 a monster loses.
                nearest_player: Some(Vec3::new(200.0, 64.0, 0.0)),
                ..w.clone()
            };
            let checked = |id: &str, name: Option<&str>| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
                m.custom_name = name.map(str::to_owned);
                despawn_check(&mut m, &far)
            };
            for fish in ["minecraft:cod", "minecraft:salmon", "minecraft:axolotl"] {
                assert!(checked(fish, Some("Bubbles")), "{fish} keeps its name");
                assert!(!checked(fish, None), "{fish} without one goes");
            }
            // A name is not persistence for anything else.
            assert!(!checked("minecraft:zombie", Some("Bubbles")));
        });
    }
}

#[cfg(test)]
mod frog_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use crate::mobs::control::MoveKind;
    use std::sync::Arc;

    /// `Frog()`: `new SmoothSwimmingMoveControl<>(this, 85, 10, 0.02F, 0.1F, true)`.
    /// The frog's movement speed is 1.0, so the controller's 0.1 outside-water
    /// speed is what keeps a walking frog at vanilla's pace. The plain ground
    /// controller walks it ten times too fast, which is the jitter, the
    /// teleporting and the flying of #2213.
    #[test]
    fn a_frog_walks_at_the_vanilla_speed() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:frog").unwrap().clone());
        assert_eq!(kind.speed, 1.0, "the frog's movement speed attribute");
        let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        register(&mut m);
        assert_eq!(
            m.ai.mov.kind,
            MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.02,
                outside: 0.1,
                gravity: true,
            }
        );
    }
}

#[cfg(test)]
mod poison_effect_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use std::sync::Arc;

    #[test]
    fn poison_effect_damages_living_mobs_but_not_undead() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();

        // Pig is not undead: should take damage from poison.
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("poison", 100, 0);
        assert_eq!(pig.extra.effects[0].id, "minecraft:poison");
        let initial_pig_health = pig.health;
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(
            pig.health,
            initial_pig_health - 1.0,
            "living mob should take 1.0 damage from poison"
        );

        // Zombie is undead: immune to poison damage.
        let zombie_kind = Arc::new(data.types.get("minecraft:zombie").unwrap().clone());
        let mut zombie = Mob::new(2, zombie_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        zombie.add_effect("poison", 100, 0);
        let initial_zombie_health = zombie.health;
        tick_effects(
            &mut zombie,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(
            zombie.health, initial_zombie_health,
            "undead mobs are immune to poison"
        );
    }

    /// `LivingEntity.addEffect` runs `AbsorptionMobEffect.onEffectStarted` for
    /// every effect it accepts: the same Absorption given again refills the
    /// shield, and a weaker one still tops it up to its own level (#11386).
    #[test]
    fn absorption_refills_on_every_accepted_add() {
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:cow").unwrap().clone());
        let mut cow = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        cow.add_effect("absorption", 600, 0);
        assert_eq!(cow.absorption, 4.0);
        // A hit used it up.
        cow.absorption = 0.0;
        cow.add_effect("absorption", 600, 0);
        assert_eq!(cow.absorption, 4.0, "the same effect again refills it");
        cow.add_effect("absorption", 600, 2);
        assert_eq!(cow.absorption, 12.0);
        cow.absorption = 2.0;
        cow.add_effect("absorption", 600, 0);
        assert_eq!(cow.absorption, 4.0, "a weaker one tops it up to its level");
    }

    /// Instant health and harm given at amplifier 32 shift by 0, as Java's
    /// `<<` does (#11386).
    #[test]
    fn instant_effects_shift_like_java_on_mobs() {
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:iron_golem").unwrap().clone());
        let mut golem = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        golem.attrs.max_health = 100.0;
        golem.health = 60.0;
        golem.add_effect("instant_health", 6, 32);
        tick_effects(
            &mut golem,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
            |_| {},
        );
        assert_eq!(golem.health, 64.0);
    }

    /// A mob's effects act on it: instant health and harm (inverted for the
    /// undead), Resistance taking 20% a level off a hit, Absorption soaking
    /// damage before health, Slow Falling capping gravity while it falls.
    #[test]
    fn mob_effects_heal_harm_resist_absorb_and_slow_the_fall() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let make = |name: &str| {
            let kind = Arc::new(data.types.get(name).unwrap().clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
            m.attrs.max_health = 100.0;
            m.health = 60.0;
            m
        };
        // instant_health 1 heals 8 a tick, instant_damage 0 hurts 6 once (hit frames).
        let mut golem = make("minecraft:iron_golem");
        golem.add_effect("instant_health", 6, 1);
        tick_effects(
            &mut golem,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(golem.health, 68.0);
        let mut golem = make("minecraft:iron_golem");
        golem.add_effect("instant_damage", 6, 0);
        tick_effects(
            &mut golem,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(golem.health, 54.0);
        // The undead are healed by harm.
        let mut zombie = make("minecraft:zombie");
        zombie.add_effect("instant_damage", 6, 0);
        tick_effects(
            &mut zombie,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(zombie.health, 64.0);
        // Resistance V (amplifier 4) takes it all, III takes 80%.
        let mut golem = make("minecraft:iron_golem");
        golem.health = 100.0;
        golem.add_effect("resistance", 100, 4);
        golem.damage(level, 5.0, Damage::of(DamageKind::Generic));
        assert_eq!(golem.health, 100.0);
        let mut golem = make("minecraft:iron_golem");
        golem.health = 100.0;
        golem.add_effect("resistance", 100, 2);
        golem.damage(level, 10.0, Damage::of(DamageKind::Generic));
        assert_eq!(golem.health, 96.0);
        // Absorption II: 8 hearts-halves, 5 damage leaves 3 and costs no health.
        let mut golem = make("minecraft:iron_golem");
        golem.health = 100.0;
        golem.add_effect("absorption", 100, 1);
        assert_eq!(golem.absorption, 8.0);
        golem.damage(level, 5.0, Damage::of(DamageKind::Generic));
        assert_eq!((golem.health, golem.absorption), (100.0, 3.0));
        // Slow Falling: gravity floors at 0.01 while falling, not while rising.
        let mut pig = make("minecraft:pig");
        let full = pig.effective_gravity();
        pig.add_effect("slow_falling", 100, 0);
        assert!(full > 0.01);
        assert_eq!(pig.effective_gravity(), 0.01);
        pig.velocity.y = 0.5;
        assert_eq!(pig.effective_gravity(), full);
    }

    #[test]
    fn invisibility_flag_updates_only_when_effects_change() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("invisibility", 4, 0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert!(pig.is_invisible());
        pig.invisible = false;
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert!(!pig.is_invisible());
        pig.add_effect("speed", 10, 0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert!(pig.is_invisible());
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert!(!pig.is_invisible());
    }

    #[test]
    fn effect_refresh_every_600_ticks_resets_manual_visibility() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("speed", 602, 0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert!(!pig.is_invisible());
        pig.invisible = true;
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(pig.extra.effects[0].duration, 600);
        assert!(
            !pig.is_invisible(),
            "LivingEntity's periodic effect update clears the manual flag at duration 600"
        );
    }

    /// `LivingEntity.tickEffects` ticks each effect with the mob's list in
    /// place: a Poison hit reads Resistance there (0.8 of 1.0 at level I).
    #[test]
    fn effect_ticks_find_the_other_effects() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind.clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.health = 20.0;
        pig.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:poison", 25, 0));
        pig.extra.effects.push(crate::mobs::StoredEffect::new(
            "minecraft:resistance",
            -1,
            0,
        ));
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(pig.health, 19.2);
        // `onMobHurt` finds Infested in the same in-place list: the hurt mob
        // leaves a silverfish call in the outbox.
        let spawned = (0..100).any(|seed| {
            let mut pig = Mob::new(2, pig_kind.clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
            pig.random = horizon_core::random::EntityRandom::own(
                horizon_core::random::LegacyRandom::new(seed),
            );
            pig.extra
                .effects
                .push(crate::mobs::StoredEffect::new("minecraft:poison", 25, 0));
            pig.extra
                .effects
                .push(crate::mobs::StoredEffect::new("minecraft:infested", -1, 0));
            tick_effects(
                &mut pig,
                crate::test_resources::server_loop_context(),
                level,
                None,
                |_| {},
            );
            pig.outbox
                .iter()
                .any(|e| matches!(e, MobEvent::Infested { .. }))
        });
        assert!(spawned, "a hurt while infested pushes Infested");
    }

    /// `MobEffectInstance.hasRemainingDuration` is false at 0 ticks, so an effect
    /// loaded with `duration: 0` is over on its first tick instead of counting
    /// down to -1, the value that means infinite.
    #[test]
    fn a_zero_duration_effect_is_dropped_not_made_infinite() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.health = 20.0;
        pig.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:poison", 0, 0));
        let mut test_level = crate::health::TestLevel::new(0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            &mut test_level.level(),
            None,
            |_| {},
        );
        assert!(pig.extra.effects.is_empty());
        // A poison with no remaining duration does not get its first tick either.
        assert_eq!(pig.health, 20.0);
    }

    /// `checkTotemDeathProtection` inside a tick's damage adds the totem's
    /// effects to the live list: they stay on the mob after the tick.
    #[test]
    fn a_totem_fired_by_effect_damage_keeps_its_effects() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.extra
            .set_main_hand(Some("minecraft:totem_of_undying".into()));
        pig.extra.effects.push(crate::mobs::StoredEffect::new(
            "minecraft:instant_damage",
            5,
            4,
        ));
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(pig.health, 1.0);
        for effect in [
            "minecraft:regeneration",
            "minecraft:absorption",
            "minecraft:fire_resistance",
        ] {
            assert!(
                pig.extra
                    .effects
                    .iter()
                    .any(|e| e.id == effect && e.duration > 0),
                "{effect} kept after the totem fired"
            );
        }
    }

    /// `WitherMobEffect.applyEffectTick` hurts with `wither`, not `magic`:
    /// the damage the tick deals keeps its kind.
    #[test]
    fn the_wither_effect_hurts_with_wither() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:wither", 40, 0));
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
            |_| {},
        );
        assert_eq!(
            pig.last_damage.map(|(d, _)| d.kind),
            Some(DamageKind::Wither)
        );
    }
}

#[cfg(test)]
mod hit_effect_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    /// The effects of a cave spider's bite, a bare-handed husk's blow and a
    /// wither skeleton's hit go to the `LivingEntity` the hit landed on:
    /// `doHurtTarget` tests `target instanceof LivingEntity`, not `Player`.
    /// Biting a mob carries the effect with the hit — poison for 140 ticks on
    /// normal, hunger for 140 (effective difficulty 1.5), wither for 200 —
    /// where the mob is hit 2. Without the fix each of these is a plain
    /// `AttackMob` and the mob keeps no effect at all (0 ticks).
    #[test]
    #[ignore = "daily"]
    fn a_melee_effect_rides_the_hit_onto_a_mob() {
        let cases = [
            ("minecraft:cave_spider", "minecraft:poison", 140),
            ("minecraft:husk", "minecraft:hunger", 140),
            ("minecraft:wither_skeleton", "minecraft:wither", 200),
        ];
        let mut carried: Vec<(&str, Option<(u32, &'static str, i32, i32)>)> = Vec::new();
        let mut strays: Vec<(&str, bool, bool)> = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            for (kind, _, _) in cases {
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                let level = &mut test_level.level();
                let mut cx = Cx {
                    w,
                    level,
                    events: &mut events,
                };
                do_hurt_target(&mut m, Who::Mob(2), &mut cx);
                carried.push((
                    kind,
                    events.iter().find_map(|e| match e {
                        MobEvent::AttackMobWithEffect {
                            target,
                            effect,
                            ticks,
                            amplifier,
                            ..
                        } => Some((*target, *effect, *ticks, *amplifier)),
                        _ => None,
                    }),
                ));
                strays.push((
                    kind,
                    events
                        .iter()
                        .any(|e| matches!(e, MobEvent::EffectPlayer { .. })),
                    events
                        .iter()
                        .any(|e| matches!(e, MobEvent::AttackMob { .. })),
                ));
            }
        });
        if !ran {
            return;
        }
        let expected: Vec<(&str, Option<(u32, &'static str, i32, i32)>)> = cases
            .iter()
            .map(|(kind, effect, ticks)| (*kind, Some((2, *effect, *ticks, 0))))
            .collect();
        assert_eq!(carried, expected);
        for (kind, on_player, effectless) in strays {
            assert!(!on_player, "{kind} left its effect on the player");
            assert!(!effectless, "{kind} sent a second, effectless hit");
        }
    }

    /// On easy the cave spider's poison timer is 0, so nothing rides the hit:
    /// the mob it bites takes the damage alone (`AttackMob`, no effect).
    #[test]
    #[ignore = "daily"]
    fn an_easy_cave_spiders_bite_carries_no_effect() {
        let mut out: Option<Vec<String>> = None;
        let ran = with_test_world(&[], |w, types| {
            let mut easy = w.clone();
            easy.difficulty = super::super::spawning::LocalDifficulty::new(1, 18000, 0, 0.0);
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:cave_spider"].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            let mut cx = Cx {
                w: &easy,
                level,
                events: &mut events,
            };
            do_hurt_target(&mut m, Who::Mob(2), &mut cx);
            out = Some(
                events
                    .iter()
                    .filter(|e| {
                        matches!(
                            e,
                            MobEvent::AttackMob { .. } | MobEvent::AttackMobWithEffect { .. }
                        )
                    })
                    .map(|e| match e {
                        MobEvent::AttackMob { target, .. } => format!("plain {target}"),
                        MobEvent::AttackMobWithEffect { effect, .. } => format!("effect {effect}"),
                        _ => unreachable!(),
                    })
                    .collect(),
            );
        });
        if !ran {
            return;
        }
        assert_eq!(out, Some(vec!["plain 2".to_owned()]));
    }
}

#[cfg(test)]
mod attack_sound_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    /// The sounds `do_hurt_target` raises for `kind` on a blow that landed,
    /// with `silent` set on the attacker; `None` without the extracted data.
    fn attack_sounds(kind: &str, silent: bool) -> Option<Vec<(&'static str, f32, f32)>> {
        let mut heard = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            let mut m = Mob::new(
                1,
                Arc::new(types.types[kind].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            m.silent = silent;
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let mut cx = Cx {
                w,
                level: &mut test_level.level(),
                events: &mut events,
            };
            do_hurt_target(&mut m, Who::Mob(2), &mut cx);
            heard = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound(s, _, volume, pitch) => Some((*s, *volume, *pitch)),
                    _ => None,
                })
                .collect();
        });
        if !ran {
            return None;
        }
        Some(heard)
    }

    /// `Mob.doHurtTarget` ends with `playAttackSound`, which `Dolphin`,
    /// `Axolotl` and `Panda` override with their own bite at volume 1 and
    /// pitch 1. Without the dolphin arm its bite is silent (no sound event at
    /// all); the axolotl and the panda sit next to it as the control.
    #[test]
    #[ignore = "daily"]
    fn a_dolphins_blow_plays_its_attack_sound() {
        let Some(dolphin) = attack_sounds("minecraft:dolphin", false) else {
            return;
        };
        assert_eq!(dolphin, vec![("entity.dolphin.attack", 1.0, 1.0)]);
        for (kind, sound) in [
            ("minecraft:axolotl", "entity.axolotl.attack"),
            ("minecraft:panda", "entity.panda.bite"),
        ] {
            assert_eq!(
                attack_sounds(kind, false),
                Some(vec![(sound, 1.0, 1.0)]),
                "{kind}"
            );
        }
        // Entity.playSound skips a silent entity, so neither attack sound plays.
        for kind in ["minecraft:dolphin", "minecraft:axolotl", "minecraft:panda"] {
            assert_eq!(attack_sounds(kind, true), Some(vec![]), "{kind}");
        }
    }
}

#[cfg(test)]
mod same_vehicle_push_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    #[test]
    #[ignore = "daily"]
    fn zombie_horse_sunburn_checks_body_not_head() {
        assert!(with_test_world(&[], |world, types| {
            let mut world = world.clone();
            world.monsters_burn = true;
            let mut horse = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:zombie_horse"].clone()),
                Vec3::ZERO,
                0.0,
            );
            horse.extra.set_slot_id(
                horizon_items::equipment::HEAD,
                Some("minecraft:iron_helmet".into()),
            );
            let mut events = Vec::new();
            for _ in 0..1000 {
                burn_undead(&mut horse, &world, &mut events);
            }
            assert!(horse.fire_ticks > 0);
            assert!(events.is_empty());
            horse.fire_ticks = 0;
            horse
                .extra
                .set_body(Some("minecraft:leather_horse_armor".into()));
            for _ in 0..1000 {
                burn_undead(&mut horse, &world, &mut events);
            }
            assert_eq!(horse.fire_ticks, 0);
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, MobEvent::SunProtectionWear { .. }))
            );
        }));
    }

    #[test]
    #[ignore = "daily"]
    fn leather_body_armor_prevents_freezing() {
        assert!(with_test_world(&[], |w, types| {
            let mut horse = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:horse"].clone()),
                Vec3::ZERO,
                0.0,
            );
            assert!(can_freeze(&horse, w));
            horse
                .extra
                .set_body(Some("minecraft:leather_horse_armor".into()));
            assert!(!can_freeze(&horse, w));
            horse
                .extra
                .set_body(Some("minecraft:iron_horse_armor".into()));
            assert!(can_freeze(&horse, w));
        }));
    }

    /// What a mob holds and wears, by `EquipmentSlot`: the hand's item, the
    /// worn armour's stack (a dyed piece keeps its colour), a horse's
    /// armour in the body slot and its saddle, and nothing else.
    #[test]
    #[ignore = "daily"]
    fn a_mobs_equipment_is_listed_by_slot() {
        assert!(
            with_test_world(&[], |_, types| {
                let mob = |name: &str| {
                    Mob::new(
                        1,
                        std::sync::Arc::new(types.types[name].clone()),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    )
                };
                let mut zombie = mob("minecraft:zombie");
                assert!(zombie.equipment_stacks().iter().all(Option::is_none));
                zombie
                    .extra
                    .set_main_hand(Some(std::sync::Arc::from("minecraft:iron_sword")));
                zombie.extra.set_slot_id(
                    horizon_items::equipment::FEET,
                    Some(std::sync::Arc::from("minecraft:iron_boots")),
                );
                let worn = zombie.equipment_stacks();
                assert_eq!(
                    worn[0].as_ref().unwrap().item.as_ref(),
                    "minecraft:iron_sword"
                );
                assert_eq!(
                    worn[2].as_ref().unwrap().item.as_ref(),
                    "minecraft:iron_boots"
                );
                assert_eq!(worn.iter().flatten().count(), 2);
                let mut horse = mob("minecraft:horse");
                horse
                    .extra
                    .set_body(Some(std::sync::Arc::from("minecraft:iron_horse_armor")));
                horse.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
                let gear = horse.equipment_stacks();
                assert_eq!(
                    gear[6].as_ref().unwrap().item.as_ref(),
                    "minecraft:iron_horse_armor"
                );
                assert_eq!(gear[7].as_ref().unwrap().item.as_ref(), "minecraft:saddle");
                assert!(gear[5].is_none());
            }),
            "vanilla data is missing"
        );
    }

    /// `Entity.push(Entity)` returns at once for `isPassengerOfSameVehicle`
    /// (the same `getRootVehicle`): a camel and the husk on its back, seated
    /// half a block toward its front and so inside its box, never push each
    /// other (#3199). The camel still pushes a mob that only overlaps it.
    #[test]
    #[ignore = "daily"]
    fn a_camel_and_its_rider_never_push_each_other() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // The mobs the camel pushes, and its velocity after, with these mobs
        // around: a husk on its back, and a zombie standing in its back half.
        let mut push = |zombie_too: bool| {
            let mut out = None;
            let mut riders = Vec::new();
            assert!(
                with_test_world(&[], |_, types| {
                    let mob = |id: u32, name: &str, at: Vec3| {
                        Mob::new(id, std::sync::Arc::new(types.types[name].clone()), at, 0.0)
                    };
                    let mut camel = mob(1, "minecraft:camel", Vec3::new(0.5, 64.0, 0.5));
                    let mut husk = mob(2, "minecraft:husk", Vec3::new(0.5, 65.5, 1.0));
                    camel.extra.rider = Some(husk.id);
                    husk.extra.mount = Some(camel.id);
                    riders.push(husk.seen(None, &crate::items::Items::empty()));
                    if zombie_too {
                        riders.push(
                            mob(3, "minecraft:zombie", Vec3::new(0.5, 64.0, 0.0))
                                .seen(None, &crate::items::Items::empty()),
                        );
                    }
                    out = Some(camel);
                }),
                "vanilla data is missing"
            );
            let mut camel = out?;
            let mut events = Vec::new();
            assert!(
                with_test_world(&riders, |w, _| {
                    push_entities(&mut camel, w, level, &mut events)
                }),
                "vanilla data is missing"
            );
            let pushed: Vec<u32> = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Push {
                        target: Who::Mob(id),
                        ..
                    } => Some(*id),
                    _ => None,
                })
                .collect();
            Some((pushed, camel.velocity))
        };
        let Some((pushed, velocity)) = push(false) else {
            return;
        };
        assert_eq!(pushed, Vec::<u32>::new(), "the camel pushes its own rider");
        assert_eq!(velocity, Vec3::ZERO, "its rider pushes the camel");
        let (pushed, _) = push(true).unwrap();
        assert_eq!(pushed, vec![3], "the camel pushes the zombie");
    }

    /// The server's collision cap (`[entities] max_collisions`): in a pen of twelve chickens one
    /// pushes every other (11) with no cap, and exactly the first N in vanilla's order with a cap
    /// of N, skipping a chicken that already had N pushes this tick.
    #[test]
    #[ignore = "daily"]
    fn a_mob_in_a_crowded_pen_pushes_at_most_the_cap() {
        let mut pen = None;
        assert!(
            with_test_world(&[], |_, types| {
                let chicken = |id: u32| {
                    // All inside one another's boxes, none exactly on the pusher.
                    let at = Vec3::new(0.5 + f64::from(id) * 0.02, 64.0, 0.5);
                    Mob::new(
                        id,
                        std::sync::Arc::new(types.types["minecraft:chicken"].clone()),
                        at,
                        0.0,
                    )
                };
                pen = Some((0..12).map(chicken).collect::<Vec<_>>());
            }),
            "vanilla data is missing"
        );
        let Some(pen) = pen else {
            return;
        };
        let pushed = |cap: u32, crowd: &[crate::mobs::Seen]| {
            let mut pusher = pen[0].clone();
            let mut events = Vec::new();
            super::super::push::with_cap(cap, || {
                with_test_world(crowd, |w, _| {
                    let mut level = crate::health::TestLevel::new(0);
                    push_entities(&mut pusher, w, &mut level.level(), &mut events)
                })
            });
            let ids: Vec<u32> = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Push {
                        target: Who::Mob(id),
                        ..
                    } => Some(*id),
                    _ => None,
                })
                .collect();
            // What the loop reports for the counts that follow.
            assert_eq!(pusher.pushed, if cap > 0 { ids.clone() } else { vec![] });
            ids
        };
        let mut crowd: Vec<crate::mobs::Seen> = pen
            .iter()
            .map(|m| m.seen(None, &crate::items::Items::empty()))
            .collect();
        assert_eq!(pushed(0, &crowd), (1..12).collect::<Vec<u32>>());
        assert_eq!(pushed(3, &crowd), vec![1, 2, 3]);
        // Chickens 1 and 2 have taken 3 pushes already: skipped, the next ones are pushed.
        crowd[1].collisions = 3;
        crowd[2].collisions = 3;
        assert_eq!(pushed(3, &crowd), vec![3, 4, 5]);
        // The pusher's own count includes what it received: two left of three.
        crowd[0].collisions = 1;
        assert_eq!(pushed(3, &crowd), vec![3, 4]);
        crowd[0].collisions = 3;
        assert!(pushed(3, &crowd).is_empty());
        // Off, the counts are not read.
        assert_eq!(pushed(0, &crowd), (1..12).collect::<Vec<u32>>());
    }

    /// MSPT of one tick of a crowded chicken pen with the cap off and at 8 (run by hand with
    /// `--ignored --nocapture`; needs the vanilla data). It measures the pushes a mob gives; the
    /// received counts the mob loop adds are not simulated.
    #[test]
    #[ignore]
    fn collision_cap_cost() {
        for n in [50u32, 100, 200] {
            let mut pen = None;
            with_test_world(&[], |_, types| {
                let ty = std::sync::Arc::new(types.types["minecraft:chicken"].clone());
                pen = Some(
                    (0..n)
                        .map(|id| {
                            let at = Vec3::new(
                                0.5 + f64::from(id % 7) * 0.05,
                                64.0,
                                0.5 + f64::from(id / 7 % 7) * 0.05 + f64::from(id) * 0.0001,
                            );
                            Mob::new(id, ty.clone(), at, 0.0)
                        })
                        .collect::<Vec<_>>(),
                );
            });
            let Some(mut pen) = pen else {
                return;
            };
            for cap in [0u32, 8] {
                let crowd: Vec<crate::mobs::Seen> = pen
                    .iter()
                    .map(|m| m.seen(None, &crate::items::Items::empty()))
                    .collect();
                let t = std::time::Instant::now();
                super::super::push::with_cap(cap, || {
                    with_test_world(&crowd, |w, _| {
                        let mut events = Vec::new();
                        let mut level = crate::health::TestLevel::new(0);
                        for m in pen.iter_mut() {
                            events.clear();
                            push_entities(m, w, &mut level.level(), &mut events);
                            m.pushed.clear();
                        }
                    })
                });
                println!(
                    "{n} chickens, cap {cap}: {:.3} ms",
                    t.elapsed().as_secs_f64() * 1e3
                );
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn sleeping_villager_vetoes_only_pushes_received_during_other_mobs_ticks() {
        let mut pair = None;
        let ran = with_test_world(&[], |_, types| {
            let make = |id, name: &str, x| {
                Mob::new(
                    id,
                    std::sync::Arc::new(types.types[name].clone()),
                    Vec3::new(x, 64.0, 0.5),
                    0.0,
                )
            };
            let mut villager = make(1, "minecraft:villager", 0.5);
            villager.extra.ints[1] = 1;
            pair = Some((villager, make(2, "minecraft:cow", 0.9)));
        });
        // Without the vanilla data under .local/horizon-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        let Some((mut villager, mut cow)) = pair else {
            return;
        };
        let mut events = Vec::new();
        let ran = with_test_world(
            &[villager.seen(None, &crate::items::Items::empty())],
            |w, _| {
                push_entities(
                    &mut cow,
                    w,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut events,
                );
            },
        );
        // Without the vanilla data under .local/horizon-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.is_empty());
        assert_eq!(cow.velocity, Vec3::ZERO);

        let ran = with_test_world(&[cow.seen(None, &crate::items::Items::empty())], |w, _| {
            push_entities(
                &mut villager,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
        });
        // Without the vanilla data under .local/horizon-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.iter().any(|e| matches!(
            e,
            MobEvent::Push {
                target: Who::Mob(2),
                ..
            }
        )));
        assert!(villager.velocity.x < 0.0);
    }

    #[test]
    #[ignore = "daily"]
    fn sleeping_fox_vetoes_pushes_received_during_other_mobs_ticks() {
        let mut pair = None;
        let ran = with_test_world(&[], |_, types| {
            let make = |id, name: &str, x| {
                Mob::new(
                    id,
                    std::sync::Arc::new(types.types[name].clone()),
                    Vec3::new(x, 64.0, 0.5),
                    0.0,
                )
            };
            let mut fox = make(1, "minecraft:fox", 0.5);
            fox.extra.ints[0] |= super::super::fox::SLEEPING;
            pair = Some((fox, make(2, "minecraft:cow", 0.9)));
        });
        // Without the vanilla data under .local/horizon-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        let Some((mut fox, mut cow)) = pair else {
            return;
        };
        let mut events = Vec::new();
        let ran = with_test_world(&[fox.seen(None, &crate::items::Items::empty())], |w, _| {
            push_entities(
                &mut cow,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
        });
        // Without the vanilla data under .local/horizon-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.is_empty());
        assert_eq!(cow.velocity, Vec3::ZERO);

        fox.extra.ints[0] &= !super::super::fox::SLEEPING;
        let ran = with_test_world(&[fox.seen(None, &crate::items::Items::empty())], |w, _| {
            push_entities(
                &mut cow,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
        });
        // Without the vanilla data under .local/horizon-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.iter().any(|e| matches!(
            e,
            MobEvent::Push {
                target: Who::Mob(1),
                ..
            }
        )));
    }

    #[test]
    #[ignore = "daily"]
    fn a_ridden_horse_is_not_pushed_but_still_pushes() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let pair = |ridden: bool| {
            let mut out = None;
            assert!(
                with_test_world(&[], |_, types| {
                    let mob = |id: u32, name: &str, at: Vec3| {
                        Mob::new(id, std::sync::Arc::new(types.types[name].clone()), at, 0.0)
                    };
                    let mut horse = mob(1, "minecraft:horse", Vec3::new(0.5, 64.0, 0.5));
                    if ridden {
                        horse.extra.rider = Some(3);
                    }
                    let cow = mob(2, "minecraft:cow", Vec3::new(1.4, 64.0, 0.5));
                    out = Some((horse, cow));
                }),
                "vanilla data is missing"
            );
            out
        };
        let pushed = |events: &[MobEvent]| -> Vec<u32> {
            events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Push {
                        target: Who::Mob(id),
                        ..
                    } => Some(*id),
                    _ => None,
                })
                .collect()
        };
        let Some((mut horse, cow)) = pair(true) else {
            return;
        };
        let mut events = Vec::new();
        assert!(
            with_test_world(&[cow.seen(None, &crate::items::Items::empty())], |w, _| {
                push_entities(&mut horse, w, level, &mut events)
            }),
            "vanilla data is missing"
        );
        assert_eq!(pushed(&events), vec![2], "the ridden horse pushes the cow");
        assert_eq!(
            horse.velocity,
            Vec3::ZERO,
            "the cow pushes the ridden horse"
        );

        let (horse, mut cow) = pair(true).unwrap();
        let mut events = Vec::new();
        assert!(
            with_test_world(
                &[horse.seen(None, &crate::items::Items::empty())],
                |w, _| push_entities(&mut cow, w, level, &mut events),
            ),
            "vanilla data is missing"
        );
        assert_eq!(
            pushed(&events),
            Vec::<u32>::new(),
            "the cow pushes the ridden horse"
        );
        assert_eq!(cow.velocity, Vec3::ZERO, "a ridden horse is not pushable");

        let (horse, mut cow) = pair(false).unwrap();
        let mut events = Vec::new();
        assert!(
            with_test_world(
                &[horse.seen(None, &crate::items::Items::empty())],
                |w, _| push_entities(&mut cow, w, level, &mut events),
            ),
            "vanilla data is missing"
        );
        assert_eq!(
            pushed(&events),
            vec![1],
            "the cow pushes a horse nobody rides"
        );
    }
}

#[cfg(test)]
mod warden_push_tests {
    use super::*;
    use crate::mobs::warden::{Pose, set_pose};
    use crate::mobs::world::{test_player, with_test_world};

    #[test]
    #[ignore = "daily"]
    fn a_warden_in_the_ground_does_not_take_collision_pushes() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        assert!(
            with_test_world(&[], |world, types| {
                let mob = |id, kind: &str, x| {
                    Mob::new(
                        id,
                        std::sync::Arc::new(types.types[kind].clone()),
                        Vec3::new(x, 64.0, 0.5),
                        0.0,
                    )
                };
                for pose in [Pose::Emerging, Pose::Digging, Pose::Standing] {
                    let mut warden = mob(1, "minecraft:warden", 0.5);
                    set_pose(&mut warden, pose);
                    let standing = pose == Pose::Standing;
                    assert_eq!(warden.pushable(), standing, "{pose:?}");
                    assert_eq!(
                        warden.seen(None, &crate::items::Items::empty()).pushable,
                        standing,
                        "{pose:?}"
                    );

                    // The warden's tick: an overlapping mob must not move it.
                    let mut cow = mob(2, "minecraft:cow", 1.0);
                    let others = [cow.seen(None, &crate::items::Items::empty())];
                    let mut w = world.clone();
                    w.others = &others;
                    let mut events = Vec::new();
                    push_entities(&mut warden, &w, level, &mut events);
                    assert_eq!(warden.velocity.x != 0.0, standing, "mob: {pose:?}");
                    // Non-pushable is not an early return: it still pushes others.
                    assert!(events.iter().any(|e| matches!(
                        e,
                        MobEvent::Push {
                            target: Who::Mob(2),
                            ..
                        }
                    )));

                    // The other mob's tick: the warden's Seen must exclude it.
                    let others = [warden.seen(None, &crate::items::Items::empty())];
                    w.others = &others;
                    events.clear();
                    push_entities(&mut cow, &w, level, &mut events);
                    let pushes_warden = events.iter().any(|e| {
                        matches!(
                            e,
                            MobEvent::Push {
                                target: Who::Mob(1),
                                ..
                            }
                        )
                    });
                    assert_eq!(pushes_warden, standing, "other tick: {pose:?}");

                    // The player branch of the warden's tick has its own target path.
                    w.others = &[];
                    w.player = Some(test_player(Vec3::new(1.0, 64.0, 0.5)));
                    warden.velocity = Vec3::ZERO;
                    events.clear();
                    push_entities(&mut warden, &w, level, &mut events);
                    assert_eq!(warden.velocity.x != 0.0, standing, "player: {pose:?}");
                }
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod ridden_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// #2920: a saddled horse ridden forward, ticked on the server; with
    /// `client` its rider's client moves it (`travelRidden` without
    /// `canSimulateMovement`): it only turns to the rider's look and stops.
    fn ridden(client: bool) -> Option<Mob> {
        let mut out = None;
        with_test_world(&[], |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:horse"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
            m.persistent = true;
            m.rider = Some(super::super::equine::Rider {
                forward: 1.0,
                yaw: 90.0,
                client,
                ..Default::default()
            });
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            for _ in 0..5 {
                tick(&mut m, w, level, &mut events);
            }
            out = Some(m);
        });
        out
    }

    #[test]
    #[ignore = "daily"]
    fn the_server_leaves_a_horse_to_its_riders_client() {
        let Some(moved) = ridden(false) else {
            return;
        };
        // The host's own ride: the horse walks west and falls through the air.
        assert!(moved.position.x < 0.4, "{:?}", moved.position);
        assert!(moved.position.y < 64.0, "{:?}", moved.position);
        let held = ridden(true).unwrap();
        assert_eq!(held.position, Vec3::new(0.5, 64.0, 0.5));
        assert_eq!(held.velocity, Vec3::ZERO);
        assert_eq!(held.yaw, 90.0, "tickRidden still turns it");
    }
}

#[cfg(test)]
mod bed_wake_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    #[test]
    #[ignore = "daily"]
    fn loaded_sleeping_position_refreshes_in_actual_mob_tick() {
        use horizon_core::nbt::Tag;
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let stone = registry.default_state("minecraft:stone").unwrap();
            for x in 0..12 {
                for z in 0..12 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            let mut bed = registry
                .state(registry.default_state("minecraft:red_bed").unwrap())
                .unwrap()
                .clone();
            bed.properties.insert("facing".into(), "north".into());
            bed.properties.insert("occupied".into(), "false".into());
            for (part, z) in [("head", 0), ("foot", 1)] {
                bed.properties.insert("part".into(), part.into());
                world.set(0, 64, z, registry.id(&bed).unwrap()).unwrap();
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            assert_eq!(w.block_name(0, 64, 0), "minecraft:red_bed");
            assert_eq!(w.prop(0, 64, 0, "part"), Some("head"));
            assert_eq!(w.prop(0, 64, 0, "facing"), Some("north"));
            let types = [(
                "minecraft:villager".to_owned(),
                std::sync::Arc::new(types.types["minecraft:villager"].clone()),
            )]
            .into_iter()
            .collect();
            let input = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:villager",Pos:[0.5d,64.5625d,0.5d],Health:20f,NoAI:1b,NoGravity:1b,PersistenceRequired:1b,sleeping_pos:[I;0,64,0],Motion:[0d,0d,0d]}"#).unwrap().0;
            let mut loaded =
                crate::entity_nbt::mob_from_nbt(&input, &types, 233, Some(w.registry)).unwrap();
            assert_eq!(
                loaded.position,
                Vec3::new(0.5, 64.5625, 0.5),
                "loading alone must not perform the first-tick correction"
            );
            assert!(loaded.first_tick && loaded.no_ai && loaded.no_gravity);
            assert!(super::super::villager::sleeping(&loaded));
            assert_eq!(
                super::super::villager::sleeping_pos(&loaded),
                Some([0, 64, 0])
            );
            let mut awake_input = input.clone();
            let Tag::Compound(ref mut fields) = awake_input else {
                unreachable!()
            };
            fields.remove(&"sleeping_pos".into());
            fields.insert(
                "Pos".into(),
                Tag::List(
                    [5.5_f64, 64.0, 5.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
            let mut awake =
                crate::entity_nbt::mob_from_nbt(&awake_input, &types, 234, Some(w.registry))
                    .unwrap();
            let mut normal = awake.clone();
            normal.id = 235;
            let mut normal_events = Vec::new();
            assert!(super::super::villager::start_sleeping(
                &mut normal,
                [0, 64, 0],
                w,
                &mut normal_events
            ));
            assert_eq!(normal.position, Vec3::new(0.5, 64.6875, 0.5));
            assert!(normal_events.iter().any(|e| matches!(
                e,
                MobEvent::BedOccupied {
                    pos: [0, 64, 0],
                    occupied: true
                }
            )));
            let mut fixture = crate::health::TestLevel::new(0);
            let mut loaded_events = Vec::new();
            for n in 1..=2 {
                assert!(loaded.tick(w, &mut fixture.level(), &mut loaded_events));
                assert_eq!(loaded.age, n);
                assert!(!loaded.first_tick);
                assert_eq!(
                    loaded.position,
                    Vec3::new(0.5, 64.6875, 0.5),
                    "actual loaded sleeper at tick {n}"
                );
                assert_eq!(loaded.velocity, Vec3::ZERO);
                assert_eq!(loaded.health, 20.0);
                assert_eq!(
                    super::super::villager::sleeping_pos(&loaded),
                    Some([0, 64, 0])
                );
                assert!(awake.tick(w, &mut fixture.level(), &mut Vec::new()));
                assert_eq!(awake.position, Vec3::new(5.5, 64.0, 5.5));
                assert!(!super::super::villager::sleeping(&awake));
                assert!(normal.tick(w, &mut fixture.level(), &mut normal_events));
                assert_eq!(normal.position, Vec3::new(0.5, 64.6875, 0.5));
                assert_eq!(normal.velocity, Vec3::ZERO);
            }
            assert!(
                !loaded_events
                    .iter()
                    .any(|e| matches!(e, MobEvent::BedOccupied { .. })),
                "refresh must not re-run startSleeping side effects"
            );
        });
        assert!(ran, "prepared Mob/bed world must actually run");
    }

    /// `LivingEntity.tick` wakes a sleeper whose bed is gone before
    /// `aiStep` — `Mob.no_ai` skips the step, not the check (#7875).
    fn no_ai_sleeper() -> Option<Mob> {
        let mut out = None;
        with_test_world(&[], |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:villager"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.no_ai = true;
            let mut events = Vec::new();
            // The empty world is all air: the "bed" it lies in is gone.
            assert!(super::super::villager::start_sleeping(
                &mut m,
                [0, 64, 0],
                w,
                &mut events
            ));
            let mut test_level = crate::health::TestLevel::new(0);
            let level = &mut test_level.level();
            tick(&mut m, w, level, &mut events);
            out = Some(m);
        });
        out
    }

    #[test]
    #[ignore = "daily"]
    fn a_sleeper_without_ai_still_stands_when_its_bed_is_gone() {
        let Some(m) = no_ai_sleeper() else { return };
        assert!(
            !super::super::villager::sleeping(&m),
            "a missing bed wakes a no-AI sleeper"
        );
    }
}

#[cfg(test)]
mod turtle_ambient_tests {
    use super::*;
    use std::sync::Arc;

    fn turtle() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:turtle".into();
        k.species = Species::Turtle;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `Turtle.getAmbientSound`: land ambient only for an adult on the ground
    /// and out of water.
    #[test]
    fn a_turtle_is_silent_in_water_in_the_air_and_as_a_baby() {
        let mut m = turtle();
        m.on_ground = true;
        assert!(!turtle_ambient_silent(&m));
        m.on_ground = false;
        assert!(turtle_ambient_silent(&m));
        m.on_ground = true;
        m.fluids.in_water = true;
        assert!(turtle_ambient_silent(&m));
    }
}

#[cfg(test)]
mod feed_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use std::sync::Arc;

    /// `Animal.mobInteract` (#2780): food grows a tamed baby cat or wolf
    /// (`canAgeUp`), so `Wolf.mobInteract` and `Cat.mobInteract` see the click
    /// consumed and do not make it sit. Only an adult that cannot breed now,
    /// or an age-locked baby, leaves the food to the sit toggle.
    #[test]
    fn food_grows_a_tamed_baby_cat_or_wolf() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let mut level = crate::health::TestLevel::new(0);
        for id in ["minecraft:wolf", "minecraft:cat"] {
            let kind = Arc::new(data.types.get(id).unwrap().clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
            m.extra.tame = true;
            m.extra.age = -24000;
            assert!(takes_food(&m), "{id}: a tamed baby takes its food");
            assert!(feed_animal(&mut m, level.random(), None).is_some(), "{id}");
            // getSpeedUpSecondsWhenFeeding(24000): a tenth of 1200 seconds.
            assert_eq!(m.extra.age, -24000 + 120 * 20, "{id}: grew a tenth");
            assert_eq!(m.extra.in_love, 0, "{id}: a baby does not fall in love");

            m.extra.age_locked = true;
            assert!(!takes_food(&m), "{id}: an age-locked baby cannot grow");
            m.extra.age_locked = false;

            m.extra.age = 0;
            assert!(takes_food(&m), "{id}: an adult can breed");
            m.extra.in_love = 600;
            assert!(!takes_food(&m), "{id}: already in love");
            m.extra.in_love = 0;
            m.extra.age = 6000;
            assert!(!takes_food(&m), "{id}: breeding cooldown");
        }
    }

    /// `Animal.mobInteract`'s growing branch is `canAgeUp()` — `isBaby() &&
    /// !isAgeLocked()` — and a species `canBeABaby` says is never a baby does
    /// not grow when fed, however negative its `Age` is: a frog spawned with
    /// `Age` at `getBabyStartAge` keeps its food. A cow at the same age takes
    /// it and grows a tenth.
    #[test]
    fn a_mob_that_cannot_be_a_baby_never_takes_the_food_that_grows_it() {
        let mob = |id: &str, species: Species, age: i32| {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let k = Arc::make_mut(&mut kind);
            k.id = id.into();
            k.species = species;
            let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
            m.extra.age = age;
            m
        };
        let mut level = crate::health::TestLevel::new(0);
        let mut frog = mob("minecraft:frog", Species::Frog, -24_000);
        assert!(!frog.is_baby());
        assert!(!takes_food(&frog), "a frog is never a baby");
        assert!(feed_animal(&mut frog, level.random(), None).is_none());
        assert_eq!(frog.extra.age, -24_000, "it does not grow");

        let mut cow = mob("minecraft:cow", Species::Cow, -24_000);
        assert!(cow.is_baby());
        assert!(takes_food(&cow));
        assert!(feed_animal(&mut cow, level.random(), None).is_some());
        assert_eq!(cow.extra.age, -24_000 + 120 * 20, "it grows a tenth");
    }

    /// `Turtle.canFallInLove` (#8612): a turtle that already carries an egg
    /// cannot fall in love, so `Animal.mobInteract` refuses its food — it is
    /// neither eaten nor turned into love mode.
    #[test]
    fn a_turtle_carrying_an_egg_refuses_its_food() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let mut level = crate::health::TestLevel::new(0);
        let kind = Arc::new(data.types.get("minecraft:turtle").unwrap().clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        m.extra.age = 0;

        m.extra.flag = true;
        assert!(crate::mobs::turtle::has_egg(&m));
        assert!(!takes_food(&m), "an egg-carrying turtle refuses its food");
        assert!(
            feed_animal(&mut m, level.random(), None).is_none(),
            "and does not enter love mode"
        );
        assert_eq!(m.extra.in_love, 0);

        m.extra.flag = false;
        assert!(takes_food(&m), "a turtle without an egg takes it");
        assert!(feed_animal(&mut m, level.random(), None).is_some());
        assert_eq!(m.extra.in_love, 600);
    }
}

#[cfg(test)]
mod forgiveness_tests {
    use super::*;
    use std::sync::Arc;

    fn mob(species: Species) -> Mob {
        let mut t = (*crate::mobs::sensor_scan_tests::kind()).clone();
        t.species = species;
        Mob::new(1, Arc::new(t), Vec3::ZERO, 0.0)
    }

    #[test]
    fn forgiveness_keeps_cancelled_target_but_ends_anger() {
        let mut golem = mob(Species::IronGolem);
        golem.extra.angry_at = Some(Who::Player);
        golem.extra.anger = 300;
        golem.ai.target = Some(Who::Player);
        assert!(forgives_dead_player(&golem));
        forgive_dead_player_with_change(&mut golem, super::super::TargetChange::Keep);
        assert_eq!(golem.ai.target, Some(Who::Player));
        assert_eq!(golem.extra.anger, 0);

        golem.extra.angry_at = Some(Who::Player);
        forgive_dead_player_with_change(
            &mut golem,
            super::super::TargetChange::Set(Some(Who::Mob(7))),
        );
        assert_eq!(golem.ai.target, Some(Who::Mob(7)));
    }

    #[test]
    #[ignore = "daily"]
    fn peaceful_drops_an_iron_golems_existing_player_target() {
        crate::mobs::world::with_test_world(&[], |world, _| {
            let mut world = world.clone();
            world.player = Some(crate::mobs::world::test_player(Vec3::new(1.0, 0.0, 0.0)));
            world.difficulty = super::super::spawning::LocalDifficulty::new(0, 18000, 0, 0.0);
            let mut golem = mob(Species::IronGolem);
            golem.ai.target = Some(Who::Player);
            golem.extra.anger = 300;
            golem.ai.last_hurt_by = Some(Who::Player);
            let mut events = Vec::new();
            tick(
                &mut golem,
                &world,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
            assert_eq!(golem.ai.target, None);
            assert_eq!(golem.extra.anger, 0);
            assert_eq!(golem.ai.last_hurt_by, None);
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, MobEvent::AttackPlayer(..)))
            );
        });
    }

    /// The enderman's and zombified piglin's persistent anger is the target
    /// they hold: a dying player they are angry at takes it (and the last
    /// hurt by) with them, as `NeutralMob.playerDied` does through
    /// `stopBeingAngry`.
    #[test]
    fn a_target_anger_at_the_dying_player_ends() {
        for species in [Species::Enderman, Species::ZombifiedPiglin] {
            let mut m = mob(species);
            m.ai.target = Some(Who::Player);
            m.ai.last_hurt_by = Some(Who::Player);
            forgive_dead_player(&mut m);
            assert_eq!(m.ai.target, None, "{species:?}");
            assert_eq!(m.ai.last_hurt_by, None, "{species:?}");
            // Anger at something else is not the dying player's to end.
            let mut m = mob(species);
            m.ai.target = Some(Who::Mob(7));
            forgive_dead_player(&mut m);
            assert_eq!(m.ai.target, Some(Who::Mob(7)), "{species:?}");
        }
    }

    /// Their anger target and timer (`updatePersistentAnger`) outlast the
    /// target they hold: `NeutralMob.playerDied` ends both through
    /// `stopBeingAngry` when they name the dying player, chased or out of
    /// sight, so the mob is not angry at them once they respawn.
    #[test]
    fn a_dying_player_ends_the_anger_target_and_timer() {
        for species in [Species::Enderman, Species::ZombifiedPiglin] {
            for target in [Some(Who::Player), None] {
                let mut m = mob(species);
                m.extra.anger = 400;
                m.extra.angry_at = Some(Who::Player);
                m.ai.target = target;
                assert!(forgives_dead_player(&m), "{species:?} {target:?}");
                forgive_dead_player(&mut m);
                assert_eq!(m.ai.target, None, "{species:?} {target:?}");
                assert!(
                    !crate::mobs::neutral_anger::angry_at_player(&m, true),
                    "{species:?} {target:?}"
                );
            }
        }
    }

    /// The iron golem's anger is a timer and its target: the dying player
    /// it is angry at ends both (`NeutralMob.playerDied`), even once the golem
    /// has lost sight of them, and anger at another mob stays.
    #[test]
    fn an_iron_golems_timer_and_target_end_together() {
        for target in [Some(Who::Player), None] {
            let mut golem = mob(Species::IronGolem);
            golem.extra.anger = 300;
            golem.extra.angry_at = Some(Who::Player);
            golem.ai.target = target;
            golem.ai.last_hurt_by = Some(Who::Player);
            forgive_dead_player(&mut golem);
            assert_eq!(golem.extra.anger, 0, "{target:?}");
            assert_eq!(golem.extra.angry_at, None, "{target:?}");
            assert_eq!(golem.ai.target, None, "{target:?}");
            assert_eq!(golem.ai.last_hurt_by, None, "{target:?}");
        }

        let mut golem = mob(Species::IronGolem);
        golem.extra.anger = 300;
        golem.extra.angry_at = Some(Who::Mob(7));
        golem.ai.target = Some(Who::Mob(7));
        forgive_dead_player(&mut golem);
        assert_eq!(golem.extra.anger, 300, "anger at another mob stays");
        assert_eq!(golem.ai.target, Some(Who::Mob(7)));
    }
}

#[cfg(test)]
mod endermite_goal_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// `Endermite.registerGoals` is the silverfish's family list without the
    /// stone tricks: a powder-snow climb, a stroll at 1.0 and a
    /// `HurtByTargetGoal` that alerts the others. The generic monster block
    /// an unlisted species gets has none of those (stroll 0.8, no alert).
    #[test]
    #[ignore = "daily"]
    fn an_endermite_takes_the_hostile_goal_list() {
        let ran = with_test_world(&[], |_, types| {
            let m = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:endermite"].clone()),
                Vec3::ZERO,
                0.0,
            );
            assert_eq!(family(m.species()), Family::Hostile);
            assert!(
                m.ai.goals.has("ClimbOnTopOfPowderSnow"),
                "ClimbOnTopOfPowderSnowGoal"
            );
            assert!(m.ai.goals.has("Stroll"), "WaterAvoidingRandomStrollGoal");
            assert!(m.ai.targets.has("HurtByTarget"));
            assert!(m.ai.targets.has("NearestTarget"));
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod villager_ambient_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// The ambient sound one tick of `play_ambient` makes, after `setup`.
    fn sound(
        w: &MobWorld<'_>,
        types: &crate::mobs::MobTypes,
        kind: &str,
        setup: impl FnOnce(&mut Mob),
    ) -> Option<&'static str> {
        let mut m = Mob::new(
            1,
            std::sync::Arc::new(types.types[kind].clone()),
            Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        setup(&mut m);
        let mut events = Vec::new();
        play_ambient(&mut m, w, &mut events);
        events.into_iter().find_map(|e| match e {
            MobEvent::Sound(s, ..) => Some(s),
            _ => None,
        })
    }

    /// `Villager.getAmbientSound`: silent while asleep, the trade sound while a
    /// player has its menu open, ambient otherwise. `WanderingTrader` only has
    /// the trade branch.
    #[test]
    #[ignore = "daily"]
    fn a_villager_is_silent_asleep_and_noisy_while_trading() {
        assert!(
            with_test_world(&[], |w, types| {
                assert_eq!(
                    sound(w, types, "minecraft:villager", |_| {}),
                    Some("entity.villager.ambient")
                );
                assert_eq!(
                    sound(w, types, "minecraft:villager", |m| m.extra.flag2 = true),
                    Some("entity.villager.trade")
                );
                assert_eq!(
                    sound(w, types, "minecraft:villager", |m| m.extra.ints[1] = 1),
                    None,
                    "asleep"
                );
                assert_eq!(
                    sound(w, types, "minecraft:villager", |m| {
                        m.extra.ints[1] = 1;
                        m.extra.flag2 = true;
                    }),
                    None,
                    "sleeping wins over trading"
                );
                let trading = |m: &mut Mob| m.extra.flag2 = true;
                assert_eq!(
                    sound(w, types, "minecraft:wandering_trader", trading),
                    Some("entity.wandering_trader.trade")
                );
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod fox_screech_tests {
    use super::*;
    use crate::mobs::PlayerView;
    use crate::mobs::world::with_test_world;

    fn player_at(x: f64) -> PlayerView {
        crate::mobs::world::test_player(Vec3::new(x, 64.0, 0.5))
    }

    /// The (sound, volume) of 200 ambient sounds from one fox in `w`.
    fn sounds(w: &MobWorld<'_>, types: &crate::mobs::MobTypes) -> Vec<(&'static str, f32)> {
        let kind = std::sync::Arc::new(types.types["minecraft:fox"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        let mut events = Vec::new();
        for _ in 0..200 {
            play_ambient(&mut m, w, &mut events);
        }
        events
            .into_iter()
            .filter_map(|e| match e {
                MobEvent::Sound(s, _, volume, _) => Some((s, volume)),
                _ => None,
            })
            .collect()
    }

    /// `Fox.getAmbientSound` and `playAmbientSound`: at night with nobody
    /// within 16 blocks, about one call in ten screeches at volume 2; by day,
    /// in the End, or with a player near, never.
    #[test]
    #[ignore = "daily"]
    fn a_fox_screeches_only_at_night_with_no_player_near() {
        assert!(
            with_test_world(&[], |w, types| {
                let mut night = w.clone();
                night.daylight = false;
                let alone = sounds(&night, types);
                assert_eq!(alone.len(), 200);
                let screeches: Vec<_> = alone
                    .iter()
                    .filter(|s| s.0 == "entity.fox.screech")
                    .collect();
                assert!(!screeches.is_empty(), "no screech in 200 night calls");
                assert!(screeches.len() < 60, "{} screeches", screeches.len());
                // Played by playAmbientSound itself at volume 2...
                assert!(screeches.iter().any(|s| s.1 == 2.0));
                // ...or picked again through Mob.playAmbientSound (makeSound, volume 1).
                assert!(screeches.iter().all(|s| s.1 == 2.0 || s.1 == 1.0));
                assert!(
                    alone
                        .iter()
                        .all(|s| s.0 == "entity.fox.screech" || s.0 == "entity.fox.ambient")
                );

                let by_day = sounds(w, types);
                assert!(by_day.iter().all(|s| s.0 == "entity.fox.ambient"));

                let mut end = night.clone();
                end.daylight = true;
                end.dimension_type =
                    horizon_world::dimension_type::DimensionFacts::vanilla("minecraft:the_end")
                        .unwrap();
                assert!(
                    sounds(&end, types)
                        .iter()
                        .any(|s| s.0 == "entity.fox.screech")
                );

                let mut near = night.clone();
                near.player = Some(player_at(16.0));
                assert!(
                    sounds(&near, types)
                        .iter()
                        .all(|s| s.0 == "entity.fox.ambient")
                );

                let mut far = night.clone();
                far.player = Some(player_at(18.0));
                assert!(
                    sounds(&far, types)
                        .iter()
                        .any(|s| s.0 == "entity.fox.screech")
                );

                let mut spectator = night.clone();
                let mut p = player_at(2.0);
                p.spectator = true;
                spectator.player = Some(p);
                assert!(
                    sounds(&spectator, types)
                        .iter()
                        .any(|s| s.0 == "entity.fox.screech")
                );
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod age_up_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn sleep_answer_is_applied_with_or_without_ai() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            for no_ai in [false, true] {
                for may_continue in [false, true] {
                    let kind = Arc::new(types.types["minecraft:villager"].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(4.0, 220.0, 4.0), 0.0);
                    m.no_ai = no_ai;
                    m.no_gravity = true;
                    m.persistent = true;
                    m.extra.ints[1] = 1;
                    let ext = super::super::villager::ext_mut(&mut m);
                    ext.sleeping_pos = Some([4, 220, 4]);
                    ext.may_continue_sleeping = Some(may_continue);
                    tick(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut Vec::new(),
                    );
                    assert_eq!(
                        super::super::villager::sleeping(&m),
                        may_continue,
                        "NoAI={no_ai}"
                    );
                }
            }
        });
        if !ran {
            eprintln!("sleep regression needs prepared vanilla resources");
        }
    }

    /// Official Java 26.3: NoAI leaves both age clocks running. AgeLocked
    /// holds a baby's age, but a positive breeding cooldown still expires.
    #[test]
    #[ignore = "daily"]
    fn age_clocks_tick_with_no_ai_and_respect_baby_age_lock() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            for species in ["pig", "cow", "horse", "turtle"] {
                for no_ai in [true, false] {
                    for locked in [false, true] {
                        for age in [-2, 2] {
                            let kind =
                                Arc::new(types.types[&format!("minecraft:{species}")].clone());
                            let mut m = Mob::new(1, kind, Vec3::new(4.0, 220.0, 4.0), 0.0);
                            m.no_ai = no_ai;
                            m.no_gravity = true;
                            m.persistent = true;
                            m.extra.age_locked = locked;
                            super::super::finalize::set_age(&mut m, age, None);
                            let (mut test_level, mut events) =
                                (crate::health::TestLevel::new(0), Vec::new());
                            let level = &mut test_level.level();
                            let expected = if age > 0 {
                                [1, 0, 0, 0]
                            } else if locked {
                                [-2, -2, -2, -2]
                            } else {
                                [-1, 0, 0, 0]
                            };
                            for (elapsed, expected_age) in expected.into_iter().enumerate() {
                                let previous_age = m.extra.age;
                                let previous_position = m.position;
                                tick(&mut m, w, level, &mut events);
                                assert_eq!(
                                    m.extra.age,
                                    expected_age,
                                    "{species}: NoAI={no_ai}, AgeLocked={locked}, initial Age={age}, tick={}",
                                    elapsed + 1
                                );
                                if no_ai {
                                    assert_eq!(m.velocity, Vec3::ZERO);
                                    // Growing dimensions can reposition the mob at adulthood.
                                    if previous_age != -1 || expected_age != 0 {
                                        assert_eq!(m.position, previous_position);
                                    }
                                }
                            }
                            if species == "turtle" && age < 0 && !locked {
                                assert_eq!(
                                    events
                                        .iter()
                                        .filter(|event| matches!(
                                            event,
                                            MobEvent::GiftLoot {
                                                table: "minecraft:gameplay/turtle_grow",
                                                ..
                                            }
                                        ))
                                        .count(),
                                    1,
                                    "growing up drops one scute"
                                );
                            }
                        }
                    }
                }
            }
        });
        if !ran {
            eprintln!("age clock regression needs prepared vanilla resources");
        }
    }

    /// A baby goat `age` ticks from adulthood, from no vanilla data.
    fn baby_goat(age: i32) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:goat".into();
        k.species = Species::Goat;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        // setAge(-24000) as finalizeSpawn does, whose own crossing is where
        // `Goat.ageBoundaryReached` gives the newborn its 1.0.
        super::super::finalize::set_age(&mut m, age, None);
        m
    }

    /// `AgeableMob.ageUp` ends in `setAge`, and `setAge` is where
    /// `Goat.ageBoundaryReached` sets the attack damage to 2.0 for an adult:
    /// feeding a baby goat its last two meals makes it a 2.0 goat, and the
    /// meal that leaves it one tick short keeps the baby's 1.0.
    #[test]
    fn feeding_a_baby_goat_past_its_boundary_sets_the_adult_attack_damage() {
        let mut m = baby_goat(-21);
        assert_eq!(m.attrs.attack_damage, 1.0, "a newborn goat hits for 1");

        // 20 ticks of growth: age -1, still a baby.
        age_up(&mut m, 1, true);
        assert!(m.is_baby());
        assert_eq!(m.attrs.attack_damage, 1.0);

        // The next 20 ticks cross the boundary.
        age_up(&mut m, 1, true);
        assert!(!m.is_baby());
        assert_eq!(m.attrs.attack_damage, 2.0);
    }

    /// The same crossing on the tick path: `AgeableMob.aiStep` also ages the
    /// mob through `setAge`, so a goat on its last baby tick grows up with the
    /// adult damage rather than keeping the baby's.
    #[test]
    #[ignore = "daily"]
    fn a_goat_that_grows_up_on_its_tick_gets_the_adult_attack_damage() {
        let mut out: Option<(bool, f32)> = None;
        crate::mobs::world::with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:goat"].clone());
            let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
            // As a natural spawn leaves it, then on its last baby tick.
            super::super::finalize::goat_spawn(
                &mut m,
                &mut horizon_core::random::LegacyRandom::new(0),
            );
            super::super::finalize::set_age(&mut m, -1, None);
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            tick(&mut m, w, level, &mut events);
            out = Some((m.is_baby(), m.attrs.attack_damage));
        });
        let Some((baby, damage)) = out else {
            return;
        };
        assert!(!baby, "the tick that reaches 0 grows it up");
        assert_eq!(damage, 2.0);
    }
}

#[cfg(test)]
mod in_wall_tests {
    use super::{Aabb, ShapeOp, Vec3, VoxelShape, in_wall};

    #[test]
    fn a_suffocating_partial_shape_must_reach_the_eye() {
        let slab = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let intersects = |x, y, z, eye: &Aabb| {
            (x, y, z) == (0, 0, 0)
                && slab
                    .join_is_not_empty(&VoxelShape::from_box(*eye).unwrap(), ShapeOp::AND)
                    .unwrap()
        };
        assert!(in_wall(0.6, [0.5, 0.25, 0.5], intersects));
        assert!(!in_wall(0.6, [0.5, 0.75, 0.5], intersects));
    }

    /// `Entity.isInWall` has no width guard (#5872): a hoglin (1.3965 wide)
    /// whose eye is in a solid block, or whose 1.117-wide eye box reaches a
    /// neighbouring wall column, takes in-wall damage like a narrow mob.
    #[test]
    fn wide_mobs_suffocate_by_the_eye_box() {
        let wall = |x: i32, y: i32, z: i32, eye: &Aabb| {
            x >= 0
                && VoxelShape::block()
                    .moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)))
                    .join_is_not_empty(&VoxelShape::from_box(*eye).unwrap(), ShapeOp::AND)
                    .unwrap()
        };
        // Eye inside the wall: hoglin and zombie (0.6) alike.
        assert!(in_wall(1.3965, [0.5, 64.5, 0.5], wall));
        assert!(in_wall(0.6, [0.5, 64.5, 0.5], wall));
        // Eye in air at x=-0.2: the zombie's 0.48 box (-0.44..0.04) clips
        // x=0, and the hoglin's wider box does too.
        assert!(in_wall(0.6, [-0.2, 64.5, 0.5], wall));
        // At x=-0.3 the zombie's box (-0.54..-0.06) clears the wall; the
        // hoglin's half-width 0.5586 (-0.8586..0.2586) does not.
        assert!(!in_wall(0.6, [-0.3, 64.5, 0.5], wall));
        assert!(in_wall(1.3965, [-0.3, 64.5, 0.5], wall));
        // Open air.
        assert!(!in_wall(1.3965, [-3.0, 64.5, 0.5], wall));
        // A box edge exactly on the wall face only touches it.
        assert!(!in_wall(1.0, [-0.4, 64.5, 0.5], wall));
    }
}

#[cfg(test)]
mod burn_and_rose_tests {
    use super::*;
    use horizon_world::registry::BlockRegistry;
    use std::sync::Arc;

    fn build_with(block: &'static str) -> impl FnOnce(&mut crate::world::World, &BlockRegistry) {
        move |world, registry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let state = registry.default_state(block).unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            world.set(0, 64, 0, state).unwrap();
        }
    }

    fn golem(types: &crate::mobs::MobTypes) -> Mob {
        let kind = Arc::new(types.types["minecraft:iron_golem"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        register(&mut m);
        m
    }

    fn zombie(types: &crate::mobs::MobTypes) -> Mob {
        let kind = Arc::new(types.types["minecraft:zombie"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        register(&mut m);
        m.no_ai = true;
        m.random =
            horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(3));
        m
    }

    /// `Mob.isSunBurnTick`: `isInWaterOrRain() || isInPowderSnow || wasInPowderSnow`
    /// is not burnable, so a zombie standing in powder snow only freezes. The
    /// draws are the mob's own, so the answer over a run of them is fixed.
    #[test]
    #[ignore = "daily"]
    fn powder_snow_keeps_a_zombie_from_burning_in_daylight() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:powder_snow"),
            |w, types| {
                let mut day = w.clone();
                day.monsters_burn = true;
                let mut m = zombie(types);
                let burns = |m: &mut Mob| (0..500).any(|_| sun_burn_tick(m, &day));
                assert!(!burns(&mut m), "in the powder snow at (0, 64, 0)");
                // The same mob out of it, in plain daylight: the same draws burn it.
                m.position = Vec3::new(3.5, 64.0, 3.5);
                assert!(!m.in_powder_snow);
                assert!(burns(&mut m), "in plain daylight");
                // Having been in the powder snow last tick (`wasInPowderSnow`).
                m.in_powder_snow = true;
                assert!(!burns(&mut m), "was in the powder snow");
                // It waits that one tick before the sun reaches it.
                m.in_powder_snow = false;
                assert!(burns(&mut m), "the tick after it stepped out");
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `PowderSnowBlock.entityInside`: a mob that is already burning destroys
    /// the powder snow (`mobGriefing`) and is put out, once, and then no longer
    /// catches fire there in daylight.
    #[test]
    #[ignore = "daily"]
    fn a_burning_zombie_in_powder_snow_breaks_the_block_and_is_put_out() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:powder_snow"),
            |w, types| {
                let mut day = w.clone();
                day.monsters_burn = true;
                let mut m = zombie(types);
                m.ignite_for_ticks(100);
                assert!(m.is_on_fire());
                let (mut lr, mut events) = (crate::health::TestLevel::new(7), Vec::new());
                assert!(tick(&mut m, &day, &mut lr.level(), &mut events));
                let broken: Vec<([i32; 3], bool)> = events
                    .iter()
                    .filter_map(|e| match e {
                        MobEvent::BreakBlock { pos, drop, .. } => Some((*pos, *drop)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(broken, vec![([0, 64, 0], false)], "the powder snow breaks");
                assert_eq!(m.fire_ticks, 0, "and puts the zombie out");
                events.clear();
                for _ in 0..200 {
                    tick(&mut m, &day, &mut lr.level(), &mut events);
                }
                assert!(!m.removed, "the zombie stays");
                assert_eq!(m.fire_ticks, 0, "no sun burn while it stands in it");
                assert!(
                    !events
                        .iter()
                        .any(|e| matches!(e, MobEvent::BreakBlock { .. })),
                    "nothing left to destroy"
                );
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `WitherRoseBlock.entityInside` tests the world difficulty and
    /// `isInvulnerableTo(wither)`, not the regional difficulty of a fresh
    /// chunk and not the hit timer: a golem that was just hurt still gets it.
    #[test]
    #[ignore = "daily"]
    fn wither_rose_applies_wither_whatever_the_hit_timer_and_local_difficulty() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:wither_rose"),
            |w, types| {
                let mut m = golem(types);
                m.invulnerable = 15;
                assert_eq!(w.difficulty.base, 2);
                flower_contact_effects(&mut m, w);
                assert!(m.has_effect("minecraft:wither"), "normal difficulty");

                let mut peaceful = w.clone();
                peaceful.difficulty = crate::mobs::spawning::LocalDifficulty::new(0, 18000, 0, 0.0);
                let mut m = golem(types);
                flower_contact_effects(&mut m, &peaceful);
                assert!(!m.has_effect("minecraft:wither"), "peaceful");
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `Entity.checkInsideBlocks` runs `WitherRoseBlock.entityInside` for every
    /// block the box sweeps between the tick's start and end: a zombie falling
    /// three blocks in one tick (about terminal speed) passes through the
    /// rose's cell between the two boxes and gets wither, while one standing
    /// beside it does not.
    #[test]
    #[ignore = "daily"]
    fn a_mob_falling_past_a_wither_rose_gets_wither_and_one_beside_it_does_not() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:wither_rose"),
            |w, types| {
                // Boxes 65..67 and 62..64: the rose's cell 64 is between them.
                let mut falling = zombie(types);
                falling.previous = Vec3::new(0.5, 65.0, 0.5);
                falling.position = Vec3::new(0.5, 62.0, 0.5);
                flower_contact_effects(&mut falling, w);
                assert!(
                    falling.has_effect("minecraft:wither"),
                    "inside the rose on the way past it"
                );

                let mut beside = zombie(types);
                beside.previous = Vec3::new(3.5, 64.0, 0.5);
                beside.position = Vec3::new(3.5, 64.0, 0.5);
                flower_contact_effects(&mut beside, w);
                assert!(!beside.has_effect("minecraft:wither"), "beside it");
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `applyEffectsFromBlocks` runs after `baseTick` counted the fire and
    /// the invulnerable time down: standing in lava a golem reads Fire 300
    /// (not 299) after its first tick, and the hit that tick starts has its
    /// full invulnerable time.
    #[test]
    #[ignore = "daily"]
    fn lava_ignites_to_the_full_time_and_hits_after_the_timers_count_down() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:lava"),
            |w, types| {
                let mut m = golem(types);
                let mut events = Vec::new();
                let start = m.health;
                base_tick(&mut m, w, level, &mut events);
                assert_eq!(m.health, start - 4.0, "one lava hit");
                assert_eq!(m.fire_ticks, 300, "fire after the first tick");
                assert_eq!(m.invulnerable, 20, "the hit's own invulnerable time");
                for _ in 0..10 {
                    base_tick(&mut m, w, level, &mut events);
                }
                assert_eq!(m.health, start - 8.0, "second hit on tick 11");
                assert_eq!(m.fire_ticks, 300);
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// `LavaCauldronBlock.entityInside` ignites and hurts a mob standing in a
    /// lava cauldron as lava does (#13100: a chicken farm's cauldron).
    #[test]
    #[ignore = "daily"]
    fn a_lava_cauldron_burns_the_mob_inside_it() {
        let build = build_with("minecraft:lava_cauldron");
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let mut m = golem(types);
            let mut events = Vec::new();
            let start = m.health;
            base_tick(&mut m, w, level, &mut events);
            assert_eq!(m.health, start - 4.0, "one lava hit");
            assert_eq!(m.fire_ticks, 300, "ignited to the full time");
        });
        assert!(ran, "the vanilla test resources must be available");
    }
}

#[cfg(test)]
mod wither_block_slowdown_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world;
    use horizon_world::registry::BlockRegistry;
    use std::sync::Arc;

    fn build_with(block: &'static str) -> impl FnOnce(&mut crate::world::World, &BlockRegistry) {
        move |world, registry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let state = registry.default_state(block).unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            world.set(0, 64, 0, state).unwrap();
        }
    }

    fn mob(types: &crate::mobs::MobTypes, id: &str) -> Mob {
        let kind = Arc::new(types.types[id].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        register(&mut m);
        m
    }

    /// `SweetBerryBushBlock.entityInside` -> `makeStuckInBlock`: a regular mob
    /// (e.g. cow) is slowed down and its fall distance is reset to 0.
    #[test]
    #[ignore = "daily"]
    fn cow_gets_stuck_in_sweet_berry_bush() {
        let ran =
            with_built_test_world(&[], build_with("minecraft:sweet_berry_bush"), |w, types| {
                let mut m = mob(types, "minecraft:cow");
                m.fall_distance = 4.5;
                m.stuck_multiplier = Vec3::ZERO;
                let mut test_level = crate::health::TestLevel::new(42);
                let mut level = test_level.level();
                let mut events = Vec::new();
                effects_from_blocks(&mut m, w, &mut level, &mut events);
                assert_eq!(m.fall_distance, 0.0, "cow fall distance reset");
                assert_eq!(
                    m.stuck_multiplier,
                    Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32)),
                    "cow stuck multiplier applied"
                );
            });
        assert!(ran, "vanilla data is missing");
    }

    /// `PowderSnowBlock.entityInside` -> `makeStuckInBlock`: a regular mob
    /// (e.g. cow) in powder snow is slowed down and its fall distance is reset to 0.
    #[test]
    #[ignore = "daily"]
    fn cow_gets_stuck_in_powder_snow() {
        let ran = with_built_test_world(&[], build_with("minecraft:powder_snow"), |w, types| {
            let mut m = mob(types, "minecraft:cow");
            m.fall_distance = 4.5;
            m.stuck_multiplier = Vec3::ZERO;
            let mut test_level = crate::health::TestLevel::new(42);
            let mut level = test_level.level();
            let mut events = Vec::new();
            effects_from_blocks(&mut m, w, &mut level, &mut events);
            assert_eq!(m.fall_distance, 0.0, "cow fall distance reset");
            assert_eq!(
                m.stuck_multiplier,
                Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)),
                "cow powder snow stuck multiplier applied"
            );
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `WitherBoss.makeStuckInBlock` is a NO-OP: a wither inside sweet berry bush
    /// does not receive stuck multiplier and preserves its fall distance.
    #[test]
    #[ignore = "daily"]
    fn wither_does_not_get_stuck_in_sweet_berry_bush() {
        let ran =
            with_built_test_world(&[], build_with("minecraft:sweet_berry_bush"), |w, types| {
                let mut m = mob(types, "minecraft:wither");
                m.fall_distance = 4.5;
                m.stuck_multiplier = Vec3::ZERO;
                let mut test_level = crate::health::TestLevel::new(42);
                let mut level = test_level.level();
                let mut events = Vec::new();
                effects_from_blocks(&mut m, w, &mut level, &mut events);
                assert_eq!(m.fall_distance, 4.5, "wither fall distance preserved");
                assert_eq!(m.stuck_multiplier, Vec3::ZERO, "wither not slowed");
            });
        assert!(ran, "vanilla data is missing");
    }

    /// `WitherBoss.makeStuckInBlock` is a NO-OP: a wither inside powder snow
    /// does not receive stuck multiplier and preserves its fall distance.
    #[test]
    #[ignore = "daily"]
    fn wither_does_not_get_stuck_in_powder_snow() {
        let ran = with_built_test_world(&[], build_with("minecraft:powder_snow"), |w, types| {
            let mut m = mob(types, "minecraft:wither");
            m.fall_distance = 4.5;
            m.stuck_multiplier = Vec3::ZERO;
            let mut test_level = crate::health::TestLevel::new(42);
            let mut level = test_level.level();
            let mut events = Vec::new();
            effects_from_blocks(&mut m, w, &mut level, &mut events);
            assert_eq!(m.fall_distance, 4.5, "wither fall distance preserved");
            assert_eq!(m.stuck_multiplier, Vec3::ZERO, "wither not slowed");
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `SweetBerryBushBlock.entityInside`: Wither is NOT exempt from damage
    /// when moving through a mature bush (age > 0).
    #[test]
    #[ignore = "daily"]
    fn wither_takes_damage_when_moving_in_mature_sweet_berry_bush() {
        let ran = with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                let default_bush = registry
                    .default_state("minecraft:sweet_berry_bush")
                    .unwrap();
                let mut state = registry.state(default_bush).unwrap().clone();
                state.properties.insert("age".into(), "3".into());
                let mature_bush = registry.id(&state).unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                world.set(0, 64, 0, mature_bush).unwrap();
            },
            |w, types| {
                let mut m = mob(types, "minecraft:wither");
                let initial_health = m.health;
                m.previous = Vec3::new(0.5, 64.0, 0.5);
                m.position = Vec3::new(0.52, 64.0, 0.5);
                let mut test_level = crate::health::TestLevel::new(42);
                let mut level = test_level.level();
                let mut events = Vec::new();
                effects_from_blocks(&mut m, w, &mut level, &mut events);
                assert_eq!(
                    m.health,
                    initial_health - 0.86,
                    "wither takes sweet berry bush damage through armor"
                );
                assert_eq!(m.stuck_multiplier, Vec3::ZERO, "wither still not slowed");
            },
        );
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod silverfish_walk_target_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world;

    /// `Silverfish.getWalkTargetValue`: a block standing on an infested host
    /// (a stone-family block) is worth 10, and everywhere else falls through
    /// to the monster's light value. Horizon treated stone like any other
    /// block, so a silverfish had no reason to walk back into the wall.
    #[test]
    #[ignore = "daily"]
    fn a_silverfish_prefers_a_block_on_an_infested_host() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world.chunks.insert(
                        (cx, cz),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                    );
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 64, z, stone).unwrap();
                }
            }
            world.set(2, 64, 0, dirt).unwrap();
        };
        let ran = with_built_test_world(&[], build, |w, types| {
            let m = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:silverfish"].clone()),
                Vec3::new(0.5, 65.0, 0.5),
                0.0,
            );
            assert_eq!(
                walk_target_value(&m, w, [0, 65, 0]),
                10.0,
                "standing on stone"
            );
            let dirt = walk_target_value(&m, w, [2, 65, 0]);
            assert!(dirt < 10.0, "standing on dirt falls through, got {dirt}");
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod iron_golem_push_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    fn mob(id: u32, species: Species, category: &str) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = species;
        kind.category = category.into();
        Mob::new(id, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// `IronGolem.doPush`: a monster that is not a creeper and bumps the golem
    /// becomes its target one time in 20, and the draw is made for those alone,
    /// so a creeper or an animal leaves the golem's random alone.
    #[test]
    #[ignore = "daily"]
    fn an_iron_golem_targets_the_monster_that_bumps_it() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // CI has no prepared 26.3 data, so a world that does not build skips
        // the whole check rather than asserting the data exists.
        let mut hits = |victim: Mob| -> Option<u32> {
            let seen = victim.seen(None, &crate::items::Items::empty());
            let mut hits = 0;
            let ran = with_test_world(&[seen], |w, _| {
                let mut golem = mob(1, Species::IronGolem, "monster");
                for _ in 0..400 {
                    let mut events = Vec::new();
                    push_entities(&mut golem, w, level, &mut events);
                    if golem.ai.target.is_some() {
                        hits += 1;
                        golem.ai.target = None;
                    }
                }
            });
            ran.then_some(hits)
        };
        let Some(zombie) = hits(mob(2, Species::Zombie, "monster")) else {
            return;
        };
        assert!(
            zombie > 0,
            "a zombie that bumps the golem is taken as its target"
        );
        assert_eq!(
            hits(mob(3, Species::Creeper, "monster")),
            Some(0),
            "IronGolem.doPush never targets a creeper"
        );
        assert_eq!(
            hits(mob(4, Species::Cow, "creature")),
            Some(0),
            "a pushed animal is not an Enemy"
        );
    }
    /// #7309: a mob's melee hit is its attack damage attribute plus the
    /// `attack_damage` modifier of the held item (`Mob.doHurtTarget`), for
    /// every mob that holds a weapon. The modifiers are vanilla's, from the
    /// items' components in the data reports (iron axe 8, iron sword 5,
    /// diamond sword 6, stone sword 4, golden axe 6, trident 8, bow none).
    #[test]
    #[ignore = "daily"]
    fn a_held_weapon_adds_its_attack_damage_to_the_melee_hit() {
        // (mob, main hand, the item's attack_damage modifier)
        let cases: [(&str, Option<&str>, f32); 9] = [
            ("minecraft:vindicator", Some("minecraft:iron_axe"), 8.0),
            ("minecraft:vindicator", None, 0.0),
            ("minecraft:zombie", Some("minecraft:iron_sword"), 5.0),
            ("minecraft:zombie", Some("minecraft:diamond_sword"), 6.0),
            ("minecraft:zombie", None, 0.0),
            (
                "minecraft:wither_skeleton",
                Some("minecraft:stone_sword"),
                4.0,
            ),
            ("minecraft:skeleton", Some("minecraft:bow"), 0.0),
            ("minecraft:piglin_brute", Some("minecraft:golden_axe"), 6.0),
            ("minecraft:drowned", Some("minecraft:trident"), 8.0),
        ];
        let mut got = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            for (kind, hand, _) in cases {
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                m.extra.set_main_hand(hand.map(Arc::from));
                let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                let mut cx = Cx {
                    w,
                    level: &mut test_level.level(),
                    events: &mut events,
                };
                do_hurt_target(&mut m, Who::Player, &mut cx);
                got.push((
                    m.attrs.attack_damage,
                    events.iter().find_map(|e| match e {
                        MobEvent::AttackPlayer(amount, _)
                        | MobEvent::AttackPlayerWithEffect { amount, .. } => Some(*amount),
                        _ => None,
                    }),
                ));
            }
        });
        if !ran {
            return;
        }
        for ((kind, hand, modifier), (base, hit)) in cases.iter().zip(got) {
            assert_eq!(hit, Some(base + modifier), "{kind} holding {hand:?}");
        }
    }
}

#[cfg(test)]
mod allay_healing_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn allays_heal_on_tenth_ticks_without_enabling_noai_work() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            for no_ai in [true, false] {
                for initial in [10.0_f32, 19.5, 20.0] {
                    let kind = Arc::new(types.types["minecraft:allay"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(16.5, 101.0, 16.5), 0.0);
                    mob.no_ai = no_ai;
                    mob.no_gravity = true;
                    mob.persistent = true;
                    mob.health = initial;
                    mob.extra.ints[2] = 37;
                    mob.extra.allay.get_or_insert_with(Default::default);
                    super::super::allay::set_pickup_cooldown(&mut mob, Some(41));
                    mob.extra.egg_time = 30;
                    assert_eq!(mob.attrs.max_health, 20.0);
                    assert_eq!(mob.age, 0);
                    let mut test_level = crate::health::TestLevel::new(0);
                    let mut level = test_level.level();
                    let mut events = Vec::new();
                    for elapsed in 1..=20 {
                        assert!(tick(&mut mob, w, &mut level, &mut events));
                        assert_eq!(mob.age, elapsed);
                        assert_eq!(
                            mob.health,
                            (initial + (elapsed / 10) as f32).min(20.0),
                            "NoAI={no_ai}, initial={initial}, tick={elapsed}"
                        );
                        assert_eq!(mob.extra.egg_time, 30 - elapsed as i32);
                        let skipped = if no_ai { 0 } else { elapsed as i32 };
                        assert_eq!(mob.extra.ints[2], 37 - skipped);
                        assert_eq!(
                            super::super::allay::pickup_cooldown(&mob),
                            Some(41 - skipped)
                        );
                    }
                }
            }
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; Allay tick coverage did not run");
        }
    }
}

#[cfg(test)]
mod ravager_noai_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn ravager_counters_tick_without_ai_and_stun_starts_a_full_roar() {
        crate::mobs::world::with_test_world(&[], |w, types| {
            for no_ai in [false, true] {
                for initial in [(20, 30, 5), (0, 0, 0), (-2, -3, -4), (2, 2, 0)] {
                    let kind = Arc::new(types.types["minecraft:ravager"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(10.5, 200.0, 10.5), 0.0);
                    mob.no_ai = no_ai;
                    mob.no_gravity = true;
                    mob.persistent = true;
                    mob.extra.counter = initial.0;
                    mob.extra.counter2 = initial.1;
                    mob.extra.anim_ticks = initial.2;
                    let mut test_level = crate::health::TestLevel::new(0);
                    let mut events = Vec::new();
                    for elapsed in 1..=22 {
                        tick(&mut mob, w, &mut test_level.level(), &mut events);
                        let attack = if initial.0 > 0 {
                            (initial.0 - elapsed).max(0)
                        } else {
                            initial.0
                        };
                        let stun = if initial.1 > 0 {
                            (initial.1 - elapsed).max(0)
                        } else {
                            initial.1
                        };
                        let roar = match initial {
                            (20, 30, 5) => (5 - elapsed).max(0),
                            (2, 2, 0) if elapsed >= 2 => (22 - elapsed).max(0),
                            _ => initial.2,
                        };
                        assert_eq!(
                            (mob.extra.counter, mob.extra.counter2, mob.extra.anim_ticks),
                            (attack, stun, roar),
                            "NoAI={no_ai}, initial={initial:?}, tick={elapsed}"
                        );
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod witch_noai_tests {
    use super::*;
    use std::sync::Arc;

    /// Vanilla 26.3: `LivingEntity.tick` calls `aiStep` whether or not the
    /// mob has AI, and `Witch.aiStep` picks a potion (fire resistance while
    /// it burns) before `super.aiStep()`. A burning NoAI witch drinks it
    /// (#16909's sweep: vanilla holds the potion, Horizon held nothing).
    #[test]
    #[ignore = "daily"]
    fn a_burning_noai_witch_still_drinks_fire_resistance() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:witch"].clone());
            let mut mob = Mob::new(1, kind, Vec3::new(16.5, 101.0, 16.5), 0.0);
            mob.no_ai = true;
            mob.no_gravity = true;
            mob.persistent = true;
            mob.fire_ticks = 300;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let drank = (1..=200).find(|_| {
                tick(&mut mob, w, &mut test_level.level(), &mut events);
                mob.has_effect("minecraft:fire_resistance")
            });
            assert!(
                drank.is_some(),
                "a burning NoAI witch had no fire resistance after 200 ticks"
            );
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; witch NoAI coverage did not run");
        }
    }
}

#[cfg(test)]
mod air_tests {
    use super::*;

    /// A mob of `kind` in a pool, its eyes under, after `ticks` base ticks.
    fn air_after(kind: &str, air: i32, ticks: u32) -> Option<i32> {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let mut out = None;
        crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types[kind].clone());
            let mut m = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
            m.air = air;
            let mut events = Vec::new();
            let mut test_level = crate::health::TestLevel::new(0);
            for _ in 0..ticks {
                base_tick(&mut m, w, &mut test_level.level(), &mut events);
            }
            out = Some(m.air);
        });
        out
    }

    /// `LivingEntity.canBreatheUnderwater` on the drown path: the
    /// `can_breathe_under_water` tag (a guardian and a copper golem are in it, an
    /// iron golem is not), with the baby happy ghast and the sulfur cube holding a
    /// block on top of it and the golem's own `decreaseAirSupply` holding its air.
    /// 300 ticks take the air from 300 to 0 and 20 more deal the drown's 2.
    #[test]
    #[ignore = "daily"]
    fn the_types_the_jar_lists_breathe_under_water() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let drowned = |id: &str, baby: bool, body: Option<&str>| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
                if baby {
                    m.extra.age = -24000;
                }
                m.extra.set_body(body.map(Into::into));
                let start = m.health;
                let mut events = Vec::new();
                let mut test_level = crate::health::TestLevel::new(0);
                for _ in 0..320 {
                    base_tick(&mut m, w, &mut test_level.level(), &mut events);
                }
                (start - m.health, m.air)
            };
            // The tag itself, from the shipped data: the golems differ.
            assert!(types.types["minecraft:copper_golem"].flags.breathes_water);
            assert!(!types.types["minecraft:iron_golem"].flags.breathes_water);
            assert_eq!(drowned("minecraft:guardian", false, None), (0.0, 300));
            assert_eq!(drowned("minecraft:copper_golem", false, None), (0.0, 300));
            // Not in the tag, and still no drowning damage: its decreaseAirSupply
            // is the identity, so its air stays at 300 and it never refills.
            assert_eq!(drowned("minecraft:iron_golem", false, None), (0.0, 300));
            assert_eq!(drowned("minecraft:happy_ghast", false, None), (2.0, 0));
            assert_eq!(drowned("minecraft:happy_ghast", true, None), (0.0, 300));
            assert_eq!(drowned("minecraft:sulfur_cube", false, None), (2.0, 0));
            assert_eq!(
                drowned("minecraft:sulfur_cube", false, Some("minecraft:stone")),
                (0.0, 300)
            );
        });
        // Without the extracted vanilla data the world cannot be built.
        assert!(ran, "vanilla data is missing");
    }

    /// `Dolphin.handleAirSupply` is empty and the dolphin isn't in
    /// `#can_breathe_under_water`: under water it loses a breath a tick from
    /// its 4800, as `LivingEntity.baseTick` takes it, so it must surface.
    #[test]
    #[ignore = "daily"]
    fn a_dolphin_holds_its_breath_under_water() {
        let Some(air) = air_after("minecraft:dolphin", 4800, 10) else {
            return;
        };
        assert_eq!(air, 4790);
    }

    /// `Dolphin.increaseAirSupply` hands back the whole 4800 maximum, so under Water
    /// Breathing a submerged dolphin is at 4800 on the next tick. An ordinary mob
    /// still climbs four at a time to 300, and a fish's own handler fills it to
    /// 300 in water. Air above the maximum is left alone.
    #[test]
    #[ignore = "daily"]
    fn a_dolphin_refills_to_its_own_maximum() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let after = |id: &str, start: i32, breathing: bool| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
                if breathing {
                    m.add_effect("minecraft:water_breathing", 100, 0);
                }
                m.air = start;
                let mut events = Vec::new();
                let mut test_level = crate::health::TestLevel::new(0);
                base_tick(&mut m, w, &mut test_level.level(), &mut events);
                (m.air, m.health)
            };
            assert_eq!(after("minecraft:dolphin", 100, true), (4800, 10.0));
            assert_eq!(after("minecraft:dolphin", 4800, false).0, 4799);
            assert_eq!(after("minecraft:dolphin", 5000, true).0, 5000);
            assert_eq!(after("minecraft:zombie", 100, true).0, 104);
            assert_eq!(after("minecraft:zombie", 299, true).0, 300);
            assert_eq!(after("minecraft:cod", 100, false).0, 300);
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `WaterAnimal.handleAirSupply` asks `shouldTakeDrowningDamage`, which is
    /// `getAirSupply() <= -20`: a cod out of water with `{Air:-25}` still drowns, is
    /// reset to 0 and takes the 2.
    #[test]
    #[ignore = "daily"]
    fn a_water_creature_below_minus_twenty_still_drowns() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let drowned = |start: i32| {
                let kind = std::sync::Arc::new(types.types["minecraft:cod"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.air = start;
                let health = m.health;
                let mut events = Vec::new();
                let mut test_level = crate::health::TestLevel::new(0);
                base_tick(&mut m, w, &mut test_level.level(), &mut events);
                (health - m.health, m.air)
            };
            assert_eq!(drowned(-25), (2.0, 0));
            assert_eq!(drowned(-20), (2.0, 0));
        });
        assert!(ran, "vanilla data is missing");
    }

    /// A copper golem is in `#can_breathe_under_water`: its air stays full.
    #[test]
    #[ignore = "daily"]
    fn a_copper_golem_breathes_under_water() {
        let Some(air) = air_after("minecraft:copper_golem", 300, 10) else {
            return;
        };
        assert_eq!(air, 300);
    }
}

#[cfg(test)]
mod flight_effect_fall_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn flight_effects_reset_accumulated_fall_distance_even_without_ai() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, _| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            },
            |world, types| {
                let mut failures = Vec::new();
                let mut rows = Vec::new();
                for effect in [
                    None,
                    Some("minecraft:levitation"),
                    Some("minecraft:slow_falling"),
                ] {
                    for no_ai in [false, true] {
                        for velocity_y in [0.0, 0.2, -0.2] {
                            let kind = Arc::new(types.types["minecraft:cow"].clone());
                            let mut mob = Mob::new(1, kind, Vec3::new(2.5, 64.0, 2.5), 0.0);
                            mob.no_ai = no_ai;
                            mob.no_gravity = true;
                            mob.persistent = true;
                            mob.fall_distance = 12.0;
                            mob.velocity.y = velocity_y;
                            if let Some(effect) = effect {
                                mob.add_effect(effect, 100, 0);
                            }
                            let mut test_level = crate::health::TestLevel::new(0);
                            assert!(tick(
                                &mut mob,
                                world,
                                &mut test_level.level(),
                                &mut Vec::new()
                            ));
                            // The pinned Java aiStep resets before travel. A
                            // downward move then adds 0.2F of NEW fall distance;
                            // NoAI skips movement but must still erase the old fall.
                            let expected = match (effect.is_some(), no_ai, velocity_y < 0.0) {
                                (false, false, true) => 12.200000002980232,
                                (true, false, true) => 0.20000000298023224,
                                (false, _, _) => 12.0,
                                (true, _, _) => 0.0,
                            };
                            let name = effect.unwrap_or("none").trim_start_matches("minecraft:");
                            rows.push(format!(
                                "{name} {no_ai} {:016x} {:016x}",
                                velocity_y.to_bits(),
                                mob.fall_distance.to_bits()
                            ));
                            if mob.fall_distance != expected {
                                failures.push((effect, no_ai, velocity_y, mob.fall_distance));
                            }
                        }
                    }
                }
                if let Some(path) = std::env::var_os("HORIZON_MOB_FALL_RESET_OUTPUT") {
                    std::fs::write(path, rows.join("\n") + "\n").unwrap();
                }
                assert!(failures.is_empty(), "fall distance differs: {failures:?}");
            },
        );
        assert!(ran, "prepared resources must run the integration fixture");
    }
}

#[cfg(test)]
mod breathing_effect_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn breathing_effects_and_bubble_columns_preserve_mob_air() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, registry| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                for (x0, name) in [(6, "minecraft:water"), (10, "minecraft:bubble_column")] {
                    let state = registry.default_state(name).unwrap();
                    for x in x0..=x0 + 1 {
                        for z in 2..=3 {
                            for y in 63..=67 {
                                world.set(x, y, z, state).unwrap();
                            }
                        }
                    }
                }
                // Mixed columns distinguish the eye-block query from feet position.
                for (x0, bubble_at_eye) in [(6, true), (10, false)] {
                    for x in x0..=x0 + 1 {
                        for z in 6..=7 {
                            for y in 63..=67 {
                                let bubble = (y >= 65) == bubble_at_eye;
                                let name = if bubble {
                                    "minecraft:bubble_column"
                                } else {
                                    "minecraft:water"
                                };
                                world
                                    .set(x, y, z, registry.default_state(name).unwrap())
                                    .unwrap();
                            }
                        }
                    }
                }
            },
            |world, types| {
                let kind = Arc::new(types.types["minecraft:cow"].clone());
                assert!(!kind.flags.needs_water && !kind.flags.breathes_water);
                let mut rows = Vec::new();
                let mut failures = Vec::new();
                for (scene, x, z) in [
                    (0, 2.5, 2.5),
                    (1, 6.5, 2.5),
                    (2, 10.5, 2.5),
                    (3, 6.5, 6.5),
                    (4, 10.5, 6.5),
                ] {
                    for mask in 0..8 {
                        for air in [301, 300, 10, 0, -19, -20, -21] {
                            let mut mob = Mob::new(1, kind.clone(), Vec3::new(x, 64.0, z), 0.0);
                            mob.air = air;
                            for (bit, effect) in [
                                (1, "minecraft:water_breathing"),
                                (2, "minecraft:conduit_power"),
                                (4, "minecraft:breath_of_the_nautilus"),
                            ] {
                                if mask & bit != 0 {
                                    mob.add_effect(effect, 100, 0);
                                }
                            }
                            base_tick(
                                &mut mob,
                                world,
                                &mut crate::health::TestLevel::new(0).level(),
                                &mut Vec::new(),
                            );
                            assert_eq!(
                                mob.fluids.eye_in_water,
                                scene != 0,
                                "invalid fixture immersion"
                            );
                            rows.push(format!(
                                "{scene} {mask} {air} {} {:08x}",
                                mob.air,
                                mob.health.to_bits()
                            ));
                            // Selected independent goldens from the pinned Cow.baseTick
                            // oracle. The parity harness compares all 280 air/health rows.
                            let expected = match (scene, mask, air) {
                                (1, 1, 10)
                                | (1, 2, 10)
                                | (1, 5, 10)
                                | (2, 0, 10)
                                | (2, 4, 10)
                                | (0, 4, 10)
                                | (3, 0, 10) => Some((14, 10.0)),
                                (1, 4, 10) | (4, 4, 10) => Some((10, 10.0)),
                                (4, 0, 10) => Some((9, 10.0)),
                                (1, 0, -19) => Some((0, 8.0)),
                                _ => None,
                            };
                            if let Some(expected) = expected
                                && (mob.air, mob.health) != expected
                            {
                                failures.push((scene, mask, air, mob.air, mob.health));
                            }
                        }
                    }
                }
                if let Some(path) = std::env::var_os("HORIZON_MOB_BREATHING_OUTPUT") {
                    std::fs::write(path, rows.join("\n") + "\n").unwrap();
                }
                assert!(failures.is_empty(), "mob air/health differs: {failures:?}");
            },
        );
        assert!(
            ran,
            "prepared resources must run the production breathing fixture"
        );
    }
}

#[cfg(test)]
mod dolphin_noai_air_tests {
    use super::*;
    use std::sync::Arc;

    fn check(water: bool, no_ai: bool, initial: i32, expected: [i32; 2]) {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            if water {
                let water = registry.default_state("minecraft:water").unwrap();
                for x in 0..=4 {
                    for z in 0..=4 {
                        for y in 0..=8 {
                            world.set(x, y, z, water).unwrap();
                        }
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = Arc::new(types.types["minecraft:dolphin"].clone());
            let mut mob = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
            mob.no_ai = no_ai;
            mob.no_gravity = true;
            mob.persistent = true;
            mob.attrs.speed = 0.0;
            mob.air = initial;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            assert_eq!(mob.health, 10.0);
            assert_eq!(mob.max_air_supply(), 4800);
            for (index, expected) in expected.into_iter().enumerate() {
                assert!(tick(&mut mob, w, &mut test_level.level(), &mut events));
                assert_eq!(mob.age, (index + 1) as u32);
                assert_eq!(mob.fluids.eye_in_water, water);
                assert_eq!(mob.health, 10.0);
                assert_eq!(mob.no_ai, no_ai);
                assert_eq!(
                    mob.air,
                    expected,
                    "water={water}, NoAI={no_ai}, initial={initial}, tick={}",
                    index + 1
                );
            }
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; Dolphin full-tick coverage did not run");
        }
    }

    /// `Dolphin.tick` restores max air after super.tick for a NoAI dolphin,
    /// even when that superclass tick refilled or consumed loaded air.
    #[test]
    #[ignore = "daily"]
    fn noai_dolphins_restore_air_after_the_full_mob_tick() {
        for water in [false, true] {
            for initial in [4800, 100] {
                check(water, true, initial, [4800, 4800]);
            }
        }
    }

    /// The NoAI exception must not suppress normal underwater consumption.
    #[test]
    #[ignore = "daily"]
    fn normal_dolphins_keep_their_underwater_countdown() {
        check(true, false, 4800, [4799, 4798]);
    }
}

#[cfg(test)]
mod nautilus_noai_air_tests {
    use super::*;
    use std::sync::Arc;

    fn check(water: bool, no_ai: bool, initial: i32, expected: [i32; 2]) {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            if water {
                let state = registry.default_state("minecraft:water").unwrap();
                for x in 0..=4 {
                    for z in 0..=4 {
                        for y in 0..=8 {
                            world.set(x, y, z, state).unwrap();
                        }
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = Arc::new(types.types["minecraft:nautilus"].clone());
            assert!(kind.flags.needs_water && kind.flags.breathes_water);
            let mut mob = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
            mob.no_ai = no_ai;
            mob.no_gravity = true;
            mob.persistent = true;
            mob.attrs.speed = 0.0;
            mob.health = 10.0;
            mob.air = initial;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            for (index, expected) in expected.into_iter().enumerate() {
                assert!(tick(&mut mob, w, &mut test_level.level(), &mut events));
                assert_eq!(mob.age, (index + 1) as u32);
                assert_eq!(mob.fluids.eye_in_water, water);
                assert_eq!(mob.health, 10.0);
                assert_eq!(mob.no_ai, no_ai);
                assert_eq!(
                    mob.air,
                    expected,
                    "water={water}, NoAI={no_ai}, initial={initial}, tick={}",
                    index + 1
                );
            }
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; Nautilus full-tick coverage did not run");
        }
    }

    /// `Nautilus.baseTick`: NoAI skips only handleAirSupply; super.baseTick
    /// still refills a breathing mob by four, capped at300.
    #[test]
    #[ignore = "daily"]
    fn noai_nautilus_keeps_superclass_refill_in_both_media() {
        for water in [false, true] {
            check(water, true, 300, [300, 300]);
            check(water, true, 100, [104, 108]);
        }
    }

    /// With AI, handleAirSupply still uses the pre-super air on land and
    /// writes300 in water. The NoAI exception must not alter that policy.
    #[test]
    #[ignore = "daily"]
    fn normal_nautilus_keeps_its_custom_handler() {
        check(false, false, 300, [299, 298]);
        check(false, false, 100, [99, 98]);
        check(true, false, 300, [300, 300]);
        check(true, false, 100, [300, 300]);
    }
}

#[cfg(test)]
mod freeze_tests {
    use super::*;
    use std::sync::Arc;

    /// `LivingEntity.aiStep`'s freezing, through `freeze_tick`: powder snow
    /// builds a mob's frozen ticks towards the 140 that freeze it, and at 140 a
    /// tick that falls on the 40-tick damage interval hurts it by 1. A skeleton
    /// answers `canFreeze` false, so it takes neither, and so does the stray (by
    /// tag), while the bogged, parched, wither skeleton and a zombie freeze.
    #[test]
    #[ignore = "daily"]
    fn a_skeleton_never_freezes() {
        crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                let stone = registry.default_state("minecraft:stone").unwrap();
                let snow = registry.default_state("minecraft:powder_snow").unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                // The mob stands in this block.
                world.set(0, 64, 0, snow).unwrap();
            },
            |w, types| {
                let frozen_with = |id: &str, ticks_frozen: i32, body: Option<&str>| {
                    let kind = Arc::new(types.types[id].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                    m.ticks_frozen = ticks_frozen;
                    m.extra.set_body(body.map(Into::into));
                    // The freeze damage falls on a tick that is a multiple of 40.
                    m.age = 40;
                    let start = m.health;
                    freeze_tick(&mut m, w, &mut crate::health::TestLevel::new(0).level());
                    (m.ticks_frozen, start - m.health)
                };
                let frozen = |id: &str, ticks_frozen: i32| frozen_with(id, ticks_frozen, None);
                // Powder snow builds no frozen ticks on a skeleton, and one
                // already frozen melts two ticks a tick instead of taking the
                // damage a fully frozen mob takes at 140.
                for skeleton in ["minecraft:skeleton", "minecraft:stray"] {
                    assert_eq!(frozen(skeleton, 0), (0, 0.0), "{skeleton}");
                    assert_eq!(frozen(skeleton, 140), (138, 0.0), "{skeleton} frozen");
                }
                // The bogged, parched and wither skeleton extend AbstractSkeleton,
                // not Skeleton, and freeze like a zombie.
                for sibling in [
                    "minecraft:bogged",
                    "minecraft:parched",
                    "minecraft:wither_skeleton",
                ] {
                    assert_eq!(frozen(sibling, 0), (1, 0.0), "{sibling}");
                    assert_eq!(frozen(sibling, 139), (140, 1.0), "{sibling} frozen");
                }
                // A zombie in the same block freezes, and at 140 takes the 1.
                assert_eq!(frozen("minecraft:zombie", 139), (140, 1.0));
                // SulfurCube.canFreeze: a cube carrying a block in its body does
                // not freeze; without one it freezes and takes the 1 like a zombie.
                assert_eq!(
                    frozen_with("minecraft:sulfur_cube", 0, Some("minecraft:stone")),
                    (0, 0.0)
                );
                assert_eq!(
                    frozen_with("minecraft:sulfur_cube", 140, Some("minecraft:stone")),
                    (138, 0.0)
                );
                assert_eq!(frozen("minecraft:sulfur_cube", 0), (1, 0.0));
                assert_eq!(frozen("minecraft:sulfur_cube", 139), (140, 1.0));
            },
        );
    }
}
#[cfg(test)]
mod happy_ghast_ambient_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn natural_happy_ghast_ambient_uses_entity_position() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, _| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            },
            |w, types| {
                for baby in [false, true] {
                    let kind = Arc::new(types.types["minecraft:happy_ghast"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(10.5, 208.0, 10.5), 0.0);
                    mob.no_ai = true;
                    mob.no_gravity = true;
                    mob.persistent = true;
                    mob.extra.age = if baby { -24000 } else { 0 };
                    let sound = super::super::happy_ghast::sounds(&mob)[0];
                    let mut test_level = crate::health::TestLevel::new(0);
                    let level = &mut test_level.level();
                    let mut heard = false;
                    // Natural scheduling: no ambient counter is forced or reset here.
                    for _ in 0..1200 {
                        let mut events = Vec::new();
                        assert!(tick(&mut mob, w, level, &mut events));
                        for event in events {
                            if let MobEvent::Sound(name, at, _, _) = event
                                && name == sound
                            {
                                assert_eq!(at, [10.5, 208.0, 10.5], "baby={baby}");
                                heard = true;
                            }
                        }
                        if heard {
                            break;
                        }
                    }
                    assert!(heard, "no natural ambient event, baby={baby}");
                }
                // Entity.playSound plays nothing for a Silent entity.
                let kind = Arc::new(types.types["minecraft:happy_ghast"].clone());
                let mut mob = Mob::new(1, kind, Vec3::new(10.5, 208.0, 10.5), 0.0);
                mob.no_ai = true;
                mob.no_gravity = true;
                mob.persistent = true;
                mob.silent = true;
                let sound = super::super::happy_ghast::sounds(&mob)[0];
                let mut test_level = crate::health::TestLevel::new(0);
                let level = &mut test_level.level();
                for _ in 0..1200 {
                    let mut events = Vec::new();
                    assert!(tick(&mut mob, w, level, &mut events));
                    assert!(
                        !events
                            .iter()
                            .any(|e| matches!(e, MobEvent::Sound(name, ..) if *name == sound)),
                        "a Silent happy ghast is heard"
                    );
                }
            },
        );
        assert!(ran, "prepared-data callback did not run");
    }
}

#[cfg(test)]
mod hidden_speed_causal_tests {
    use super::*;
    use std::sync::Arc;
    fn mob() -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        Mob::new(202, Arc::new(kind), Vec3::ZERO, 0.0)
    }
    #[test]
    fn zero_active_never_promotes_and_infinite_fallback_counts_once() {
        let mut m = mob();
        m.add_effect("speed", 200, 0);
        m.add_effect("speed", 0, 1);
        assert_eq!(m.extra.effect_chains.len(), 1);
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
            |_| {},
        );
        assert!(m.extra.effects.is_empty() && m.extra.effect_chains.is_empty());
        m.add_effect("speed", -1, 0);
        m.add_effect("speed", 2, 1);
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
            |_| {},
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", 1, 1)]
        );
        assert_eq!(m.extra.effect_chains[0].duration, -1);
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
            |_| {},
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", -1, 0)]
        );
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
            |_| {},
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", -1, 0)]
        );
        assert!(m.extra.effect_chains.is_empty());
    }
    #[test]
    fn native_cancellation_drops_the_whole_chain_before_promotion() {
        struct Cancel(std::cell::Cell<usize>);
        impl crate::mobs::ModEffectTicks for Cancel {
            fn effect_expired(&self, _: u32, _: usize, _: i32, _: &mut bool) {}
            fn tick_mod_effect(
                &self,
                _: &mut DamageLevel<'_>,
                _: usize,
                m: &mut Mob,
                count: i32,
                amp: i32,
            ) -> bool {
                let (id, mob) = (m.extra.effects[0].id, m.id);
                assert_eq!((id, mob, amp), ("h202:cancel", 202, 1));
                assert!(count == 1 || count == 17);
                self.0.set(self.0.get() + 1);
                false
            }
        }
        for duration in [1, -1] {
            let mut m = mob();
            m.age = 17;
            m.add_effect("h202:cancel", 200, 0);
            if duration == -1 {
                // A loaded subtree can accompany an infinite active instance.
                m.extra.effects[0].duration = -1;
                let h = crate::effects::EffectInstance {
                    id: "h202:cancel",
                    registry_index: Some(0),
                    client_blend: (false, false),
                    duration: 200,
                    amplifier: 0,
                    ambient: false,
                    visible: true,
                    show_icon: true,
                    hidden: None,
                };
                m.extra.effects[0].amplifier = 1;
                m.extra.effect_chains.push(h);
            } else {
                m.add_effect("h202:cancel", duration, 1);
            }
            // This fixture stands in for a registered mod effect at slot zero.
            m.extra.effects[0].registry_index = Some(0);
            let cancel = Cancel(std::cell::Cell::new(0));
            tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                Some(&cancel),
                |_| {},
            );
            assert_eq!(cancel.0.get(), 1);
            assert!(m.extra.effects.is_empty() && m.extra.effect_chains.is_empty());
        }
    }
}

#[cfg(test)]
mod trader_llama_no_ai_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn eligible_trader_llama_delay_advances_in_actual_no_ai_tick() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let mut differences = Vec::new();
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let kind = Arc::new(types.types["minecraft:trader_llama"].clone());
                let fresh = || {
                    let mut mob = Mob::new(1, kind.clone(), Vec3::new(4.5, 200.0, 4.5), 0.0);
                    mob.no_ai = true;
                    mob.no_gravity = true;
                    mob.silent = true;
                    mob.extra.ints[0] = 5;
                    mob
                };
                // Existing eligibility stays in the shared helper. These are focused
                // model controls, not runtime claims about real leash/rider sessions.
                let mut trader = Mob::new(
                    50,
                    Arc::new(types.types["minecraft:wandering_trader"].clone()),
                    Vec3::new(30.5, 200.0, 30.5),
                    0.0,
                );
                trader.extra.ints[5] = 13;
                let cow = Mob::new(
                    51,
                    Arc::new(types.types["minecraft:cow"].clone()),
                    Vec3::new(40.5, 200.0, 40.5),
                    0.0,
                );
                let seen = [
                    trader.seen(None, &crate::items::Items::empty()),
                    cow.seen(None, &crate::items::Items::empty()),
                ];
                assert_eq!(seen[0].delay, 13);
                let view = MobWorld {
                    others: &seen,
                    ..w.clone()
                };
                for guard in [
                    "tame",
                    "persistent",
                    "age-lock",
                    "rider",
                    "other-mob-leash",
                    "player-leash",
                ] {
                    let mut mob = fresh();
                    match guard {
                        "tame" => mob.extra.tame = true,
                        "persistent" => mob.persistent = true,
                        "age-lock" => mob.extra.age_locked = true,
                        "rider" => mob.rider = Some(super::super::equine::Rider::default()),
                        "other-mob-leash" => {
                            mob.extra.leash = Some(super::super::leash::Holder::Mob(51))
                        }
                        "player-leash" => {
                            mob.extra.leash = Some(super::super::leash::Holder::Player)
                        }
                        _ => unreachable!(),
                    }
                    super::super::caravan::trader_llama_tick(&mut mob, &view);
                    assert_eq!(mob.extra.ints[0], 5, "{guard}");
                    assert!(!mob.removed, "{guard}");
                }
                let mut linked = fresh();
                linked.extra.leash = Some(super::super::leash::Holder::Mob(50));
                super::super::caravan::trader_llama_tick(&mut linked, &view);
                assert_eq!(linked.extra.ints[0], 12, "trader delay minus one");

                for initial in [0, 5, 47999] {
                    let mut mob = fresh();
                    mob.extra.ints[0] = initial;
                    mob.health = mob.attrs.max_health - 2.0;
                    let health = mob.health;
                    let steps = if initial == 0 { 1 } else { 5 };
                    for elapsed in 1..=steps {
                        let mut expected_random = mob.random.clone();
                        expected_random.next_int_bound(1000).unwrap(); // Existing Mob.baseTick ambient draw only.
                        let mut test_level = crate::health::TestLevel::new(0);
                        let mut events = Vec::new();
                        tick(&mut mob, w, &mut test_level.level(), &mut events);
                        assert_eq!(mob.age, elapsed as u32, "actual tick reached");
                        assert_eq!(
                            mob.health, health,
                            "NoAI must not run unrelated llama healing"
                        );
                        assert_eq!(
                            mob.random.state(),
                            expected_random.state(),
                            "NoAI must not add the healing draw"
                        );
                        let expected = initial - elapsed;
                        if mob.extra.ints[0] != expected || mob.removed != (expected <= 0) {
                            differences.push((
                                initial,
                                elapsed,
                                expected,
                                mob.extra.ints[0],
                                mob.removed,
                            ));
                        }
                    }
                }
                // The normal-AI caller already reaches the same timer.
                let mut normal = fresh();
                normal.no_ai = false;
                normal.health = normal.attrs.max_health;
                let mut events = Vec::new();
                tick(
                    &mut normal,
                    w,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut events,
                );
                assert_eq!(normal.extra.ints[0], 4);
            },
        );
        assert!(
            ran,
            "prepared resources must actually admit the mob tick world"
        );
        assert!(
            differences.is_empty(),
            "NoAI trader timer mismatch: {differences:?}"
        );
    }
}

#[cfg(test)]
#[path = "hurt_teleport_contact_tests.rs"]
mod hurt_teleport_contact_tests;

#[cfg(test)]
mod hidden_absorption_promotion_tests {
    use super::*;
    use std::sync::Arc;

    /// #11384: a promoted hidden instance goes through `onEffectUpdated(effect,
    /// true)`, whose `refreshDirtyAttributes` clamps the absorption amount to the
    /// weaker effect's `max_absorption` (Absorption II's 8 down to I's 4).
    #[test]
    fn a_promoted_weaker_absorption_clamps_the_amount() {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        let mut m = Mob::new(203, Arc::new(kind), Vec3::ZERO, 0.0);
        m.add_effect("absorption", 600, 0);
        m.add_effect("absorption", 2, 1);
        assert_eq!(m.extra.effect_chains.len(), 1, "Absorption I waits");
        assert_eq!(
            (m.absorption, super::super::armor::max_absorption(&m)),
            (8.0, 8.0)
        );
        for _ in 0..3 {
            tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
                |_| {},
            );
        }
        assert_eq!(m.extra.effects.len(), 1);
        assert_eq!(m.extra.effects[0].amplifier, 0, "Absorption I took over");
        assert_eq!(super::super::armor::max_absorption(&m), 4.0);
        assert_eq!(m.absorption, 4.0);
    }
}

#[cfg(test)]
mod baby_speed_tests {
    use super::*;
    use crate::mobs::{finalize, sensor_scan_tests::kind};
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn baby(species: Species, id: &str) -> Mob {
        let mut k = kind();
        let km = Arc::make_mut(&mut k);
        km.id = id.into();
        km.species = species;
        let mut m = Mob::new(1, k, Vec3::ZERO, 0.0);
        finalize::set_baby(&mut m, true);
        m
    }

    /// #11994: movement reads the transient `minecraft:baby` modifier itself:
    /// `Zombie.SPEED_MODIFIER_BABY` +0.5 and `Piglin.SPEED_MODIFIER_BABY` +0.2
    /// (`ADD_MULTIPLIED_BASE`), and none once `/attribute ... modifier remove`
    /// took it away, until the next `setBaby`.
    #[test]
    fn a_babys_speed_follows_its_baby_modifier() {
        let mut zombie = baby(Species::Zombie, "minecraft:zombie");
        assert_eq!(speed_bonus(&zombie), 1.5);
        zombie
            .extra
            .attributes
            .get_or_insert_default()
            .baby_speed_removed = true;
        assert_eq!(speed_bonus(&zombie), 1.0, "removed with /attribute");
        finalize::set_baby(&mut zombie, true);
        assert_eq!(speed_bonus(&zombie), 1.5, "setBaby adds it back");
        // The piglin's is 0.2F widened to a double, not the zombie's 0.5.
        let piglin = baby(Species::Piglin, "minecraft:piglin");
        assert_eq!(speed_bonus(&piglin), 1.0 + f64::from(0.2f32));
        let mut adult = baby(Species::Zombie, "minecraft:zombie");
        finalize::set_baby(&mut adult, false);
        assert_eq!(speed_bonus(&adult), 1.0);
    }
}

#[cfg(test)]
mod walk_target_tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// `PathfinderMob.getWalkTargetValue` and its overrides, at three positions
    /// that differ only in the block below. A cow scores grass 10 and mycelium by
    /// the light; a mushroom cow, whose own override replaces Animal's, mycelium
    /// 10 and grass by the light; a monster the negated light; and an enderman, a
    /// pillager and a warden the same 0 everywhere.
    #[test]
    #[ignore = "daily"]
    fn a_stroll_scores_its_ground_the_way_the_jar_does() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                let stone = registry.default_state("minecraft:stone").unwrap();
                let grass = registry.default_state("minecraft:grass_block").unwrap();
                let mycelium = registry.default_state("minecraft:mycelium").unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                world.set(-2, 63, 0, grass).unwrap();
                world.set(0, 63, 0, stone).unwrap();
                world.set(2, 63, 0, mycelium).unwrap();
            },
            |w, types| {
                let value = |id: &str, x: i32| {
                    let kind = Arc::new(types.types[id].clone());
                    let m = Mob::new(1, kind, Vec3::ZERO, 0.0);
                    walk_target_value(&m, w, [x, 64, 0])
                };
                // The light term of the middle position, which the report quotes.
                let light = w.brightness(0, 64, 0) - 0.5;
                // A cow: grass 10, mycelium and stone by the light.
                assert_eq!(value("minecraft:cow", -2), 10.0, "cow on grass");
                assert_eq!(value("minecraft:cow", 0), light, "cow on stone");
                assert_eq!(value("minecraft:cow", 2), light, "cow on mycelium");
                // A mushroom cow: mycelium 10, grass by the light, not 10.
                assert_eq!(
                    value("minecraft:mooshroom", 2),
                    10.0,
                    "mooshroom on mycelium"
                );
                assert_eq!(
                    value("minecraft:mooshroom", -2),
                    light,
                    "mooshroom on grass"
                );
                // A monster: the negated light, on grass like anywhere else.
                assert_eq!(value("minecraft:zombie", -2), -light, "zombie");
                // The three that score every position the same.
                for flat in [
                    "minecraft:enderman",
                    "minecraft:pillager",
                    "minecraft:warden",
                ] {
                    assert_eq!(value(flat, -2), 0.0, "{flat} on grass");
                    assert_eq!(value(flat, 0), 0.0, "{flat} on stone");
                    assert_eq!(value(flat, 2), 0.0, "{flat} on mycelium");
                }
            },
        );
        // Without the extracted vanilla data the world cannot be built.
        assert!(ran, "vanilla data is missing");
    }

    #[test]
    fn static_mannequin_fast_path_ticks_and_invalidates_on_world_edit_or_teleport() {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:mannequin".into();
        kind.species = Species::Mannequin;
        kind.category = "misc".into();
        let mut m = Mob::new(1, Arc::new(kind), Vec3::new(0.5, 71.0, 0.5), 90.0);
        m.first_tick = false;
        m.invulnerable_flag = true;
        m.extra.immovable = true;
        m.effects_dirty = false;
        m.extra.effects.push(crate::mobs::StoredEffect::new(
            "minecraft:resistance",
            -1,
            4,
        ));
        assert!(m.is_static_mannequin());
        assert!(!tick_static_mannequin(&mut m, 42, -64));

        m.extra.static_env_stamp = Some((
            42,
            [0.5_f64.to_bits(), 71.0_f64.to_bits(), 0.5_f64.to_bits()],
            [m.width().to_bits(), m.height().to_bits()],
        ));
        let age_before = m.age;
        assert!(tick_static_mannequin(&mut m, 42, -64));
        assert_eq!(m.age, age_before + 1);

        // Pose / box-size change invalidates the stamp -> falls back to full tick.
        m.extra.avatar_pose = crate::pose::Pose::Crouching;
        assert!(!tick_static_mannequin(&mut m, 42, -64));
        m.extra.avatar_pose = crate::pose::Pose::Standing;
        assert!(tick_static_mannequin(&mut m, 42, -64));

        // Any block edit bumps world.revision -> automatically falls back to full tick.
        assert!(!tick_static_mannequin(&mut m, 43, -64));

        // Any teleport updates position (even with previous == position and unchanged world.revision)
        // -> position-bit stamp mismatch invalidates fast path and falls back to full tick.
        m.position = Vec3::new(10.5, 71.0, 10.5);
        m.previous = m.position;
        assert!(!tick_static_mannequin(&mut m, 42, -64));
        m.position = Vec3::new(0.5, 71.0, 0.5);
        m.previous = m.position;

        // Finite effect also falls back so duration counts down.
        m.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:speed", 20, 0));
        assert!(!m.is_static_mannequin());
        assert!(!tick_static_mannequin(&mut m, 42, -64));
    }

    #[test]
    fn static_mannequin_rejects_non_minecraft_and_hazard_blocks() {
        assert!(is_inert_static_mannequin_below("minecraft:stone"));
        assert!(is_inert_static_mannequin_inside("minecraft:air"));
        assert!(!is_inert_static_mannequin_below("mymod:magic_pad"));
        assert!(!is_inert_static_mannequin_inside("mymod:magic_pad"));
        assert!(!is_inert_static_mannequin_below("minecraft:magma_block"));
        assert!(!is_inert_static_mannequin_inside("minecraft:powder_snow"));
    }
}
