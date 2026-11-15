//! Mod goals on mobs (arismoko/horizon#1077): the parts a mod attaches to an
//! entity type register their goals at every `Mob` construction, right after
//! the species' own, and the engine wraps each in a [`ModGoal`] that gives the
//! mod its own mob, the tick's read view, its random and its writes.
//!
//! The read view answers through the generated `Backend` impl at the bottom:
//! the ticking mob itself, the player the tick sees and the tick's snapshot of
//! the other entities; entity data the snapshot does not carry is refused its
//! typed refusal, and every other backend read forwards to the real backend of
//! the game host behind the tick. The engine's own goals are
//! untouched, and a mob whose type no mod attached a part to pays nothing.
use super::{
    Mob, MobWorld,
    goals::{Cx, Goal as EngineGoal, GoalSelector},
};
use mod_api::{
    WorldEpoch,
    content::crash::{ModCall, enter_native_mod},
};

/// The native side a mod goal runs against (the `mob_hooks` pattern): the
/// game's facts for the tick's read view, and the one call that runs a goal
/// with its live context, which only the game side can build. Handed to
/// the mob tick once beside `mob_world_base`, `None` in `MobWorld::goal_host`
/// wherever no mod attached an entity part — so no other side runs one.
pub trait GoalHost {
    /// The handles' epoch: a handle of an earlier world refuses.
    fn epoch(&self) -> Option<WorldEpoch>;
    /// Whether a merchant lent to this tick remains available to server hooks.
    fn merchant_is_live(&self, mob: &Mob) -> bool;
    /// Read the current offers, including a guest's more recent trade session.
    fn merchant_offers_lent(&self, mob: &Mob) -> Option<Vec<mod_api::MerchantOffer>>;
    /// Resolve the player currently trading with the lent merchant.
    fn merchant_trading_player_lent(&self, mob: &Mob) -> Option<mod_api::PlayerRef>;
    /// Current XP and level, including an open guest's trading session.
    fn merchant_trade_state_lent(&self, mob: &Mob) -> (i32, i32);
    /// Live player passengers of a vehicle, with their boarding order.
    fn player_passenger_seats(&self, vehicle: u32) -> Vec<(i64, u32)>;
    /// Whether this player steers the lent mob.
    fn mob_player_controls(&self, mob: &Mob, player: u32) -> bool;
    /// Dismount location using a mob lent out of the game list.
    fn mob_dismount_location(
        &self,
        vehicle: Option<&Mob>,
        vehicle_id: u32,
        rider: Option<&Mob>,
        passenger: mod_api::EntityRef,
    ) -> Option<[f64; 3]>;
    /// The game's complete portal eligibility, including player passengers.
    fn mob_can_use_portal(&self, mob: &Mob) -> bool;
    /// Compute climbing from the game's collision and climbing registry.
    fn mob_is_climbing(&self, mob: &Mob) -> bool;
    /// Player credit for a mob, distinguishing absent credit from an unresolved player.
    fn mob_player_credit(&self, mob: &Mob) -> Option<Option<u32>>;
    /// Read a lent mob's current target with the game's target validation.
    fn mob_target(
        &self,
        mob: &Mob,
        actor: &Mob,
        world: &MobWorld<'_>,
    ) -> Option<mod_api::LivingRef>;
    /// Tags of a mob currently lent to the tick.
    fn mob_tags(&self, mob: &Mob) -> Option<Vec<String>>;
    /// Display name of a mob currently lent to the tick.
    fn mob_name(&self, mob: &Mob, actor: &Mob, world: &MobWorld<'_>) -> String;
    /// Whether a mob currently lent to the tick has joined its level.
    fn mob_added_to_level(&self, mob: &Mob) -> bool;
    /// Portal cooldown of a mob currently lent to the tick.
    fn mob_portal_cooldown(&self, mob: &Mob) -> i32;
    /// Whether a mob currently lent to the tick refuses a damage type.
    fn mob_invulnerable_to_damage(&self, mob: &Mob, damage_type: &str) -> bool;
    /// A lent mob's supporting block, or the block just below its feet.
    fn mob_on_block(&self, mob: &Mob) -> [i32; 3];
    /// Whether two validated entities may damage each other through teams.
    fn entities_team_damage_allowed(
        &self,
        entity: mod_api::EntityRef,
        entity_mob: Option<&Mob>,
        source: mod_api::EntityRef,
        source_mob: Option<&Mob>,
        actor: &Mob,
        world: &MobWorld<'_>,
    ) -> Option<bool>;
    /// Run `goal`'s `op` on the mob in hand with the tick's `MobWorld`, naming
    /// `mod_id`'s `part` in a panic. None where this side runs no mod goals.
    fn run_goal(
        &self,
        goal: &mut Box<dyn mod_api::Goal>,
        m: &mut Mob,
        w: &MobWorld<'_>,
        op: GoalOp,
        mod_id: &'static str,
        part: &'static str,
    ) -> Option<bool>;
    /// Run `behavior`'s `op` on the mob in hand with the tick's `MobWorld`,
    /// naming `mod_id`'s `part` in a panic, over the same mob-tick view a
    /// goal gets (arismoko/horizon#824). None where this side runs no mod behaviors.
    fn run_behavior(
        &self,
        behavior: &mut Box<dyn mod_api::Behavior>,
        m: &mut Mob,
        w: &MobWorld<'_>,
        op: BehaviorOp,
        mod_id: &'static str,
        part: &'static str,
    ) -> Option<bool>;
}

/// One mod goal call the engine's selectors make.
#[derive(Clone, Copy, Debug)]
pub enum GoalOp {
    /// `Goal::can_use`.
    CanUse,
    /// `Goal::can_continue`.
    CanContinue,
    /// `Goal::start`.
    Start,
    /// `Goal::stop`.
    Stop,
    /// `Goal::tick`.
    Tick,
}

/// One mod behavior call the ported brains make (arismoko/horizon#824), as [`GoalOp`] names
/// a goal's.
#[derive(Clone, Copy, Debug)]
pub enum BehaviorOp {
    /// `Behavior::check_extra_start_conditions`.
    CheckExtraStartConditions,
    /// `Behavior::start`.
    Start,
    /// `Behavior::can_still_use`.
    CanStillUse,
    /// `Behavior::tick`.
    Tick,
    /// `Behavior::stop`.
    Stop,
}

/// Ask the installed entity parts for the mob's mod goals (arismoko/horizon#1077), right after
/// the species' own `ai::register` — vanilla's `registerGoals` moment, which a
/// mob loaded from a chunk passes through as a spawned one does.
pub fn attach(m: &mut Mob) {
    if !mod_api::__private::has_entity_parts() {
        return;
    }
    let parts = mod_api::__private::entity_parts();
    let list = parts.entity_parts(&m.kind.id);
    if list.is_empty() {
        return;
    }
    let (id, mob_id) = (m.id, &m.kind.id);
    for attached in list {
        let _guard = enter_native_mod(ModCall {
            id: attached.mod_id(),
            hook: "register_goals",
            part: Some(attached.type_name()),
        });
        let ai = &mut m.ai;
        let mut registrar = RegistrarFor {
            goals: &mut ai.goals,
            targets: &mut ai.targets,
            mob_id,
            mob_entity: id,
            mod_id: attached.mod_id(),
            part: attached.type_name(),
        };
        attached
            .part()
            .register_goals(&mut mod_api::GoalRegistrar::new(&mut registrar));
    }
}

/// The game's side of [`mod_api::GoalRegistrar`]: the mob's two selectors.
struct RegistrarFor<'a> {
    goals: &'a mut GoalSelector,
    targets: &'a mut GoalSelector,
    mob_id: &'a str,
    mob_entity: u32,
    mod_id: &'static str,
    part: &'static str,
}

