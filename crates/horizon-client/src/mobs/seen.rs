// Included in mobs.rs: the snapshot retains its canonical mobs::Seen path.
/// A snapshot of another entity mobs can see this tick.
#[derive(Clone, Debug)]
pub struct Seen {
    /// Actual registry type, retained without copying its id string.
    pub kind: Arc<MobType>,
    pub in_water: bool,
    pub playing_dead: bool,
    pub last_damage: Option<Damage>,
    pub id: u32,
    pub species: Species,
    pub position: Vec3,
    pub bounds: Aabb,
    pub eye_y: f64,
    pub alive: bool,
    /// `Entity.isInvulnerable` (the ability flag or the temporary window,
    /// #916), which `asValidTarget` refuses a target by (#1175): a
    /// valid-target rule the tick's view asks.
    pub invulnerable: bool,
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
    /// What decides how visible it is to a mob that looks for it.
    pub cover: visibility::Cover,
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
    /// A villager's vanilla profession projection, absent for unknown IDs.
    pub profession: Option<u8>,
    pub custom_profession: Option<Arc<str>>,
    /// A llama's strength or a horse's markings.
    pub secondary_variant: u8,
    /// A villager's experience.
    pub xp: i32,
    /// What other villagers read of a villager's state and brain.
    pub villager: Option<Arc<villager::Seen>>,
    /// `MAX_HEALTH`, `JUMP_STRENGTH` and `MOVEMENT_SPEED` base values (a
    /// horse's breeding stock).
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
