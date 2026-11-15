//! `Seen` and `Mob::seen`: the per-tick snapshot goals read (`game` writes one per mob).

use super::*;

/// A snapshot of another entity mobs can see this tick.
#[derive(Clone, Debug)]
pub struct Seen {
    pub in_water: bool,
    pub playing_dead: bool,
    pub last_damage: Option<Damage>,
    pub id: u32,
    pub species: Species,
    pub position: Vec3,
    pub bounds: Aabb,
    pub eye_y: f64,
    pub alive: bool,
    pub baby: bool,
    pub in_love: bool,
    pub tame: bool,
    /// `AbstractHorse.isBred`: the `Bred` flag (`followMommy` reads it).
    pub bred: bool,
    /// A tamed animal's owner (`getOwner`).
    pub owner: Option<[i32; 4]>,
    /// `instanceof Enemy` (what the golems' target predicates test).
    pub hostile: bool,
    pub target: Option<Who>,
    pub health: f32,
    pub variant: u8,
    /// A slime's or magma cube's size.
    pub size: i32,
    /// Dye colour (sheep).
    pub color: u8,
    /// A villager's claimed job site and bed.
    pub job: Option<[i32; 3]>,
    pub bed: Option<[i32; 3]>,
    pub yaw: f32,
    pub velocity: Vec3,
    /// `Entity.getKnownSpeed`: what it moved in its last tick.
    pub known_speed: Vec3,
    /// Who holds this mob's lead.
    pub leash: Option<leash::Holder>,
    /// A llama's caravan head (its id plus one, 0 when not in a caravan).
    pub caravan: u32,
    /// A schooling fish's leader and how many fish follow it.
    pub school: (Option<u32>, i32),
    /// `MobExtra::section` and `section_seq`.
    pub section: ([i32; 3], u64),
    /// A wandering trader's despawn delay.
    pub delay: i32,
    /// `getLastHurtByMob` and its timestamp.
    pub hurt_by: Option<(Who, i32)>,
    /// `Entity.isInvisible`: the flag, or the invisibility effect.
    pub invisible: bool,
    /// `PathfinderMob.isPanicking`: a running panic goal or the brain's
    /// `IS_PANICKING` memory.
    pub panicking: bool,
    /// A raider in an active raid.
    pub active_raid: bool,
    /// A copper golem's `openedChestPos` (`ContainerUser.hasContainerOpen`).
    pub opened_chest: Option<[i32; 3]>,
    /// `isPassenger`.
    pub passenger: bool,
    /// The vehicle it rides (`getVehicle`): a mob's or a boat's id.
    pub vehicle: Option<u32>,
    /// `LivingEntity.getHitbox`: the box, cut below the seat when riding.
    pub hitbox: Aabb,
    /// `isPushable` and pushed by others (`Entity.push(Entity)` moves it).
    pub pushable: bool,
    /// A sleeping villager, wandering trader or fox vetoes its own `push(Entity)`.
    pub sleeping: bool,
    /// A cold strider (`Strider.isSuffocating`).
    pub cold_strider: bool,
    /// `MobCategory` (`monster`, `creature`, ...).
    pub category: &'static str,
    /// A villager's `POTENTIAL_JOB_SITE` and `MEETING_POINT`.
    pub potential_job: Option<[i32; 3]>,
    pub meeting: Option<[i32; 3]>,
    /// `variant2`: a villager's profession (index into
    /// `render::PROFESSIONS`), a llama's strength. And a villager's experience.
    pub profession: u8,
    pub xp: i32,
    /// What other villagers read of a villager's state and brain.
    pub villager: Option<Arc<villager::Seen>>,
    /// `MAX_HEALTH`, `JUMP_STRENGTH` and `MOVEMENT_SPEED` base values (a horse's breeding stock).
    pub stats: [f64; 3],
    /// `Wolf.isInSittingPose`.
    pub sitting: bool,
    /// `Entity.isSilent`.
    pub silent: bool,
    /// `Cat.isLying` or `isRelaxStateOne`.
    pub lying: bool,
    /// `Entity.isVehicle` or `isPassenger`.
    pub ridden: bool,
    pub is_vehicle: bool,
    /// `Attributes.KNOCKBACK_RESISTANCE`.
    pub knockback_resistance: f64,
    /// The active effects, without namespace (`slowness`, `poison`...).
    pub effects: Vec<&'static str>,
    /// A piglin's `HUNTED_RECENTLY` memory.
    pub hunted_recently: bool,
    /// A piglin's `HURT_BY` memory.
    pub hurt_recently: bool,
    /// `Hoglin.canBeHunted`: an adult hoglin without `CannotBeHunted`.
    pub huntable: bool,
    /// `Entity.isInvulnerable`: `Invulnerable`, or `invulnerableTime` left.
    pub invulnerable: bool,
    /// A happy ghast's `isOnStillTimeout` (a player on its back holds it).
    pub still: bool,
    /// `Entity.isVehicle`: something rides it.
    pub carrying: bool,
    /// Pushes given or taken so far this tick, while the collision cap is on.
    pub collisions: u32,
}
impl Seen {
    /// Whether `canBeCollidedWith` can ever hold for this kind: the slots
    /// [`MobWorld::others_solid`] lists.
    pub fn may_block_mobs(&self) -> bool {
        matches!(self.species, Species::Shulker | Species::HappyGhast)
    }
    /// `canBeCollidedWith(mover)` for a mob of species `mover` moving into
    /// it (`EntityGetter.getEntityCollisions`): a living shulker
    /// (`Shulker`), or an adult happy ghast held still, or carrying someone
    /// when the mover is a happy ghast too (`HappyGhast`). Every other mob
    /// keeps `Entity`'s false.
    pub fn blocks_mob(&self, mover: Species) -> bool {
        match self.species {
            Species::Shulker => self.alive,
            Species::HappyGhast => {
                self.alive
                    && !self.baby
                    && (self.still || (self.carrying && mover == Species::HappyGhast))
            }
            _ => false,
        }
    }
    /// `LivingEntity.hasEffect`, by id without namespace.
    pub fn has_effect(&self, effect: &str) -> bool {
        self.effects.contains(&effect)
    }
}
impl Mob {
    /// What other mobs see of this one this tick (`_player` is unused: it keyed the villager
    /// reputation that nothing read). Inline: the game's snapshot of every mob writes each one in
    /// its slot, not through a copy.
    #[inline]
    pub fn seen(&self, _player: Option<[i32; 4]>) -> Seen {
        let m = self;
        let villager = m.species() == Species::Villager;
        Seen {
            in_water: self.fluids.in_water,
            playing_dead: axolotl::is_playing_dead(self),
            last_damage: self.last_damage.map(|(d, _)| d),
            id: m.id,
            species: m.species(),
            position: m.position,
            bounds: m.bounds(),
            hitbox: m.hitbox(),
            eye_y: m.eye_y(),
            alive: m.alive(),
            baby: m.is_baby(),
            // A villager is "in love" when it is willing to breed (`canBreed`).
            in_love: if villager {
                villager::can_breed(m)
            } else {
                m.extra.in_love > 0
            },
            tame: m.extra.tame,
            bred: m.extra.bred,
            owner: m.extra.owner,
            hostile: m.is_enemy(),
            target: m.ai.target,
            health: m.health,
            variant: m.extra.variant,
            size: m.extra.size,
            color: m.extra.color,
            job: villager.then_some(m.extra.home).flatten(),
            bed: villager.then_some(m.extra.bed).flatten(),
            yaw: m.yaw,
            velocity: m.velocity,
            known_speed: m.position.subtract(m.previous),
            leash: m.extra.leash,
            caravan: caravan::head_of(m),
            school: (m.extra.leader, m.extra.followers),
            section: (m.extra.section, m.extra.section_seq),
            delay: m.extra.ints[5],
            hurt_by: m.ai.last_hurt_by.map(|w| (w, m.ai.last_hurt_by_time)),
            invisible: m.invisible
                || m.extra
                    .effects
                    .iter()
                    .any(|e| e.0 == "minecraft:invisibility"),
            panicking: m.ai.panicking,
            active_raid: raider::has_active_raid(m),
            opened_chest: m.extra.opened_chest,
            passenger: m.extra.mount.is_some(),
            vehicle: m.extra.mount,
            pushable: m.pushable(),
            sleeping: villager::sleeping(m)
                || (m.species() == Species::Fox && fox::flag(m, fox::SLEEPING)),
            cold_strider: strider::suffocating(m),
            category: m.category(),
            potential_job: villager_brain::potential_job_site(m),
            meeting: villager.then_some(m.extra.meeting).flatten(),
            profession: m.extra.variant2,
            xp: villager::xp(m),
            villager: villager.then(|| Arc::new(villager::seen(m))),
            stats: [
                f64::from(m.attrs.max_health),
                m.extra.vector[0],
                m.attrs.speed,
            ],
            sitting: m.extra.sitting_pose,
            silent: m.silent,
            lying: m.species() == Species::Cat && (m.extra.flag || m.extra.relax_one),
            ridden: m.rider.is_some() || m.extra.rider.is_some() || m.extra.mount.is_some(),
            is_vehicle: equine::is_vehicle(m),
            still: m.species() == Species::HappyGhast && happy_ghast::on_still_timeout(m),
            carrying: m.rider.is_some() || m.extra.rider.is_some(),
            collisions: 0,
            knockback_resistance: m.attrs.knockback_resistance,
            effects: m
                .extra
                .effects
                .iter()
                .map(|e| e.0.trim_start_matches("minecraft:"))
                .collect(),
            hunted_recently: m.species() == Species::Piglin && m.extra.ints[0] > 0,
            hurt_recently: m.species() == Species::Piglin && m.extra.piglin.hurt,
            huntable: m.species() == Species::Hoglin && !m.is_baby() && !m.extra.cannot_be_hunted,
            invulnerable: m.is_invulnerable(),
        }
    }
}

impl Seen {
    /// `LivingEntity.getVisibilityPercent` for a mob target: 1, or
    /// ×0.7×armor cover (at least 0.1) while `isInvisible`. A mob does not
    /// sneak (`isShiftKeyDown` is a player's) and its `getArmorCoverPercentage`
    /// is 0, which the 0.1 floor turns into 0.07 - the sheep the evoker looks
    /// for has no armour, and no worn `mob_visibility` either. `invisible` is
    /// the flag or the effect: Horizon does not model vanilla's
    /// `updateInvisibilityStatus` sync of the effect into the flag.
    pub fn visibility_percent(&self) -> f64 {
        if self.invisible { 0.7 * 0.1 } else { 1.0 }
    }
}
