//! Foxes (`net.minecraft.world.entity.animal.fox.Fox`): the goal list, the
//! flags byte (sleeping, ...), napping in the shade by day, eating berries
//! and the food held in the mouth. Faceplanting, stalking and pouncing,
//! picking up items, sheltering and trusting players are not ported yet.
use super::{
    Mob, MobEvent, MobWorld, Seen, Species,
    goals::{
        Avoid, AvoidEntity, Breed, Cx, Float, FollowParent, Goal, JUMP, LOOK, LeapAtTarget,
        LookAtPlayer, MOVE, MeleeAttack, MoveToBlock, NearestTarget, Panic, Prey, Stroll,
        goal_clone, reduced,
    },
};
use horizon_core::{geometry::Axis, random::RandomSource};
use horizon_world::registry::BlockStateId;

// Fox.DATA_FLAGS_ID, kept in `extra.ints[0]` (and sent to clients as is).
pub const SITTING: i32 = 1;
pub const CROUCHING: i32 = 4;
pub const INTERESTED: i32 = 8;
pub const POUNCING: i32 = 16;
pub const SLEEPING: i32 = 32;
pub const FACEPLANTED: i32 = 64;
pub const DEFENDING: i32 = 128;
/// `Fox.ticksSinceEaten`.
const TICKS_SINCE_EATEN: usize = 1;

pub fn flag(m: &Mob, f: i32) -> bool {
    m.extra.ints[0] & f != 0
}
fn set_flag(m: &mut Mob, f: i32, on: bool) {
    if on {
        m.extra.ints[0] |= f;
    } else {
        m.extra.ints[0] &= !f;
    }
}
/// `Fox.clearStates`.
fn clear_states(m: &mut Mob) {
    m.extra.ints[0] = 0;
}

/// `Fox.registerGoals` and `setTargetGoals` (the targets wait for the variant:
/// red foxes prefer chickens and rabbits, snow foxes fish).
pub fn register(m: &mut Mob) {
    let countdown = m.random.next_int_bound(reduced(140)).unwrap_or(0);
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, Panic::new(2.2));
    g.add(3, Breed::new(1.0));
    // AVOID_PLAYERS: not a sneaking player.
    g.add(
        4,
        AvoidEntity::new(Avoid::Player, 16.0, 1.6, 1.4)
            .player_when(|p| !p.sneaking && !p.spectator),
    );
    g.add(
        4,
        AvoidEntity::new(Avoid::Species(vec![Species::Wolf]), 8.0, 1.6, 1.4).seen_when(|s| !s.tame),
    );
    g.add(
        4,
        AvoidEntity::new(Avoid::Species(vec![Species::PolarBear]), 8.0, 1.6, 1.4),
    );
    g.add(7, MeleeAttack::new(1.2, true));
    g.add(7, Sleep { countdown });
    g.add(8, FollowParent::new(1.25));
    g.add(10, EatBerries::new());
    g.add(10, LeapAtTarget::new(0.4));
    g.add(11, Stroll::avoiding_water(1.0));
    g.add(12, LookAtPlayer::new(24.0));
    let red = |m: &Mob, _: &MobWorld| m.extra.variant == 0;
    let snow = |m: &Mob, _: &MobWorld| m.extra.variant != 0;
    let land = || NearestTarget::new(Prey::Filter(chicken_or_rabbit), false).interval(10);
    let turtle = || NearestTarget::new(Prey::Filter(baby_turtle_on_land), false).interval(10);
    let fish = || NearestTarget::new(Prey::Filter(schooling_fish), false).interval(20);
    let t = &mut m.ai.targets;
    t.add(4, land().gate(red));
    t.add(4, turtle().gate(red));
    t.add(6, fish().gate(red));
    t.add(4, fish().gate(snow));
    t.add(6, land().gate(snow));
    t.add(6, turtle().gate(snow));
}

fn chicken_or_rabbit(s: &Seen) -> bool {
    matches!(s.species, Species::Chicken | Species::Rabbit)
}
fn baby_turtle_on_land(s: &Seen) -> bool {
    s.species == Species::Turtle && s.baby && !s.in_water
}
fn schooling_fish(s: &Seen) -> bool {
    matches!(
        s.species,
        Species::Cod | Species::Salmon | Species::TropicalFish
    )
}

