//! Mobs: entity types (dimensions and default attributes exported from
//! vanilla), models (vanilla `LayerDefinitions` baked part trees) and the
//! `Mob` entity with vanilla's AI framework: goal and target selectors with
//! priorities and control flags (`goals`), A* pathfinding with walk, swim,
//! fly and amphibious node evaluators (`path`, `nav`), move/look/jump/body
//! controls (`control`), `LivingEntity` physics (`living`) and per-species
//! behaviour, models, animations and sounds.
pub mod ai;
pub mod allay;
pub mod anim;
pub mod aquatic;
pub mod camel;
pub mod control;
pub mod creaking;
pub mod cube;
pub mod data;
pub mod enderman;
pub mod equine;
pub mod farm;
pub mod goals;
pub mod golem;
pub mod guardian;
pub mod hostile;
pub mod illager;
pub mod keyframes;
pub mod living;
pub mod nav;
pub mod nether;
pub mod path;
pub mod phantom;
pub mod piglin;
pub mod raider;
pub mod ravager;
pub mod render;
pub mod shulker;
pub mod spawning;
pub mod species;
pub mod tame;
pub mod undead;
pub mod vex;
pub mod villager;
pub mod witch;
pub mod wither;
mod world;

use crate::{blocks::BlockFacts, mesher::Vertex, world::World};
pub use data::MobData;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

#[derive(Clone, Debug)]
pub struct Polygon {
    pub vertices: [[f32; 5]; 4],
}
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    /// x, y, z, xRot, yRot, zRot, xScale, yScale, zScale
    pub pose: [f32; 9],
    pub polygons: Vec<Polygon>,
    pub children: Vec<Part>,
}
impl Part {
    fn parse(name: &str, v: &Value) -> Self {
        let pose: Vec<f32> = v["pose"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| x.as_f64().unwrap_or(0.0) as f32)
            .collect();
        let mut p = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        for (i, x) in pose.iter().enumerate().take(9) {
            p[i] = *x;
        }
        let polygons = v["polygons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|poly| {
                let verts: Vec<[f32; 5]> = poly
                    .as_array()?
                    .iter()
                    .map(|vx| {
                        let a: Vec<f32> = vx
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|x| x.as_f64().unwrap_or(0.0) as f32)
                            .collect();
                        [a[0], a[1], a[2], a[3], a[4]]
                    })
                    .collect();
                (verts.len() == 4).then(|| Polygon {
                    vertices: [verts[0], verts[1], verts[2], verts[3]],
                })
            })
            .collect();
        let mut children: Vec<Part> = v["children"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, c)| Part::parse(k, c))
            .collect();
        children.sort_by(|a, b| a.name.cmp(&b.name));
        // A model whose root holds a part also named "root" (allay, vex,
        // frog...): that inner part is what `root.getChild("root")` and the
        // keyframe bones mean, so the outer one gives up the name.
        let name = if name == "root" && children.iter().any(|c| c.name == "root") {
            ""
        } else {
            name
        };
        Self {
            name: name.to_owned(),
            pose: p,
            polygons,
            children,
        }
    }
}
/// Kept for callers of the earlier simple AI; species now select behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brain {
    Passive,
    Chicken,
    Zombie,
    Skeleton,
    Spider,
    Creeper,
}
/// Model animation family (vanilla `EntityModel.setupAnim` implementations).
pub use anim::Anim;
pub use species::Species;