impl mod_api::GoalRegistrarBackend for RegistrarFor<'_> {
    fn add_goal(&mut self, priority: i32, goal: Box<dyn mod_api::Goal>) {
        let goal = ModGoal::new(self.mob_entity, self.mod_id, self.part, goal);
        self.goals.add_boxed(priority, Box::new(goal));
    }

    fn add_target_goal(&mut self, priority: i32, goal: Box<dyn mod_api::Goal>) {
        let goal = ModGoal::new(self.mob_entity, self.mod_id, self.part, goal);
        self.targets.add_boxed(priority, Box::new(goal));
    }

    fn mob_id(&self) -> &str {
        self.mob_id
    }
}

/// One mod goal, wrapped for the engine's selectors: the mob it runs on, the
/// mod and part a panic in it names, and the goal itself. A clone of the mob
/// (a corpse kept for its death events) keeps the wrapper's shape with the
/// goal left out — a mod goal's state is its own and no copy of it exists, and
/// a corpse runs no goals.
pub struct ModGoal {
    mob: u32,
    mod_id: &'static str,
    part: &'static str,
    goal: Option<Box<dyn mod_api::Goal>>,
}

impl ModGoal {
    fn new(
        mob: u32,
        mod_id: &'static str,
        part: &'static str,
        goal: Box<dyn mod_api::Goal>,
    ) -> Self {
        Self {
            mob,
            mod_id,
            part,
            goal: Some(goal),
        }
    }

    /// Run `op` on the goal through the tick's goal host, which builds the
    /// context and names the mod in a panic. None — the caller's idle answer —
    /// where the goal is gone (a clone) or the side ticking is not the
    /// native-authoritative one (`goal_host`, as `mob_hooks` asks nothing
    /// there).
    fn run(&mut self, m: &mut Mob, cx: &mut Cx, op: GoalOp) -> Option<bool> {
        let goal = self.goal.as_mut()?;
        cx.w.goal_host?
            .run_goal(goal, m, cx.w, op, self.mod_id, self.part)
    }
}

impl std::fmt::Debug for ModGoal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModGoal")
            .field("mob", &self.mob)
            .field("goal", &self.goal.as_ref().map(|goal| goal.name()))
            .finish()
    }
}

impl EngineGoal for ModGoal {
    fn flags(&self) -> u8 {
        self.goal.as_ref().map_or(0, |goal| goal.flags().bits())
    }

    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.run(m, cx, GoalOp::CanUse).unwrap_or(false)
    }

    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.run(m, cx, GoalOp::CanContinue).unwrap_or(false)
    }

    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let _ = self.run(m, cx, GoalOp::Start);
    }

    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        let _ = self.run(m, cx, GoalOp::Stop);
    }

    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let _ = self.run(m, cx, GoalOp::Tick);
    }

    fn every_tick(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(|goal| goal.requires_update_every_tick())
    }

    fn interruptable(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(|goal| goal.is_interruptable())
    }

    fn box_clone(&self) -> Box<dyn EngineGoal> {
        Box::new(ModGoal {
            mob: self.mob,
            mod_id: self.mod_id,
            part: self.part,
            goal: None,
        })
    }

    fn name(&self) -> &'static str {
        self.goal.as_ref().map_or("ModGoal", |goal| goal.name())
    }
}