/// `Fox.tick` and the eating part of `Fox.aiStep`.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    m.extra.ints[TICKS_SINCE_EATEN] += 1;
    if let Some(item) = m.extra.main_hand.clone()
        && eats(w, &item)
        && m.ai.target.is_none()
        && m.on_ground
        && !flag(m, SLEEPING)
    {
        let eaten = m.extra.ints[TICKS_SINCE_EATEN];
        if eaten > 600 {
            // ItemStack.finishUsingItem: the food is gone, a use remainder stays.
            m.extra.main_hand = w
                .items
                .and_then(|i| i.items.get(&item))
                .and_then(|i| i.use_remainder.as_deref())
                .map(Into::into);
            m.extra.ints[TICKS_SINCE_EATEN] = 0;
        } else if eaten > 560 && m.random.next_float() < 0.1 {
            let at = [m.position.x, m.position.y, m.position.z];
            events.push(MobEvent::Sound("entity.fox.eat", at, 1.0, 1.0));
        }
    }
    if m.ai.target.is_none() {
        set_flag(m, CROUCHING | INTERESTED, false);
    }
    let in_water = m.fluids.in_water;
    if in_water || m.ai.target.is_some() || w.thundering {
        set_flag(m, SLEEPING, false);
    }
    if in_water || flag(m, SLEEPING) {
        set_flag(m, SITTING, false);
    }
}
/// `Fox.isConsumableFood`: the item has food and consumable components.
fn eats(w: &MobWorld, item: &str) -> bool {
    w.items
        .and_then(|i| i.items.get(item))
        .is_some_and(|i| i.food.is_some() && i.consumable.is_some())
}

/// `Fox.FoxBehaviorGoal.hasShelter`: out of the sky's sight and not a bad spot.
fn has_shelter(m: &Mob, w: &MobWorld) -> bool {
    let at = [
        m.position.x.floor() as i32,
        m.bounds().max(Axis::Y).floor() as i32,
        m.position.z.floor() as i32,
    ];
    !w.can_see_sky(at[0], at[1], at[2]) && m.walk_target_value(w, at) >= 0.0
}
/// `Fox.FoxBehaviorGoal.alertable`: someone within 12 blocks the fox would
/// keep watch over (`FoxAlertableEntitiesSelector`).
fn alertable(m: &Mob, w: &MobWorld) -> bool {
    let (from, id, y) = (m.position, m.id, m.position.y);
    let mob = w
        .nearby(from, 12.0, |s: &Seen| {
            s.id != id
                && (s.position.y - y).abs() <= 6.0
                && m.distance_sqr(s.position) <= 144.0
                && match s.species {
                    Species::Fox => false,
                    Species::Chicken | Species::Rabbit => true,
                    Species::Wolf | Species::Cat | Species::Parrot | Species::Nautilus => !s.tame,
                    _ => true,
                }
        })
        .next()
        .is_some();
    mob || w.player.as_ref().is_some_and(|p| {
        p.alive
            && !p.creative
            && !p.spectator
            && !p.sneaking
            && (p.position.y - y).abs() <= 6.0
            && m.distance_sqr(p.position) <= 144.0
    })
}

/// `Fox.SleepGoal`: nap in the shade by day when nothing is around.
#[derive(Clone, Debug)]
struct Sleep {
    countdown: i32,
}
impl Sleep {
    fn can_sleep(&mut self, m: &Mob, w: &MobWorld) -> bool {
        if self.countdown > 0 {
            self.countdown -= 1;
            return false;
        }
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        w.daylight
            && has_shelter(m, w)
            && !alertable(m, w)
            && w.block_name(x, y, z) != "minecraft:powder_snow"
    }
}
impl Goal for Sleep {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.nav.is_done() && (self.can_sleep(m, cx.w) || flag(m, SLEEPING))
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_sleep(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        set_flag(m, SITTING | CROUCHING | INTERESTED, false);
        m.ai.jumping = false;
        set_flag(m, SLEEPING, true);
        m.nav_stop();
        let p = m.position;
        m.ai.mov.set_wanted(p.x, p.y, p.z, 0.0);
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        self.countdown = m.random.next_int_bound(reduced(140)).unwrap_or(0);
        clear_states(m);
    }
}