/// Per-type behaviour switches derived from vanilla classes and tags.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    /// WaterAnimal / fish: not affected by fluid physics (swims with its own control).
    pub aquatic_swimmer: bool,
    pub fall_immune: bool,
    /// FlyingMob-style travel (ghast, phantom): travelFlying without gravity.
    pub flying: bool,
    /// Spiders: onClimbable while colliding horizontally.
    pub climber: bool,
    /// Striders stand on lava.
    pub lava_walker: bool,
    /// LivingEntity.omnidirectionalAirMover (bees, parrots, bats, allays).
    pub omnidirectional: bool,
    pub fire_immune: bool,
    /// EntityTypeTags.BURN_IN_DAYLIGHT
    pub burns_in_day: bool,
    pub undead: bool,
    pub breathes_water: bool,
    /// Drowns out of water (fish, squid, dolphins after their air runs out).
    pub needs_water: bool,
    /// Hurt by water (endermen, blazes, striders out of lava, snow golems).
    pub water_sensitive: bool,
    pub animal: bool,
}
#[derive(Clone, Debug)]
pub struct MobType {
    pub id: String,
    pub species: Species,
    pub width: f64,
    pub height: f64,
    pub eye_height: f64,
    pub category: String,
    pub max_health: f32,
    pub speed: f64,
    pub flying_speed: f64,
    pub attack_damage: f32,
    pub attack_knockback: f64,
    pub follow_range: f64,
    pub step_height: f64,
    pub armor: f32,
    pub armor_toughness: f32,
    pub knockback_resistance: f64,
    pub jump_strength: f64,
    pub gravity: f64,
    pub safe_fall_distance: f64,
    pub fall_damage_multiplier: f64,
    pub water_efficiency: f64,
    pub water_slowdown: f32,
    /// Air speed for travelFlying mobs (ghast 0.02, phantom 0.2).
    pub air_speed: f32,
    pub baby_scale: f64,
    pub head_rot_speed: f32,
    pub max_head_x_rot: f32,
    pub max_head_y_rot: f32,
    pub ambient_interval: i32,
    pub flags: Flags,
    pub model: Arc<Part>,
    /// Every baked layer of this entity's models keyed by `model#layer`
    /// (e.g. `sheep#wool`, `zombie_baby#main`).
    pub layers: HashMap<String, Arc<Part>>,
    pub texture: String,
    pub brain: Brain,
    pub anim: Anim,
    /// Ambient, hurt and death sound events (without the namespace).
    pub sounds: [&'static str; 3],
    pub step_sound: Option<&'static str>,
    /// Animal.isFood items (resolved from the item tag).
    pub food: Arc<[String]>,
    /// TemptGoal items when they differ from the food.
    pub tempt: Arc<[String]>,
}
impl MobType {
    pub fn layer(&self, key: &str) -> Option<&Arc<Part>> {
        self.layers.get(key)
    }
}
pub struct MobTypes {
    pub types: HashMap<String, MobType>,
}
impl MobTypes {
    pub fn load(root: &Path) -> crate::Result<Self> {
        let types: Value = serde_json::from_slice(&fs::read(root.join("entity-types.json"))?)?;
        let models: Value = serde_json::from_slice(&fs::read(root.join("entity-models.json"))?)?;
        let item_tags = crate::items::load_tags(root, "item").unwrap_or_default();
        let tag = |name: &str| -> Arc<[String]> {
            let mut v: Vec<String> = item_tags
                .get(&format!("minecraft:{name}"))
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            v.sort();
            v.into()
        };
        let mut out = HashMap::new();
        for def in species::ALL {
            let id = format!("minecraft:{}", def.name);
            let t = &types[&id];
            if t.is_null() {
                continue;
            }
            let model_key = format!("minecraft:{}", def.model);
            let Some(model) = models.get(&model_key) else {
                continue;
            };
            let attr = |k: &str| t["attributes"][format!("minecraft:{k}")].as_f64();
            let mut layers = HashMap::new();
            if let Some(all) = models.as_object() {
                for prefix in def.layer_prefixes() {
                    let start = format!("minecraft:{prefix}#");
                    for (k, v) in all.iter().filter(|(k, _)| k.starts_with(&start)) {
                        layers.insert(
                            k.trim_start_matches("minecraft:").to_owned(),
                            Arc::new(Part::parse("root", v)),
                        );
                    }
                }
            }
            let category = t["category"].as_str().unwrap_or("creature").to_owned();
            let speed = attr("movement_speed").unwrap_or(0.25);
            let flags = def.flags(t["fire_immune"].as_bool().unwrap_or(false));
            out.insert(
                id.clone(),
                MobType {
                    species: def.species,
                    width: t["width"].as_f64().unwrap_or(0.6),
                    height: t["height"].as_f64().unwrap_or(1.8),
                    eye_height: t["eye_height"].as_f64().unwrap_or(1.6),
                    max_health: attr("max_health").unwrap_or(10.0) as f32,
                    speed,
                    flying_speed: attr("flying_speed").unwrap_or(0.4),
                    attack_damage: attr("attack_damage").unwrap_or(0.0) as f32,
                    attack_knockback: attr("attack_knockback").unwrap_or(0.0),
                    follow_range: attr("follow_range").unwrap_or(16.0),
                    step_height: attr("step_height").unwrap_or(0.6),
                    armor: attr("armor").unwrap_or(0.0) as f32,
                    armor_toughness: attr("armor_toughness").unwrap_or(0.0) as f32,
                    knockback_resistance: attr("knockback_resistance").unwrap_or(0.0),
                    jump_strength: attr("jump_strength").unwrap_or(0.42),
                    gravity: attr("gravity").unwrap_or(0.08),
                    safe_fall_distance: attr("safe_fall_distance").unwrap_or(3.0),
                    fall_damage_multiplier: attr("fall_damage_multiplier").unwrap_or(1.0),
                    water_efficiency: attr("water_movement_efficiency").unwrap_or(0.0),
                    water_slowdown: def.water_slowdown(),
                    air_speed: def.air_speed(),
                    baby_scale: def.baby_scale(),
                    head_rot_speed: def.head_rot_speed(),
                    max_head_x_rot: def.max_head_x_rot(),
                    max_head_y_rot: def.max_head_y_rot(),
                    ambient_interval: def.ambient_interval(),
                    flags,
                    model: Arc::new(Part::parse("root", model)),
                    layers,
                    texture: def.texture.to_owned(),
                    brain: def.brain(),
                    anim: def.anim,
                    sounds: def.sounds,
                    step_sound: def.step_sound,
                    food: def
                        .food_tag()
                        .map(tag)
                        .unwrap_or_else(|| Arc::from(Vec::new())),
                    tempt: def
                        .tempt_tag()
                        .or(def.food_tag())
                        .map(tag)
                        .unwrap_or_else(|| Arc::from(Vec::new())),
                    category,
                    id,
                },
            );
        }
        Ok(Self { types: out })
    }
}
/// Who caused damage or is being targeted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    Player,
    Mob(u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageKind {
    PlayerAttack,
    MobAttack,
    Arrow,
    Thrown,
    Fireball,
    Explosion,
    Fall,
    OnFire,
    InFire,
    Lava,
    Drown,
    Magic,
    Sting,
    Cactus,
    Generic,
    InWall,
    Thorns,
}
/// A damage source: its type, the attacking entity and where it came from.
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    pub kind: DamageKind,
    pub attacker: Option<Who>,
    pub from: Option<[f64; 3]>,
}
impl Damage {
    pub fn fall() -> Self {
        Self {
            kind: DamageKind::Fall,
            attacker: None,
            from: None,
        }
    }
    pub fn of(kind: DamageKind) -> Self {
        Self {
            kind,
            attacker: None,
            from: None,
        }
    }
    pub fn player(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::PlayerAttack,
            attacker: Some(Who::Player),
            from: Some(from),
        }
    }
    pub fn mob(id: u32, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(id)),
            from: Some(from),
        }
    }
    /// DamageTypeTags.PANIC_CAUSES
    pub fn causes_panic(&self) -> bool {
        !matches!(
            self.kind,
            DamageKind::Fall | DamageKind::Drown | DamageKind::Generic | DamageKind::InWall
        )
    }
    /// DamageTypeTags.BYPASSES_ARMOR
    pub fn bypasses_armor(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::OnFire
                | DamageKind::Drown
                | DamageKind::Magic
                | DamageKind::InWall
                | DamageKind::Generic
        )
    }
    pub fn is_fire(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::OnFire | DamageKind::InFire | DamageKind::Lava | DamageKind::Fireball
        )
    }
    pub fn is_projectile(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::Arrow | DamageKind::Thrown | DamageKind::Fireball
        )
    }
}
/// Per-mob attribute base values (the type's defaults unless changed).
#[derive(Clone, Copy, Debug)]
pub struct Attrs {
    pub speed: f64,
    pub attack_damage: f32,
    pub max_health: f32,
    pub follow_range: f64,
}
/// Species-specific persistent state (variants, colours, taming, ageing...).
#[derive(Clone, Debug, Default)]
pub struct Extra {
    /// Players: crouching and riding (HumanoidModel's poses).
    pub crouching: bool,
    pub passenger: bool,
    /// AgeableMob.age: negative while a baby (a tadpole counts up to 24000).
    pub age: i32,
    /// `AgeLocked` (tadpoles fed a slime ball... and ageable mobs).
    pub age_locked: bool,
    pub in_love: i32,
    pub love_player: bool,
    /// Texture/model variant index (cat type, horse colour, rabbit type...).
    pub variant: u8,
    /// Secondary variant (horse markings, tropical fish pattern colours, panda hidden gene).
    pub variant2: u8,
    pub variant3: u8,
    /// Dye colour id (sheep wool, collars).
    pub color: u8,
    pub sheared: bool,
    /// `Mob.canPickUpLoot` (also lets dispensers equip it).
    pub pick_up_loot: bool,
    pub saddled: bool,
    pub chested: bool,
    pub tame: bool,
    pub sitting: bool,
    /// The owner's UUID (tamed animals).
    pub owner: Option<[i32; 4]>,
    /// Slime/magma cube size (0 = not sized).
    pub size: i32,
    /// Sheep eat-grass / iron golem attack / ravager animation ticks.
    pub anim_ticks: i32,
    pub egg_time: i32,
    pub anger: i32,
    pub carried: Option<horizon_world::registry::BlockStateId>,
    pub charged: bool,
    pub levitation: i32,
    /// Held item ids (main hand, off hand) and head item.
    pub main_hand: Option<Arc<str>>,
    pub off_hand: Option<Arc<str>>,
    pub head: Option<Arc<str>>,
    /// Worn armour below the head (humanoid mobs).
    pub chest: Option<Arc<str>>,
    pub legs: Option<Arc<str>>,
    pub feet: Option<Arc<str>>,
    pub using_item: i32,
    pub flag: bool,
    pub flag2: bool,
    /// Species scratch values (the wither's head targets and clocks).
    pub ints: [i32; 6],
    pub counter: i32,
    pub counter2: i32,
    pub home: Option<[i32; 3]>,
    pub point: Option<[f64; 3]>,
    pub vector: [f64; 3],
    /// Active status effects: (effect id, ticks left, amplifier).
    pub effects: Vec<(&'static str, i32, i32)>,
    /// A villager's claimed bed head and meeting bell.
    pub bed: Option<[i32; 3]>,
    pub meeting: Option<[i32; 3]>,
    /// Villager and wandering trader offers and experience.
    pub trade: Option<Box<crate::merchant::Trader>>,
    /// A raider's patrol and raid state.
    pub raider: Option<Box<raider::State>>,
    /// A villager's raid activity and gossip.
    pub villager: Option<Box<villager::Ext>>,
}
#[derive(Clone, Debug)]
pub struct Mob {
    pub id: u32,
    pub kind: Arc<MobType>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    /// Entity.yRot (movement facing).
    pub yaw: f32,
    pub previous_yaw: f32,
    pub body_yaw: f32,
    pub previous_body_yaw: f32,
    pub head_yaw: f32,
    pub previous_head_yaw: f32,
    /// Entity.xRot
    pub head_pitch: f32,
    pub previous_head_pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision_below: bool,
    pub health: f32,
    pub hurt_time: i32,
    pub invulnerable: i32,
    pub death_time: i32,
    pub age: u32,
    pub limb_position: f32,
    pub limb_speed: f32,
    pub previous_limb_speed: f32,
    pub swell: i32,
    pub previous_swell: i32,
    pub ambient_timer: i32,
    pub fall_distance: f64,
    pub persistent: bool,
    /// `CustomName` (plain text) and `CustomNameVisible`.
    pub custom_name: Option<String>,
    pub name_visible: bool,
    pub random: LegacyRandom,
    pub ai: Box<ai::Ai>,
    pub extra: Extra,
    pub fluids: living::Fluids,
    pub no_gravity: bool,
    pub climbing: bool,
    pub fire_ticks: i32,
    pub air: i32,
    pub last_hurt: f32,
    pub last_damage: Option<(Damage, i64)>,
    /// Set by `remove`: the mob disappears without dying (conversion, despawn, pickup).
    pub removed: bool,
    /// Client-side animation clocks (attack swing, eating, shaking...).
    pub anim: anim::AnimState,
    /// Attribute values that differ per mob (slime size, babies, effects).
    pub attrs: Attrs,
    /// The player riding this mob and its input.
    pub rider: Option<equine::Rider>,
}
/// Events a mob tick asks the game to act on.
#[derive(Clone, Debug)]
pub enum MobEvent {
    Sound(&'static str, [f64; 3], f32, f32),
    /// A sound event id that is not a static string (variant sounds).
    SoundId(String, [f64; 3], f32, f32),
    AttackPlayer(f32, [f64; 3]),
    /// Damage of a given type to the player (guardian lasers, thorns).
    DamagePlayer {
        amount: f32,
        from: [f64; 3],
        kind: &'static str,
    },
    /// `ElderGuardian`'s curse on players within 50 blocks.
    ElderCurse {
        at: [f64; 3],
    },
    /// `Entity.push`: add to a target's velocity.
    Push {
        target: Who,
        velocity: [f64; 3],
    },
    /// `EvokerWololoSpellGoal`: a blue sheep turns red.
    Wololo {
        mob: u32,
    },
    /// `EvokerFangs` rising at a point after `delay` ticks.
    Fangs {
        at: [f64; 3],
        yaw: f32,
        delay: i32,
        owner: u32,
    },
    Shoot([f64; 3], [f64; 3]),
    Explode([f64; 3], f32),
    /// Melee or ranged damage to another mob.
    AttackMob {
        target: u32,
        amount: f32,
        source: Damage,
    },
    /// Set another mob's attack target (HurtByTargetGoal alerts).
    SetTarget {
        mob: u32,
        target: Who,
    },
    Projectile(crate::projectiles::Projectile),
    /// Spawn a new mob (breeding, slime splitting, chicken eggs...).
    Spawn(Box<Mob>),
    /// Spawn a mob by type id at a position (conversions, splits).
    SpawnKind {
        kind: String,
        at: [f64; 3],
        setup: SpawnSetup,
    },
    DropItem {
        item: Arc<str>,
        count: u32,
        at: [f64; 3],
    },
    SetBlock {
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    },
    /// Destroy a block, dropping nothing (sheep eating grass becomes dirt via SetBlock).
    BreakBlock {
        pos: [i32; 3],
        drop: bool,
    },
    Particles {
        kind: &'static str,
        at: [f64; 3],
        count: u32,
    },
    Experience {
        at: [f64; 3],
        amount: u32,
    },
    /// Apply an effect to the player (witch potions, bee stings, cave spider poison).
    EffectPlayer {
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    },
    /// Set an entity on fire.
    Ignite {
        target: Who,
        seconds: i32,
    },
    /// Two animals bred: the game spawns the baby and resets the partner.
    Breed {
        parent: u32,
        partner: u32,
    },
    /// Replace a block by name (sheep turning grass to dirt), optionally with
    /// the destroy effect.
    SetBlockName {
        pos: [i32; 3],
        name: &'static str,
        effect: bool,
    },
    /// A cat hisses (phantoms flee).
    Hiss {
        mob: u32,
    },
    /// A horse's rider tamed it.
    Tamed {
        mob: u32,
    },
    /// A horse throws its rider off.
    Buck {
        mob: u32,
    },
    /// A piglin's admiring is over: barter or keep the item.
    FinishAdmiring {
        mob: u32,
    },
    /// Throw an entity upward (iron golem swings).
    Launch {
        target: Who,
        up: f64,
    },
    /// Damage the mob itself (snow golems melting).
    HurtSelf {
        mob: u32,
        kind: DamageKind,
        amount: f32,
    },
    /// A villager launches a firework rocket (celebrating a won raid).
    Firework {
        at: [f64; 3],
        color: usize,
        flight: u8,
    },
    /// A villager throws a hero's gift (a loot table) toward the player.
    ThrowGift {
        table: String,
        from: [f64; 3],
        to: [f64; 3],
    },
    /// A patrol leader hands its patrol target to a companion.
    PatrolTarget {
        mob: u32,
        target: [i32; 3],
    },
    /// A raider is told to attack (`HoldGroundAttackGoal` shouting).
    RaiderTarget {
        mob: u32,
        target: Who,
        aggressive: bool,
    },
    /// Drop the loot of a gift table (chicken eggs, armadillo scutes).
    GiftLoot {
        table: &'static str,
        at: [f64; 3],
    },
    /// A creaking bound to the heart at `home` was hurt (`creakingHurt` when a player did it).
    CreakingHurt {
        home: [i32; 3],
        by_player: bool,
        id: u32,
    },
    /// `Creaking.tearDown`: crumbling particles around the creaking.
    CreakingTearDown {
        center: [f64; 3],
        spread: [f64; 3],
    },
}
/// How a spawned-by-kind mob should be set up.
#[derive(Clone, Debug, Default)]
pub struct SpawnSetup {
    pub baby: bool,
    pub size: i32,
    pub yaw: f32,
    pub velocity: [f64; 3],
    pub health: Option<f32>,
    pub persistent: bool,
    pub variant: Option<u8>,
    /// Zombified piglins and zoglins wake up nauseous (`ConversionParams`).
    pub nausea: bool,
    /// Vexes: the summoning mob, the bound origin and the life in ticks.
    pub owner_mob: Option<u32>,
    pub home: Option<[i32; 3]>,
    pub life: Option<i32>,
}
/// A snapshot of another entity mobs can see this tick.
#[derive(Clone, Debug)]
pub struct Seen {
    pub id: u32,
    pub species: Species,
    pub position: Vec3,
    pub bounds: Aabb,
    pub eye_y: f64,
    pub alive: bool,
    pub baby: bool,
    pub in_love: bool,
    pub tame: bool,
    pub hostile: bool,
    pub target: Option<Who>,
    pub health: f32,
    pub variant: u8,
    /// Dye colour (sheep).
    pub color: u8,
    /// A villager's claimed job site and bed.
    pub job: Option<[i32; 3]>,
    pub bed: Option<[i32; 3]>,
    /// A raider in an active raid.
    pub active_raid: bool,
}
/// The player as mobs see it.
#[derive(Clone, Debug)]
pub struct PlayerView {
    pub position: Vec3,
    pub bounds: Aabb,
    pub eye: [f64; 3],
    pub alive: bool,
    pub creative: bool,
    pub sneaking: bool,
    pub look: [f64; 3],
    pub main_hand: Option<Arc<str>>,
    pub off_hand: Option<Arc<str>>,
    pub head: Option<Arc<str>>,
    pub armor_cover: f32,
    pub spectator: bool,
    /// `PiglinAi.isWearingSafeArmor`: some golden armour.
    pub gold_armor: bool,
    /// `getLastHurtByMob` (within 100 ticks) and when.
    pub hurt_by: Option<(Who, i64)>,
    /// `getLastHurtMob` and when.
    pub last_hurt: Option<(Who, i64)>,
    /// The Hero of the Village effect.
    pub hero: bool,
}
pub struct MobWorld<'a> {
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub data: &'a MobData,
    pub player: Option<PlayerView>,
    pub others: &'a [Seen],
    pub daylight: bool,
    pub game_time: i64,
    pub day_time: i64,
    pub raining: bool,
    pub thundering: bool,
    pub nether: bool,
    pub block_tags: &'a HashMap<String, std::collections::HashSet<String>>,
    pub biomes: &'a crate::biome::Biomes,
    /// The `mob_griefing` game rule.
    pub mob_griefing: bool,
}
impl Mob {
    pub fn new(id: u32, kind: Arc<MobType>, position: Vec3, yaw: f32) -> Self {
        let mut random = LegacyRandom::new(
            i64::from(id).wrapping_mul(0x5DEE_CE66D)
                ^ (position.x.to_bits() as i64)
                ^ (position.z.to_bits() as i64).rotate_left(17),
        );
        let _ = random.next_int();
        let mut mob = Self {
            id,
            health: kind.max_health,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            body_yaw: yaw,
            previous_body_yaw: yaw,
            head_yaw: yaw,
            previous_head_yaw: yaw,
            head_pitch: 0.0,
            previous_head_pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision_below: false,
            hurt_time: 0,
            invulnerable: 0,
            death_time: 0,
            age: 0,
            limb_position: 0.0,
            limb_speed: 0.0,
            previous_limb_speed: 0.0,
            swell: 0,
            previous_swell: 0,
            ambient_timer: -kind.ambient_interval,
            fall_distance: 0.0,
            persistent: false,
            random,
            ai: Box::default(),
            extra: Extra::default(),
            fluids: living::Fluids::default(),
            no_gravity: false,
            climbing: false,
            fire_ticks: 0,
            air: 300,
            last_hurt: 0.0,
            last_damage: None,
            removed: false,
            custom_name: None,
            name_visible: false,
            anim: anim::AnimState::default(),
            rider: None,
            attrs: Attrs {
                speed: kind.speed,
                attack_damage: kind.attack_damage,
                max_health: kind.max_health,
                follow_range: kind.follow_range,
            },
            kind,
        };
        ai::register(&mut mob);
        mob
    }
    pub fn bounds(&self) -> Aabb {
        let w = f64::from(self.width()) / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - w, self.position.y, self.position.z - w),
            Vec3::new(
                self.position.x + w,
                self.position.y + f64::from(self.height()),
                self.position.z + w,
            ),
        )
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    pub fn hostile(&self) -> bool {
        self.kind.category == "monster"
    }
    pub fn species(&self) -> Species {
        self.kind.species
    }
    /// The player hit this mob (melee): LivingEntity.hurt with knockback.
    pub fn hurt(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        _random: &mut LegacyRandom,
    ) -> bool {
        let source = match from {
            Some(f) => Damage::player(f),
            None => Damage::of(DamageKind::Generic),
        };
        self.damage(amount, source)
    }
    /// LivingEntity.hurtServer: invulnerability frames, armor, knockback and
    /// the last-hurt-by bookkeeping goals read.
    /// The `armor` and `armor_toughness` modifiers of worn humanoid armour.
    pub fn worn_armor(&self) -> (f32, f32) {
        if equine::is_equine(self.species()) {
            return (0.0, 0.0);
        }
        [
            &self.extra.head,
            &self.extra.chest,
            &self.extra.legs,
            &self.extra.feet,
        ]
        .into_iter()
        .flatten()
        .map(|i| spawning::armor_stats(i))
        .fold((0.0, 0.0), |(a, t), (x, y)| (a + x, t + y))
    }
    pub fn damage(&mut self, amount: f32, source: Damage) -> bool {
        if !self.alive() || self.removed {
            return false;
        }
        if source.is_fire() && (self.kind.flags.fire_immune) {
            return false;
        }
        if !ai::accepts_damage(self, &source, amount) {
            return false;
        }
        let mut amount = amount;
        let (worn, worn_toughness) = self.worn_armor();
        if !source.bypasses_armor() && self.kind.armor + worn > 0.0 {
            // CombatRules.getDamageAfterAbsorb
            let armor = self.kind.armor + worn;
            let toughness = 2.0 + (self.kind.armor_toughness + worn_toughness) / 4.0;
            let effective = (armor - amount / toughness).clamp(armor * 0.2, 20.0);
            amount *= 1.0 - effective / 25.0;
        }
        let mut knock = true;
        if self.invulnerable as f32 > 10.0 {
            if amount <= self.last_hurt {
                return false;
            }
            self.health -= amount - self.last_hurt;
            self.last_hurt = amount;
            knock = false;
        } else {
            self.last_hurt = amount;
            self.invulnerable = 20;
            self.health -= amount;
            self.hurt_time = 10;
        }
        self.last_damage = Some((source, i64::from(self.age)));
        if let Some(who) = source.attacker {
            self.ai.last_hurt_by = Some(who);
            self.ai.last_hurt_by_time = self.age as i32;
            if who == Who::Player {
                self.ai.last_hurt_by_player = 100;
            }
        }
        if knock
            && let Some(f) = source.from
            && !matches!(source.kind, DamageKind::Explosion)
        {
            let (dx, dz) = (f[0] - self.position.x, f[2] - self.position.z);
            self.knockback(0.4, dx, dz);
        }
        ai::after_hurt(self, &source);
        if self.health <= 0.0 {
            self.health = 0.0;
        }
        true
    }
    /// LivingEntity.knockback(power, xd, zd) towards the negative direction.
    pub fn knockback(&mut self, power: f64, mut xd: f64, mut zd: f64) {
        let power = power * (1.0 - self.kind.knockback_resistance);
        if power <= 0.0 {
            return;
        }
        while xd * xd + zd * zd < 1.0e-5 {
            xd = (self.random.next_double() - self.random.next_double()) * 0.01;
            zd = (self.random.next_double() - self.random.next_double()) * 0.01;
        }
        let len = (xd * xd + zd * zd).sqrt();
        let (kx, kz) = (xd / len * power, zd / len * power);
        self.velocity = Vec3::new(
            self.velocity.x / 2.0 - kx,
            if self.on_ground {
                (self.velocity.y / 2.0 + power).min(0.4)
            } else {
                self.velocity.y
            },
            self.velocity.z / 2.0 - kz,
        );
    }
    /// One game tick. Returns false once the mob should be removed.
    pub fn tick(
        &mut self,
        w: &MobWorld,
        _random: &mut LegacyRandom,
        events: &mut Vec<MobEvent>,
    ) -> bool {
        ai::tick(self, w, events)
    }
    pub fn distance_to(&self, p: [f64; 3]) -> f64 {
        let d = [
            p[0] - self.position.x,
            p[1] - self.position.y,
            p[2] - self.position.z,
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
    pub fn distance_sqr(&self, p: Vec3) -> f64 {
        (p.x - self.position.x).powi(2)
            + (p.y - self.position.y).powi(2)
            + (p.z - self.position.z).powi(2)
    }
    /// Movement speed attribute bonus (baby zombies, sprinting modifiers).
    /// MOVEMENT_SPEED including effects (speed +20%/level, slowness -15%/level).
    pub fn movement_speed(&self) -> f64 {
        let mut v = self.attrs.speed * self.speed_bonus();
        for (e, _, amp) in &self.extra.effects {
            match *e {
                "minecraft:speed" => v *= 1.0 + 0.2 * f64::from(amp + 1),
                "minecraft:slowness" => v *= (1.0 - 0.15 * f64::from(amp + 1)).max(0.0),
                _ => {}
            }
        }
        v
    }
    pub fn has_effect(&self, effect: &str) -> bool {
        self.extra.effects.iter().any(|e| e.0 == effect)
    }
    pub fn add_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32) {
        if let Some(e) = self.extra.effects.iter_mut().find(|e| e.0 == effect) {
            e.1 = e.1.max(ticks);
            e.2 = e.2.max(amplifier);
        } else {
            self.extra.effects.push((effect, ticks, amplifier));
        }
    }
    pub fn speed_bonus(&self) -> f64 {
        ai::speed_bonus(self)
    }
    pub fn center(&self) -> [f64; 3] {
        [
            self.position.x,
            self.position.y + f64::from(self.height()) / 2.0,
            self.position.z,
        ]
    }
    pub fn sound(&self, event: &'static str, events: &mut Vec<MobEvent>) {
        let at = [self.position.x, self.eye_y(), self.position.z];
        events.push(MobEvent::Sound(event, at, 1.0, 1.0));
    }
    /// Mob.playSound with LivingEntity.getVoicePitch.
    pub fn voice(&mut self, event: &'static str, events: &mut Vec<MobEvent>) {
        let pitch = (self.random.next_float() - self.random.next_float()) * 0.2
            + if self.is_baby() { 1.5 } else { 1.0 };
        let at = [self.position.x, self.eye_y(), self.position.z];
        events.push(MobEvent::Sound(event, at, ai::sound_volume(self), pitch));
    }
}
/// Model geometry for a mob, posed for its animation state, as camera-relative
/// vertices textured from the shared atlas.
pub fn render(
    mob: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    render::render_single(mob, alpha, camera, uv, light, out);
}
/// Load one baked model layer from `entity-models.json` (e.g. `minecraft:chest#main`).
pub fn load_model(root: &Path, key: &str) -> Option<Part> {
    let models: Value =
        serde_json::from_slice(&fs::read(root.join("entity-models.json")).ok()?).ok()?;
    models.get(key).map(|m| Part::parse("root", m))
}
/// ModelPart.render: translate, rotate ZYX, scale, then children.
pub fn render_model(
    part: &Part,
    base: glam::Mat4,
    pose_for: &dyn Fn(&Part) -> [f32; 9],
    uv: [f32; 4],
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    visit(part, base, pose_for, uv, light, overlay, out);
    fn visit(
        part: &Part,
        parent: glam::Mat4,
        pose_for: &dyn Fn(&Part) -> [f32; 9],
        uv: [f32; 4],
        light: u32,
        overlay: u32,
        out: &mut Vec<Vertex>,
    ) {
        let p = pose_for(part);
        let m = parent
            * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
            * glam::Mat4::from_quat(glam::Quat::from_euler(
                glam::EulerRot::ZYX,
                p[5],
                p[4],
                p[3],
            ))
            * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
        for poly in &part.polygons {
            for v in poly.vertices {
                let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                out.push(Vertex {
                    pos: pos.to_array(),
                    uv: [
                        uv[0] + (uv[2] - uv[0]) * v[3],
                        uv[1] + (uv[3] - uv[1]) * v[4],
                    ],
                    layer: overlay,
                    color: 0xFFFF_FFFF,
                    light,
                });
            }
        }
        for c in &part.children {
            visit(c, m, pose_for, uv, light, overlay, out);
        }
    }
}
