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
pub mod animal_brain;
pub mod aquatic;
pub mod axolotl;
pub mod bat;
pub mod bee;
pub mod body_anim;
pub mod brain;
pub mod breeze;
pub mod camel;
pub mod caravan;
pub mod control;
pub mod copper_golem;
pub mod creaking;
pub mod creaking_heart;
pub mod cube;
pub mod data;
pub mod drops;
pub mod enderman;
pub mod equine;
pub mod farm;
pub mod finalize;
pub mod gear;
pub mod goals;
pub mod golem;
pub mod guardian;
pub mod happy_ghast;
pub mod hostile;
pub mod illager;
pub mod illusioner;
pub mod keyframes;
pub mod leash;
pub mod living;
pub mod llama;
pub mod monster_brain;
pub mod nautilus;
pub mod nav;
pub mod nether;
pub mod path;
pub mod phantom;
pub mod piglin;
pub mod rabbit;
pub mod raider;
pub mod ravager;
pub mod render;
pub mod riders;
pub mod seen_grid;
pub mod shulker;
pub mod sniffer;
pub mod spawn_rules;
pub mod spawn_selection;
pub mod spawner;
pub mod spawning;
pub mod species;
pub mod strider;
pub mod sulfur_cube;
pub mod tame;
pub mod trader;
pub mod transport_items;
pub mod trial_spawner;
pub mod undead;
pub mod vex;
pub mod villager;
pub mod villager_brain;
pub mod villager_hats;
pub mod warden;
pub mod warden_brain;
pub mod witch;
pub mod wither;
mod world;
pub use world::visual_clip;
pub mod zombie_villager;

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
    pub normal: [f32; 3],
    pub vertices: [[f32; 5]; 4],
}
impl Polygon {
    fn vertex_normal(&self, pose: ModelNormals) -> [i8; 4] {
        let n = pose.matrix * glam::Vec3::from_array(self.normal);
        let n = if pose.trusted {
            n
        } else {
            n.normalize_or_zero()
        };
        let packed = n.to_array().map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
        [packed[0], packed[1], packed[2], -127]
    }
}