/// `BlockState` with one property changed.
fn with_prop(w: &MobWorld, p: [i32; 3], key: &str, value: &str) -> Option<BlockStateId> {
    let mut s = w.registry.state(w.world.block(p[0], p[1], p[2]))?.clone();
    s.properties.insert(key.into(), value.into());
    w.registry.id(&s)
}
/// `FoxEatBerriesGoal.isValidTarget`: a ripe sweet berry bush or glow berries.
fn has_berries(w: &MobWorld, p: [i32; 3]) -> bool {
    match w.block_name(p[0], p[1], p[2]) {
        "minecraft:sweet_berry_bush" => w
            .prop(p[0], p[1], p[2], "age")
            .and_then(|a| a.parse::<i32>().ok())
            .is_some_and(|a| a >= 2),
        "minecraft:cave_vines" | "minecraft:cave_vines_plant" => {
            w.prop(p[0], p[1], p[2], "berries") == Some("true")
        }
        _ => false,
    }
}

/// `Fox.FoxEatBerriesGoal`: walk to a ripe berry bush and pick it after 40 ticks.
#[derive(Clone, Debug)]
struct EatBerries {
    block: MoveToBlock,
    waited: i32,
}
impl EatBerries {
    fn new() -> Self {
        Self {
            block: MoveToBlock::new(1.2, 12, 1),
            waited: 0,
        }
    }
    fn pick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let p = self.block.block;
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        if cx.w.block_name(p[0], p[1], p[2]) == "minecraft:sweet_berry_bush" {
            let age: i32 =
                cx.w.prop(p[0], p[1], p[2], "age")
                    .and_then(|a| a.parse().ok())
                    .unwrap_or(0);
            let mut count = 1 + m.random.next_int_bound(2).unwrap_or(0) + i32::from(age == 3);
            if m.extra.main_hand.is_none() {
                m.extra.main_hand = Some("minecraft:sweet_berries".into());
                count -= 1;
            }
            if count > 0 {
                cx.events.push(MobEvent::DropItem {
                    item: "minecraft:sweet_berries".into(),
                    count: count as u32,
                    at,
                });
            }
            if let Some(state) = with_prop(cx.w, p, "age", "1") {
                cx.events.push(MobEvent::SetBlock { pos: p, state });
            }
            cx.events.push(MobEvent::Sound(
                "block.sweet_berry_bush.pick_berries",
                at,
                1.0,
                1.0,
            ));
        } else {
            // CaveVines.use: one glow berry, the vine goes back to bare.
            cx.events.push(MobEvent::DropItem {
                item: "minecraft:glow_berries".into(),
                count: 1,
                at,
            });
            if let Some(state) = with_prop(cx.w, p, "berries", "false") {
                cx.events.push(MobEvent::SetBlock { pos: p, state });
            }
            let pitch = 0.8 + m.random.next_float() * 0.4;
            cx.events.push(MobEvent::Sound(
                "block.cave_vines.pick_berries",
                at,
                1.0,
                pitch,
            ));
        }
    }
}
impl Goal for EatBerries {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if flag(m, SLEEPING) {
            return false;
        }
        // MoveToBlockGoal.canUse draws the next delay only once the last one
        // has run out.
        if self.block.next_start > 0 {
            self.block.next_start -= 1;
            return false;
        }
        let next = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.block.can_use(m, cx.w, next, has_berries)
    }
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        self.block.can_continue(cx.w, has_berries)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.waited = 0;
        set_flag(m, SITTING, false);
        self.block.start(m, cx.w);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.block.reached {
            if self.waited >= 40 {
                if cx.w.mob_griefing {
                    self.pick(m, cx);
                }
            } else {
                self.waited += 1;
            }
        } else if m.random.next_float() < 0.05 {
            let at = [m.position.x, m.position.y, m.position.z];
            cx.events
                .push(MobEvent::Sound("entity.fox.sniff", at, 1.0, 1.0));
        }
        self.block.tick_within(m, cx.w, 2.0);
    }
}

/// `FoxModel`'s flags as the renderer reads them.
pub fn pose(m: &Mob) -> super::body_anim::Fox {
    super::body_anim::Fox {
        crouching: flag(m, CROUCHING),
        sleeping: flag(m, SLEEPING),
        sitting: flag(m, SITTING),
        pouncing: flag(m, POUNCING),
        faceplanted: flag(m, FACEPLANTED),
        ..Default::default()
    }
}