/// ModelPart uses a separate normal stack: uniform scaling keeps normal lengths,
/// while nonuniform scaling requires normalization after the complete part transform.
#[derive(Clone, Copy)]
struct ModelNormals {
    matrix: glam::Mat3,
    trusted: bool,
}
impl ModelNormals {
    // shortcut: infer the root normal pose from its geometry matrix; pass an explicit
    // root normal stack when root-transform rounding is ported as well.
    fn new(base: glam::Mat4) -> Self {
        let m = glam::Mat3::from_mat4(base);
        let lengths = glam::Vec3::new(m.x_axis.length(), m.y_axis.length(), m.z_axis.length());
        let uniform =
            lengths.min_element() > 0.0 && lengths.max_element() - lengths.min_element() < 1.0e-6;
        Self {
            matrix: if uniform {
                m / lengths.x
            } else {
                m.inverse().transpose()
            },
            trusted: uniform,
        }
    }
    fn rotation(x: f32, y: f32, z: f32) -> glam::Mat3 {
        // JOML derives cosine from the rounded sine, then retains quaternion length.
        // Assuming an exactly unit quaternion changes a packed normal by one byte.
        let sin_cos = |angle: f32| {
            let angle = angle * 0.5;
            let sin = f64::from(angle).sin() as f32;
            let cos = (1.0 - sin * sin).sqrt();
            let a = angle + std::f32::consts::FRAC_PI_2;
            let tau = std::f32::consts::TAU;
            let mut quadrant = a - (a / tau) as i32 as f32 * tau;
            if quadrant < 0.0 {
                quadrant += tau;
            }
            (
                sin,
                if quadrant >= std::f32::consts::PI {
                    -cos
                } else {
                    cos
                },
            )
        };
        let (sx, cx) = sin_cos(x);
        let (sy, cy) = sin_cos(y);
        let (sz, cz) = sin_cos(z);
        let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
        let (w, x, y, z) = (
            cx * cycz + sx * sysz,
            sx * cycz - cx * sysz,
            cx * sycz + sx * cysz,
            cx * cysz - sx * sycz,
        );
        let (ww, xx, yy, zz) = (w * w, x * x, y * y, z * z);
        let (xy, xz, xw, yz, yw, zw) = (x * y, x * z, x * w, y * z, y * w, z * w);
        glam::Mat3::from_cols_array(&[
            ww + xx - zz - yy,
            (xy + xy) + (zw + zw),
            (xz + xz) - (yw + yw),
            (xy + xy) - (zw + zw),
            yy - zz + ww - xx,
            (yz + yz) + (xw + xw),
            (yw + yw) + (xz + xz),
            (yz + yz) - (xw + xw),
            zz - yy - xx + ww,
        ])
    }
    fn part(mut self, pose: [f32; 9]) -> Self {
        self.matrix *= Self::rotation(pose[3], pose[4], pose[5]);
        let scale = glam::Vec3::new(pose[6], pose[7], pose[8]);
        if scale.x.abs() == scale.y.abs() && scale.x.abs() == scale.z.abs() {
            if scale.min_element() < 0.0 {
                self.matrix *= glam::Mat3::from_diagonal(scale.signum());
            }
        } else {
            self.matrix *= glam::Mat3::from_diagonal(scale.recip());
            self.trusted = false;
        }
        self
    }
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
            .enumerate()
            .filter_map(|(index, poly)| {
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
                    normal: v["normals"][index].as_array().map_or_else(
                        || {
                            // shortcut: collapsed legacy faces have no recoverable normal; re-prepare resources.
                            let p = verts
                                .iter()
                                .map(|v| glam::Vec3::new(v[0], v[1], v[2]))
                                .collect::<Vec<_>>();
                            (p[1] - p[0])
                                .cross(p[2] - p[0])
                                .normalize_or_zero()
                                .to_array()
                        },
                        |n| std::array::from_fn(|i| n[i].as_f64().unwrap_or(0.0) as f32),
                    ),
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
    /// LivingEntity.omnidirectionalAirMover (bees and parrots).
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
    /// Every default attribute value of the type (`entity-types.json`).
    pub attributes: Arc<HashMap<String, f64>>,
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
/// Each type's default attribute values, parsed exactly (serde_json's
/// default float parsing can miss the nearest double by one ulp).
fn exact_attributes(bytes: &[u8]) -> HashMap<String, HashMap<String, f64>> {
    use serde_json::value::RawValue;
    type Raw = HashMap<String, Box<RawValue>>;
    let Ok(types) = serde_json::from_slice::<Raw>(bytes) else {
        return HashMap::new();
    };
    types
        .into_iter()
        .filter_map(|(id, t)| {
            let t: Raw = serde_json::from_str(t.get()).ok()?;
            let attrs: Raw = serde_json::from_str(t.get("attributes")?.get()).ok()?;
            let values = attrs
                .into_iter()
                .filter_map(|(k, v)| v.get().trim().parse::<f64>().ok().map(|v| (k, v)))
                .collect();
            Some((id, values))
        })
        .collect()
}
pub struct MobTypes {
    pub types: HashMap<String, MobType>,
}
impl MobTypes {
    pub fn load(root: &Path) -> crate::Result<Self> {
        let bytes = fs::read(root.join("entity-types.json"))?;
        let types: Value = serde_json::from_slice(&bytes)?;
        let exact = exact_attributes(&bytes);
        let models: Value = serde_json::from_slice(&fs::read(root.join("entity-models.json"))?)?;
        let item_tags = crate::items::load_tags(root, "item").unwrap_or_default();
        sulfur_cube::load_archetype_items(&item_tags);
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
            let defaults = exact.get(&id).cloned().unwrap_or_default();
            let attr = |k: &str| defaults.get(&format!("minecraft:{k}")).copied();
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
                    attributes: Arc::new(defaults.clone()),
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
    /// `mace_smash`: a mace hit after a fall (the player attack tags too).
    MaceSmash,
    MobAttack,
    Arrow,
    Thrown,
    Fireball,
    Explosion,
    Fall,
    FallingBlock,
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
    SonicBoom,
    /// `fell_out_of_world`: below the world's floor.
    OutOfWorld,
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
    /// `DamageSources.mace`: a player's smash attack.
    pub fn mace(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MaceSmash,
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
                | DamageKind::SonicBoom
                | DamageKind::OutOfWorld
        )
    }
    pub fn is_fire(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::OnFire | DamageKind::InFire | DamageKind::Lava | DamageKind::Fireball
        )
    }
    /// DamageTypeTags.NO_KNOCKBACK
    pub fn no_knockback(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::Explosion
                | DamageKind::Fall
                | DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Lava
                | DamageKind::Drown
                | DamageKind::Magic
                | DamageKind::Cactus
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::OutOfWorld
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
    pub knockback_resistance: f64,
    pub spawn_reinforcements: f64,
    pub jump_strength: f64,
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
    /// AgeableMob.forcedAge: the age fed food added, restored as the age
    /// when it reaches 0 (`ForcedAge`).
    pub forced_age: i32,
    /// AgeableMob.forcedAgeTimer: ticks of happy villager particles left.
    pub forced_age_timer: i32,
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
    /// The saddle's components (name, enchantments...) when it is not a bare
    /// one; `saddled` stays the gameplay flag.
    pub saddle_stack: Option<crate::items::ItemStack>,
    pub chested: bool,
    /// The chest contents of a donkey or mule (15 slots).
    pub items: Vec<Option<crate::items::ItemStack>>,
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
    pub using_off_hand: bool,
    pub blocking: bool,
    pub fall_flying: bool,
    pub fall_fly_ticks: i32,
    /// `LivingEntity.isAutoSpinAttack` (Riptide).
    pub auto_spin_attack: bool,
    pub main_hand_left: bool,
    pub elytra: ElytraAnimation,
    pub swim: SwimAnimation,
    /// The arm the current swing moves (`currentSwing.hand().asArm`): `Some(true)`
    /// for the right arm.
    pub swing_right_arm: Option<bool>,
    pub flag: bool,
    pub flag2: bool,
    /// Species scratch values (the wither's head targets and clocks).
    pub ints: [i32; 6],
    pub counter: i32,
    pub counter2: i32,
    pub home: Option<[i32; 3]>,
    /// A copper golem's `openedChestPos`.
    pub opened_chest: Option<[i32; 3]>,
    pub point: Option<[f64; 3]>,
    pub vector: [f64; 3],
    /// Active status effects: (effect id, ticks left, amplifier).
    pub effects: Vec<(&'static str, i32, i32)>,
    /// An invisible illusioner's mirror images.
    pub illusions: Option<illusioner::Illusions>,
    /// A villager's claimed bed head and meeting bell.
    pub bed: Option<[i32; 3]>,
    pub meeting: Option<[i32; 3]>,
    /// Villager and wandering trader offers and experience.
    pub trade: Option<Box<crate::merchant::Trader>>,
    /// Who holds this mob's lead, a holder not yet found after loading, and
    /// the spin the rope's torque gives it.
    pub leash: Option<leash::Holder>,
    pub leash_pending: Option<leash::Pending>,
    pub leash_spin: f64,
    /// More species scratch values.
    pub scratch: [i32; 4],
    /// Block positions a species remembers (a sniffer's explored ground).
    pub positions: Vec<[i32; 3]>,
    /// A warden's brain, anger and vibrations.
    pub warden: Option<Box<warden::Warden>>,
    /// A raider's patrol and raid state.
    pub raider: Option<Box<raider::State>>,
    /// A villager's raid activity and gossip.
    pub villager: Option<Box<villager::Ext>>,
    /// The sniffer's and the ghastling's brain.
    pub animal_brain: Option<Box<animal_brain::Brain>>,
    pub axolotl: axolotl::Animation,
    /// The mob this one rides, and the mob riding it (`getVehicle`, `getFirstPassenger`).
    pub mount: Option<u32>,
    pub rider: Option<u32>,
    /// A chicken carrying a zombie (`IsChickenJockey`).
    pub jockey: bool,
    /// A skeleton horse waiting to spring its trap, and how long it has waited.
    pub trap: bool,
    pub trap_time: i32,
    /// Enchantments on the equipment above.
    pub gear: Option<Box<gear::Gear>>,
    /// `Mob.dropChances` by `EquipmentSlot` (`drops::SLOTS` order) when set
    /// explicitly; None uses the defaults of `drops::drop_chance`.
    pub drop_chances: Option<Box<[f32; 8]>>,
    /// `Mob.leftHanded`.
    pub left_handed: bool,
    /// The sound variant: index in the species' `*_sound_variant` registry.
    pub sound_variant: u8,
    /// `Zombie.canBreakDoors`.
    pub can_break_doors: bool,
    /// A goat's `IsScreamingGoat` and the horns it lost (left, right).
    pub screaming: bool,
    pub lost_horns: [bool; 2],
    /// Attribute bases and permanent modifiers set by `finalizeSpawn`.
    pub attributes: Option<Box<finalize::Attributes>>,
    /// Brain memories `finalizeSpawn` sets: (memory, value, ticks to expiry).
    pub memories: Vec<(&'static str, i64, i64)>,
    /// A camel's `LAST_POSE_CHANGE_TICK`.
    pub pose_change_tick: i64,
    /// The school leader a fish follows, and a tropical fish that swims alone.
    pub leader: Option<u32>,
    pub lone: bool,
    /// A breeze's or creaking's brain (`monster_brain`).
    pub brain: Option<Box<monster_brain::Brain>>,
    /// A bee's nectar, anger, flower and hive state.
    pub bee: Option<Box<bee::Bee>>,
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
    /// Entity.wasEyeInWater: the eyes were in water before this tick's fluid
    /// update (`isUnderWater` reads it).
    pub was_eye_in_water: bool,
    /// `Entity.firstTick`: no splash for a mob that appears in water.
    pub first_tick: bool,
    pub no_gravity: bool,
    /// `Entity.mainSupportingBlockPos`: the block it stood on after its last
    /// move.
    pub supporting: Option<[i32; 3]>,
    /// `NoAI`: no goals and no movement.
    pub no_ai: bool,
    /// `Silent`.
    pub silent: bool,
    /// `Invulnerable`: immune to damage (but not to `/kill`).
    pub invulnerable_flag: bool,
    pub climbing: bool,
    pub fire_ticks: i32,
    pub ticks_frozen: i32,
    pub glowing: bool,
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
    /// Events raised outside a tick (hurt and death sounds of damage taken
    /// from the game), handed over with the next tick's events or drained
    /// by the game.
    pub outbox: Vec<MobEvent>,
}
/// Events a mob tick asks the game to act on.
#[derive(Clone, Debug)]
pub enum MobEvent {
    /// Rabbit entity event 1, sent only when a ground jump actually happens.
    RabbitHop {
        mob: u32,
        at: [f64; 3],
    },
    Sound(&'static str, [f64; 3], f32, f32),
    /// `ChestBlockEntity.startOpen` / `stopOpen` by a copper golem: the
    /// chest's opener count goes up or down (interaction range 3).
    ChestOpen {
        chest: [i32; 3],
        open: bool,
    },
    /// `BeeEnterHiveGoal.start`: the bee goes into its hive (the host stores
    /// its tag with `BeehiveBlockEntity.addOccupant` and discards it).
    EnterHive {
        mob: u32,
        hive: [i32; 3],
    },
    /// `CopperGolem.turnToStatue`.
    BecomeStatue {
        mob: u32,
    },
    /// A copper golem took from or filled a chest slot.
    ChestSlot {
        chest: [i32; 3],
        slot: usize,
        stack: Option<crate::items::ItemStack>,
    },
    /// `OfferFlowerGoal.stop`: the item goes to the target's antenna (saddle)
    /// slot if it is empty, as a guaranteed drop.
    Gift {
        target: u32,
        item: &'static str,
    },
    /// `AnimalMakeLove`: the partner remembers this mob as its breed target
    /// and walks to it and looks at it.
    BreedTarget {
        mob: u32,
        partner: u32,
    },
    /// `BehaviorUtils.lockGazeAndWalkToEachOther`'s other half: the partner
    /// looks at this mob and walks to it.
    LockGaze {
        mob: u32,
        partner: u32,
    },
    /// A sound event id that is not a static string (variant sounds).
    SoundId(String, [f64; 3], f32, f32),
    /// `Level.playLocalSound`: a sound the client plays for itself, never
    /// sent by a server (a breeze's whirl and idle sounds).
    LocalSound(&'static str, [f64; 3], f32, f32),
    /// A level event's sound (`levelEvent`/`globalLevelEvent`): the client
    /// plays it with its own random pitch spread (`(a - b) * spread + 1`),
    /// nothing is drawn from the mob.
    LevelEventSound {
        sound: &'static str,
        at: [f64; 3],
        volume: f32,
        spread: f32,
    },
    /// `Level.gameEvent` from a mob's own movement (`GameEvent.STEP`): the
    /// game's sculk layer tracks steps itself, the mob AI probe hears them
    /// through the wardens (`warden::listen`).
    GameEvent {
        event: &'static str,
        at: [f64; 3],
        source: u32,
        state: horizon_world::registry::BlockStateId,
    },
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
    /// Client particles about a mob's box (`particles::around_self`).
    AroundSelf {
        kind: &'static str,
        feet: [f64; 3],
        width: f64,
        height: f64,
        burst: crate::particles::Burst,
    },
    /// One particle exactly at `at`, moving at `velocity`.
    ParticleAt {
        kind: &'static str,
        at: [f64; 3],
        velocity: [f64; 3],
    },
    Experience {
        at: [f64; 3],
        amount: u32,
    },
    /// Regeneration support after an axolotl-assisted kill.
    AxolotlSupportPlayer,
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
        /// A llama's baby, drawn from the parent's random as
        /// `Llama.getBreedOffspring` does.
        llama: Option<Box<super::mobs::llama::Offspring>>,
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
    /// Two villagers are close: swap gossip and share food and wares.
    VillagerMeet {
        mob: u32,
        other: u32,
    },
    /// Two villagers make a baby (`VillagerMakeLove.tryToGiveBirth`): the
    /// parent has eaten and, with a bed for the child, rests; the partner
    /// eats, and rests too if `child` holds the child's bed and type
    /// (`None`: the biome's).
    VillagerBirth {
        parent: u32,
        partner: u32,
        child: Option<([i32; 3], Option<u8>)>,
    },
    /// A villager asks whether enough neighbours want an iron golem.
    GolemCheck {
        mob: u32,
        needed: usize,
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
    /// A lead snaps or loses its holder: the lead item drops when asked.
    LeashBreak {
        mob: u32,
        drop: bool,
        sound: bool,
    },
    /// A zombie villager's cure is complete: the game turns it back into a villager.
    CureZombieVillager {
        mob: u32,
    },
    /// A vibration travelling to a listener (`VibrationParticleOption`).
    Vibration {
        from: [f64; 3],
        to: [f64; 3],
        ticks: i32,
    },
    /// The crumbs of the block under a digging or emerging warden.
    DigParticles {
        at: [f64; 3],
    },
    /// `Warden.applyDarknessAround`: darkness for survival players nearby.
    Darkness {
        at: [f64; 3],
        radius: f64,
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
    pub hostile: bool,
    pub target: Option<Who>,
    pub health: f32,
    pub variant: u8,
    /// Dye colour (sheep).
    pub color: u8,
    /// A villager's claimed job site and bed.
    pub job: Option<[i32; 3]>,
    pub bed: Option<[i32; 3]>,
    pub yaw: f32,
    pub velocity: Vec3,
    /// Who holds this mob's lead.
    pub leash: Option<leash::Holder>,
    /// A llama's caravan head (its id plus one, 0 when not in a caravan).
    pub caravan: u32,
    /// A wandering trader's despawn delay.
    pub delay: i32,
    /// `getLastHurtByMob` and its timestamp.
    pub hurt_by: Option<(Who, i32)>,
    /// A raider in an active raid.
    pub active_raid: bool,
    /// A copper golem's `openedChestPos` (`ContainerUser.hasContainerOpen`).
    pub opened_chest: Option<[i32; 3]>,
    /// A villager's reputation of the player (`getPlayerReputation`).
    pub player_rep: i32,
    /// `isPushable` and pushed by others (`Entity.push(Entity)` moves it).
    pub pushable: bool,
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
    /// `MAX_HEALTH`, `JUMP_STRENGTH` and `MOVEMENT_SPEED` base values (a
    /// horse's breeding stock).
    pub stats: [f64; 3],
}
impl Mob {
    pub fn burning_time(&self) -> f64 {
        let base = finalize::value(self, finalize::Attr::BurningTime);
        let protection: u32 = gear::ARMOR
            .into_iter()
            .map(|slot| gear::level(self, slot, "minecraft:fire_protection"))
            .sum();
        (base - 0.15 * f64::from(protection)).max(0.0)
    }
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        self.fire_ticks = self
            .fire_ticks
            .max((f64::from(ticks) * self.burning_time()).ceil() as i32);
        self.ticks_frozen = 0;
    }

    /// What other mobs see of this one this tick (`player`: the player's
    /// UUID, which villagers' reputation is keyed by).
    pub fn seen(&self, player: Option<[i32; 4]>) -> Seen {
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
            hostile: m.hostile(),
            target: m.ai.target,
            health: m.health,
            variant: m.extra.variant,
            color: m.extra.color,
            job: villager.then_some(m.extra.home).flatten(),
            bed: villager.then_some(m.extra.bed).flatten(),
            yaw: m.yaw,
            velocity: m.velocity,
            leash: m.extra.leash,
            caravan: caravan::head_of(m),
            delay: m.extra.ints[5],
            hurt_by: m.ai.last_hurt_by.map(|w| (w, m.ai.last_hurt_by_time)),
            active_raid: raider::has_active_raid(m),
            opened_chest: m.extra.opened_chest,
            player_rep: match (villager, player) {
                (true, Some(uuid)) => villager::ext(m).map_or(0, |e| e.gossips.reputation(uuid)),
                _ => 0,
            },
            pushable: m.pushable(),
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
        }
    }
    /// `EntityType.getCategory`'s name.
    pub fn category(&self) -> &'static str {
        match self.kind.category.as_str() {
            "monster" => "monster",
            "creature" => "creature",
            "ambient" => "ambient",
            "axolotls" => "axolotls",
            "underground_water_creature" => "underground_water_creature",
            "water_creature" => "water_creature",
            "water_ambient" => "water_ambient",
            _ => "misc",
        }
    }
}
/// `Entity.push` on other mobs acts at once in vanilla: a mob ticking later
/// moves with it this tick. Takes the `Push` events aimed at mobs out of
/// `events[from..]` into `pending`, which the caller adds to each mob's
/// velocity before its tick (and to every mob once all have ticked).
pub fn take_mob_pushes(events: &mut Vec<MobEvent>, from: usize, pending: &mut Vec<(u32, Vec3)>) {
    let mut i = from;
    while i < events.len() {
        if let MobEvent::Push {
            target: Who::Mob(id),
            velocity,
        } = events[i]
        {
            pending.push((id, Vec3::new(velocity[0], velocity[1], velocity[2])));
            events.remove(i);
        } else {
            i += 1;
        }
    }
}
/// Adds the pending pushes aimed at `m` to its velocity, in push order.
pub fn apply_mob_pushes(m: &mut Mob, pending: &mut Vec<(u32, Vec3)>) {
    pending.retain(|(id, v)| {
        if *id != m.id {
            return true;
        }
        m.velocity = Vec3::new(m.velocity.x + v.x, m.velocity.y + v.y, m.velocity.z + v.z);
        false
    });
}
/// Applies the brain memories a mob's tick set on other mobs (from event
/// index `from` on), at once: vanilla sets them on the other entity during
/// the tick, before the mobs after it tick.
pub fn apply_brain_events(mobs: &mut [Mob], events: &mut Vec<MobEvent>, from: usize) {
    let mut i = from;
    while i < events.len() {
        match events[i] {
            MobEvent::BreedTarget { mob, partner } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == mob) {
                    animal_brain::set_breed_partner(m, partner);
                }
                events.remove(i);
            }
            MobEvent::LockGaze { mob, partner } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == mob) {
                    animal_brain::lock_gaze(m, partner);
                    villager_brain::lock_gaze_half(m, Who::Mob(partner));
                }
                events.remove(i);
            }
            // Animal.finalizeSpawnChildFromBreeding: the partner rests and
            // forgets it at once (the game still makes the baby).
            MobEvent::Breed {
                parent, partner, ..
            } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == partner) {
                    m.extra.age = 6000;
                    m.extra.in_love = 0;
                    animal_brain::forget_breed_target(m, parent);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
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
    pub velocity: Vec3,
    pub yaw: f32,
    /// The Hero of the Village effect.
    pub hero: bool,
    /// The player's UUID (villagers' gossip is keyed by it).
    pub uuid: [i32; 4],
    /// The mobs on the player's team (`isAlliedTo`): combat targeting
    /// leaves the player alone.
    pub allies: std::sync::Arc<std::collections::HashSet<u32>>,
    /// A LAN guest: its own client moves it, and touching mobs push it there
    /// (`EntitySelector.pushableBy` on the client).
    pub remote: bool,
}
pub struct MobWorld<'a> {
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    pub movement: &'a horizon_world::movement_properties::BoundMovementProperties,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub data: &'a MobData,
    pub player: Option<PlayerView>,
    /// `getNearestPlayer(mob, -1)`: where the nearest non-spectator of every
    /// player in the level stands, alive or not (the despawn distance).
    pub nearest_player: Option<Vec3>,
    pub others: &'a [Seen],
    /// `others` by position, when the caller keeps one (the mob ticks).
    pub others_grid: Option<&'a seen_grid::SeenGrid>,
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
    /// `getCurrentDifficultyAt` near the player.
    pub difficulty: spawning::LocalDifficulty,
    /// The End (its dimension type's monster spawn light rules).
    pub end: bool,
    /// The world seed (slime chunks).
    pub world_seed: i64,
    /// `Level.getSeaLevel` (63 in the overworld, -63 in a flat world).
    pub sea_level: i32,
    /// The scoreboard teams of the player and the mobs (collision rules).
    pub teams: Teams,
    /// Item data (max stack sizes), when the host has it.
    pub items: Option<&'a crate::items::Items>,
}
/// `Entity.getTeam` of the player and of the mobs on a team.
#[derive(Clone, Debug, Default)]
pub struct Teams {
    pub player: Option<crate::scoreboard::Team>,
    pub mobs: HashMap<u32, crate::scoreboard::Team>,
}
impl Teams {
    pub fn of(&self, who: Who) -> Option<&crate::scoreboard::Team> {
        match who {
            Who::Player => self.player.as_ref(),
            Who::Mob(id) => self.mobs.get(&id),
        }
    }
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
            // Mob.ambientSoundTime starts at 0.
            ambient_timer: 0,
            fall_distance: 0.0,
            persistent: false,
            random,
            ai: Box::default(),
            extra: Extra::default(),
            fluids: living::Fluids::default(),
            was_eye_in_water: false,
            first_tick: true,
            no_gravity: false,
            supporting: None,
            no_ai: false,
            silent: false,
            invulnerable_flag: false,
            climbing: false,
            fire_ticks: 0,
            ticks_frozen: 0,
            glowing: false,
            air: 300,
            last_hurt: 0.0,
            last_damage: None,
            removed: false,
            custom_name: None,
            name_visible: false,
            anim: anim::AnimState::default(),
            rider: None,
            outbox: Vec::new(),
            attrs: Attrs {
                speed: kind.speed,
                attack_damage: kind.attack_damage,
                max_health: kind.max_health,
                follow_range: kind.follow_range,
                knockback_resistance: kind.knockback_resistance,
                spawn_reinforcements: kind
                    .attributes
                    .get("minecraft:spawn_reinforcements")
                    .copied()
                    .unwrap_or(0.0),
                jump_strength: kind.jump_strength,
            },
            kind,
        };
        // setCanPickUpLoot(true) in the Villager, AbstractPiglin, Dolphin,
        // Fox and Panda constructors (an allay's is false until it holds
        // something: it overrides canPickUpLoot).
        mob.extra.pick_up_loot = matches!(
            mob.species(),
            Species::Villager
                | Species::Piglin
                | Species::PiglinBrute
                | Species::Dolphin
                | Species::Fox
                | Species::Panda
        );
        ai::register(&mut mob);
        mob
    }
    /// `Mob.canPickUpLoot`, with the overrides: an allay that holds
    /// something off its pickup cooldown, a sulfur cube with no block.
    pub fn can_pick_up_loot(&self) -> bool {
        match self.species() {
            Species::Allay => self.extra.main_hand.is_some() && self.extra.ints[3] <= 0,
            Species::SulfurCube => !sulfur_cube::has_body(self),
            _ => self.extra.pick_up_loot,
        }
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
        self.damage_with(amount, source, None)
    }
    /// `damage` by a source with a weapon: `armor_effectiveness` runs the
    /// weapon's armor-effectiveness enchantments (breach) over the armor fraction.
    pub fn damage_with(
        &mut self,
        amount: f32,
        source: Damage,
        armor_effectiveness: Option<&mut dyn FnMut(f32) -> f32>,
    ) -> bool {
        axolotl::before_damage(self, amount, &source);
        raider::note_hurt(self);
        if !self.alive() || self.removed || self.invulnerable_flag {
            return false;
        }
        // Ghast.isReflectedFireball: a fireball a player turned back is
        // no fire to it, and kills it (Ghast.hurtServer: 1000 damage).
        let reflected = &*self.kind.id == "minecraft:ghast"
            && source.kind == DamageKind::Fireball
            && source.attacker == Some(Who::Player);
        if source.is_fire() && (self.kind.flags.fire_immune) && !reflected {
            return false;
        }
        if !ai::accepts_damage(self, &source, amount) {
            return false;
        }
        let mut amount = if reflected { 1000.0 } else { amount };
        if source.kind == DamageKind::FallingBlock
            && !species::body_slot(self.species())
            && self.extra.head.is_some()
        {
            amount *= 0.75;
        }
        let (worn, worn_toughness) = self.worn_armor();
        if !source.bypasses_armor() && self.kind.armor + worn > 0.0 {
            // CombatRules.getDamageAfterAbsorb
            let armor = self.kind.armor + worn;
            let toughness = 2.0 + (self.kind.armor_toughness + worn_toughness) / 4.0;
            let effective = (armor - amount / toughness).clamp(armor * 0.2, 20.0);
            let mut fraction = effective / 25.0;
            if let Some(modify) = armor_effectiveness {
                fraction = modify(fraction).clamp(0.0, 1.0);
            }
            amount *= 1.0 - fraction;
        }
        // CombatRules.getDamageAfterMagicAbsorb: enchantment protection.
        let epf = gear::protection(self, source.kind);
        if epf > 0.0 {
            amount *= 1.0 - epf / 25.0;
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
        // dealDefaultKnockback, unless the damage type is tagged no_knockback;
        // a source without a position knocks in a random direction.
        if knock && !source.no_knockback() {
            let (dx, dz) = source.from.map_or((0.0, 0.0), |f| {
                (f[0] - self.position.x, f[2] - self.position.z)
            });
            self.knockback(f64::from(0.4f32), dx, dz);
        }
        ai::after_hurt(self, &source);
        if self.health <= 0.0 {
            self.health = 0.0;
        }
        if knock {
            // playHurtSound (Mob: also resets the ambient sound clock) or the
            // death sound: makeSound draws the voice pitch whenever there is a sound.
            let dead = !self.alive();
            let sound = ai::voice_sounds(self)[if dead { 2 } else { 1 }];
            if !dead {
                self.ambient_timer = -self.kind.ambient_interval;
            }
            if !sound.is_empty() {
                let pitch = self.voice_pitch();
                if !self.silent {
                    let at = [self.position.x, self.position.y, self.position.z];
                    self.outbox
                        .push(MobEvent::Sound(sound, at, ai::sound_volume(self), pitch));
                }
            }
        }
        true
    }
    /// LivingEntity.knockback(power, xd, zd) towards the negative direction.
    pub fn knockback(&mut self, power: f64, mut xd: f64, mut zd: f64) {
        let power = power * (1.0 - self.attrs.knockback_resistance);
        if power <= 0.0 {
            return;
        }
        while xd * xd + zd * zd < f64::from(1.0e-5_f32) {
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
        level_random: &mut LegacyRandom,
        events: &mut Vec<MobEvent>,
    ) -> bool {
        ai::tick(self, w, level_random, events)
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
        let mut v = strider::movement_speed(self, self.attrs.speed) * self.speed_bonus();
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
    /// LivingEntity.getVoicePitch.
    pub fn voice_pitch(&mut self) -> f32 {
        let pitch = (self.random.next_float() - self.random.next_float()) * 0.2
            + if self.is_baby() { 1.5 } else { 1.0 };
        // Bat.getVoicePitch: a little lower.
        if self.species() == Species::Bat {
            pitch * 0.95
        } else {
            pitch
        }
    }
    /// Mob.playSound with LivingEntity.getVoicePitch.
    pub fn voice(&mut self, event: &'static str, events: &mut Vec<MobEvent>) {
        let pitch = self.voice_pitch();
        // Entity.playSound: at the entity's feet.
        let at = [self.position.x, self.position.y, self.position.z];
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
/// A parsed `entity-models.json` document, shared while loading model batches.
/// Each lookup still constructs an independent part tree for the caller.
pub struct EntityModelDocument(Value);

impl EntityModelDocument {
    pub fn load(root: &Path) -> crate::Result<Self> {
        Ok(Self(serde_json::from_slice(&fs::read(
            root.join("entity-models.json"),
        )?)?))
    }

    pub fn model(&self, key: &str) -> Option<Part> {
        self.0.get(key).map(|m| Part::parse("root", m))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0
            .as_object()
            .into_iter()
            .flat_map(|all| all.keys().map(String::as_str))
    }
}

/// Load one baked model layer from `entity-models.json` (e.g. `minecraft:chest#main`).
/// Batch loaders should share an `EntityModelDocument` instead.
pub fn load_model(root: &Path, key: &str) -> Option<Part> {
    EntityModelDocument::load(root).ok()?.model(key)
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
    visit(
        part,
        base,
        ModelNormals::new(base),
        pose_for,
        uv,
        light,
        overlay,
        out,
    );
    fn visit(
        part: &Part,
        parent: glam::Mat4,
        normals: ModelNormals,
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
        let normals = normals.part(p);
        for poly in &part.polygons {
            let normal = poly.vertex_normal(normals);
            for v in poly.vertices {
                let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                out.push(Vertex {
                    normal,
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
            visit(c, m, normals, pose_for, uv, light, overlay, out);
        }
    }
}

impl Mob {
    /// The saddle worn, with its components.
    pub fn saddle_item(&self) -> Option<crate::items::ItemStack> {
        self.extra.saddled.then(|| {
            self.extra
                .saddle_stack
                .clone()
                .unwrap_or_else(|| crate::items::ItemStack::new("minecraft:saddle", 1))
        })
    }
    /// Wear (or take off) a saddle stack, keeping its components.
    pub fn set_saddle(&mut self, saddle: Option<crate::items::ItemStack>) {
        self.extra.saddled = saddle.is_some();
        // A plain saddle is implied by `saddled`; anything else (a copper
        // golem's antenna flower) is kept as it is.
        self.extra.saddle_stack = saddle
            .map(|s| crate::items::ItemStack { count: 1, ..s })
            .filter(|s| s.damage != 0 || s.extra.is_some() || &*s.item != "minecraft:saddle");
    }
}

#[cfg(test)]
mod lighting_tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use serde_json::json;

    fn plane(normal: [f32; 3]) -> Part {
        Part::parse(
            "root",
            &json!({
                "polygons": [[[0,0,0,0,0], [0,0,0,1,0], [0,0,0,1,1], [0,0,0,0,1]]],
                "normals": [normal]
            }),
        )
    }

    #[test]
    fn exported_normals_survive_collapsed_faces_and_mirroring() {
        let part = plane([1.0, 0.0, 0.0]);
        assert_eq!(
            part.polygons[0].vertex_normal(ModelNormals::new(Mat4::IDENTITY)),
            [127, 0, 0, -127]
        );
        assert_eq!(
            part.polygons[0].vertex_normal(ModelNormals::new(Mat4::from_scale(Vec3::new(
                -1.0, -1.0, 1.0
            )))),
            [-127, 0, 0, -127]
        );
    }

    #[test]
    fn normal_matrix_includes_parent_scale_and_child_rotation() {
        let mut child = plane([1.0, 0.0, 0.0]);
        child.pose[5] = std::f32::consts::FRAC_PI_4;
        let parent = Part {
            name: "parent".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 1.0, 0.5],
            polygons: Vec::new(),
            children: vec![child],
        };
        let mut animated = Vec::new();
        render::draw(
            &parent,
            Mat4::IDENTITY,
            &anim::Poses::new(&parent),
            [0.0, 0.0, 1.0, 1.0],
            0x80402010,
            240,
            1,
            &mut animated,
        );
        let mut direct = Vec::new();
        render_model(
            &parent,
            Mat4::IDENTITY,
            &|p| p.pose,
            [0.0, 0.0, 1.0, 1.0],
            240,
            1,
            &mut direct,
        );
        assert_eq!(animated.len(), 4);
        for (a, b) in animated.iter().zip(&direct) {
            assert_eq!(a.normal, [56, 113, 0, -127]);
            assert_eq!(a.normal, b.normal);
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.uv, b.uv);
            assert_eq!((a.color, a.light, a.layer), (0x80402010, 240, 1));
        }
    }

    #[test]
    fn trusted_rotation_retains_native_quaternion_rounding() {
        let mut part = plane([1.0, 0.0, 0.0]);
        part.pose[3] = std::f32::consts::FRAC_PI_2;
        let mut vertices = Vec::new();
        render::draw(
            &part,
            Mat4::IDENTITY,
            &anim::Poses::new(&part),
            [0.0, 0.0, 1.0, 1.0],
            0xffffffff,
            240,
            0,
            &mut vertices,
        );
        assert_eq!(vertices[0].normal, [126, 0, 0, -127]);
    }

    #[test]
    fn legacy_models_infer_non_degenerate_face_normals() {
        let part = Part::parse(
            "root",
            &json!({"polygons": [
                [[0,0,0,0,0], [1,0,0,1,0], [1,1,0,1,1], [0,1,0,0,1]]
            ]}),
        );
        assert_eq!(part.polygons[0].normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn packed_normal_uses_native_snorm_bytes_without_changing_vertex_offsets() {
        assert_eq!(std::mem::offset_of!(Vertex, normal), 32);
        assert_eq!(std::mem::size_of::<Vertex>(), 36);
        let part = plane([0.0, 1.0, 0.0]);
        assert_eq!(
            part.polygons[0].vertex_normal(ModelNormals::new(Mat4::from_scale(Vec3::ZERO))),
            [0, 0, 0, -127]
        );
    }
}

/// `LivingEntity.elytraAnimationState`: eased wing angles and render interpolation.
#[derive(Clone, Copy, Debug, Default)]
pub struct ElytraAnimation {
    pub old: [f32; 3],
    pub angles: [f32; 3],
}
impl ElytraAnimation {
    pub fn tick(&mut self, flying: bool, crouching: bool, velocity: horizon_core::geometry::Vec3) {
        use std::f32::consts::PI;
        self.old = self.angles;
        let target = if flying {
            let length =
                (velocity.x * velocity.x + velocity.y * velocity.y + velocity.z * velocity.z)
                    .sqrt();
            let ratio = if velocity.y < 0.0 && length > 0.0 {
                1.0 - horizon_core::jmath::pow(-velocity.y / length, 1.5) as f32
            } else {
                1.0
            };
            [
                PI / 12.0 + (PI / 9.0 - PI / 12.0) * ratio,
                0.0,
                -PI / 12.0 + (-PI / 2.0 + PI / 12.0) * ratio,
            ]
        } else if crouching {
            [PI * 2.0 / 9.0, 0.08726646, -PI / 4.0]
        } else {
            [PI / 12.0, 0.0, -PI / 12.0]
        };
        for (angle, target) in self.angles.iter_mut().zip(target) {
            *angle += (target - *angle) * 0.3;
        }
    }
    pub fn angles(&self, partial: f32) -> [f32; 3] {
        std::array::from_fn(|i| self.old[i] + (self.angles[i] - self.old[i]) * partial)
    }
}

/// `LivingEntity.swimAmount` and the render state the swimming pose reads
/// (`isVisuallySwimming`, `isInWater`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SwimAnimation {
    pub old: f32,
    pub amount: f32,
    /// `isVisuallySwimming`: in the swimming pose (or still in the gliding
    /// pose after a glide ended).
    pub visually: bool,
    pub in_water: bool,
}
impl SwimAnimation {
    /// `LivingEntity.updateSwimAmount`.
    pub fn tick(&mut self, visually: bool, in_water: bool) {
        self.old = self.amount;
        self.amount = if visually {
            (self.amount + 0.09).min(1.0)
        } else {
            (self.amount - 0.09).max(0.0)
        };
        self.visually = visually;
        self.in_water = in_water;
    }
    /// `LivingEntity.getSwimAmount`.
    pub fn amount(&self, partial: f32) -> f32 {
        self.old + partial * (self.amount - self.old)
    }
}

#[cfg(test)]
mod elytra_tests {
    use super::*;
    #[test]
    fn wings_ease_between_ground_flight_and_crouch() {
        let mut wings = ElytraAnimation::default();
        wings.tick(false, false, Vec3::ZERO);
        assert!((wings.angles[0] - std::f32::consts::PI / 12.0 * 0.3).abs() < 0.000001);
        assert_eq!(wings.angles(0.0), wings.old);
        assert_eq!(wings.angles(1.0), wings.angles);
        for _ in 0..40 {
            wings.tick(true, false, Vec3::new(1.0, 0.0, 0.0));
        }
        assert!((wings.angles[2] + std::f32::consts::FRAC_PI_2).abs() < 0.00001);
        for _ in 0..40 {
            wings.tick(false, true, Vec3::ZERO);
        }
        assert!((wings.angles[1] - 0.08726646).abs() < 0.00001);
    }
}
