//! Mobs: entity types (dimensions and default attributes exported from vanilla), models (vanilla
//! `LayerDefinitions` baked part trees) and the `Mob` entity with vanilla's AI framework: goal and
//! target selectors with priorities and control flags (`goals`), A* pathfinding with walk, swim,
//! fly and amphibious node evaluators (`path`, `nav`), move/look/jump/body controls (`control`),
//! `LivingEntity` physics (`living`) and per-species behaviour, models, animations and sounds.
pub mod ai;
pub mod allay;
pub mod ambient;
pub mod anim;
pub mod animal_brain;
pub mod aquatic;
pub mod armadillo;
pub mod armor;
pub mod axolotl;
pub mod bat;
pub mod bee;
pub mod body_anim;
pub mod brain;
pub mod breeze;
pub mod camel;
pub mod caravan;
pub mod cem;
pub mod cem_anim;
pub mod combat;
pub mod control;
pub mod conversion;
pub mod copper_golem;
pub mod creaking;
pub mod creaking_heart;
pub mod cube;
pub mod dab;
pub mod data;
pub mod dolphin;
pub mod doors;
pub mod drops;
pub mod enderman;
pub mod entity_sounds;
pub mod equine;
pub mod equip;
pub mod farm;
pub mod finalize;
pub mod fox;
pub mod frog;
pub mod gear;
pub mod goals;
pub mod goat;
pub mod golem;
pub mod guardian;
pub mod happy_ghast;
pub mod hostile;
mod hurt_by;
pub mod illager;
pub mod illusioner;
pub mod item_break;
pub mod keyframes;
pub mod landing;
pub mod leash;
pub mod living;
pub mod llama;
mod mob_slot;
pub mod mod_behaviors;
pub mod model_layers;
pub mod monster_brain;
pub mod nautilus;
pub mod neutral_anger;
pub use mob_slot::MobSlot;
mod push;
mod type_attributes;
pub use neutral_anger::{
    adopt_anger_end, anger_left, persistent_anger, resume_anger, settle_anger_end,
};
mod hit_attempt;
mod knockback;
pub mod nav;
pub mod nether;
pub mod panda;
pub mod parrot;
pub mod part_goals;
pub mod path;
pub mod phantom;
pub mod piglin;
pub mod player_selection;
pub mod polar_bear;
pub mod rabbit;
pub mod raider;
pub mod random_textures;
pub mod ravager;
pub mod render;
pub mod riders;
pub mod seen_grid;
pub mod sheep_dye;
pub mod shulker;
pub mod skin;
pub mod skin_layers;
pub mod sniffer;
pub mod spawn_rules;
pub mod spawn_selection;
pub mod spawner;
pub mod spawning;
pub mod spear_use;
pub mod species;
pub mod strider;
pub mod stuck;
pub mod suffocation;
pub mod sulfur_cube;
pub mod sync;
pub mod tame;
mod target;
pub mod trader;
pub mod transport_items;
pub mod trial_spawner;
pub mod turtle;
pub mod undead;
pub mod vex;
pub mod villager;
pub mod villager_brain;
pub mod villager_hats;
pub mod visibility;
pub use visibility::mob_visibility;
pub mod warden;
pub mod warden_brain;
pub mod witch;
pub mod wither;

#[cfg(test)]
mod invisible_follow_tests;
mod world;
pub use world::{TargetInfo, single_player_pos, visual_clip};
pub mod dragon_contacts;
pub mod zombie_villager;
/// A mob's stored active effect with its registry index resolved once at creation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoredEffect {
    pub id: &'static str,
    pub duration: i32,
    pub amplifier: i32,
    pub registry_index: Option<usize>,
    /// `MobEffectInstance.showIcon`, kept apart from the particle flag: the saved `show_icon`.
    pub show_icon: bool,
}
impl StoredEffect {
    /// `new MobEffectInstance(effect, duration, amplifier)`: its icon shows.
    pub fn new(id: &'static str, duration: i32, amplifier: i32) -> Self {
        Self {
            id,
            duration,
            amplifier,
            registry_index: horizon_client_core::potions::effect(id).map(|def| def.index),
            show_icon: true,
        }
    }
    /// Copy a registered instance after an update or hidden-effect promotion, without resolving again.
    pub fn from_instance(id: &'static str, instance: &crate::effects::EffectInstance) -> Self {
        Self {
            id,
            duration: instance.duration,
            amplifier: instance.amplifier,
            registry_index: instance.registry_index,
            show_icon: instance.show_icon,
        }
    }
}

use crate::{
    blocks::BlockFacts,
    health::{DamageContainer, DamageLevel, Reduction},
    mesher::Vertex,
    world::World,
};
pub use data::MobData;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use horizon_world::native_load_context::LoopContext;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

mod model;
use model::ModelNormals;
pub use model::{Part, Polygon};

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

mod breeding;
pub use breeding::{draws_parent_variant, orb_after_child};

mod collision_subject;
mod flags;
pub use flags::Flags;
#[cfg(test)]
mod powder_snow_move_tests;

#[derive(Clone, Debug)]
pub struct MobType {
    /// Resolved content renderer shadow; vanilla uses its normal table.
    pub shadow: Option<(f32, f32)>,
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
    /// Item defaults used when loading inventory-bearing mobs before a Game exists.
    pub item_stack_limits: Arc<HashMap<String, u32>>,
    /// The bounds `finalize`'s attribute record clamps to, from the catalog.
    pub attribute_bounds: finalize::AttrBounds,
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
    pub layers: crate::fast_hash::FxHashMap<String, Arc<Part>>,
    pub texture: String,
    pub brain: Brain,
    pub anim: Anim,
    pub sounds: [&'static str; 3],
    pub sound_overrides: entity_sounds::Sounds,
    /// Animal.isFood items (resolved from the item tag).
    pub food: Arc<[String]>,
    /// TemptGoal items when they differ from the food.
    pub tempt: Arc<[String]>,
    pub passenger_attachments: Vec<[f32; 3]>,
}
impl MobType {
    pub fn inventory_stack_limit(&self, stack: &crate::items::ItemStack) -> u32 {
        stack
            .data()
            .and_then(|d| d.other("minecraft:max_stack_size"))
            .and_then(|t| match t {
                horizon_core::nbt::Tag::Int(n) => u32::try_from(*n).ok(),
                _ => None,
            })
            .unwrap_or_else(|| {
                self.item_stack_limits
                    .get(stack.item.as_ref())
                    .copied()
                    .unwrap_or(64)
            })
    }

    pub fn layer(&self, key: &str) -> Option<&Arc<Part>> {
        self.layers.get(key)
    }
}
/// Each type's default attribute values, parsed exactly (serde_json's
/// default float parsing can miss the nearest double by one ulp).
pub(crate) fn exact_attributes(bytes: &[u8]) -> HashMap<String, HashMap<String, f64>> {
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

/// The models that draw with `RenderTypes::entityCutoutCull`.
fn culls_back_faces(key: &str) -> bool {
    matches!(key, "minecraft:bat#main" | "minecraft:turtle_baby#main")
}

pub struct MobTypes {
    pub types: HashMap<String, MobType>,
}
impl MobTypes {
    /// The types with the attribute catalog of the same resources
    /// (`attributes.json`), for a caller without the game's own.
    pub fn load(root: &Path) -> crate::Result<Self> {
        Self::load_with_models(root, true, None, None, &Self::root_catalog(root)?)
    }

    /// Load client mob data using defaults already decoded by the resource
    /// bootstrap. This avoids decoding `items.json` a second time for Allay
    /// inventory limits.
    pub fn load_with_defaults(
        root: &Path,
        defaults: &horizon_world::item_defaults::Defaults,
        attributes: &horizon_world::attributes::entity::Catalog,
    ) -> crate::Result<Self> {
        Self::load_with_models(root, true, None, Some(defaults), attributes)
    }

    /// The dedicated server needs entity facts and AI, but never their render
    /// geometry. Keep one empty part for the fields shared with the client
    /// rather than parsing and retaining every entity-model layer.
    pub fn load_for_server(
        root: &Path,
        attributes: &horizon_world::attributes::entity::Catalog,
    ) -> crate::Result<Self> {
        Self::load_with_models(root, false, None, None, attributes)
    }

    /// The client startup already parsed `entity-models.json` for the handoff
    /// and the other model loaders; read the model layers from that copy.
    pub fn load_with_document(root: &Path, document: &EntityModelDocument) -> crate::Result<Self> {
        Self::load_with_models(root, true, Some(document), None, &Self::root_catalog(root)?)
    }

    /// The client startup already has the entity model document, the item
    /// defaults and the attribute catalog. Reuse them instead of decoding
    /// their files again.
    pub fn load_with_document_and_defaults(
        root: &Path,
        document: &EntityModelDocument,
        defaults: &horizon_world::item_defaults::Defaults,
        attributes: &horizon_world::attributes::entity::Catalog,
    ) -> crate::Result<Self> {
        Self::load_with_models(root, true, Some(document), Some(defaults), attributes)
    }

    fn root_catalog(root: &Path) -> crate::Result<horizon_world::attributes::entity::Catalog> {
        let json = horizon_world::json_file::read(&root.join("attributes.json"))?;
        Ok(horizon_world::attributes::entity::Catalog::from_json(
            &json,
        )?)
    }

    /// `attributes`: the catalog whose bounds the types' mobs clamp to.
    fn load_with_models(
        root: &Path,
        models_needed: bool,
        document: Option<&EntityModelDocument>,
        item_defaults: Option<&horizon_world::item_defaults::Defaults>,
        attributes: &horizon_world::attributes::entity::Catalog,
    ) -> crate::Result<Self> {
        let attribute_bounds = finalize::AttrBounds::of(attributes);
        let bytes = fs::read(root.join("entity-types.json"))?;
        let types: Value = serde_json::from_slice(&bytes)?;
        let exact = exact_attributes(&bytes);
        let models = models_needed
            .then(|| -> crate::Result<EntityModelDocument> {
                match document {
                    Some(document) => Ok(document.clone()),
                    None => EntityModelDocument::load(root),
                }
            })
            .transpose()?;
        let entity_type_tags =
            crate::items::load_tags(root, "entity_type", &horizon_world::loot::PackOverlay::NONE)?;
        let empty_model = Arc::new(Part::parse("root", &Value::Null));
        let item_tags =
            crate::items::load_tags(root, "item", &horizon_world::loot::PackOverlay::NONE)
                .unwrap_or_default();
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
        // The Allay's InventoryCarrier loading needs item limits before a
        // Game exists. Other mob-only resource sets need no item data.
        let item_stack_limits: Arc<HashMap<String, u32>> = if types.get("minecraft:allay").is_some()
        {
            match item_defaults {
                Some(items) => Arc::new(
                    items
                        .iter()
                        .map(|(id, components)| {
                            (
                                id.clone(),
                                components["minecraft:max_stack_size"]
                                    .as_u64()
                                    .unwrap_or(64) as u32,
                            )
                        })
                        .collect(),
                ),
                None => {
                    let items: HashMap<String, Value> =
                        serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
                    Arc::new(
                        items
                            .into_iter()
                            .map(|(id, components)| {
                                (
                                    id,
                                    components["minecraft:max_stack_size"]
                                        .as_u64()
                                        .unwrap_or(64) as u32,
                                )
                            })
                            .collect(),
                    )
                }
            }
        } else {
            Arc::new(HashMap::new())
        };
        let mut out = HashMap::new();
        // A content mod's living type (its row names a vanilla `template`) is
        // its template's mob under its own id: the template's model, sounds
        // and behaviour, with the attributes its row has.
        let mod_types = types
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(id, row)| {
                let template = row["template"].as_str()?.strip_prefix("minecraft:")?;
                let def = species::ALL.iter().find(|d| d.name == template)?;
                Some((id.clone(), def))
            });
        let vanilla = species::ALL
            .iter()
            .map(|def| (format!("minecraft:{}", def.name), def));
        for (id, def) in vanilla.chain(mod_types) {
            let t = &types[&id];
            if t.is_null() {
                continue;
            }
            let model_key = format!("minecraft:{}", def.model);
            let model = models
                .as_ref()
                .and_then(|models| models.value().get(&model_key));
            if models_needed && model.is_none() {
                continue;
            }
            let defaults = exact.get(&id).cloned().unwrap_or_default();
            let fields = MobType::catalog_fields(&defaults, attributes);
            let mut layers = crate::fast_hash::FxHashMap::default();
            if let Some(all) = models.as_ref().and_then(|m| m.value().as_object()) {
                for prefix in def.layer_prefixes() {
                    let start = format!("minecraft:{prefix}#");
                    for (k, v) in all.iter().filter(|(k, _)| k.starts_with(&start)) {
                        let mut part = Part::parse("root", v);
                        if culls_back_faces(k) {
                            part.set_cull_back();
                        }
                        layers.insert(
                            k.trim_start_matches("minecraft:").to_owned(),
                            Arc::new(part),
                        );
                    }
                }
            }
            let category = t["category"].as_str().unwrap_or("creature").to_owned();
            let mut flags = def.flags(t["fire_immune"].as_bool().unwrap_or(false));
            flags.powder_snow_walkable = entity_type_tags
                .get("minecraft:powder_snow_walkable_mobs")
                .is_some_and(|members| members.contains(&id));
            let passenger_attachments =
                riders::parse_passenger_attachments(&t["passenger_attachments"]);
            out.insert(
                id.clone(),
                MobType {
                    shadow: crate::shadows::content_shadow(t),
                    species: def.species,
                    item_stack_limits: item_stack_limits.clone(),
                    attribute_bounds,
                    width: t["width"].as_f64().unwrap_or(0.6),
                    height: t["height"].as_f64().unwrap_or(1.8),
                    eye_height: t["eye_height"].as_f64().unwrap_or(1.6),
                    max_health: fields.max_health,
                    speed: fields.speed,
                    flying_speed: fields.flying_speed,
                    attack_damage: fields.attack_damage,
                    attack_knockback: fields.attack_knockback,
                    follow_range: fields.follow_range,
                    step_height: fields.step_height,
                    armor: fields.armor,
                    armor_toughness: fields.armor_toughness,
                    knockback_resistance: fields.knockback_resistance,
                    jump_strength: fields.jump_strength,
                    attributes: Arc::new(defaults.clone()),
                    gravity: fields.gravity,
                    safe_fall_distance: fields.safe_fall_distance,
                    fall_damage_multiplier: fields.fall_damage_multiplier,
                    water_efficiency: fields.water_efficiency,
                    water_slowdown: def.water_slowdown(),
                    air_speed: def.air_speed(),
                    baby_scale: def.baby_scale(),
                    head_rot_speed: def.head_rot_speed(),
                    max_head_x_rot: def.max_head_x_rot(),
                    max_head_y_rot: def.max_head_y_rot(),
                    ambient_interval: def.ambient_interval(),
                    flags,
                    model: model.map_or_else(
                        || empty_model.clone(),
                        |model| {
                            Arc::new({
                                let mut part = Part::parse("root", model);
                                if culls_back_faces(&model_key) {
                                    part.set_cull_back();
                                }
                                part
                            })
                        },
                    ),
                    layers,
                    texture: def.texture.to_owned(),
                    brain: def.brain(),
                    anim: def.anim,
                    sounds: def.sounds,
                    sound_overrides: entity_sounds::Sounds::load(&id, t)?,
                    food: def.food_tag().map_or_else(|| Arc::from(Vec::new()), tag),
                    tempt: def
                        .tempt_tag()
                        .or(def.food_tag())
                        .map_or_else(|| Arc::from(Vec::new()), tag),
                    passenger_attachments,
                    category,
                    id,
                },
            );
        }
        Ok(Self { types: out })
    }
}
pub use horizon_client_core::who::Who;
/// A native damage type's registry name and the tags consumed by mob combat.
///
/// Native code supplies a static name and flags matching its data-pack damage
/// tags; `NamedDamage::registered` builds both from a damage registry entry,
/// interning its name. Register the same name/data for player damage, death
/// messages and loot predicates. Unset tags are false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedDamage {
    pub name: &'static str,
    pub tags: u32,
}
impl NamedDamage {
    pub const IS_FIRE: u32 = 1 << 0;
    pub const BYPASSES_ARMOR: u32 = 1 << 1;
    pub const NO_KNOCKBACK: u32 = 1 << 2;
    pub const BYPASSES_INVULNERABILITY: u32 = 1 << 3;
    pub const BYPASSES_EFFECTS: u32 = 1 << 4;
    pub const BYPASSES_RESISTANCE: u32 = 1 << 5;
    pub const PANIC_CAUSES: u32 = 1 << 6;
    pub const IS_PROJECTILE: u32 = 1 << 7;
    pub const IS_EXPLOSION: u32 = 1 << 8;
    pub const IS_FALL: u32 = 1 << 9;
    pub const BYPASSES_ENCHANTMENTS: u32 = 1 << 10;

    /// Each flag with the damage tag it stands for.
    const TAG_FLAGS: [(u32, &'static str); 11] = [
        (Self::IS_FIRE, "minecraft:is_fire"),
        (Self::BYPASSES_ARMOR, "minecraft:bypasses_armor"),
        (Self::NO_KNOCKBACK, "minecraft:no_knockback"),
        (
            Self::BYPASSES_INVULNERABILITY,
            "minecraft:bypasses_invulnerability",
        ),
        (Self::BYPASSES_EFFECTS, "minecraft:bypasses_effects"),
        (Self::BYPASSES_RESISTANCE, "minecraft:bypasses_resistance"),
        (Self::PANIC_CAUSES, "minecraft:panic_causes"),
        (Self::IS_PROJECTILE, "minecraft:is_projectile"),
        (Self::IS_EXPLOSION, "minecraft:is_explosion"),
        (Self::IS_FALL, "minecraft:is_fall"),
        (
            Self::BYPASSES_ENCHANTMENTS,
            "minecraft:bypasses_enchantments",
        ),
    ];

    pub const fn new(name: &'static str, tags: u32) -> Self {
        Self { name, tags }
    }

    /// The registered damage type `name` with the tags `catalog` gives it,
    /// or None when the registry has no such type. The name is interned: the
    /// registries hold a handful of ids, so one copy of each is kept for the
    /// rest of the run.
    pub fn registered(catalog: &horizon_world::damage_source::Catalog, name: &str) -> Option<Self> {
        use std::{collections::HashSet, sync::Mutex};
        static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
        let id = catalog.id(name)?;
        let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let names = names.get_or_insert_with(HashSet::new);
        let name = match names.get(name) {
            Some(&found) => found,
            None => {
                let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
                names.insert(leaked);
                leaked
            }
        };
        let tags = Self::TAG_FLAGS
            .iter()
            .filter(|(_, tag)| catalog.in_tag(id, tag))
            .fold(0, |tags, (flag, _)| tags | flag);
        Some(Self { name, tags })
    }
    pub const fn has(self, tag: u32) -> bool {
        self.tags & tag != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageKind {
    /// A registered native damage type; vanilla kinds keep their existing tags.
    Named(NamedDamage),
    PlayerAttack,
    /// `mace_smash`: a mace hit after a fall (the player attack tags too).
    MaceSmash,
    /// `spear`: a stab of a spear (the player attack tags too).
    Spear,
    MobAttack,
    Arrow,
    Thrown,
    Fireball,
    Explosion,
    Fall,
    /// `fly_into_wall`: gliding into a horizontal collision.
    FlyIntoWall,
    FallingBlock,
    FallingStalactite,
    OnFire,
    InFire,
    /// `campfire`: standing in a lit campfire (`CampfireBlock.entityInside`).
    Campfire,
    Lava,
    Drown,
    /// `dry_out`: an axolotl out of water for too long.
    DryOut,
    Magic,
    /// `indirect_magic`: a thrown potion's hit (`DamageSources.indirectMagic`),
    /// which, unlike `magic`, knocks back.
    IndirectMagic,
    Sting,
    Cactus,
    /// `sweet_berry_bush`.
    SweetBerryBush,
    Generic,
    InWall,
    Thorns,
    SonicBoom,
    /// `freeze`: a fully frozen mob in powder snow.
    Freeze,
    /// `fell_out_of_world`: below the world's floor.
    OutOfWorld,
    /// `cramming`: crushed by the mobs around it.
    Cramming,
    /// `falling_anvil`: an anvil landing on it (it damages a helmet).
    FallingAnvil,
    /// `fireworks`: a firework rocket's burst (an explosion for Blast Protection).
    Fireworks,
    /// `generic_kill`: a kill that bypasses invulnerability.
    GenericKill,
    /// `lightning_bolt`.
    LightningBolt,
    /// `outside_border`: past the world border.
    OutsideBorder,
    /// `spit`: a llama's spit.
    Spit,
    /// `stalagmite`: a fall onto a stalagmite (a fall for Feather Falling).
    Stalagmite,
    /// `starve`: no food left.
    Starve,
    /// `hot_floor`: stepping on a magma block.
    HotFloor,
    /// `sulfur_cube_hot`: a sulfur cube's hot floor (a fire).
    SulfurCubeHot,
    /// `wind_charge`: a wind charge's burst (a projectile).
    WindCharge,
    /// `wither`: the wither effect.
    Wither,
    /// `wither_skull`: an owned wither skull's hit (`DamageSources.witherSkull`).
    WitherSkull,
}
impl DamageKind {
    /// DamageTypeTags.BYPASSES_INVULNERABILITY
    pub fn bypasses_invulnerability(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_INVULNERABILITY);
        }
        matches!(self, DamageKind::OutOfWorld | DamageKind::GenericKill)
    }
    /// DamageTypeTags.BYPASSES_EFFECTS
    pub fn bypasses_effects(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_EFFECTS);
        }
        matches!(self, DamageKind::Starve)
    }
    /// DamageTypeTags.BYPASSES_RESISTANCE
    pub fn bypasses_resistance(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_RESISTANCE);
        }
        matches!(self, DamageKind::OutOfWorld | DamageKind::GenericKill)
    }
    /// DamageTypeTags.BYPASSES_ENCHANTMENTS: `sonic_boom` alone in vanilla.
    pub fn bypasses_enchantments(self) -> bool {
        if let Self::Named(named) = self {
            return named.has(NamedDamage::BYPASSES_ENCHANTMENTS);
        }
        matches!(self, DamageKind::SonicBoom)
    }
}
/// `source.getEntity()` of a blow, as `SulfurCube.knockback` reads it: `getEyePosition()`,
/// `getLookAngle()` and `position()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub eye: [f64; 3],
    pub look: [f64; 3],
    pub feet: [f64; 3],
}
/// A damage source: its type, the attacking entity and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Damage {
    pub kind: DamageKind,
    pub attacker: Option<Who>,
    pub from: Option<[f64; 3]>,
    pub attacker_species: Option<Species>,
}
impl Damage {
    pub fn fall() -> Self {
        Self {
            kind: DamageKind::Fall,
            attacker: None,
            from: None,
            attacker_species: None,
        }
    }
    pub fn of(kind: DamageKind) -> Self {
        Self {
            kind,
            attacker: None,
            from: None,
            attacker_species: None,
        }
    }
    pub fn player(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::PlayerAttack,
            attacker: Some(Who::Player),
            from: Some(from),
            attacker_species: None,
        }
    }
    /// `DamageSources.mace`: a player's smash attack.
    pub fn mace(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MaceSmash,
            attacker: Some(Who::Player),
            from: Some(from),
            attacker_species: None,
        }
    }
    /// `DamageSources.spear`: a player's stab with a spear.
    pub fn spear(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::Spear,
            attacker: Some(Who::Player),
            from: Some(from),
            attacker_species: None,
        }
    }
    /// `DamageSources.indirectMagic(potion, owner)`: `attacker` is the owner
    /// (none for a dispenser's potion), `from` the potion's position, which
    /// the knockback pushes away from.
    pub fn indirect_magic(attacker: Option<Who>, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::IndirectMagic,
            attacker,
            from: Some(from),
            attacker_species: None,
        }
    }
    /// `DamageSources.fireworks(rocket, owner)`: `attacker` is the owner
    /// (none for a rocket nobody fired), `from` the rocket's position, which
    /// the knockback pushes away from (`calculateHorizontalHurtKnockbackDirection`).
    pub fn fireworks(attacker: Option<Who>, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::Fireworks,
            attacker,
            from: Some(from),
            attacker_species: None,
        }
    }
    pub fn mob(id: u32, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(id)),
            from: Some(from),
            attacker_species: None,
        }
    }
    pub fn with_species(mut self, species: Species) -> Self {
        self.attacker_species = Some(species);
        self
    }
    /// DamageTypeTags.PANIC_CAUSES
    pub fn causes_panic(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::PANIC_CAUSES);
        }
        !matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::Drown
                | DamageKind::DryOut
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::SweetBerryBush
                | DamageKind::OutOfWorld
                | DamageKind::FallingBlock
                | DamageKind::FallingAnvil
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Spit
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::Campfire
        )
    }
    /// DamageTypeTags.BYPASSES_ARMOR
    pub fn bypasses_armor(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::BYPASSES_ARMOR);
        }
        matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::FlyIntoWall
                | DamageKind::OnFire
                | DamageKind::Drown
                | DamageKind::Magic
                | DamageKind::IndirectMagic
                | DamageKind::InWall
                | DamageKind::Generic
                | DamageKind::SonicBoom
                | DamageKind::Freeze
                | DamageKind::OutOfWorld
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::Wither
        )
    }
    pub fn is_fire(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::IS_FIRE);
        }
        matches!(
            self.kind,
            DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Campfire
                | DamageKind::HotFloor
                | DamageKind::Lava
                | DamageKind::Fireball
                | DamageKind::SulfurCubeHot
        )
    }
    /// DamageTypeTags.NO_KNOCKBACK
    pub fn no_knockback(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::NO_KNOCKBACK);
        }
        matches!(
            self.kind,
            DamageKind::Explosion
                | DamageKind::Fall
                | DamageKind::FlyIntoWall
                | DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Campfire
                | DamageKind::HotFloor
                | DamageKind::Lava
                | DamageKind::Drown
                | DamageKind::DryOut
                | DamageKind::Magic
                | DamageKind::Cactus
                | DamageKind::SweetBerryBush
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::Freeze
                | DamageKind::OutOfWorld
                | DamageKind::LightningBolt
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::SulfurCubeHot
                | DamageKind::Wither
                | DamageKind::Spear
        )
    }
    pub fn is_projectile(&self) -> bool {
        if let DamageKind::Named(named) = self.kind {
            return named.has(NamedDamage::IS_PROJECTILE);
        }
        matches!(
            self.kind,
            DamageKind::Arrow
                | DamageKind::Thrown
                | DamageKind::Fireball
                | DamageKind::WindCharge
                | DamageKind::WitherSkull
        )
    }
}
/// Per-mob attribute base values (the type's defaults unless changed).
#[derive(Clone, Copy, Debug)]
pub struct Attrs {
    pub speed: f64,
    pub attack_damage: f32,
    /// `Attributes.ATTACK_KNOCKBACK`: what a melee hit adds to its push (half of it).
    pub attack_knockback: f64,
    pub max_health: f32,
    pub follow_range: f64,
    pub knockback_resistance: f64,
    pub spawn_reinforcements: f64,
    pub jump_strength: f64,
}
/// `minecraft:wolf_armor`'s maximum damage.
pub const WOLF_ARMOR_DURABILITY: i32 = 64;
/// Species-specific persistent state (variants, colours, taming, ageing...).
#[derive(Clone, Debug, Default)]
pub struct Extra {
    pub client_side: bool, // `Level.isClientSide`: a client copy, which `Entity.hurt` leaves alone.
    /// Saved fish/axolotl bucket flag; tadpoles save a constant true value.
    pub from_bucket: bool,
    /// Tags a mod gave the mob before it joined a level (`finalize_spawn`'s
    /// `UnspawnedMobEdits`): the level keeps a joined entity's tags with its
    /// saved data, so they move there when the mob joins.
    pub spawn_tags: Vec<String>,
    /// Command-created armor instances, shared by commands and runtime damage.
    pub armor_attributes: Option<Box<horizon_world::attributes::entity::EntityAttributes>>,
    /// `Mob.lootTable`: the table its death rolls instead of its type's
    /// (saved as `DeathLootTable`).
    pub death_loot_table: Option<String>,
    /// `Mob.lootTableSeed` (`DeathLootTableSeed`): 0 for none.
    pub death_table_seed: i64,
    /// `Rabbit.moreCarrotTicks`: how long it is full of carrots (saved as `MoreCarrotTicks`).
    pub more_carrot_ticks: i32,
    /// `Ocelot.DATA_TRUSTING`: it no longer runs from players.
    pub trusting: bool,
    /// `Mannequin.DATA_IMMOVABLE`: it neither moves nor falls (`isEffectiveAi` is false).
    pub immovable: bool,
    /// `Mannequin.DATA_DESCRIPTION`: the text under its name.
    pub description: MannequinDescription,
    /// `LivingEntity`'s `minecraft:scale` (`getScale`), which scales its box
    /// (`getDimensions`), eye height and model (`LivingEntityRenderState.scale`):
    /// the local player's own, and on a server every living entity's from its
    /// attribute updates; 0 is the default, 1 (`Mob::model_scale`).
    pub model_scale: f32,
    /// Players: crouching and riding (HumanoidModel's poses).
    pub crouching: bool,
    /// A player's or mannequin's synced pose (`Entity.DATA_POSE`), which
    /// sets its box (`Avatar.POSES`).
    pub avatar_pose: crate::pose::Pose,
    /// `Entity.isSprinting` (`Entity.DATA_SHARED_FLAGS_ID`'s flag 3): the dust a
    /// player raises; a cat or ocelot dashing (`Cat/Ocelot.customServerAiStep`),
    /// which `LivingEntity.setSprinting`'s speed modifier makes faster.
    pub sprinting: bool,
    pub passenger: bool,
    /// AgeableMob.age: negative while a baby (a tadpole counts up to 24000).
    pub age: i32,
    /// `AgeLocked` (tadpoles fed a slime ball... and ageable mobs).
    pub age_locked: bool,
    /// `AgeableMob.ageLockParticleTimer`: growth particles left, and the cooldown. Not saved.
    pub age_lock_particle_timer: i32,
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
    /// Villager or zombie-villager profession; no numeric mirror is stored.
    pub profession: villager::ProfessionState,
    pub variant3: u8,
    /// VillagerData.level for villagers and zombie villagers.
    pub villager_level: i32,
    /// `MushroomCow.stewEffects`: what a brown one took from a flower, given
    /// back in its next bowl of stew. `None` is vanilla's null.
    pub stew_effects: Option<Vec<(Arc<str>, i32)>>,
    /// Dye colour id (sheep wool, collars).
    pub color: u8,
    /// `Fox.DATA_TRUSTED_ID_0` and `_1`: the players a fox trusts, first free
    /// slot first, and a third replaces the second (`Fox.addTrustedEntity`).
    pub trusted: [Option<[i32; 4]>; 2],
    pub last_stare_sound: Option<u32>,
    /// `Animal.LoveCause`: the player who fed this animal into love, which a
    /// fox's breeding hands to its baby as trust (`Fox.FoxBreedGoal.breed`).
    pub love_cause: Option<[i32; 4]>,
    pub sheared: bool,
    /// `Mob.canPickUpLoot` (also lets dispensers equip it).
    pub pick_up_loot: bool,
    /// What the mob holds and wears, in each `EquipmentSlot` (`LivingEntity.equipment`):
    /// the hands, armour, body (horse and wolf armour, harnesses, carpets)
    /// and saddle, with their count, damage and components. The enchantments
    /// and trims of the hands and armour are overlaid from `gear`.
    pub equipment: horizon_items::equipment::Equipment,
    /// `Cat.RELAX_STATE_ONE`: watching a sleeping owner before lying down.
    pub relax_one: bool,
    /// `Cat.isLyingOnTopOfSleepingPlayer` (the client's own, from `Cat.handleLieDown`).
    pub on_sleeper: bool,
    pub chested: bool,
    /// The chest contents of a donkey or mule (15 slots).
    pub items: Vec<Option<crate::items::ItemStack>>,
    /// Occupied InventoryCarrier stacks: Piglin/trader (8 slots), Pillager (5).
    pub inventory: Vec<crate::items::ItemStack>,
    /// `AbstractHorse.temper` of every equine (horses, llamas, camels).
    /// Loaded signed values are independent of the gameplay modification cap.
    pub equine_temper: i32,
    pub tame: bool,
    /// `TamableAnimal.isOrderedToSit` (the saved order), and in `sitting_pose`
    /// `isInSittingPose` (bit 1 of `DATA_FLAGS_ID`, which only a sit goal writes).
    pub sitting: bool,
    pub sitting_pose: bool,
    /// `Parrot.partyParrot` and `jukebox`: the playing jukebox it dances to.
    pub party_jukebox: Option<[i32; 3]>,
    /// The owner's UUID: a tamed animal's, or the reference a vex was loaded with.
    pub owner: Option<[i32; 4]>,
    /// `Vindicator.isJohnny`: a custom name that was ever `Johnny`, which
    /// `setCustomName` latches and nothing clears.
    pub johnny: bool,
    /// Slime/magma cube size (0 = not sized).
    pub size: i32,
    /// Sheep eat-grass / iron golem attack / ravager animation ticks.
    pub anim_ticks: i32,
    pub egg_time: i32,
    pub anger: i32,
    /// `NeutralMob.getPersistentAngerEndTime`: the absolute game time the
    /// anger ends at, -1 when the mob is calm. The save keeps it, so the
    /// anger goes on while the chunk is unloaded.
    pub anger_end: i64,
    /// Countdown and age when `anger_end` was last settled (`neutral_anger`).
    pub anger_stamp: Option<(i32, u32)>,
    /// `NeutralMob.getPersistentAngerTarget`: the entity the mob is angry
    /// at, which the save keeps with it.
    pub angry_at: Option<Who>,
    /// `IronGolem.isPlayerCreated` (`PlayerCreated`): built from blocks.
    pub player_created: bool,
    pub carried: Option<horizon_world::registry::BlockStateId>,
    pub charged: bool,
    /// `Creeper.explosionRadius` (`ExplosionRadius`, a byte): the blast's
    /// radius, 3 unless the save or a command says otherwise; a powered
    /// creeper blows with twice it (`explodeCreeper`).
    pub explosion_radius: i8,
    pub adopted_body: Option<Arc<str>>,
    pub using_item: i32,
    pub using_off_hand: bool,
    /// `LivingEntity.recentKineticEnemies`: when the entities this mob's
    /// spear stabbed in the current use were stabbed (game time). It exists
    /// only while a kinetic weapon is in use.
    pub stabbed: Option<Vec<(Who, i64)>>,
    /// `ItemStack.getUseAnimation` of the item in use (`None` when nothing is);
    /// a player model's arm pose and `isBlocking` follow it.
    pub use_animation: horizon_core::use_animation::UseAnimation,
    /// The `arm_pose` the items in the main and off hand declare, if any: the pose
    /// a humanoid model's arm takes holding it, in place of vanilla's.
    pub held_arm_pose: [Option<horizon_core::arm_pose::ArmPose>; 2],
    pub fall_flying: bool,
    pub fall_fly_ticks: i32,
    /// `Entity.deltaMovementOnPreviousTick`: the velocity before this tick's
    /// move, which the elytra lean interpolates from (`getDeltaMovementLerped`).
    pub previous_velocity: Vec3,
    /// `Entity.DATA_POSE` of another player, as the server sent it.
    pub synced_pose: u8,
    /// `LivingEntity.isAutoSpinAttack` (Riptide).
    pub auto_spin_attack: bool,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID`: the
    /// arrows and bee stingers stuck in the body (`ArrowLayer`,
    /// `BeeStingerLayer`), with the timers that pull them out.
    pub stuck: stuck::Stuck,
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT`: the variant of the
    /// parrot on each shoulder (`ParrotOnShoulderLayer`).
    pub shoulder_parrots: [Option<i32>; 2],
    pub main_hand_left: bool,
    /// The skin layers a player hides (`PlayerModelPart` bits, set to
    /// `Avatar.DATA_PLAYER_MODE_CUSTOMISATION` inverted); 0 shows all.
    pub hidden_model_parts: u8,
    /// `AvatarRenderState.isSpectator`: a player in spectator mode shows only its head.
    pub spectator: bool,
    pub elytra: ElytraAnimation,
    pub swim: SwimAnimation,
    /// `ClientAvatarState` of a player: the cloak, bob and walked distance its cape swings with.
    pub avatar: crate::camera::AvatarBob,
    /// The arm the current swing moves (`currentSwing.hand().asArm`): `Some(true)`
    /// for the right arm.
    pub swing_right_arm: Option<bool>,
    /// The current swing is a STAB (`SwingAnimationType` 2): a spear's jab.
    pub swing_stab: bool,
    pub flag: bool,
    pub flag2: bool,
    /// `AbstractHorse`'s `Bred` save flag: nothing sets it at runtime (no
    /// vanilla code calls `setBred` outside `readAdditionalSaveData`), so only
    /// a save or `/summon` gives it, and `followMommy` reads it.
    pub bred: bool,
    /// `Hoglin`'s `CannotBeHunted` save flag: a bastion's own hoglins have it,
    /// so its piglins leave them alone (`Hoglin.canBeHunted`).
    pub cannot_be_hunted: bool,
    /// `AbstractHorse.allowStandSliding`: the rider's jump sets it, and a
    /// horse still rearing then goes on with the rider's keys until the rear
    /// ends (`tick` clears it).
    pub stand_sliding: bool,
    /// Species scratch values (the wither's head targets and clocks).
    pub ints: [i32; 6],
    pub counter: i32,
    /// Nautilus ATTACK_TARGET_COOLDOWN saved slot: None is absent,
    /// Some(MAX) is permanent, otherwise the remaining lifetime. The value is `counter`.
    pub nautilus_attack_cooldown_ttl: Option<i64>,
    pub counter2: i32,
    pub home: Option<[i32; 3]>,
    /// A copper golem's `openedChestPos`.
    pub opened_chest: Option<[i32; 3]>,
    pub point: Option<[f64; 3]>,
    /// `NEAREST_REPELLENT`: the nearest repelling block a hoglin or piglin
    /// sensed, looked up every 20 ticks.
    pub repellent: Option<[i32; 3]>,
    pub vector: [f64; 3],
    /// Active status effects: (effect id, ticks left, amplifier).
    pub effects: Vec<StoredEffect>,
    /// Hidden effect instances behind an active effect, keyed by effect id.
    pub effect_chains: Vec<crate::effects::EffectInstance>,
    /// The effects above given with `hideParticles`: no swirls for them.
    pub hidden_effects: Vec<&'static str>,
    /// The effects above that are ambient (`MobEffectInstance.isAmbient`: a beacon's), whose swirls
    /// are faint.
    pub ambient_effects: Vec<&'static str>,
    /// `LivingEntity.DATA_EFFECT_PARTICLES` and `DATA_EFFECT_AMBIENCE_ID` as a
    /// server gave them: the swirls around it, and whether they are all faint.
    pub effect_particles: Vec<crate::particles::ParticleOptions>,
    pub effect_ambient: bool,
    /// The swirls of `effects` as the native mods left them
    /// (`effect_particle_modification`), sent while one listens.
    pub modified_effect_particles: Option<crate::effects::ModifiedEffectParticles>,
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
    /// The UUID of the player behind `Holder::Player` (host or guest).
    pub leash_player: Option<[i32; 4]>,
    pub leash_spin: f64,
    /// More species scratch values.
    pub scratch: [i32; 4],
    /// Block positions a species remembers (a sniffer's explored ground).
    pub positions: Vec<[i32; 3]>,
    /// A warden's brain, anger and vibrations.
    pub warden: Option<Box<warden::Warden>>,
    /// An allay's vibration listener and liked note block.
    pub allay: Option<Box<allay::State>>,
    /// A raider's patrol and raid state.
    pub raider: Option<Box<raider::State>>,
    /// A villager's raid activity and gossip.
    pub villager: Option<Box<villager::Ext>>,
    /// The sniffer's and the ghastling's brain.
    pub animal_brain: Option<Box<animal_brain::Brain>>,
    pub axolotl: axolotl::Animation,
    /// The mob or boat this one rides, and the mob riding it (`getVehicle`,
    /// `getFirstPassenger`).
    pub mount: Option<u32>,
    pub rider: Option<u32>,
    /// This rider's place in its vehicle's `getPassengers`, beside `Guest.vehicle_order`: the
    /// lowest goes first. Passengers that load together keep 0 and their list order.
    pub vehicle_order: i64,
    /// Where `positionRider` last seated this rider: its own vehicle
    /// attachment (`getPassengerRidingPosition(this).y` is its feet plus this)
    /// and the vehicle's box (`Mob.getAttackBoundingBox`, `getHitbox`).
    pub seat: Option<(u32, f64, Aabb)>,
    /// The living vehicle `positionRider` last seated this rider on, with
    /// its `yBodyRotO` and `yBodyRot`: a rider of a living vehicle is drawn
    /// from that body (`LivingEntityRenderer.solveBodyRot`), not its own.
    pub vehicle_body: Option<(u32, f32, f32)>,
    /// `Entity.boardingCooldown`: ticks after leaving a boat before a boat
    /// can pick this mob up again.
    pub boarding_cooldown: i32,
    /// A chicken carrying a zombie (`IsChickenJockey`).
    pub jockey: bool,
    /// A skeleton horse waiting to spring its trap, and how long it has waited.
    pub trap: bool,
    pub trap_time: i32,
    /// Enchantments on the equipment.
    pub gear: Option<Box<gear::Gear>>,
    /// `Mob.dropChances` by `EquipmentSlot` (`drops::SLOTS` order) when set
    /// explicitly; None uses the defaults of `drops::drop_chance`.
    pub drop_chances: Option<Box<[f32; 8]>>,
    /// `Mob.leftHanded`.
    pub left_handed: bool,
    /// The sound variant: index in the species' `*_sound_variant` registry
    /// (`finalize::sound_variants`); None is the default, `classic`.
    pub sound_variant: Option<u8>,
    /// `Zombie.canBreakDoors`.
    pub can_break_doors: bool,
    /// A goat's `IsScreamingGoat` and the horns it lost (left, right).
    pub screaming: bool,
    pub lost_horns: [bool; 2],
    /// A goat's lowered head and ram behaviours.
    pub goat: goat::State,
    /// Attribute bases and permanent modifiers set by `finalizeSpawn`.
    pub attributes: Option<Box<finalize::Attributes>>,
    /// Brain memories `finalizeSpawn` sets: (memory, value, ticks to expiry).
    pub memories: Vec<(&'static str, i64, i64)>,
    /// A camel's `LAST_POSE_CHANGE_TICK`, and its game time minus age at the last tick.
    pub pose_change_tick: i64,
    pub pose_epoch: Option<i64>,
    /// Whether a camel sits (`egg_time` is then the age it sat down at).
    pub camel_sitting: bool,
    /// `Entity.DATA_POSE` as a mob of a server has it (`sync::pose_id`).
    pub pose: u8,
    /// The school leader a fish follows, and a tropical fish that swims alone.
    pub leader: Option<u32>,
    pub lone: bool,
    /// The fish that follow this one (`AbstractSchoolingFish.schoolSize` - 1).
    pub followers: i32,
    /// The entity section it is in and when it entered it (a global sequence):
    /// vanilla's entity queries list the entities of a section in the order they
    /// entered it, the sections by x, z, y.
    pub section: [i32; 3],
    pub section_seq: u64,
    /// A breeze's or creaking's brain (`monster_brain`).
    pub brain: Option<Box<monster_brain::Brain>>,
    /// A bee's nectar, anger, flower and hive state.
    pub bee: Option<Box<bee::Bee>>,
    /// An armadillo's state and scare brain.
    pub armadillo: armadillo::State,
    /// A frog's brain memories and tongue target.
    pub frog: frog::State,
    /// A panda's flags, counters and amounts.
    pub panda: Option<Box<panda::State>>,
    /// What a piglin's or hoglin's brain remembers: its avoid target, the
    /// adults its sensor sees and the activity it is in.
    pub piglin: piglin::Memories,
    /// What the native mods last heard it hold and wear
    /// (`LivingEntity.lastEquipmentItems`, empty at first), by
    /// `EquipmentSlot` ordinal: see `Mob::equipment_stacks`.
    pub equipment_heard: Option<Box<horizon_items::equipment::Equipment>>,
    /// The live attribute map (`AttributeMap`), built when an item with
    /// attribute modifiers is worn or it is read. Mob logic still reads
    /// `MobType::attributes` and `attributes` above.
    pub live: crate::living_attributes::LivingAttributes,
    /// The knockback resistance modifiers of the items it wears or holds.
    pub gear_knockback: Option<Box<GearKnockback>>,
    /// What the game last read from the hands' `minecraft:death_protection`
    /// components (`game/totem.rs`); `None` until it has, when the totem's id
    /// stands in for the component.
    pub hand_protection: Option<HandProtection>,
}
/// The knockback resistance modifiers of the items a mob wears or holds, with
/// the stacks they were read from: the game reads them again only when one
/// of those stacks changes (`ItemStack.matches`).
#[derive(Clone, Debug, Default)]
pub struct GearKnockback {
    /// The hand and armor stacks, mainhand to head.
    pub worn: [Option<crate::items::ItemStack>; 6],
    pub modifiers: Vec<(f64, finalize::Op)>,
}
/// The `death_effects` of the `death_protection` component each hand's stack
/// carries; `None` for a hand without the component.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandProtection {
    pub main: Option<Vec<crate::items::ConsumeEffect>>,
    pub off: Option<Vec<crate::items::ConsumeEffect>>,
}
impl Extra {
    /// The body item id (`getItemBySlot(BODY)`).
    pub fn body_id(&self) -> Option<&str> {
        self.body_stack().map(|s| &*s.item)
    }
    /// The body stack, with its count, damage and components.
    pub fn body_stack(&self) -> Option<&crate::items::ItemStack> {
        self.equipment.get(horizon_items::equipment::BODY)
    }
    pub fn set_body_stack(&mut self, stack: Option<crate::items::ItemStack>) {
        self.equipment.set(horizon_items::equipment::BODY, stack);
    }
    /// Wears a plain body item of this id.
    pub fn set_body(&mut self, item: Option<Arc<str>>) {
        self.set_body_stack(item.map(|id| crate::items::ItemStack::new(&id, 1)));
    }
    /// The durability the body armour has lost.
    pub fn body_damage(&self) -> i32 {
        self.body_stack().map_or(0, |s| s.damage as i32)
    }
    pub fn set_body_damage(&mut self, damage: i32) {
        if let Some(stack) = &mut self.equipment[horizon_items::equipment::BODY] {
            stack.damage = damage.max(0) as u32;
        }
    }
    /// `Mob.isSaddled`.
    pub fn saddled(&self) -> bool {
        self.equipment
            .get(horizon_items::equipment::SADDLE)
            .is_some()
    }
    /// Wears or takes off a plain saddle.
    pub fn set_saddled(&mut self, saddled: bool) {
        let saddle = saddled.then(|| crate::items::ItemStack::new("minecraft:saddle", 1));
        self.equipment.set(horizon_items::equipment::SADDLE, saddle);
    }

    /// The id of the item in `slot` (`EquipmentSlot` ordinal).
    pub fn slot_id(&self, slot: usize) -> Option<&str> {
        self.equipment.get(slot).map(|s| &*s.item)
    }
    /// The stack in `slot`, as it was kept (the `gear` enchantments and trim
    /// are not on it: `finalize::item_stack` overlays them).
    pub fn slot_stack(&self, slot: usize) -> Option<&crate::items::ItemStack> {
        self.equipment.get(slot)
    }
    pub fn set_slot_stack(&mut self, slot: usize, stack: Option<crate::items::ItemStack>) {
        self.equipment.set(slot, stack);
    }
    /// Puts one plain item of this id in `slot`.
    pub fn set_slot_id(&mut self, slot: usize, item: Option<Arc<str>>) {
        self.set_slot_stack(slot, item.map(|id| crate::items::ItemStack::new(&id, 1)));
    }
    pub fn main_hand_id(&self) -> Option<&str> {
        self.slot_id(horizon_items::equipment::MAINHAND)
    }
    pub fn off_hand_id(&self) -> Option<&str> {
        self.slot_id(horizon_items::equipment::OFFHAND)
    }
    pub fn head_id(&self) -> Option<&str> {
        self.slot_id(horizon_items::equipment::HEAD)
    }
    pub fn set_main_hand(&mut self, item: Option<Arc<str>>) {
        self.set_slot_id(horizon_items::equipment::MAINHAND, item);
    }
    pub fn set_main_hand_stack(&mut self, stack: Option<crate::items::ItemStack>) {
        self.set_slot_stack(horizon_items::equipment::MAINHAND, stack);
    }
    pub fn main_hand_stack(&self) -> Option<&crate::items::ItemStack> {
        self.slot_stack(horizon_items::equipment::MAINHAND)
    }
    /// `LivingEntity.swapHandItems`: the two hands trade their whole stacks,
    /// with the enchantments and trims `gear` keeps for them.
    pub fn swap_hands(&mut self) {
        use horizon_items::equipment::{MAINHAND, OFFHAND};
        let (main, off) = (
            self.equipment[MAINHAND].take(),
            self.equipment[OFFHAND].take(),
        );
        self.equipment[MAINHAND] = off;
        self.equipment[OFFHAND] = main;
        if let Some(g) = self.gear.as_deref_mut() {
            g.slots.swap(gear::MAIN, gear::OFF);
            g.trims.swap(gear::MAIN, gear::OFF);
        }
    }
    /// Empties `slot` and returns the id of what it held.
    pub fn take_slot_id(&mut self, slot: usize) -> Option<Arc<str>> {
        self.equipment[slot].take().map(|s| s.item)
    }
}
/// How far the boxes of a set of mobs reach from their positions: the widest reach sideways, below
/// and above among them, worked out from each mob's real box ([`Mob::bounds`]: a shulker's grows
/// toward its open lid, sideways or below its position). A box query over many mobs (a vehicle's,
/// against every mob of the level) tests each mob's position against this first, and works out the
/// box only for the mobs whose position comes close.
#[derive(Clone, Copy, Debug, Default)]
pub struct BoxReach {
    half_width: f64,
    below: f64,
    height: f64,
}
impl BoxReach {
    pub fn of(mobs: &[Mob]) -> Self {
        mobs.iter().fold(Self::default(), |r, m| {
            let (b, p) = (m.bounds(), m.position);
            let (lo, hi) = (b.min_position(), b.max_position());
            Self {
                half_width: r
                    .half_width
                    .max(p.x - lo.x)
                    .max(hi.x - p.x)
                    .max(p.z - lo.z)
                    .max(hi.z - p.z),
                below: r.below.max(p.y - lo.y),
                height: r.height.max(hi.y - p.y),
            }
        })
    }
    /// Whether the box of `m`, one of the mobs this was worked out for, can meet `area`: a
    /// superset, the caller still tests the box.
    #[inline]
    pub fn may_meet(self, m: &Mob, area: Aabb) -> bool {
        let (lo, hi, p) = (area.min_position(), area.max_position(), m.position);
        p.x - self.half_width <= hi.x
            && p.x + self.half_width >= lo.x
            && p.z - self.half_width <= hi.z
            && p.z + self.half_width >= lo.z
            && p.y - self.below <= hi.y
            && p.y + self.height >= lo.y
    }
}

#[derive(Clone, Debug)]
pub struct Mob {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list ([`crate::join_seq`]); set by the list, not by the spawn site.
    pub join: crate::join_seq::JoinSeq,
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
    /// `Entity.needsSync`: knocked back, pushed or jumped since its server
    /// last sent it, so the next update goes out at once.
    pub needs_sync: bool,
    /// The mobs it pushed this turn, kept while the collision cap is on (`push::Cap`).
    pub pushed: Vec<u32>,
    /// `Entity.syncPosition`: it bounced this tick (`restituteMovementAfterCollisions`),
    /// so its tracker keeps this position as a step of the next move it sends.
    pub sync_position: bool,
    /// `getAttributeValue(KNOCKBACK_RESISTANCE)` with the modifiers of the
    /// items it wears or holds (`Extra::gear_knockback`), which the game
    /// works out each tick for a mob that has any (the mob itself knows no
    /// item): `None` for the rest.
    pub gear_knockback_resistance: Option<f64>,
    pub health: f32,
    pub hurt_time: i32,
    /// `LivingEntity.damageCooldownTime`: 20 after a hit, counting down;
    /// while it is above 10 a weaker hit does only the difference.
    pub invulnerable: i32,
    /// `Entity.invulnerableTime` (#916): the entity-level invulnerability
    /// window, its own field — never the damage cooldown beside it
    /// (`Entity.isTemporarilyInvulnerable`), decremented by the base tick.
    pub invulnerable_time: i32,
    pub death_time: i32,
    /// `LivingEntity.currentImpulseContextResetGraceTime`: ticks left to keep
    /// the impulse context after a wind charge or mace impulse.
    pub impulse_grace_time: i32,
    pub age: u32,
    pub limb_position: f32,
    pub limb_speed: f32,
    pub previous_limb_speed: f32,
    pub swell: i32,
    pub previous_swell: i32,
    /// Creeper.maxSwell (`Fuse`): local NBT state, not synced entity data.
    pub max_swell: i32,
    pub ambient_timer: i32,
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: the next move is scaled by it.
    pub stuck_multiplier: Vec3,
    /// `Entity.pistonDeltas` and `pistonDeltasGameTime`: how far a moving
    /// block has pushed this mob this tick (0.51 at most on any axis).
    pub piston: horizon_world::piston_movement::PistonMovement,
    pub persistent: bool,
    /// `CustomName` (plain text) and `CustomNameVisible`.
    pub custom_name: Option<String>,
    /// The styled `CustomName`, when it came from a component.
    pub custom_name_text: Option<crate::text::Text>,
    pub name_visible: bool,
    /// `Entity.DATA_SHARED_FLAGS_ID` bit 0x20: the model is hidden.
    pub invisible: bool,
    /// Effects changed and need a shared flag update on the next tick.
    pub effects_dirty: bool,
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
    /// `Entity.noPhysics`: its moves pass through blocks. No save writes it
    /// (`Entity.saveWithoutId` omits it) and no mob tick rewrites it except a
    /// vex, which sets it only for the length of its own tick.
    pub no_physics: bool,
    /// `Entity.mainSupportingBlockPos`: the block it stood on after its last
    /// move.
    pub supporting: Option<[i32; 3]>,
    /// The block under the mob's feet by name (`minecraft:grass_block`), kept for the
    /// packs' random-model rules (`rule_index`); `None` without rules or off the ground.
    pub standing_on: Option<String>,
    /// `NoAI`: no goals and no movement.
    pub no_ai: bool,
    /// `Silent`.
    pub silent: bool,
    /// `Invulnerable`: immune to damage (but not to `/kill`).
    pub invulnerable_flag: bool,
    /// The entity that causes the blow being dealt (`source.getEntity()`): its eye, view
    /// direction and feet, which `SulfurCube.knockback` turns the launch by. Set by whoever deals
    /// the blow (`Game::attacker_aim`) and cleared once it is dealt.
    pub attacker_aim: Option<Aim>,
    /// The horizontal motion `(x, z)` of the projectile that is the direct entity of the blow
    /// being dealt (`Projectile.calculateHorizontalHurtKnockbackDirection`): the knock goes the
    /// way it was travelling. None for any other direct entity.
    pub projectile_motion: Option<[f64; 2]>,
    pub climbing: bool,
    /// `LivingEntity.onClimbable` as of the last mob tick, for `isPushable`
    /// checks made outside the mob world (minecarts, boats).
    pub on_climbable: bool,
    pub fire_ticks: i32,
    pub ticks_frozen: i32,
    /// `Entity.isInPowderSnow` as of the last mob tick, for the skeleton's
    /// freezing tracker read at save time.
    pub in_powder_snow: bool,
    pub glowing: bool,
    pub air: i32,
    pub last_hurt: f32,
    /// `LivingEntity.getAbsorptionAmount`: golden hearts that soak damage before health.
    pub absorption: f32,
    /// Tenths of absorbed damage a player's hit dealt, which the attack awards as
    /// `Stats.DAMAGE_DEALT_ABSORBED` to the player whose turn it is (see [`Mob::soak_hit`]).
    pub dealt_absorbed: i32,
    pub last_damage: Option<(Damage, i64)>,
    pub accepted_hit: Option<std::sync::Arc<crate::health::AcceptedHit>>,
    pub death_canceled: bool,
    /// `DamageSource.getDirectEntity` of `last_damage` when [`Damage`] cannot name it (a
    /// thrown potion, an area effect cloud): its entity type.
    pub last_direct: Option<&'static str>,
    /// `LivingEntity.combatTracker`: the hits taken lately.
    pub combat: combat::Tracker,
    /// Set by `remove`: the mob disappears without dying (conversion, despawn, pickup).
    pub removed: bool,
    /// What `mob_despawn` answered for this tick, read and reset by `check_despawn`.
    pub despawn: mod_api::Despawn,
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
    /// The skin texture of a player mob (`PlayerInfo.getSkin`), which
    /// replaces the type's default when its download is ready.
    pub skin: Option<String>,
    /// `PlayerSkin.model` is `SLIM`: the player model with 3-pixel arms.
    pub slim: bool,
    /// The 3D skin layers feature's model for this player: its outer layer
    /// with every painted texel extruded (`skin_layers`, off by default).
    pub skin_3d: Option<Arc<Part>>,
    /// `PlayerSkin.cape`: a player's cape texture, once every download of
    /// its skin is ready.
    pub cape: Option<String>,
    /// A pose laid over this player's body after the vanilla animation (`anim::setup`), timed on
    /// `age`: a Horizon emote (docs/cosmetics.md), given by the game. Visual only.
    pub emote: Option<Arc<dyn anim::BodyPose>>,
    /// A mannequin's `DATA_PROFILE` (ResolvableProfile): the profile's
    /// `textures` property resolves to `skin` like a player's.
    pub profile: Option<horizon_protocol::entity_data::Profile>,
    /// The packs' random texture variants it wears (`random_textures::choose`).
    pub random_textures: random_textures::Choice,
    /// What `entity_size` answered for its pose and vanilla size (`EntityEvent.Size`):
    /// `dims` uses it while it holds.
    pub size_answer: Option<crate::pose::SizeAnswer>,
}
/// A mannequin's `description` and `hide_description` (`Mannequin.updateDescription`):
/// the default `entity.minecraft.mannequin.label`, none at all, or a component.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum MannequinDescription {
    #[default]
    Default,
    Hidden,
    Custom(horizon_core::nbt::Tag),
}
impl MannequinDescription {
    /// `Mannequin.getDescription`: the component `DATA_DESCRIPTION` holds, none when hidden.
    pub fn component(&self) -> Option<horizon_core::nbt::Tag> {
        match self {
            Self::Default => Some(horizon_core::nbt::Tag::compound([(
                "translate",
                horizon_core::nbt::Tag::String("entity.minecraft.mannequin.label".into()),
            )])),
            Self::Hidden => None,
            Self::Custom(tag) => Some(tag.clone()),
        }
    }
    /// `DATA_DESCRIPTION` as a client reads it.
    pub fn from_component(tag: Option<&horizon_core::nbt::Tag>) -> Self {
        match tag {
            None => Self::Hidden,
            Some(tag) if Some(tag) == Self::Default.component().as_ref() => Self::Default,
            Some(tag) => Self::Custom(tag.clone()),
        }
    }
}
/// Whether breeding experience was drawn in the parent's tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreedExperience {
    Deferred,
    Drawn(Option<u32>),
}

/// Makes the events pushed since `start` (the effects of a break a mod may stop)
/// conditional on the game having broken one of `mob`'s blocks. Unless a listener
/// can cancel the break (`MobWorld::asks_living_destroy_block`, passed as `asks`)
/// they stay plain events: nothing is allocated.
pub fn after_break(events: &mut Vec<MobEvent>, asks: bool, mob: u32, start: usize) {
    if asks {
        let effects = events.split_off(start);
        events.push(MobEvent::IfBroke {
            mob,
            effects,
            otherwise: Vec::new(),
        });
    }
}

/// Events a mob tick asks the game to act on.
#[derive(Clone, Debug)]
pub enum MobEvent {
    /// A player-directed event remembers the player selected at emission.
    /// `None` is the server's own player; `Some` is a guest connection.
    ForPlayer {
        conn: Option<u32>,
        event: Box<MobEvent>,
    },
    /// Squid.handleEntityEvent(19), resetting the remote tentacle cycle.
    SquidCycle {
        mob: u32,
        at: [f64; 3],
    },
    /// Rabbit entity event 1, sent only when a ground jump actually happens.
    RabbitHop {
        mob: u32,
        at: [f64; 3],
    },
    /// `LivingEntity.baseTick`'s drowning: entity event 67 for 8 `bubbles`, else that many particles.
    Drown {
        mob: u32,
        at: [f64; 3],
        bubbles: i32,
    },
    /// PowderSnowBlock.entityInside's snowflake with vanilla's sampled velocity.
    PowderSnowParticle {
        at: [f64; 3],
        velocity: [f64; 3],
    },
    /// `Mob.burnUndead` in daylight on a mob wearing something in its
    /// `sunProtectionSlot`: the item loses up to 1 durability (the game knows
    /// the item data) instead of the mob catching fire.
    SunProtectionWear {
        mob: u32,
    },
    /// `Level.broadcastEntityEvent` for a mob: guests run its
    /// `handleEntityEvent`.
    EntityEvent {
        mob: u32,
        event: i8,
        at: [f64; 3],
    },
    /// `ServerLevel.sendParticles`: drawn here and sent to the players within
    /// 32 blocks, which spread and speed the way the client does for a count
    /// (a count of 0 is one particle moving by speed times spread).
    LevelParticles {
        kind: &'static str,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
    },
    /// `ServerLevel.sendParticles` of an `ItemParticleOption`: `count` crumbs of
    /// `item`, drawn in this window.
    ItemParticles {
        item: &'static str,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
    },
    Sound(&'static str, [f64; 3], f32, f32),
    ProfessionWorkSound(Arc<str>, [f64; 3], f32, f32),
    /// `level.playSound(null, mob, sound, source, volume, pitch)`: a sound
    /// bound to mob `mob` (`ServerLevel.playSeededSound`'s entity overload),
    /// in vanilla's `source`, from where the mob stands.
    BoundSound {
        mob: u32,
        sound: &'static str,
        source: mod_api::SoundSource,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    },
    /// `Entity.sendBubbleColumnParticles`: a mob at the top of the bubble column
    /// block at `pos` (the splash and bubble particles over it).
    BubbleColumnParticles {
        pos: [i32; 3],
    },
    /// A `BlockParticleOption` of `state` drawn `count` times at `at`, each crumb
    /// leaving at `speed`: the dust `LivingEntity.checkFallDamage` throws up from
    /// the block a mob landed on, and the puffs `Breeze.emitGroundParticles`
    /// leaves at a breeze's feet (which it draws at rest).
    LandingDust {
        state: horizon_world::registry::BlockStateId,
        at: [f64; 3],
        count: i32,
        speed: f64,
    },
    /// `UseBonemeal`: villager `mob`'s bone meal on the crop at `pos`
    /// (`BoneMealItem.growCrop`, then level event 1505 when it grew).
    Bonemeal {
        mob: u32,
        pos: [i32; 3],
    },
    /// `RingBell`: the villager rings the meeting-point bell
    /// (`BellBlock.attemptToRing`: the bell shakes, the sound and the game
    /// event follow).
    RingBell {
        bell: [i32; 3],
    },
    /// `ChestBlockEntity.startOpen` / `stopOpen` by a copper golem: the
    /// chest's opener count goes up or down (interaction range 3).
    ChestOpen {
        chest: [i32; 3],
        open: bool,
        /// The golem, the source of the container's game event.
        mob: u32,
    },
    /// `BeeEnterHiveGoal.start`: the bee goes into its hive (the host stores
    /// its tag with `BeehiveBlockEntity.addOccupant` and discards it).
    EnterHive {
        mob: u32,
        hive: [i32; 3],
    },
    /// `LandOnOwnersShoulderGoal`: a tame parrot touched its owner and sits
    /// on their shoulder (the host stores its tag and discards it).
    ParrotToShoulder {
        mob: u32,
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
    /// `InfestedMobEffect.onMobHurt`: the mob was hurt while infested and its
    /// draws (`infested_silverfish`) spawn silverfish at `at`, half its height
    /// up where it was hurt, each thrown with its velocity.
    Infested {
        at: [f64; 3],
        velocities: Vec<Vec3>,
    },
    /// `LivingEntity.hurtArmor` (`doHurtEquipment`): a hit that landed on a mob
    /// wearing body armour (a horse's) leaves the piece owing
    /// `max(1, damage / 4)` durability. The host applies it: it is the one
    /// that knows the item's maximum, whether it wears when hurt at all and
    /// whether a fire hit leaves it alone (`ItemStack.canBeHurtBy`).
    ArmorHurt {
        mob: u32,
        amount: f32,
        source: crate::health::HurtSource<'static>,
    },
    /// `LivingEntity.updateFallFlying`: every 20 glide ticks the glider in
    /// `slot` (the one `Util.getRandom` picked) takes one point of wear,
    /// `ItemStack.hurtAndBreak(1, this, slot)`. The host applies it with the
    /// item's maximum and Unbreaking.
    GliderWear {
        mob: u32,
        slot: horizon_core::equipment_slot::EquipmentSlot,
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
    /// `AbstractSchoolingFish.startFollowing` on a fish a leader recruits:
    /// it follows the leader, whose school grows by one.
    StartFollowing {
        fish: u32,
        leader: u32,
    },
    /// `addFollower` / `removeFollower`: a leader's school grows or shrinks.
    SchoolSize {
        leader: u32,
        by: i32,
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
    /// `Level.gameEvent` from the block a mob landed on (`FarmlandBlock.turnToBaseBlock`'s
    /// BLOCK_CHANGE from the mob that trampled it, `TurtleEggBlock.decreaseEggs`' and
    /// `Level.destroyBlock`'s BLOCK_DESTROY with no source): the game posts it, a
    /// sculk sensor, a calibrated sensor and a warden hear it.
    LandingGameEvent {
        event: &'static str,
        at: [i32; 3],
        /// The entity that caused the change, `None` for a destroyed egg.
        source: Option<u32>,
        state: horizon_world::registry::BlockStateId,
    },
    AttackPlayer(f32, [f64; 3]),
    /// `Bee.doHurtTarget` on a player: a sting that lands leaves its stinger
    /// in them and poisons them for `poison_ticks` (none on easy).
    StingPlayer {
        amount: f32,
        from: [f64; 3],
        poison_ticks: i32,
    },
    /// A melee hit whose effect comes only if the hit lands (husk hunger,
    /// cave spider poison, wither skeleton wither, bee and pufferfish
    /// poison): a shield-blocked hit gives nothing.
    AttackPlayerWithEffect {
        amount: f32,
        from: [f64; 3],
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    },
    /// Damage of a given type to the player (guardian lasers, thorns).
    DamagePlayer {
        amount: f32,
        from: [f64; 3],
        kind: &'static str,
    },
    /// `ElderGuardian`'s curse on players within 50 blocks; a silent elder's
    /// shows the apparition without the curse sound.
    ElderCurse {
        at: [f64; 3],
        silent: bool,
    },
    /// `Entity.setDeltaMovement`: replace a target's velocity.
    SetVelocity {
        target: Who,
        velocity: [f64; 3],
    },
    /// `Entity.remove(RemovalReason.KILLED)` of a mob that is no longer alive: it vanishes at once,
    /// with no death animation (`ShootTongue.eatEntity`).
    RemoveKilled {
        target: u32,
    },
    /// `Entity.push`: add to a target's velocity.
    Push {
        target: Who,
        velocity: [f64; 3],
    },
    /// `SonicBoom`: 10 damage to the target, then a push along `direction`
    /// (the unit vector from the warden's chest to the target's eyes) only
    /// if the hit landed, less the target's knockback resistance.
    SonicBoom {
        target: Who,
        attacker: u32,
        from: [f64; 3],
        direction: [f64; 3],
    },
    /// `Shulker.onPeekAmountChange`: the opening lid of `shulker` moves every
    /// entity it may push whose box meets `area` (the box the lid swept) by
    /// `Entity.move(MoverType.SHULKER, delta)`.
    LidPush {
        shulker: u32,
        area: Aabb,
        delta: [f64; 3],
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
    /// The mob the `Explode` right after this comes from
    /// (`ServerExplosion.source`), and whether it is a charged creeper: the
    /// exploding creeper is already out of the mob list when the game applies
    /// the blast, so its victims' loot cannot look it up, nor the death
    /// message its name (`name`, `getDisplayName`).
    ExplosionSource {
        mob: u32,
        charged: bool,
        name: crate::text::Text,
        /// `Entity.isInWater` of the source: `BlockAttached.ignoreExplosion`.
        in_water: bool,
        /// Its entity type: a player the blast kills is still credited to it
        /// (`getKillCredit` finds the source during its own explosion).
        kind: String,
    },
    /// The centre, the radius and the kind of the mob blowing up (it may be
    /// gone from the list by the time the blast is handled), and its `Silent`.
    Explode([f64; 3], f32, Species, bool),
    /// `Creeper.spawnLingeringCloud`, right after its `Explode`: an `AreaEffectCloud` at the
    /// creeper with its active effects `(id, remaining ticks, amplifier)`, in their mixed colour.
    LingeringCloud {
        at: [f64; 3],
        effects: Vec<(&'static str, i32, i32)>,
        /// The effects among them added hidden (`hideParticles`).
        hidden: Vec<&'static str>,
        /// The effects among them that are ambient.
        ambient: Vec<&'static str>,
        color: u32,
    },
    /// Damage that bypasses `EnchantmentHelper.doPostAttackEffects` (roars,
    /// beams, retaliation and hot-cube contact).
    DamageMob {
        target: u32,
        amount: f32,
        source: Damage,
    },
    /// An attack that runs post-attack effects when the hit lands.
    AttackMob {
        target: u32,
        amount: f32,
        source: Damage,
    },
    /// A melee hit on another mob whose effect rides the hit:
    /// `CaveSpider`, `Husk` and `WitherSkeleton.doHurtTarget` add it inside
    /// the `super.doHurtTarget` branch, to any `LivingEntity`, so a mob the
    /// hit lands on gets it too.
    AttackMobWithEffect {
        target: u32,
        amount: f32,
        source: Damage,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    },
    /// `AbstractCubeMob.dealDamage` on the player (`playerTouch`): the hit,
    /// then, only when `hurtServer` let it land, the cube's
    /// `entity.slime.attack` with its pitch drawn from the cube's random.
    CubeHurtPlayer {
        cube: u32,
        amount: f32,
        from: [f64; 3],
    },
    /// `AbstractCubeMob.dealDamage` on a mob (an iron golem it touches): the
    /// hit, then, only when `hurtServer` let it land, the cube's
    /// `entity.slime.attack` with its pitch drawn from the cube's random.
    CubeHurtMob {
        cube: u32,
        target: u32,
        amount: f32,
        source: Damage,
    },
    /// `Pufferfish.touch` on a mob (`Pufferfish.aiStep`): the hit, then, only
    /// when `hurtServer` let it land, the fish's `entity.puffer_fish.sting`
    /// and the poison it gives (`poison_ticks`, `60 * getPuffState()`).
    PufferfishSting {
        mob: u32,
        target: u32,
        amount: f32,
        source: Damage,
        poison_ticks: i32,
    },
    /// `LivingEntity.stabAttack`'s `target.stopRiding()`: a spear knocks the
    /// target off its mount.
    StabDismount {
        target: Who,
    },
    /// `LivingEntity.knockback(power, x, z, source)`; a ram's blocked again first (`RamTarget.tick`).
    Knockback {
        target: Who,
        power: f64,
        x: f64,
        z: f64,
        source: Damage,
        apply_blocking: Option<f32>,
    },
    /// Set another mob's attack target (HurtByTargetGoal alerts).
    SetTarget {
        mob: u32,
        target: Who,
    },
    PiglinAlertTarget(undead::PiglinAlert),
    /// `HoglinAi.broadcastAttackTarget`: receiver and source player connection.
    HoglinAttackTarget {
        mob: u32,
        target: Who,
        player: Option<u32>,
    },
    /// `StartHuntingHoglin` for a nearby adult piglin: it takes the hoglin as
    /// its anger target (when it has none) and starts its `HUNTED_RECENTLY` wait.
    HuntHoglin {
        mob: u32,
        target: u32,
        ticks: i32,
    },
    /// `HoglinAi.broadcastRetreat` and `PiglinAi.broadcastRetreat` for one
    /// adult that sees the broadcaster: it retreats from `from`, standing at
    /// `at`, or from what it already avoids or attacks if that is nearer.
    Retreat {
        mob: u32,
        from: Who,
        at: [f64; 3],
    },
    /// `Entity.startRiding` from a mob's brain (`Mount.create`): `rider` boards
    /// `vehicle` if `Piglin.startRiding` and `canRide` allow it.
    StartRiding {
        rider: u32,
        vehicle: u32,
    },
    Projectile(crate::projectiles::Projectile),
    /// An arrow `mob` shot with `weapon` (`AbstractSkeleton.performRangedAttack`):
    /// the game asks for the ammunition (`getProjectile`) before it flies.
    BowShot {
        mob: u32,
        weapon: crate::items::ItemStack,
        arrow: crate::projectiles::Projectile,
    },
    /// `mob` finished charging `weapon` (`CrossbowItem.onUseTick`): the game
    /// asks for the ammunition (`getProjectile`), and plays the end sound
    /// at `at` only when the crossbow loaded.
    CrossbowLoad {
        mob: u32,
        weapon: crate::items::ItemStack,
        at: [f64; 3],
        pitch: f32,
    },
    /// `arrow` shot from the `ammo` a mob's crossbow was charged with.
    CrossbowShot {
        ammo: crate::items::ItemStack,
        arrow: crate::projectiles::Projectile,
    },
    /// Spawn a new mob (breeding, slime splitting, chicken eggs...).
    Spawn(Box<Mob>),
    /// Spawn a mob by type id at a position (conversions, splits).
    SpawnKind {
        kind: String,
        at: [f64; 3],
        setup: SpawnSetup,
    },
    /// `Mob.convertTo` with `ConversionType.SINGLE` ([`conversion::single`]):
    /// a new mob of `kind` takes the place of `from` (already removed) and
    /// keeps its gear, name and flags.
    Convert {
        from: Box<Mob>,
        kind: &'static str,
        params: conversion::Params,
    },
    DropItem {
        mob: u32,
        item: Arc<str>,
        count: u32,
        at: [f64; 3],
    },
    /// `Entity.spawnAtLocation` with the complete stack and its components.
    DropStack {
        mob: u32,
        stack: crate::items::ItemStack,
        at: [f64; 3],
    },
    /// `new ItemEntity(.., stack)` thrown by `mob` (`setThrower`): `Dolphin.ItemGoal.drop`.
    ThrowItem {
        mob: u32,
        stack: crate::items::ItemStack,
        at: [f64; 3],
        motion: [f64; 3],
        pickup_delay: u32,
    },
    /// `Goat.dropHorn`: a horn from goat `mob` (`Goat.createHorn` picks the
    /// instrument from its UUID), thrown with `motion` as
    /// `new ItemEntity(.., deltaX, deltaY, deltaZ)`.
    DropGoatHorn {
        mob: u32,
        screaming: bool,
        at: [f64; 3],
        motion: [f64; 3],
    },
    /// `Mob.dropFromGiftLootTable`: a gift loot table's items dropped at `at`.
    DropGift {
        table: &'static str,
        at: [f64; 3],
    },
    /// A leave-block goal's sampled candidate, before neighbor-shape updates.
    EndermanPlaceBlock {
        mob: u32,
        pos: [i32; 3],
        carried: horizon_world::registry::BlockStateId,
        occupants: Vec<horizon_core::geometry::Aabb>,
    },
    SetBlock {
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    },
    /// A mob placing a block (such as a snow golem's trail): `entity_place`
    /// may refuse it. Enderman leave-block attempts use `EndermanPlaceBlock`.
    PlaceBlock {
        mob: u32,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    },
    /// `TurtleEggBlock.decreaseEggs` after a mob's step won its 1 in 100: the
    /// break sound, then one egg fewer, or the block destroyed when it was
    /// the last.
    CrackTurtleEgg {
        pos: [i32; 3],
    },
    /// `LivingEntity.startSleeping` / `stopSleeping`: the bed head at `pos`
    /// gets `occupied` (`setBlockAndUpdate`; the foot follows it).
    BedOccupied {
        pos: [i32; 3],
        occupied: bool,
    },
    /// `DoorBlock.setOpen` by a mob (`InteractWithDoor`): the door opens or
    /// closes, both halves, with its sound and game event.
    SetDoorOpen {
        pos: [i32; 3],
        open: bool,
    },
    /// `BreakDoorGoal.tick` at its end: the door is removed (`removeBlock`),
    /// then level events 1021 and 2001 with the state that is left there.
    BreakDoor {
        mob: u32,
        pos: [i32; 3],
    },
    /// `Level.destroyBlockProgress` for a mob's crack on a block (`-1`
    /// clears it).
    DestroyProgress {
        mob: u32,
        pos: [i32; 3],
        progress: i32,
    },
    /// `Rabbit.RaidGardenGoal.tick`: the carrots at `pos` become `state` (`setBlock` with
    /// flags 2); a crop that only lost an age shows the break particles of the state it had
    /// (`levelEvent(2001, pos, old)`) and fires `BLOCK_CHANGE` for the rabbit.
    CropRaided {
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
        old: Option<horizon_world::registry::BlockStateId>,
        mob: u32,
    },
    /// `Level.levelEvent`: the game plays and sends the event.
    LevelEvent {
        id: i32,
        pos: [i32; 3],
        data: i32,
    },
    /// `InfestedBlock.infestedStateByHost` (`infested`) or `hostStateByInfested`
    /// for the block there, keeping its axis; `poof` is the silverfish that
    /// hid in it (`spawnAnim`).
    Infest {
        pos: [i32; 3],
        infested: bool,
        poof: Option<u32>,
    },
    /// `Level.destroyBlock(pos, drop)`: the block gives way to its fluid's
    /// block (air when it has none). Sheep eating grass becomes dirt via SetBlock.
    BreakBlock {
        pos: [i32; 3],
        drop: bool,
        /// The mob whose breaking this is, when it is one `LivingDestroyBlockEvent`
        /// hears (a wither's, a ravager's, a silverfish's, a zombie's).
        by: Option<u32>,
        /// The entity `destroyBlock` names when that event doesn't hear of it (a
        /// villager's harvest, a ravager's crop): it asks the mods nothing, and
        /// its loot's `LootContextParams.THIS_ENTITY` is this one, as `by`'s is.
        breaker: Option<u32>,
    },
    /// `EndermanTakeBlockGoal`: the enderman `mob` picks up the block `state` at
    /// `pos` (it already holds it); the block turns to air unless a mod stops it
    /// or it is no longer that block.
    TakeBlock {
        mob: u32,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    },
    /// What follows a `BreakBlock { by: Some(mob) }` of this batch: `effects` when
    /// the game broke at least one of the mob's blocks, `otherwise` when a mod kept
    /// them all (vanilla plays the break's sound and particles only on a destruction).
    /// Only made while a mod is loaded; without one the effects are plain events.
    IfBroke {
        mob: u32,
        effects: Vec<MobEvent>,
        otherwise: Vec<MobEvent>,
    },
    /// The mob `mob` jumps (`jumpFromGround`'s flag, set after a ravager broke no leaves).
    Jump {
        mob: u32,
    },
    Particles {
        kind: &'static str,
        at: [f64; 3],
        count: u32,
    },
    /// Client particles about a mob's box (`particles::around_self`) and the
    /// entity event `event` that shows them, sent for mob `mob`.
    AroundSelf {
        mob: u32,
        event: i8,
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
    /// One `minecraft:entity_effect` particle at `at` in the colour `color`
    /// (`ColorParticleOption.create(ENTITY_EFFECT, r, g, b)`), standing still.
    ParticleColored {
        at: [f64; 3],
        color: [f32; 3],
    },
    Experience {
        at: [f64; 3],
        amount: u32,
    },
    /// Regeneration support after an axolotl-assisted kill, from the axolotl `mob`.
    AxolotlSupportPlayer {
        mob: u32,
    },
    /// A dolphin that was given a fish asks the game for the nearest
    /// shipwreck or ocean ruin (`DolphinSwimToTreasureGoal.start`): the game
    /// sets the dolphin's `extra.home` to it, or leaves it empty.
    DolphinTreasure {
        mob: u32,
    },
    /// Apply an effect to the player (witch potions, bee stings, cave spider poison).
    EffectPlayer {
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
        /// `MobEffectInstance` ambient flag (the beacon-style translucent icon).
        ambient: bool,
    },
    /// Apply an effect to another mob: `IllusionerBlindnessSpellGoal.performSpellCasting`
    /// blinds whatever `LivingEntity` the illusioner targets, not only a player.
    EffectMob {
        target: u32,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    },
    /// Set an entity on fire for so many seconds.
    Ignite(Who, i32),
    /// `Zombie.doHurtTarget`'s burn: lit only if the attack event before it hurt.
    ZombieIgnite(Who, i32),
    /// Two animals bred: the game spawns the baby and resets the partner.
    Breed {
        parent: u32,
        partner: u32,
        /// Equine inheritance, drawn at `getBreedOffspring`'s goal moment,
        /// before later same-tick healing and grazing rolls.
        equine: Option<Box<equine::Offspring>>,
        /// Whether the parent supplies the variant, if already drawn.
        parent_variant: Option<bool>,
        /// Wolf voice drawn before experience, unless baby-spawn approval defers inheritance.
        wolf_voice: Option<u8>,
        /// The parent-RNG experience decision, if already made.
        experience: BreedExperience,
    },
    /// Replace a block by name (sheep turning grass to dirt), optionally with
    /// the destroy effect.
    SetBlockName {
        mob: Option<u32>,
        pos: [i32; 3],
        name: &'static str,
        effect: bool,
        flags: i32,
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
    /// `Entity.unRide`'s `ejectPassengers` before a move (a shulker's
    /// teleport): whoever rides `mob` gets off where it stood, `from`.
    EjectPassengers {
        mob: u32,
        from: Vec3,
    },
    /// A piglin's admiring is over: barter or keep the item.
    FinishAdmiring {
        mob: u32,
    },
    /// `IronGolem.doHurtTarget`: the hit, then, only when `hurtServer` let it
    /// land, the target thrown up by `0.4f` scaled by
    /// `max(0, 1 - KNOCKBACK_RESISTANCE)`.
    IronGolemHit {
        target: Who,
        amount: f32,
        source: Damage,
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
    /// `Villager.customServerAiStep`: deliver the pending TRADE reputation
    /// event for the actual player recorded by rewardTradeXp.
    VillagerTrade {
        mob: u32,
        player: [i32; 4],
    },
    /// `AbstractVillager.getOffers` for a villager whose offers are not made
    /// yet: the level rolls them (the brain has no trade data), so the
    /// behaviour that asked reads them from the next tick on.
    VillagerOffers {
        mob: u32,
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
    /// `WorkAtComposter.useWorkstation`: a farmer at its composter makes
    /// bread and composts seeds (the game applies it to the villager's
    /// inventory and the block).
    WorkAtComposter {
        mob: u32,
        pos: [i32; 3],
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
    /// Drop the loot of a gift table (chicken eggs, sniffer seeds) with the
    /// mob as `this` (`LivingEntity.dropFromGiftLootTable`).
    GiftLoot {
        mob: u32,
        table: &'static str,
        at: [f64; 3],
    },
    /// `Chicken.aiStep`: the egg timer ran out. The `chicken_lay` gift table
    /// drops at the chicken; only when it dropped something does the egg sound
    /// play (at `pitch`) and `entity_place` reach listeners.
    LayEgg {
        mob: u32,
        at: [f64; 3],
        pitch: f32,
    },
    /// `Armadillo.customServerAiStep`: the armadillo sheds a scute.
    ShedScute {
        mob: u32,
        at: [f64; 3],
        pitch: f32,
    },
    /// A lead snaps or loses its holder: the lead item drops when asked.
    LeashBreak {
        mob: u32,
        drop: bool,
        sound: bool,
    },
    /// `Leashable.whenLeashedTo`: the holder is told a quad-leashed mob hangs
    /// on it (`HappyGhast.notifyLeashHolder`).
    NotifyLeashHolder {
        holder: u32,
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
        /// `VibrationSystem.User.getPositionSource`: the listener itself.
        source: horizon_protocol::entity_data::PositionSource,
    },
    /// The crumbs of the block under a digging or emerging warden.
    DigParticles {
        at: [f64; 3],
    },
    /// `Zombie.hurtServer`: a zombie hurt with a target in sight (its own,
    /// else the attacker) may call a reinforcement against it on Hard.
    Reinforcements {
        mob: u32,
        target: Who,
    },
    /// `LevelEvent.PARTICLES_DESTROY_BLOCK` (2001) at `pos` for the block at
    /// `block`: a turtle's digging.
    BlockBreakEffect {
        pos: [i32; 3],
        block: [i32; 3],
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
    /// `Panda.afterSneeze`: the grown pandas within 10 blocks start.
    PandaSneezed {
        mob: u32,
    },
    /// `Entity.propagateFallToPassengers`: the mob landed and what rides it
    /// (a player or a mob) takes `causeFallDamage` of the same fall.
    FallToPassengers {
        mob: u32,
        distance: f64,
        modifier: f32,
    },
}

impl MobEvent {
    /// Whether this event acts on the `Who::Player` seen by the mob.
    pub fn targets_player(&self) -> bool {
        match self {
            Self::AttackPlayer(..)
            | Self::CubeHurtPlayer { .. }
            | Self::AttackPlayerWithEffect { .. }
            | Self::StingPlayer { .. }
            | Self::DamagePlayer { .. }
            | Self::EffectPlayer { .. }
            | Self::AxolotlSupportPlayer { .. }
            | Self::ParrotToShoulder { .. } => true,
            Self::SonicBoom { target, .. }
            | Self::Push { target, .. }
            | Self::Knockback { target, .. }
            | Self::StabDismount { target }
            | Self::Ignite(target, _)
            | Self::ZombieIgnite(target, _)
            | Self::IronGolemHit { target, .. } => *target == Who::Player,
            Self::Projectile(projectile) => matches!(
                &projectile.kind,
                crate::projectiles::ProjectileKind::ShulkerBullet {
                    target: Some(Who::Player),
                    ..
                }
            ),
            _ => false,
        }
    }
    /// `level.playSound(null, m, sound, source, volume, pitch)` from where
    /// `m` stands.
    pub fn bound_sound(
        m: &Mob,
        sound: &'static str,
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> Self {
        Self::BoundSound {
            mob: m.id,
            sound,
            source,
            at: [m.position.x, m.position.y, m.position.z],
            volume,
            pitch,
        }
    }
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
    /// Whether to skip `finalizeSpawn`'s rolls (e.g. conversions or EntityType.create).
    pub skip_finalize: bool,
}
include!("mobs/seen.rs");

impl Mob {
    /// The name used by command replies: CustomName takes precedence over the type.
    pub fn command_name(&self) -> crate::text::Text {
        villager::command_name(self)
    }

    /// What the mob holds and wears, by `EquipmentSlot` ordinal: main hand,
    /// off hand, feet, legs, chest, head, body and saddle
    /// (`ClientboundSetEquipmentPacket`'s slots).
    pub fn equipment_stacks(&self) -> horizon_items::equipment::Equipment {
        use horizon_items::equipment::{HEAD, MAINHAND};
        let mut out = self.extra.equipment.clone();
        // The worn armour keeps its stack: a dyed leather piece keeps its
        // colour, and the enchantments and trim come from `gear`.
        for slot in MAINHAND..=HEAD {
            if let Some(stack) = finalize::stored_stack(self, slot) {
                out.set(slot, Some(stack));
            }
        }
        out
    }
    /// Remove and return the complete body stack.
    pub fn take_body_equipment(&mut self) -> Option<crate::items::ItemStack> {
        let stack = self.equipment_stacks()[6].clone();
        self.extra.set_body_stack(None);
        stack
    }

    /// Set the complete equipment stack by the protocol slot ordinal.
    pub fn set_equipment_stack(
        &mut self,
        slot: usize,
        stack: Option<crate::items::ItemStack>,
    ) -> bool {
        if slot == 6 {
            self.extra.set_body_stack(stack);
            return true;
        }
        if slot == 7 {
            self.set_saddle(stack);
            return true;
        }
        let gear_slot = match slot {
            0 => gear::MAIN,
            1 => gear::OFF,
            2 => gear::FEET,
            3 => gear::LEGS,
            4 => gear::CHEST,
            5 => gear::HEAD,
            _ => return false,
        };
        if let Some(stack) = stack {
            finalize::set_item_stack(self, slot, stack);
        } else {
            self.extra.set_slot_stack(slot, None);
            gear::set(self, gear_slot, Vec::new());
        }
        true
    }

    pub fn burning_time(&self) -> f64 {
        let base = finalize::value(self, finalize::Attr::BurningTime);
        let protection: u32 = gear::ARMOR
            .into_iter()
            .map(|slot| gear::level(self, slot, "minecraft:fire_protection"))
            .sum();
        (base - 0.15 * f64::from(protection)).max(0.0)
    }
    /// Entity.isOnFire: fire-immune mobs never count as burning, even on the
    /// tick a sun or fire check set their timer before `base_tick` clears it.
    pub fn is_on_fire(&self) -> bool {
        !self.kind.flags.fire_immune && self.fire_ticks > 0
    }
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        self.fire_ticks = self
            .fire_ticks
            .max((f64::from(ticks) * self.burning_time()).ceil() as i32);
        self.ticks_frozen = 0;
    }
    /// `Entity.extinguishFire`: a burning mob first plays the extinguish sound
    /// (`playEntityOnFireExtinguishedSound`: volume 0.7, pitch from two draws
    /// of its random, at its feet), then `clearFire` keeps only the
    /// fire-immune countdown.
    pub fn extinguish_fire(&mut self) {
        if self.is_on_fire() {
            let pitch = 1.6 + (self.random.next_float() - self.random.next_float()) * 0.4;
            let at = [self.position.x, self.position.y, self.position.z];
            self.outbox.push(MobEvent::Sound(
                "entity.generic.extinguish_fire",
                at,
                0.7,
                pitch,
            ));
        }
        self.fire_ticks = self.fire_ticks.min(0);
    }
    /// `Entity.isFullyFrozen`.
    pub fn is_fully_frozen(&self) -> bool {
        self.ticks_frozen >= crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE
    }

    /// What other mobs see of this one this tick, its villager food counted by `items` (`_player`
    /// is unused: it keyed the villager reputation that nothing read). Inline: the game's snapshot
    /// of every mob writes each one in its slot, not through a copy.
    #[inline]
    pub fn seen(&self, _player: Option<[i32; 4]>, items: &crate::items::Items) -> Seen {
        let m = self;
        let villager = m.species() == Species::Villager;
        Seen {
            kind: Arc::clone(&m.kind),
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
            invulnerable: m.is_invulnerable(),
            baby: m.is_baby(),
            // A villager is "in love" when it is willing to breed (`canBreed`).
            in_love: if villager {
                villager::can_breed(m, items)
            } else {
                m.extra.in_love > 0
            },
            tame: m.extra.tame,
            bred: m.extra.bred,
            owner: m.extra.owner,
            invisible: m.invisible || m.has_effect("minecraft:invisibility"),
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
            cover: visibility::cover(m),
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
            profession: villager.then(|| m.extra.profession.index()).flatten(),
            custom_profession: villager::seen_profession_name(m, villager),
            secondary_variant: m.extra.variant2,
            xp: villager::xp(m),
            villager: villager.then(|| Arc::new(villager::seen(m, items))),
            stats: [
                f64::from(m.attrs.max_health),
                m.extra.vector[0],
                m.attrs.speed,
            ],
            sitting: m.extra.sitting,
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
                .map(|e| e.id.trim_start_matches("minecraft:"))
                .collect(),
            hunted_recently: m.species() == Species::Piglin && m.extra.ints[0] > 0,
            hurt_recently: m.species() == Species::Piglin && m.extra.piglin.hurt,
            huntable: m.species() == Species::Hoglin && !m.is_baby() && !m.extra.cannot_be_hunted,
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
/// velocity before its tick (and to every mob once all have ticked). The
/// other events keep their order.
/// The part of `LivingEntity.canUsePortal` a mob's own state decides: it is alive,
/// not sleeping (including fox sleep, but not bat resting or cat lying),
/// it is not a boss that never travels, and — over `Monster.canUsePortal` —
/// it is not a creaking bound to a heart (`Creaking.canUsePortal`). The
/// rides and the players a side carries are the caller's half.
pub fn mob_can_use_portal(m: &Mob) -> bool {
    m.alive()
        && !villager::sleeping(m)
        && !(m.species() == Species::Fox && fox::flag(m, fox::SLEEPING))
        && m.species() != Species::Wither
        && creaking::can_use_portal(m)
}
pub fn take_mob_pushes(events: &mut Vec<MobEvent>, from: usize, pending: &mut PendingPushes) {
    // One pass: a crowd's tick holds a push per neighbour, and removing each
    // in place would shift the rest every time.
    let mut kept = from;
    for i in from..events.len() {
        if let MobEvent::Push {
            target: Who::Mob(id),
            velocity,
        } = events[i]
        {
            pending.push(id, Vec3::new(velocity[0], velocity[1], velocity[2]));
        } else {
            events.swap(kept, i);
            kept += 1;
        }
    }
    events.truncate(kept);
}
/// Pushes waiting for their mob's tick, in push order. A list while short;
/// past [`PendingPushes::LIST_MAX`] (a crowd pushing each other) one list
/// per mob, as scanning the whole list for each mob grows with its square.
#[derive(Clone, Default)]
pub struct PendingPushes {
    list: Vec<(u32, Vec3)>,
    by_mob: crate::fast_hash::FxHashMap<u32, Vec<Vec3>>,
}
impl PendingPushes {
    /// `other`'s pushes after these, each mob's in their order.
    pub fn append(&mut self, other: PendingPushes) {
        for (id, v) in other.list {
            self.push(id, v);
        }
        for (id, list) in other.by_mob {
            for v in list {
                self.push(id, v);
            }
        }
    }
    const LIST_MAX: usize = 64;
    /// Whether every queued push has been consumed or retired with its target.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.by_mob.is_empty()
    }
    pub fn push(&mut self, id: u32, v: Vec3) {
        if self.by_mob.is_empty() && self.list.len() < Self::LIST_MAX {
            self.list.push((id, v));
            return;
        }
        // Older pushes first, so each mob's stay in order.
        for (id, v) in self.list.drain(..) {
            self.by_mob.entry(id).or_default().push(v);
        }
        self.by_mob.entry(id).or_default().push(v);
    }
}
#[cfg(test)]
mod pending_push_tests {
    use super::PendingPushes;
    use horizon_core::geometry::Vec3;

    /// Each mob's pushes, in the order they apply.
    fn per_mob(p: &PendingPushes) -> std::collections::BTreeMap<u32, Vec<[f64; 3]>> {
        let mut out = std::collections::BTreeMap::<u32, Vec<[f64; 3]>>::new();
        for (id, v) in &p.list {
            out.entry(*id).or_default().push([v.x, v.y, v.z]);
        }
        for (id, list) in &p.by_mob {
            out.entry(*id)
                .or_default()
                .extend(list.iter().map(|v| [v.x, v.y, v.z]));
        }
        out
    }

    /// Two lists of pushes, the second appended to the first, apply as one list of them all in
    /// that order, on either side of the switch from one list to a list per mob.
    #[test]
    fn appended_pushes_apply_in_order() {
        let pushes: Vec<(u32, Vec3)> = (0..200u32)
            .map(|k| (k * 7 % 11, Vec3::new(f64::from(k), 0.0, -f64::from(k))))
            .collect();
        let mut all = PendingPushes::default();
        for &(id, v) in &pushes {
            all.push(id, v);
        }
        for split in [0, 30, 64, 70, 150, 200] {
            let (mut first, mut second) = (PendingPushes::default(), PendingPushes::default());
            for &(id, v) in &pushes[..split] {
                first.push(id, v);
            }
            for &(id, v) in &pushes[split..] {
                second.push(id, v);
            }
            first.append(second);
            assert_eq!(per_mob(&first), per_mob(&all), "split at {split}");
        }
    }
}
/// `ServerLevel.sendBlockUpdated`: for each write that changed a block's collision
/// shape, a mob whose path runs near it re-paths (`PathNavigation.shouldRecomputePath`).
/// The writes whose shape changed are found once per tick ([`collision_changes`]).
pub fn blocks_changed(m: &mut Mob, changed: &[[i32; 3]]) {
    if m.ai.nav.is_done() {
        return;
    }
    for &at in changed {
        if m.should_recompute_path(at) {
            m.queue_path_recompute();
            return;
        }
    }
}
/// The writes of `writes` that changed the block's collision shape, in order:
/// `sendBlockUpdated` compares the two shapes once per write, not per mob.
pub fn collision_changes(w: &MobWorld, writes: &[((i32, i32, i32), u32, u32)]) -> Vec<[i32; 3]> {
    writes
        .iter()
        .filter_map(|&((x, y, z), old, new)| {
            let at = [x, y, z];
            let shape =
                |state| w.collision_bounds_of(horizon_world::registry::BlockStateId(state), at);
            (shape(old) != shape(new)).then_some(at)
        })
        .collect()
}
/// Applies the brain memories a mob's tick set on other mobs (from event
/// index `from` on), at once: vanilla sets them on the other entity during
/// the tick, before the mobs after it tick.
pub fn apply_brain_events(
    mobs: &mut crate::join_seq::JoinList<Mob>,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    from: usize,
) {
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
            MobEvent::Retreat { mob, from, at } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == mob) {
                    piglin::receive_retreat(m, from, at);
                }
                events.remove(i);
            }
            MobEvent::StartRiding { rider, vehicle } => {
                riders::start_riding(mobs, rider, vehicle);
                events.remove(i);
            }
            MobEvent::StartFollowing { fish, leader } => {
                aquatic::start_following(mobs, fish, leader);
                events.remove(i);
            }
            MobEvent::SchoolSize { leader, by } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == leader) {
                    m.extra.followers += by;
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
            // Pufferfish.touch: the poison and playSound sit inside the
            // hurtServer branch, so only a hit that landed poisons and stings.
            // In vanilla, this runs in Pufferfish.aiStep during the fish's tick,
            // so mobs ticking later in the same level tick tick their effects.
            MobEvent::PufferfishSting {
                mob,
                target,
                amount,
                mut source,
                poison_ticks,
            } => {
                let mut sound = None;
                let mut raised = Vec::new();
                let fish = mobs.iter().find(|m| m.id == mob);
                if source.attacker_species.is_none() {
                    source.attacker_species = fish.map(Mob::species);
                }
                let held = if crate::native_client_hooks::has_mods() {
                    fish.and_then(|m| m.extra.main_hand_stack()).cloned()
                } else {
                    None
                };
                let attack = crate::health::Attack {
                    attacker: mob as i32,
                    // Pufferfish.touch uses DamageSources.sting: the fish, no projectile.
                    projectile: None,
                    direct: None,
                    held: [held, None],
                    player_uuid: None,
                    ..Default::default()
                };
                let (aim, fish_pos, fish_silent) = fish
                    .map(|m| {
                        (
                            Aim {
                                eye: [m.position.x, m.eye_y(), m.position.z],
                                look: m.look_angle(),
                                feet: [m.position.x, m.position.y, m.position.z],
                            },
                            [m.position.x, m.position.y, m.position.z],
                            m.silent,
                        )
                    })
                    .map(|(aim, pos, silent)| (Some(aim), Some(pos), silent))
                    .unwrap_or((None, None, true));
                let mut hit_level = level.reborrow().attacked_by(&attack);
                if let Some(t) = mobs.iter_mut().find(|m| m.id == target)
                    && t.damage_aimed(&mut hit_level, amount, source, aim)
                {
                    t.add_effect("minecraft:poison", poison_ticks, 0);
                    raised = std::mem::take(&mut t.outbox);
                    if !fish_silent && let Some(at) = fish_pos {
                        sound = Some(MobEvent::Sound("entity.puffer_fish.sting", at, 1.0, 1.0));
                    }
                }
                events.remove(i);
                if let Some(s) = sound {
                    events.insert(i, s);
                    i += 1;
                }
                for e in raised {
                    events.insert(i, e);
                    i += 1;
                }
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
    /// The `invulnerable` ability, the `Invulnerable` flag or its window,
    /// which `asValidTarget` refuses a target by (#1175, #916): a
    /// valid-target rule a goal tick's view asks.
    pub invulnerable: bool,
    pub creative: bool,
    pub sneaking: bool,
    /// `Entity.isSwimming`.
    pub swimming: bool,
    pub look: [f64; 3],
    pub main_hand: Option<Arc<str>>,
    pub off_hand: Option<Arc<str>>,
    pub head: Option<Arc<str>>,
    /// `getItemBySlot` for chest, legs and feet.
    pub chest: Option<Arc<str>>,
    pub legs: Option<Arc<str>>,
    pub feet: Option<Arc<str>>,
    pub armor_cover: f32,
    /// The Invisibility effect (`isInvisible`).
    pub invisible: bool,
    pub spectator: bool,
    /// `PiglinAi.isWearingSafeArmor`: some golden armour.
    pub gold_armor: bool,
    /// `getLastHurtByMob` (within 100 ticks) and when.
    pub hurt_by: Option<(Who, i64)>,
    /// `getLastHurtMob` and when.
    pub last_hurt: Option<(Who, i64)>,
    pub velocity: Vec3,
    pub yaw: f32,
    /// `Player.isSprinting` and `isPassenger` (what scares an armadillo).
    pub sprinting: bool,
    pub riding: bool,
    /// The Hero of the Village effect.
    pub hero: bool,
    /// The active effects, without namespace (`slowness`, `poison`...).
    pub effects: Vec<&'static str>,
    /// The player's UUID (villagers' gossip is keyed by it).
    pub uuid: [i32; 4],
    /// The mobs on the player's team (`isAlliedTo`): combat targeting
    /// leaves the player alone.
    pub allies: std::sync::Arc<std::collections::HashSet<u32>>,
    /// A LAN guest: its own client moves it, and touching mobs push it there
    /// (`EntitySelector.pushableBy` on the client).
    pub remote: bool,
    /// `isSleeping` and `getSleepTimer` (100 once slept through, then the
    /// ticks fading out after waking up).
    pub sleeping: bool,
    pub sleep_timer: u32,
    /// `isPassenger`.
    pub passenger: bool,
    /// The vehicle entity id, including mounts without a `Vehicle` follow view.
    pub riding_vehicle: Option<u32>,
    /// The boat or nautilus the player rides and steers
    /// (`getControllingPassenger`).
    pub vehicle: Option<Vehicle>,
}
/// What a player rides, for the mobs that follow it (`FollowPlayerRiddenEntityGoal`).
#[derive(Clone, Debug)]
pub struct Vehicle {
    pub nautilus: bool,
    pub bounds: Aabb,
    /// `Entity.getKnownSpeed`: what it moved in its last tick.
    pub velocity: Vec3,
}
impl PlayerView {
    /// `LivingEntity.getVisibilityPercent(level, looking)` before the visibility
    /// event and the clamp: sneaking, invisibility by armour cover, and the
    /// worn head's `mob_visibility` for the `looking` mob's type.
    pub fn visibility_percent(&self, looking: Species) -> f64 {
        let mut visibility = 1.0;
        if self.sneaking {
            visibility *= 0.8;
        }
        if self.invisible {
            visibility *= 0.7 * f64::from(self.armor_cover).max(0.1);
        }
        if let Some(head) = self.head.as_deref() {
            visibility *= f64::from(mob_visibility(head, looking));
        }
        visibility
    }
    /// `LivingEntity.hasEffect`, by id without namespace.
    pub fn has_effect(&self, effect: &str) -> bool {
        self.effects.contains(&effect)
    }
}
/// `Animal.finalizeSpawnChildFromBreeding`: draw only when mob drops are enabled.
pub fn breeding_experience(parent: &mut Mob, mob_drops: bool) -> Option<u32> {
    mob_drops.then(|| 1 + parent.random.next_int_bound(7).unwrap_or(0) as u32)
}

/// Read-only access to the live mobs outside the actor's current tick loan.
pub trait LiveMobs {
    fn find(&self, id: u32) -> Option<&Mob>;
    /// Visit each mob in the disjoint loan, without allocating a snapshot.
    fn for_each(&self, visitor: &mut dyn FnMut(&Mob));
}
#[derive(Clone)]
pub struct MobWorld<'a> {
    pub mob_maps: &'a crate::neoforge_mob_maps::NeoForgeMobMaps,
    pub professions: &'a villager::ResolvedProfessions,
    /// What the motion value hooks are given: the loop context of the side that
    /// ticks these mobs.
    pub loop_context: &'a horizon_world::native_load_context::LoopContext,
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    /// Fluid states and flow facts, for the currents that push mobs.
    pub fluid_tables: &'a crate::motion::FluidResources,
    /// The owning side's attribute definitions for species-specific writes.
    pub attributes: &'a Arc<horizon_world::attributes::entity::Catalog>,
    pub movement: &'a horizon_world::movement_properties::BoundMovementProperties,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub suffocation: &'a horizon_world::suffocation::SuffocationRegistry<'a>,
    pub data: &'a MobData,
    pub player: Option<PlayerView>,
    /// Switchable per-tick player views, present only when target listeners
    /// can redirect a mob to a different real player.
    pub player_selection: Option<&'a player_selection::PlayerSelection>,
    /// The uuid and entity id of the player whose view `player` is (#1077):
    /// captured once where the tick lends its player — the game's own, or the
    /// guest a server's mob sees — so a goal resolves that player without a
    /// scan, and a fake player's view resolves as none. None when `player` is
    /// None.
    pub player_identity: Option<([i32; 4], u32)>,
    /// `getKnownMovement` of a mob's controlling player passenger, if any.
    pub controlling_player_movement: Option<Vec3>,
    /// `getNearestPlayer(mob, -1)`: where the nearest non-spectator of every
    /// player in the level stands, alive or not (the despawn distance).
    pub nearest_player: Option<Vec3>,
    pub others: &'a [Seen],
    /// Retained passenger objects for topology only, never targets or colliders.
    pub passenger_objects: &'a [Seen],
    /// Disjoint live mob state, available only while an actor is ticked.
    pub live_mobs: Option<&'a dyn LiveMobs>,
    /// The separately simulated dragon, visible to revenge and melee goals.
    pub dragon: Option<(u32, TargetInfo)>,
    /// The boats and rafts a lead can hang on (`Holder::Boat`): id and state.
    pub leash_boats: &'a [(u32, leash::HolderState)],
    /// `others` by position, when the caller keeps one (the mob ticks).
    pub others_grid: Option<&'a seen_grid::SeenGrid>,
    /// Where each id stands in `others`, when the caller keeps one.
    pub others_index: Option<&'a crate::fast_hash::FxHashMap<u32, u32>>,
    /// The slots of `others` holding a villager, in order, when the caller
    /// keeps them (the mob ticks: a slot keeps its mob, and its species, all
    /// tick). Each zombie looking for a village read every slot.
    pub others_villagers: Option<&'a [u32]>,
    /// The slots of `others` that may stop a moving mob
    /// ([`Seen::may_block_mobs`]), when the caller keeps them; None: any.
    pub others_solid: Option<&'a [u32]>,
    pub daylight: bool,
    /// The `gameplay/monsters_burn` attribute: the overworld day timeline's
    /// step track, false in every other dimension.
    pub monsters_burn: bool,
    pub game_time: i64,
    pub day_time: i64,
    pub raining: bool,
    pub thundering: bool,
    pub level: horizon_core::LevelKey,
    pub block_tags: &'a HashMap<String, std::collections::HashSet<String>>,
    /// Each block's `fallOn` rule (the player's landing registry).
    pub landing: &'a horizon_world::block_landing::BlockLandingRegistry,
    pub biomes: &'a crate::biome::Biomes,
    /// The `mob_griefing` game rule.
    pub mob_griefing: bool,
    /// The `universal_anger` game rule (`NeutralMob.isAngryAtAllPlayers`).
    pub universal_anger: bool,
    /// The `mob_drops` game rule (`shouldDropLoot`).
    pub mob_drops: bool,
    /// The `max_entity_cramming` game rule.
    pub max_entity_cramming: i32,
    /// `getCurrentDifficultyAt` at the mob being ticked (near the player
    /// outside the mob tick).
    pub difficulty: spawning::LocalDifficulty,
    /// The level's dimension type.
    pub dimension_type: &'a horizon_world::dimension_type::DimensionFacts,
    /// The world seed (slime chunks).
    pub world_seed: i64,
    /// `Level.getSeaLevel` (63 in the overworld, -63 in a flat world).
    pub sea_level: i32,
    /// The scoreboard teams of the player and the mobs (collision rules),
    /// shared: the mob ticks build them once a tick, not once a mob.
    pub teams: std::sync::Arc<Teams<'a>>,
    /// Item data (max stack sizes), when the host has it.
    pub items: Option<&'a crate::items::Items>,
    /// `ChestBlockEntity.getOpenCount`: the players with each chest open.
    pub lid_openers: &'a HashMap<(i32, i32, i32), i32>,
    /// What a content mod's effects do, on a server (None on a client).
    pub mod_effects: Option<&'a dyn ModEffectTicks>,
    /// The mod hooks the AI asks mid-tick, on a server where a mod listens
    /// (None otherwise: the AI asks nothing).
    pub mob_hooks: Option<&'a dyn MobHooks>,
    /// The goal view's facts (#1077), on the native-authoritative side where
    /// a mod attached an entity part (None otherwise: no mod goal runs).
    pub goal_host: Option<&'a dyn part_goals::GoalHost>,
    /// Whether a listener can cancel animal taming.
    pub asks_animal_tame: bool,
    /// Whether a listener can cancel a mob's block destruction.
    pub asks_living_destroy_block: bool,
    /// Whether a server listener can cancel baby spawning.
    pub asks_baby_spawn: bool,
    /// `Level.getEntity(uuid)`, for the UUIDs a mob kept of entities that were
    /// not in the level (a warden's anger, `AngerManagement.angerByUuid`).
    /// None: this world keeps no entity UUIDs, so nothing is resolved.
    pub who_lookup: Option<&'a dyn WhoLookup>,
    /// Another thread's mobs, for a lookup by id that misses `others`: a group of the server's
    /// parallel mob loop (`game/mob_islands.rs`) answers as the serial loop would.
    pub foreign: Option<&'a dyn ForeignSeen>,
    /// The level's world border, whose walls stop a mob's moves
    /// (`Entity.collectColliders`); None: nothing stops them there.
    pub border: Option<&'a horizon_core::geometry::border::BorderGeometry>,
    pub section_writer: Option<living::section_writes::Observer>,
}
/// `MobEffectInstance.tickServer` for a content mod's effect: its
/// `shouldApplyEffectTickThisTick` and `applyEffectTick`, which the game's
/// native mods answer. False removes the effect.
pub trait ModEffectTicks {
    fn tick_mod_effect(
        &self,
        level: &mut DamageLevel<'_>,
        effect: usize,
        mob: &mut Mob,
        tick_count: i32,
        amplifier: i32,
    ) -> bool;
    /// `MobEffectEvent.Expired`: `mob`'s `effect` (`amplifier` from 0)
    /// counted down to nothing. A mod that cancels refuses the expiry: the
    /// effect stays at no ticks and asks again next tick.
    fn effect_expired(&self, mob: u32, effect: usize, amplifier: i32, cancel: &mut bool);
}
mod mob_hooks;
pub use mob_hooks::MobHooks;
/// Outcome of a target-change event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetChange {
    /// Apply the resulting target.
    Set(Option<Who>),
    /// Keep the previous target.
    Keep,
}
impl MobWorld<'_> {
    /// The currently selected player view. The ordinary tick uses `player`;
    /// target-change callbacks can switch a scoped selection during a tick.
    pub fn selected_player(&self) -> Option<&PlayerView> {
        match self.player_selection {
            Some(selection) => selection.selected().map(|player| &player.view),
            None => self.player.as_ref(),
        }
    }

    /// UUID and entity ID of the currently selected player, if any.
    pub fn selected_player_identity(&self) -> Option<([i32; 4], u32)> {
        match self.player_selection {
            Some(selection) => selection
                .selected()
                .map(|player| (player.view.uuid, player.entity_id)),
            None => self.player_identity,
        }
    }

    /// Capture a player-directed event's recipient after a listener switches
    /// the selected player. Before the first switch retain the plain event and
    /// the existing allocation-free routing path.
    pub fn route_player_event(&self, event: MobEvent) -> MobEvent {
        if event.targets_player()
            && let Some(selection) = self.player_selection
            && selection.has_switched()
            && let Some(player) = selection.selected()
        {
            return MobEvent::ForPlayer {
                conn: player.connection,
                event: Box::new(event),
            };
        }
        event
    }

    /// An action directed at the player already bound at tick start, such as
    /// retaliation against the attacker, must not follow a later target change.
    pub fn route_initial_player_event(&self, event: MobEvent) -> MobEvent {
        if event.targets_player()
            && let Some(selection) = self.player_selection
            && selection.has_switched()
            && let Some(player) = selection.initial()
        {
            return MobEvent::ForPlayer {
                conn: player.connection,
                event: Box::new(event),
            };
        }
        event
    }

    /// Keep a mount's rider as the recipient even when a target-change hook
    /// redirects the mount to a different player later in the same tick.
    /// Before any switch the existing allocation-free routing is unchanged.
    pub fn route_rider_event(&self, mount: u32, event: MobEvent) -> MobEvent {
        if event.targets_player()
            && let Some(selection) = self.player_selection
            && selection.has_switched()
            && let Some(rider) = selection.rider(mount).or_else(|| selection.initial())
        {
            return MobEvent::ForPlayer {
                conn: rider.connection,
                event: Box::new(event),
            };
        }
        event
    }

    /// Dispatch a proposed AI target change, validating a substituted target.
    pub fn change_target(
        &self,
        mob: &mut Mob,
        is_brain: bool,
        target: Option<Who>,
    ) -> TargetChange {
        let Some(hooks) = self.mob_hooks else {
            return TargetChange::Set(target);
        };
        match hooks.living_change_target(mob, self, is_brain, target) {
            TargetChange::Set(Some(who))
                if Some(who) != target && self.target_info(who).is_none() =>
            {
                TargetChange::Keep
            }
            change => change,
        }
    }
    /// Whether a mob may grief here at all: the rule, or a mod that may say
    /// yes against it.
    pub fn griefing_possible(&self) -> bool {
        self.mob_griefing || self.mob_hooks.is_some_and(|hooks| hooks.grief_hooked())
    }
    /// `EventHooks.canEntityGrief(level, mob)`: the `mob_griefing` rule, or
    /// what a mod answers for this mob (`native_entity_mob_griefing`); the
    /// places vanilla reads the rule with no entity still read `mob_griefing`.
    pub fn can_grief(&self, mob: u32) -> bool {
        match self.mob_hooks {
            Some(hooks) => hooks.can_grief(mob, self.mob_griefing),
            None => self.mob_griefing,
        }
    }
    /// `LivingEntity.getVisibilityPercent(level, looking)` of the player `p`:
    /// `visibility` holds vanilla's factors; NeoForge's `LivingVisibilityEvent`
    /// scales it last, before the clamp to 0..=10.
    pub fn player_visibility(&self, p: &PlayerView, looking: u32, visibility: f64) -> f64 {
        let visibility = match self.mob_hooks {
            Some(hooks) => hooks.living_visibility(p, looking, visibility),
            None => visibility,
        };
        visibility.clamp(0.0, 10.0)
    }
}
/// `Level.getEntity(UUID)` for a reference a mob stored: the entity that UUID
/// names while it is in the level, never some other entity as a stand-in. Only
/// the game layer keeps entity UUIDs, so it answers this (`Game::who_by_uuid`)
/// and the mob layer reaches it through `MobWorld`.
/// A mob outside this thread's part of the mob loop, as the serial loop would see it now.
pub trait ForeignSeen {
    fn seen(&self, id: u32) -> Option<&Seen>;
}
pub trait WhoLookup {
    fn who_by_uuid(&self, uuid: [i32; 4]) -> Option<Who>;
}
/// `Entity.getTeam` of the player and of the mobs on a team.
#[derive(Clone, Debug, Default)]
pub struct Teams<'a> {
    pub player: Option<&'a crate::scoreboard::Team>,
    pub mobs: HashMap<u32, &'a crate::scoreboard::Team>,
}
impl<'a> Teams<'a> {
    pub fn of(&self, who: Who) -> Option<&'a crate::scoreboard::Team> {
        match who {
            Who::Player => self.player,
            Who::Mob(id) => self.mobs.get(&id).copied(),
        }
    }
}
/// `InfestedMobEffect.onMobHurt`'s draws from the bearer's random
/// (`mob.getRandom()`): a 10% chance of one or two silverfish, each thrown
/// along `look` (`spawnSilverfish`). Their velocities; none when the chance
/// fails.
pub fn infested_silverfish(random: &mut LegacyRandom, look: [f64; 3]) -> Vec<Vec3> {
    if random.next_float() > 0.1 {
        return Vec::new();
    }
    let count = 1 + random.next_int_bound(2).unwrap_or(0);
    (0..count)
        .map(|_| {
            // Mth.randomBetween(-PI / 2, PI / 2), then in floats (JOML
            // Vector3f): the look direction `.mul(0.3f).mul(1, 1.5f, 1)`,
            // `.rotateY(angle)`.
            let half = std::f32::consts::FRAC_PI_2;
            let angle = random.next_float() * (2.0 * half) - half;
            let sin = horizon_assets::math::joml_sin(angle);
            let cos = horizon_assets::math::cos_from_sin(sin, angle);
            let x = look[0] as f32 * 0.3;
            let y = look[1] as f32 * 0.3 * 1.5;
            let z = look[2] as f32 * 0.3;
            Vec3::new(
                f64::from(x * cos + z * sin),
                f64::from(y),
                f64::from(-x * sin + z * cos),
            )
        })
        .collect()
}
/// `Entity.random` of a client's copy of entity `id` first seen at `position`: vanilla's is its
/// own (`RandomSource.create()`), drawn on by nothing else; this one is seeded from the two.
pub fn entity_random(id: u32, position: Vec3) -> LegacyRandom {
    let mut random = LegacyRandom::new(
        i64::from(id).wrapping_mul(0x5DEE_CE66D)
            ^ (position.x.to_bits() as i64)
            ^ (position.z.to_bits() as i64).rotate_left(17),
    );
    let _ = random.next_int();
    random
}
/// The own random (`Entity.random`) of a living entity the server makes, as
/// `LivingEntity.<init>` leaves it: its first two draws made the UUID
/// (`Mth.createInsecureUUID`), returned here, then the kind's constructor draws its synced data and yaw in `constructor_draws`.
pub fn living_random() -> (LegacyRandom, [i32; 4]) {
    let mut random = crate::random_source::entity();
    let uuid = crate::save::insecure_uuid(&mut random);
    (random, uuid)
}
impl Mob {
    /// A mob as its authoritative side builds it — spawned, or loaded from a
    /// chunk (`Mob.finalizeSpawn` runs around it): the species' own goals, then
    /// the goals every mod part attached to its type (#1077), vanilla's
    /// `registerGoals` moment.
    pub fn new(id: u32, kind: Arc<MobType>, position: Vec3, yaw: f32) -> Self {
        Self::with_random(id, kind, position, yaw, entity_random(id, position))
    }
    pub fn with_random(
        id: u32,
        kind: Arc<MobType>,
        position: Vec3,
        yaw: f32,
        random: LegacyRandom,
    ) -> Self {
        let mut mob = Self::remote_with_random(id, kind, position, yaw, random);
        // The parts a mod attached to this entity type add their goals right
        // after the species' own (#1077): vanilla's `registerGoals` moment,
        // which belongs to the authoritative construction. A type with no
        // brain is told so at the same moment (#824); a brain-driven type
        // registers its behaviors at the brain's own construction.
        part_goals::attach(&mut mob);
        mod_behaviors::attach_at_construction(&mob);
        mob
    }
    /// The same construction without the mod parts' goals: a client's copy of
    /// a server's mob, from its spawn packet. The species' goals stand — the
    /// client's own reads of its copy ask them — but a mod part's
    /// `register_goals` runs nowhere here: the authoritative side already ran
    /// it for the mob itself, and a copy's would only allocate goal state no
    /// tick runs (#1077).
    pub fn new_remote(id: u32, kind: Arc<MobType>, position: Vec3, yaw: f32) -> Self {
        Self::remote_with_random(id, kind, position, yaw, entity_random(id, position))
    }
    fn remote_with_random(
        id: u32,
        kind: Arc<MobType>,
        position: Vec3,
        yaw: f32,
        random: LegacyRandom,
    ) -> Self {
        let mut mob = Self {
            emote: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
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
            needs_sync: false,
            pushed: Vec::new(),
            sync_position: false,
            gear_knockback_resistance: None,
            hurt_time: 0,
            invulnerable: 0,
            invulnerable_time: 0,
            death_time: 0,
            impulse_grace_time: 0,
            age: 0,
            limb_position: 0.0,
            limb_speed: 0.0,
            previous_limb_speed: 0.0,
            swell: 0,
            previous_swell: 0,
            max_swell: 30,
            // Mob.ambientSoundTime starts at 0.
            ambient_timer: 0,
            fall_distance: 0.0,
            stuck_multiplier: Vec3::ZERO,
            piston: Default::default(),
            persistent: false,
            random,
            ai: Box::default(),
            extra: Extra::default(),
            fluids: living::Fluids::default(),
            was_eye_in_water: false,
            first_tick: true,
            no_gravity: false,
            no_physics: false,
            supporting: None,
            standing_on: None,
            no_ai: false,
            silent: false,
            invulnerable_flag: false,
            attacker_aim: None,
            projectile_motion: None,
            climbing: false,
            on_climbable: false,
            fire_ticks: 0,
            ticks_frozen: 0,
            in_powder_snow: false,
            glowing: false,
            air: 300,
            last_hurt: 0.0,
            absorption: 0.0,
            dealt_absorbed: 0,
            last_damage: None,
            accepted_hit: None,
            death_canceled: false,
            last_direct: None,
            combat: combat::Tracker::default(),
            removed: false,
            despawn: mod_api::Despawn::Default,
            custom_name: None,
            custom_name_text: None,
            name_visible: false,
            invisible: false,
            effects_dirty: true,
            anim: anim::AnimState::default(),
            rider: None,
            outbox: Vec::new(),
            skin: None,
            slim: false,
            skin_3d: None,
            cape: None,
            profile: None,
            random_textures: Default::default(),
            size_answer: None,
            attrs: Attrs {
                speed: kind.speed,
                attack_damage: kind.attack_damage,
                attack_knockback: kind.attack_knockback,
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
        // Salmon.defineSynchedData: DATA_TYPE starts at Variant.DEFAULT (MEDIUM,
        // id 1), so a salmon finalizeSpawn never sized keeps the type's box.
        if mob.species() == Species::Salmon {
            mob.extra.variant = 1;
        }
        zombie_villager::constructor_draws(&mut mob);
        neutral_anger::init(&mut mob);
        // `Creeper.explosionRadius = 3` until a save or a command sets it.
        mob.extra.explosion_radius = 3;
        // The PathNavigation constructor: the base follow range.
        mob.ai.nav.max_visited_nodes = (mob.kind.follow_range * 16.0).floor() as i32;
        ai::register(&mut mob);
        mob
    }
    /// `Mob.canPickUpLoot`, with the overrides: an allay that holds
    /// something off its pickup cooldown, a sulfur cube with no block.
    pub fn can_pick_up_loot(&self) -> bool {
        match self.species() {
            Species::Allay => allay::can_pick_up_loot(self),
            Species::SulfurCube => !sulfur_cube::has_body(self),
            _ => self.extra.pick_up_loot,
        }
    }
    #[inline]
    /// `LivingEntity.getHitbox`: a rider's box starts no lower than its seat
    /// (`vehicle.getPassengerRidingPosition(this).y`).
    pub fn hitbox(&self) -> Aabb {
        let b = self.bounds();
        match self.extra.seat.filter(|s| self.extra.mount == Some(s.0)) {
            Some((_, offset, _)) => {
                let (lo, hi) = (b.min_position(), b.max_position());
                let y = (self.position.y + offset).max(lo.y);
                Aabb::new(Vec3::new(lo.x, y, lo.z), hi)
            }
            None => b,
        }
    }
    /// `Mob.getAttackBoundingBox(range)`: its box, widened to its vehicle's
    /// (`vehicle`, when it rides) on x and z, grown by `range` on x and z.
    pub fn attack_box(&self, range: f64, vehicle: Option<Aabb>) -> Aabb {
        let b = self.bounds();
        let (mut lo, mut hi) = (b.min_position(), b.max_position());
        if let Some(vehicle) = vehicle.filter(|_| self.extra.mount.is_some()) {
            let (vlo, vhi) = (vehicle.min_position(), vehicle.max_position());
            lo = Vec3::new(lo.x.min(vlo.x), lo.y, lo.z.min(vlo.z));
            hi = Vec3::new(hi.x.max(vhi.x), hi.y, hi.z.max(vhi.z));
        }
        Aabb::new(lo, hi).inflate(Vec3::new(range, 0.0, range))
    }
    /// `positionRider` seated it on vehicle `id`: its own vehicle attachment
    /// and the vehicle's box, good while it rides that vehicle.
    pub fn seated(&mut self, id: u32, offset: f64, vehicle: Aabb) {
        self.extra.seat = Some((id, offset, vehicle));
    }
    pub fn bounds(&self) -> Aabb {
        self.bounds_at(self.position)
    }
    /// `Entity.makeBoundingBox(position)`.
    pub fn bounds_at(&self, position: Vec3) -> Aabb {
        // Shulker.makeBoundingBox: the lid's lift, not the dimensions.
        if self.kind.species == Species::Shulker {
            return shulker::bounds_at(self, position);
        }
        // `width()` and `height()` in one `dims()`: every box asked for (each
        // collision, push and cart test) read the whole size twice.
        let (width, height, _) = self.dims();
        let w = f64::from(width as f32) / 2.0;
        Aabb::new(
            Vec3::new(position.x - w, position.y, position.z - w),
            Vec3::new(
                position.x + w,
                position.y + f64::from(height as f32),
                position.z + w,
            ),
        )
    }
    /// `CombatTracker.recordDamage`: a hit that took `damage` health, with
    /// the fall it ends.
    pub fn record_damage(&mut self, source: Damage, damage: f32) {
        let entry = combat::Entry {
            source,
            damage,
            in_water: self.fluids.in_water,
            fall_distance: self.fall_distance as f32,
        };
        self.combat.record(entry, self.age);
    }
    #[inline]
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    /// `Entity.isInvulnerable` (#916): the permanent `Invulnerable` flag or
    /// the temporary window `invulnerableTime > 0` — what damage refusal
    /// (`isInvulnerableToBase`) and enemy targeting (`canBeSeenAsEnemy`)
    /// both read.
    pub fn is_invulnerable(&self) -> bool {
        self.invulnerable_flag || self.invulnerable_time > 0
    }
    pub fn hostile(&self) -> bool {
        self.kind.category == "monster"
    }
    /// `instanceof Enemy`. The zombie horse, camel husk and zombie nautilus are in
    /// the monster category but extend `Animal`/`AbstractNautilus`, which do
    /// not implement `Enemy`.
    pub fn is_enemy(&self) -> bool {
        self.hostile()
            && !matches!(
                self.species(),
                Species::ZombieHorse | Species::CamelHusk | Species::ZombieNautilus
            )
    }
    #[inline]
    pub fn species(&self) -> Species {
        self.kind.species
    }
    /// Entity.getMaxAirSupply and the aquatic species' overrides.
    pub fn max_air_supply(&self) -> i32 {
        match self.species() {
            Species::Axolotl => axolotl::MAX_AIR,
            Species::Dolphin => 4800,
            _ => 300,
        }
    }
    /// `Mob.getMainArm`: the left arm when the mob is left-handed, a player's
    /// `mainHand` option for a player mob.
    pub fn main_arm_left(&self) -> bool {
        self.extra.main_hand_left || self.extra.left_handed
    }
    /// The player hit this mob (melee): LivingEntity.hurt with knockback.
    pub fn hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        amount: f32,
        from: Option<[f64; 3]>,
    ) -> bool {
        let source = match from {
            Some(f) => Damage::player(f),
            None => Damage::of(DamageKind::Generic),
        };
        self.damage(level, amount, source)
    }
    /// LivingEntity.hurtServer: invulnerability frames, armor, knockback and
    /// the last-hurt-by bookkeeping goals read.
    /// `damage` of a blow whose causing entity is `aim` (`SulfurCube.knockback` reads it).
    pub fn damage_aimed(
        &mut self,
        level: &mut DamageLevel<'_>,
        amount: f32,
        source: Damage,
        aim: Option<Aim>,
    ) -> bool {
        self.attacker_aim = aim;
        let hurt = self.damage(level, amount, source);
        self.attacker_aim = None;
        hurt
    }
    /// `hurtServer` of `amount` from `source` in `level`.
    pub fn damage(&mut self, level: &mut DamageLevel<'_>, amount: f32, source: Damage) -> bool {
        self.damage_with(level, amount, source, None)
    }
    /// `LivingEntity.isInvulnerableTo` and dead or removed: only #bypasses_invulnerability gets
    /// past the flag. Fire-immune mobs also refuse fire, except a ghast's reflected fireball.
    /// The guard every damage hook runs after (`LivingIncomingDamageEvent`).
    pub fn refuses_damage(&self, source: &Damage) -> bool {
        self.extra.client_side
            || !self.alive()
            || self.removed
            || (self.is_invulnerable() && !source.kind.bypasses_invulnerability())
            || (source.is_fire() && self.kind.flags.fire_immune && !self.reflected_fireball(source))
    }
    /// `Ghast.isReflectedFireball`: its fire immunity does not refuse a
    /// player's return hit.
    fn reflected_fireball(&self, source: &Damage) -> bool {
        &*self.kind.id == "minecraft:ghast"
            && source.kind == DamageKind::Fireball
            && source.attacker == Some(Who::Player)
    }
    /// `LivingEntity.hurtServer`'s partial mob adapter, with health reductions inline.
    /// `damage` by a source with a weapon: `armor_effectiveness` runs the
    /// weapon's armor-effectiveness enchantments (breach) over the armor fraction, with the
    /// level's random.
    pub fn damage_with(
        &mut self,
        level: &mut DamageLevel<'_>,
        amount: f32,
        source: Damage,
        armor_effectiveness: Option<&mut dyn FnMut(f32, &mut LegacyRandom) -> f32>,
    ) -> bool {
        // Enderman.hurtServer: past `isInvulnerableTo`, the teleport draw
        // follows `super.hurtServer` whatever it answered (a hurt cooldown, a
        // dead enderman), its hurt sound's pitch included.
        let enderman = self.species() == Species::Enderman;
        let mut passed_invulnerability = false;
        let hurt = self.hurt_server(
            level,
            amount,
            source,
            armor_effectiveness,
            &mut passed_invulnerability,
        );
        if enderman && passed_invulnerability {
            enderman::after_hurt(self, &source);
        }
        hurt
    }
    fn hurt_server(
        &mut self,
        level: &mut DamageLevel<'_>,
        amount: f32,
        source: Damage,
        armor_effectiveness: Option<&mut dyn FnMut(f32, &mut LegacyRandom) -> f32>,
        passed_invulnerability: &mut bool,
    ) -> bool {
        axolotl::before_damage(self, amount, &source);
        raider::note_hurt(self);
        // SulfurCube.hurtServer: the priming and a block's immune knockback
        // come before LivingEntity's checks, so a primed (invulnerable) cube
        // is still knocked about.
        if self.species() == Species::SulfurCube
            && !sulfur_cube::accepts_damage(self, level, &source, amount)
        {
            // `SulfurCube.hurtServer` returns true for the blows it shrugs off and only knocks
            // about, so `Player.attack` goes on with its extra knockback and its aftermath.
            return sulfur_cube::immune_to(&source);
        }
        // `Entity.hurt` on a client level does nothing: a client copy refuses before
        // any invulnerability observation or hook, as `refuses_damage` does for the rest.
        if self.extra.client_side {
            return false;
        }
        let reflected = self.reflected_fireball(&source);
        if self.invulnerable_to_attempt(level, &source) {
            return reflected;
        }
        *passed_invulnerability = true;
        if !self.alive() {
            return false;
        }
        if !self.attacker_allows_damage(&source)
            || !ai::accepts_damage(self, &source, amount, level.attack())
        {
            return false;
        }
        // LivingEntity.hurtServer refuses fire before hooks and damage processing.
        if source.is_fire() && self.has_effect("minecraft:fire_resistance") {
            // Ghast.hurtServer reports success for a reflected fireball even
            // when LivingEntity refuses its health damage.
            return reflected;
        }
        // IronGolem.hurtServer: the crack level before the damage, compared after.
        let crackiness = (self.species() == Species::IronGolem).then(|| golem::crackiness(self));
        let mut amount = if reflected { 1000.0 } else { amount };
        if self.species() == Species::Armadillo {
            amount = armadillo::reduce_damage(self, amount);
        }
        // LivingEntity.hurtServer, past its guards: the hit's damage sequence starts, and
        // `LivingIncomingDamageEvent` may change or cancel it.
        let mut container = DamageContainer::new(amount);
        let victim = self.victim();
        if level.incoming_canceled(
            crate::health::Hurting::Mob(self, &source),
            victim,
            &mut container,
        ) {
            // Ghast.hurtServer: true for a reflected fireball whatever
            // super.hurtServer did.
            return reflected;
        }
        amount = container.new_damage();
        if amount < 0.0 {
            amount = 0.0;
        }
        // LivingEntity.applyItemBlocking: no mob raises a blocking item, but a native
        // mod may block for it.
        let blocked = if amount > 0.0 {
            level
                .shield_block(
                    crate::health::Hurting::Mob(self, &source),
                    amount,
                    crate::health::no_block(amount),
                )
                .map_or(0.0, |b| b.blocked_damage)
        } else {
            0.0
        };
        amount -= blocked;
        container.record_block(blocked);
        // #damages_helmet: falling anvils, blocks and stalactites.
        if matches!(
            source.kind,
            DamageKind::FallingBlock | DamageKind::FallingAnvil | DamageKind::FallingStalactite
        ) && self.extra.head_id().is_some()
        {
            amount *= 0.75;
        }
        // LivingEntity.hurtServer: `#freeze_hurts_extra_types` take five times.
        if source.kind == DamageKind::Freeze
            && matches!(
                self.species(),
                Species::Strider | Species::Blaze | Species::MagmaCube
            )
        {
            amount *= 5.0;
        }
        if !amount.is_finite() {
            amount = f32::MAX;
        }
        // LivingEntity.hurtServer: inside the hurt cooldown a hit is compared
        // with `lastHurt` by this incoming amount, before armor and Resistance,
        // and `actuallyHurt` gets only what it adds.
        let incoming = amount;
        let cooldown = self.invulnerable as f32 > 10.0;
        if cooldown {
            if incoming <= self.last_hurt {
                return false;
            }
            amount = incoming - self.last_hurt;
        }
        // LivingEntity.getDamageAfterArmorAbsorb asks `hurtArmor` *before*
        // `CombatRules.getDamageAfterAbsorb`, so a worn piece wears by the
        // damage the armour, toughness and enchantment reductions have not
        // touched yet, and only a hit outside `#bypasses_armor` asks at all.
        let armor_hit = (!source.bypasses_armor() && amount > 0.0).then_some(amount);
        let entered = amount;
        let armor_ran = !source.bypasses_armor();
        if armor_ran {
            let (armor, armor_toughness) = armor::values(self);
            if armor > 0.0 {
                // CombatRules.getDamageAfterAbsorb
                let toughness = 2.0 + armor_toughness / 4.0;
                let effective = (armor - amount / toughness).clamp(armor * 0.2, 20.0);
                let mut fraction = effective / 25.0;
                if let Some(modify) = armor_effectiveness {
                    fraction = modify(fraction, level.random()).clamp(0.0, 1.0);
                }
                amount *= 1.0 - fraction;
            }
        }
        amount = container.reduce(
            Reduction::Armor,
            entered,
            entered - amount,
            amount,
            armor_ran,
        );
        let after_armor = amount;
        // LivingEntity.getDamageAfterMagicAbsorb: Resistance takes 20% per level off
        // (`#bypasses_resistance` and `#bypasses_effects` skip it) ...
        let mut resisted = false;
        if !source.kind.bypasses_effects()
            && !source.kind.bypasses_resistance()
            && let Some(&StoredEffect { amplifier: amp, .. }) = self
                .extra
                .effects
                .iter()
                .find(|e| e.id == "minecraft:resistance")
        {
            let remainder = 25i32.wrapping_sub(amp.wrapping_add(1).wrapping_mul(5));
            amount = (amount * remainder as f32 / 25.0).max(0.0);
            resisted = true;
            // actuallyHurt is void: zero health damage still lets hurtServer set hurt state.
        }
        amount = container.reduce(
            Reduction::MobEffects,
            after_armor,
            after_armor - amount,
            amount,
            resisted,
        );
        let after_effects = amount;
        // ... then, with damage left and outside `#bypasses_enchantments` (a
        // warden's sonic boom), CombatRules.getDamageAfterMagicAbsorb's
        // enchantment protection (#11308).
        let enchanted = amount > 0.0 && !source.kind.bypasses_enchantments();
        if enchanted {
            let epf = gear::protection(self, source.kind);
            if epf > 0.0 {
                amount *= 1.0 - epf / 25.0;
            }
        }
        amount = container.reduce(
            Reduction::Enchantments,
            after_effects,
            after_effects - amount,
            amount,
            enchanted,
        );
        let after_enchantments = amount;
        // Witch.getDamageAfterMagicAbsorb: none from its own doing, else
        // 85% less from `#witch_resistant_to`.
        let witch = self.species() == Species::Witch;
        if witch {
            if source.attacker == Some(Who::Mob(self.id)) {
                amount = 0.0;
            } else if matches!(
                source.kind,
                DamageKind::Magic
                    | DamageKind::IndirectMagic
                    | DamageKind::SonicBoom
                    | DamageKind::Thorns
            ) {
                amount *= 0.15;
            }
        }
        amount = container.reduce(
            Reduction::InnateResistance,
            after_enchantments,
            after_enchantments - amount,
            amount,
            witch,
        );
        // Wolf.actuallyHurt: wolf armor takes the damage instead of the wolf.
        let absorbed = self.wolf_armor_absorbs(&source);
        let mut knock = true;
        let window = cooldown;
        container.record_should_cause_side_effects(!window);
        if window {
            container.record_reduction(Reduction::Invulnerability, self.last_hurt);
        }
        let mut dealt = amount;
        // The cooldown the hit leaves: what is left of it within it, else vanilla's 20 or
        // what `LivingIncomingDamageEvent` set.
        if window {
            container.set_post_attack_invulnerability_ticks(self.invulnerable);
        }
        // Wolf.actuallyHurt keeps a hit its armor takes away from LivingEntity.actuallyHurt.
        if !absorbed {
            container.enter_pre(dealt);
            level.damage_pre(crate::health::Hurting::Mob(self, &source), &mut container);
            dealt = container.capture_inflicted();
        }
        let mut through = 0.0;
        if absorbed {
            self.hurt_wolf_armor(dealt);
        } else {
            through = self.soak_hit(dealt, &source, &container);
            container.record_reduction(Reduction::Absorption, dealt - through);
            self.health -= through;
        }
        // `lastHurt` takes what Pre left: vanilla's value plus what Pre
        // added, so a hit no listener changes keeps vanilla's.
        self.last_hurt = incoming + container.pre_change();
        self.invulnerable = container.post_attack_invulnerability_ticks();
        if window {
            knock = false;
        } else {
            self.hurt_time = 10;
        }
        // LivingEntity.actuallyHurt: a hit that reaches the health is
        // recorded (`CombatTracker.recordDamage`), with the fall it ends;
        // Wolf.actuallyHurt keeps a hit its armor takes out of it.
        if !absorbed && dealt != 0.0 {
            self.record_damage(source, dealt);
        }
        container.record_health_damage(through);
        // LivingEntity.hurtArmor: `Horse` is the only mob that overrides it, and
        // it hands `doHurtEquipment` the body slot alone. A wolf's armour has
        // its own rule (`Wolf.actuallyHurt`, `hurt_wolf_armor`) and a player's
        // four pieces theirs, so the other "body slot" species — donkey, mule,
        // llama, camel, the skeleton and zombie horses — wear nothing.
        if let Some(damage) = armor_hit
            && self.species() == Species::Horse
            && self.extra.body_id().is_some()
        {
            self.outbox.push(MobEvent::ArmorHurt {
                mob: self.id,
                amount: damage,
                source: crate::health::mob_source(&source, level.attack()),
            });
        }
        // Animal.actuallyHurt: damage taken ends love mode (`resetLove`).
        if drops::is_animal(self.species()) {
            self.extra.in_love = 0;
        }
        // A hit blocked whole is not a success: vanilla records the source and runs the
        // effects' `onMobHurt` only for a success.
        let success = blocked <= 0.0 || amount > 0.0;
        if success {
            self.last_damage = Some((source, i64::from(self.age)));
        }
        self.last_direct = None;
        if let Some(who) = source.attacker {
            self.ai.hurt_by(who, self.age as i32);
            if who == Who::Player {
                self.ai.last_hurt_by_player = 100;
            }
        }
        // dealDefaultKnockback, unless the damage type is tagged no_knockback or
        // the hit was blocked whole; a source without a position knocks in a
        // random direction.
        if knock && !source.no_knockback() && !(blocked > 0.0 && amount <= 0.0) {
            // dealDefaultKnockback: a projectile knocks the way it travelled
            // (`calculateHorizontalHurtKnockbackDirection`), else from the source's position.
            let (dx, dz) = match self.projectile_motion {
                Some([x, z]) => (-x, -z),
                None => source.from.map_or((0.0, 0.0), |f| {
                    (f[0] - self.position.x, f[2] - self.position.z)
                }),
            };
            self.knockback(level, f64::from(0.4f32), dx, dz, &source, amount);
        }
        ai::after_hurt(self, &source);
        if self.health <= 0.0 {
            self.health = 0.0;
            // `LivingEntity.hurtServer`: a killing blow is taken by a
            // `death_protection` item in either hand instead of dying.
            self.use_death_protection(level, &source);
        }
        self.remember_accepted_hit(level, &source, success);
        if knock {
            // playHurtSound (Mob: also resets the ambient sound clock) or the
            // death sound: makeSound draws the voice pitch whenever there is a sound.
            let dead = !self.alive();
            // Wolf.getHurtSound: the armor's own sound when it takes the hit.
            let sound = if absorbed {
                "item.wolf_armor.damage"
            } else {
                entity_sounds::voices(self)[if dead { 2 } else { 1 }]
            };
            if !dead {
                self.ambient_timer = -self.ambient_interval();
            }
            if !sound.is_empty() {
                let pitch = self.voice_pitch();
                if !self.silent {
                    let at = [self.position.x, self.position.y, self.position.z];
                    self.outbox
                        .push(MobEvent::Sound(sound, at, ai::sound_volume(self), pitch));
                }
            }
            // playSecondaryHurtSound: a thorns hit sounds where the mob
            // stands, silent or not.
            if source.kind == DamageKind::Thorns {
                let at = [self.position.x, self.position.y, self.position.z];
                self.outbox
                    .push(MobEvent::Sound("enchant.thorns.hit", at, 1.0, 1.0));
            }
        }
        // IronGolem.hurtServer: a new crack level plays the damage sound.
        if let Some(before) = crackiness
            && before != golem::crackiness(self)
            && !self.silent
        {
            let at = [self.position.x, self.position.y, self.position.z];
            self.outbox
                .push(MobEvent::Sound("entity.iron_golem.damage", at, 1.0, 1.0));
        }
        // LivingEntity.hurtServer ends with MobEffectInstance.onMobHurt, so
        // Infested draws from the mob's random here, before a subclass's own
        // draws after `super.hurtServer` (an enderman's or a shulker's
        // teleport), and its silverfish come out where the mob was hurt.
        if success && self.has_effect("minecraft:infested") {
            let look = self.view_vector();
            let velocities = infested_silverfish(&mut self.random, look);
            if !velocities.is_empty() {
                let at = [
                    self.position.x,
                    self.position.y + f64::from(self.height()) / 2.0,
                    self.position.z,
                ];
                self.outbox.push(MobEvent::Infested { at, velocities });
            }
        }
        // The last of `LivingEntity.hurtServer`: `LivingDamageEvent.Post`, health taken or not.
        level.damage_post(crate::health::Hurting::Mob(self, &source), &container);
        // Shulker.hurtServer: the low-health teleport roll after `super.hurtServer`.
        if success && self.species() == Species::Shulker {
            shulker::after_hurt(self);
        }
        // Squid.hurtServer: a squid hurt after a living entity hit it squirts ink.
        if matches!(self.species(), Species::Squid | Species::GlowSquid)
            && self.ai.last_hurt_by.is_some()
        {
            aquatic::spawn_ink(self);
        }
        // AbstractHorse.hurtServer: the rear roll after the hurt.
        if equine::is_equine(self.species()) {
            equine::after_hurt_server(self);
        }
        // GlowSquid.hurtServer dims the squid only when Squid.hurtServer took the
        // hit, which it reports only for a living entity's blow: environmental
        // damage (suffocation, a campfire, drying out) leaves the glow alone.
        if self.species() == Species::GlowSquid && self.ai.last_hurt_by.is_some() {
            aquatic::hurt_server(self);
        }
        // Squid.hurtServer (also GlowSquid's result): parent health damage has
        // happened, but without a last living attacker the subclass returns false.
        success
            && (!matches!(self.species(), Species::Squid | Species::GlowSquid)
                || self.ai.last_hurt_by.is_some())
    }
    /// `damage` by a source whose direct entity is `direct` (an entity type id), which `source`
    /// cannot carry: `DamageSources.indirectMagic(potion or cloud, owner)`.
    pub fn damage_by(
        &mut self,
        level: &mut DamageLevel<'_>,
        amount: f32,
        source: Damage,
        direct: &'static str,
    ) -> bool {
        // Enderman.hurtServer: only a thrown potion skips the teleport draw;
        // a lingering cloud's `indirect_magic` goes through it.
        let cloud = self.species() == Species::Enderman
            && direct == "minecraft:area_effect_cloud"
            && !self.removed
            && !(self.is_invulnerable() && !source.kind.bypasses_invulnerability());
        let hurt = self.damage(level, amount, source);
        if cloud {
            enderman::teleport_chance(self, &source);
        }

        if hurt {
            self.last_direct = Some(direct);
        }
        hurt
    }
    /// `Wolf.canArmorAbsorb`: it wears wolf armor and the damage type is not
    /// in `#bypasses_wolf_armor` (drowning, magic, thorns, in a wall, out of
    /// the world and the untyped kills).
    fn wolf_armor_absorbs(&self, source: &Damage) -> bool {
        self.species() == Species::Wolf
            && self.extra.body_id() == Some("minecraft:wolf_armor")
            && !matches!(
                source.kind,
                DamageKind::Drown
                    | DamageKind::Magic
                    | DamageKind::IndirectMagic
                    | DamageKind::InWall
                    | DamageKind::Thorns
                    | DamageKind::OutOfWorld
                    | DamageKind::Cramming
                    | DamageKind::Generic
                    | DamageKind::GenericKill
                    | DamageKind::OutsideBorder
                    | DamageKind::Starve
                    | DamageKind::Wither
            )
    }
    /// `Wolf.actuallyHurt`: the armor loses `ceil(damage)` durability, cracks
    /// at `Crackiness.WOLF_ARMOR` steps and breaks at its maximum.
    fn hurt_wolf_armor(&mut self, damage: f32) {
        let crack = |lost: i32| {
            let left = (WOLF_ARMOR_DURABILITY - lost) as f32 / WOLF_ARMOR_DURABILITY as f32;
            [0.95_f32, 0.69, 0.32].iter().filter(|t| left < **t).count()
        };
        let before = self.extra.body_damage();
        let after = before + damage.ceil() as i32;
        let at = [self.position.x, self.position.y, self.position.z];
        if after >= WOLF_ARMOR_DURABILITY {
            self.extra.set_body_stack(None);
            self.outbox
                .push(MobEvent::Sound("item.wolf_armor.break", at, 1.0, 1.0));
        } else {
            self.extra.set_body_damage(after);
            if crack(before) != crack(after) {
                self.outbox
                    .push(MobEvent::Sound("item.wolf_armor.crack", at, 1.0, 1.0));
                // Twenty scutes fly off, a block above the wolf's feet.
                self.outbox.push(MobEvent::ItemParticles {
                    item: "minecraft:armadillo_scute",
                    at: [at[0], at[1] + 1.0, at[2]],
                    spread: [0.2, 0.1, 0.2],
                    speed: 0.1,
                    count: 20,
                });
            }
        }
    }
    /// One game tick. Returns false once the mob should be removed.
    pub fn tick(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
    ) -> bool {
        self.prepare_tick();
        neutral_anger::around_tick(self, w.game_time, |m| ai::tick(m, w, level, events))
    }
    /// A game-owned tick may pause before fox consumption to release its loans.
    pub fn tick_until_consumption(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
    ) -> ai::TickProgress {
        self.prepare_tick();
        neutral_anger::tick_until_consumption(self, w, level, events)
    }
    fn prepare_tick(&mut self) {
        self.track_section();
        // LivingEntity.tick: CombatTracker.recheckStatus.
        let alive = self.alive();
        self.combat.recheck(alive, self.age);
        self.extra.stuck.tick();
    }
    /// The section the mob is in, noted when it changes (`EntitySection` lists).
    fn track_section(&mut self) {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let section = [
            (self.position.x.floor() as i32) >> 4,
            (self.position.y.floor() as i32) >> 4,
            (self.position.z.floor() as i32) >> 4,
        ];
        if self.extra.section_seq == 0 || section != self.extra.section {
            self.extra.section = section;
            self.extra.section_seq = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
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
        let mut v = strider::movement_speed(self, witch::speed(self, self.attrs.speed))
            * self.speed_bonus();
        // LivingEntity.SPEED_MODIFIER_SPRINTING: 0.3F, ADD_MULTIPLIED_TOTAL.
        if self.extra.sprinting {
            v *= 1.0 + f64::from(0.3f32);
        }
        for effect in &self.extra.effects {
            let amp = effect.amplifier;
            match effect.id {
                "minecraft:speed" => v *= 1.0 + 0.2 * f64::from(amp + 1),
                "minecraft:slowness" => v *= (1.0 - 0.15 * f64::from(amp + 1)).max(0.0),
                _ => {}
            }
        }
        v
    }
    pub fn has_effect(&self, effect: &str) -> bool {
        self.extra.effects.iter().any(|e| e.id == effect)
    }
    /// `LivingEntity.isCurrentlyGlowing`: the Glowing effect or the `Glowing` tag (on a
    /// server's client, `glowing` holds shared flag 6 instead).
    pub fn is_currently_glowing(&self) -> bool {
        self.glowing || self.has_effect("minecraft:glowing")
    }
    /// `Entity.isInvisible`: shared flag 5.
    pub fn is_invisible(&self) -> bool {
        self.invisible
    }
    /// Removes effects and schedules the effect-status update when needed.
    pub fn retain_effects(&mut self, mut keep: impl FnMut(&StoredEffect) -> bool) -> bool {
        let before = self.extra.effects.len();
        let mut boost_removed = false;
        let mut absorption_removed = false;
        self.extra.effects.retain(|e| {
            let keep = keep(e);
            boost_removed |= !keep && e.id == "minecraft:health_boost";
            absorption_removed |= !keep && e.id == "minecraft:absorption";
            keep
        });
        if absorption_removed {
            // LivingEntity.onAttributeUpdated clamps to the remaining maximum;
            // removing Absorption does not subtract the hearts it granted.
            self.absorption = self.absorption.min(armor::max_absorption(self));
        }
        let changed = self.extra.effects.len() != before;
        if changed {
            let effects = &self.extra.effects;
            self.extra
                .effect_chains
                .retain(|e| effects.iter().any(|x| x.id == e.id));
            self.extra
                .hidden_effects
                .retain(|e| effects.iter().any(|x| x.id == *e));
            self.extra
                .ambient_effects
                .retain(|e| effects.iter().any(|x| x.id == *e));
        }
        if boost_removed {
            finalize::health_boost_changed(self);
        }
        self.effects_dirty |= changed;
        changed
    }
    /// Loads an effect without applying its attribute modifiers a second time.
    pub fn load_effect(&mut self, effect: StoredEffect) {
        self.extra.effects.retain(|e| e.id != effect.id);
        self.extra.effects.push(effect);
        self.effects_dirty = true;
    }
    /// `LivingEntity.addEffect` of a `MobEffectInstance` with its flags: the effect as
    /// `add_effect` has it, and `MobEffectInstance.update`'s rules for the flags of one already
    /// there: it takes the new ambient flag when the new one is not ambient or the effect
    /// changed, and always the new visibility. Returns whether the active instance or its
    /// flags changed (`MobEffectInstance.update` does not count a hidden-chain-only update).
    pub fn add_effect_with(
        &mut self,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
        ambient: bool,
        visible: bool,
    ) -> bool {
        let effect = horizon_client_core::potions::namespaced(effect);
        let mut incoming = crate::effects::EffectInstance::new(effect, ticks, amplifier);
        incoming.ambient = ambient;
        incoming.visible = visible;
        self.add_effect_instance(effect, incoming)
    }

    /// Apply an already resolved effect instance through the same update path as content input.
    pub fn add_effect_instance(
        &mut self,
        effect: &'static str,
        mut incoming: crate::effects::EffectInstance,
    ) -> bool {
        // Player instances use short vanilla names; every mob chain uses its canonical name,
        // including a weaker incoming instance saved behind the active effect.
        incoming.id = effect;
        let index = self.extra.effects.iter().position(|e| e.id == effect);
        let mut active = if let Some(i) = index {
            let stored = &self.extra.effects[i];
            let mut active = crate::effects::EffectInstance {
                id: stored.id,
                registry_index: stored.registry_index,
                duration: stored.duration,
                amplifier: stored.amplifier,
                ambient: false,
                visible: true,
                show_icon: stored.show_icon,
                client_blend: (false, false),
                hidden: None,
            };
            active.ambient = self.extra.ambient_effects.contains(&effect);
            active.visible = !self.extra.hidden_effects.contains(&effect);
            if let Some(i) = self.extra.effect_chains.iter().position(|e| e.id == effect) {
                active.hidden = Some(Box::new(self.extra.effect_chains.remove(i)));
            }
            active
        } else {
            incoming.clone()
        };
        let changed = index.is_none() || active.update(&incoming);
        if let Some(hidden) = active.hidden.take() {
            self.extra.effect_chains.push(*hidden);
        }
        if let Some(i) = index {
            self.extra.effects[i] = StoredEffect::from_instance(effect, &active);
        } else {
            self.extra
                .effects
                .push(StoredEffect::from_instance(effect, &active));
        }
        self.extra.ambient_effects.retain(|h| *h != effect);
        if active.ambient {
            self.extra.ambient_effects.push(effect);
        }
        self.extra.hidden_effects.retain(|h| *h != effect);
        if !active.visible {
            self.extra.hidden_effects.push(effect);
        }
        if effect == "minecraft:absorption" {
            self.absorption = self
                .absorption
                .max(4.0 * (incoming.amplifier + 1) as f32)
                .clamp(0.0, armor::max_absorption(self));
        }
        if changed && effect == "minecraft:health_boost" {
            finalize::health_boost_changed(self);
        }
        self.effects_dirty |= changed;
        changed
    }
    /// [`Mob::soak_absorption`] for a hit from `source`: `LivingEntity.actuallyHurt`
    /// credits a `ServerPlayer` attacker with the absorbed damage in tenths.
    fn soak_hit(&mut self, damage: f32, source: &Damage, container: &DamageContainer) -> f32 {
        let through = self.soak_absorption(damage, container);
        let absorbed = damage - through;
        if absorbed > 0.0
            && absorbed < 3.402_823_5e37
            && matches!(source.attacker, Some(Who::Player))
        {
            self.dealt_absorbed = self
                .dealt_absorbed
                .saturating_add((absorbed * 10.0).round() as i32);
        }
        through
    }
    /// `LivingEntity.actuallyHurt`: the absorption amount takes the damage first;
    /// what is left reaches the health.
    /// A modifier registered for the absorption reduction (`container`) changes what the
    /// hearts take, within the damage and the hearts there are.
    pub fn soak_absorption(&mut self, damage: f32, container: &DamageContainer) -> f32 {
        let mut through = (damage - self.absorption).max(0.0);
        if let Some(taken) = container.modified_reduction(
            Reduction::Absorption,
            damage - through,
            damage.min(self.absorption),
        ) {
            through = damage - taken;
        }
        let left = self.absorption - (damage - through);
        // LivingEntity.setAbsorptionAmount clamps the remainder to the attribute maximum.
        self.absorption = if left < 0.0 {
            0.0
        } else {
            left.min(armor::max_absorption(self))
        };
        through
    }
    /// `LivingEntity.checkTotemDeathProtection`: the first hand with a
    /// `death_protection` stack uses one item up, the mob survives at 1 health
    /// with that component's `death_effects`, and entity event 35 tells the
    /// clients.
    fn use_death_protection(&mut self, level: &mut DamageLevel<'_>, source: &Damage) -> bool {
        // `DamageTypeTags.BYPASSES_INVULNERABILITY`: the void and /kill take it.
        if source.kind.bypasses_invulnerability() {
            return false;
        }
        // `getItemInHand`: the main hand first. The game reads the components
        // (`hand_protection`); before it has, the totem is the only item in the
        // shipped data with `minecraft:death_protection`.
        const TOTEM: &str = "minecraft:totem_of_undying";
        let (main, off) = match self.extra.hand_protection.clone() {
            Some(h) => (h.main, h.off),
            None => {
                let totem = || Some(living::totem_death_effects());
                (
                    (self.extra.main_hand_id() == Some(TOTEM))
                        .then(totem)
                        .flatten(),
                    (self.extra.off_hand_id() == Some(TOTEM))
                        .then(totem)
                        .flatten(),
                )
            }
        };
        let (slot, effects) = if let Some(effects) = main {
            (horizon_items::equipment::MAINHAND, effects)
        } else if let Some(effects) = off {
            (horizon_items::equipment::OFFHAND, effects)
        } else {
            return false;
        };
        if level.totem_canceled(
            crate::health::Hurting::Mob(self, source),
            slot == horizon_items::equipment::OFFHAND,
        ) {
            return false;
        }
        let remaining = self.extra.slot_stack(slot).cloned().and_then(|mut s| {
            s.count -= 1;
            (s.count > 0).then_some(s)
        });
        self.extra.set_slot_stack(slot, remaining);
        self.extra.hand_protection = None;
        self.health = 1.0;
        for effect in &effects {
            self.apply_consume_effect(level, effect);
        }
        let at = [self.position.x, self.position.y, self.position.z];
        self.outbox.push(MobEvent::EntityEvent {
            mob: self.id,
            event: 35,
            at,
        });
        true
    }
    /// One `ConsumeEffect` of death protection or a fox's consumed item.
    /// Damage uses the owning typed level. Teleporting and sounds belong to
    /// the consumption caller; mod-defined kinds are not applied here.
    pub fn apply_consume_effect(
        &mut self,
        level: &mut DamageLevel<'_>,
        effect: &crate::items::ConsumeEffect,
    ) {
        use crate::items::ConsumeEffect;
        match effect {
            // `LivingEntity.removeAllEffects` -> `onEffectsRemoved` ->
            // `refreshDirtyAttributes` -> `onAttributeUpdated(MAX_ABSORPTION)`:
            // the absorption amount is clamped to the remaining maximum.
            ConsumeEffect::ClearAllEffects => {
                let had_absorption = self.has_effect("minecraft:absorption");
                let had_boost = self.has_effect("minecraft:health_boost");
                self.extra.effect_chains.clear();
                self.extra.effects.clear();
                if had_absorption {
                    self.absorption = self.absorption.min(armor::max_absorption(self));
                }
                if had_boost {
                    finalize::health_boost_changed(self);
                }
            }
            ConsumeEffect::RemoveEffects(ids) => {
                let had_absorption = self.has_effect("minecraft:absorption");
                let had_boost = self.has_effect("minecraft:health_boost");
                self.extra.effects.retain(|e| {
                    let held = e.id.trim_start_matches("minecraft:");
                    !ids.iter()
                        .any(|id| id.trim_start_matches("minecraft:") == held)
                });
                self.extra
                    .effect_chains
                    .retain(|h| self.extra.effects.iter().any(|e| e.id == h.id));
                if had_absorption && !self.has_effect("minecraft:absorption") {
                    self.absorption = self.absorption.min(armor::max_absorption(self));
                }
                if had_boost && !self.has_effect("minecraft:health_boost") {
                    finalize::health_boost_changed(self);
                }
            }
            // `ApplyStatusEffectsConsumeEffect.apply`: one random draw, even at
            // probability 1.
            ConsumeEffect::ApplyEffects {
                effects,
                probability,
            } => {
                if self.random.next_float() >= *probability {
                    return;
                }
                for spec in effects {
                    let Some(def) = horizon_client_core::potions::effect(&spec.id) else {
                        continue;
                    };
                    if def.instant {
                        match def.id {
                            "instant_damage" | "instant_health" => {
                                let amp = spec.amplifier.clamp(0, 30);
                                if (def.id == "instant_damage") != self.kind.flags.undead {
                                    self.damage(
                                        level,
                                        (6i32 << amp) as f32,
                                        Damage::of(DamageKind::Magic),
                                    );
                                } else {
                                    let amount = (4i32 << amp) as f32;
                                    // No owning loop is lent to a DamageLevel yet. Keep this
                                    // component's instant healing vanilla-only, not deferred.
                                    if self.health > 0.0 {
                                        self.health = (self.health + amount)
                                            .clamp(0.0, self.attrs.max_health);
                                    }
                                }
                            }
                            _ => {}
                        }
                    } else {
                        self.add_effect(def.id, spec.duration, spec.amplifier);
                    }
                }
            }
            _ => {}
        }
    }
    /// `LivingEntity.heal`, with the native heal hook.
    pub fn heal(&mut self, context: &LoopContext, mut amount: f32) {
        let mut cancel = false;
        crate::hunger::native_living_heal(context, self.id, &mut amount, &mut cancel);
        if !cancel && self.health > 0.0 {
            self.health = (self.health + amount).clamp(0.0, self.attrs.max_health);
        }
    }
    /// `HealOrHarmMobEffect.applyInstantenousEffect`: a harming effect hurts a
    /// living mob (6 << amplifier, magic) and heals an undead one, a healing effect
    /// the reverse (4 << amplifier), both scaled by `scale`.
    pub fn apply_heal_or_harm(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        harm: bool,
        inverted: bool,
        amplifier: i32,
        scale: f64,
    ) {
        let shift = amplifier as u32;
        if harm != inverted {
            self.damage(
                level,
                (scale * f64::from(6i32.wrapping_shl(shift))) as i32 as f32,
                Damage::of(DamageKind::Magic),
            );
        } else {
            let heal = (scale * f64::from(4i32.wrapping_shl(shift).max(0))) as i32 as f32;
            self.heal(context, heal);
        }
    }
    /// `LivingEntity.canBeAffected`: whether an effect of this kind may be added at
    /// all, asked by `addEffect` before it stores anything. Five classes narrow the
    /// rule first — a spider (a cave spider with it) and a nautilus refuse poison, a
    /// wither and a wither skeleton the wither effect, a parched weakness — and the
    /// entity type tags answer for the rest: `immune_to_infested` keeps infested off
    /// a silverfish, `immune_to_oozing` keeps oozing off a slime, and
    /// `ignores_poison_and_regen` (which is `#minecraft:undead`) keeps poison and
    /// regeneration off a zombie, a skeleton, a phantom, a wither and the rest.
    pub fn can_be_affected(&self, effect: &'static str) -> bool {
        let effect = horizon_client_core::potions::namespaced(effect);
        let refused = match self.species() {
            Species::Spider | Species::CaveSpider | Species::Nautilus | Species::ZombieNautilus => {
                effect == "minecraft:poison"
            }
            Species::Wither | Species::WitherSkeleton => effect == "minecraft:wither",
            Species::Parched => effect == "minecraft:weakness",
            Species::Silverfish => effect == "minecraft:infested",
            Species::Slime => effect == "minecraft:oozing",
            _ => false,
        };
        if refused {
            return false;
        }
        !(self.kind.flags.undead && matches!(effect, "minecraft:poison" | "minecraft:regeneration"))
    }
    pub fn add_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32) {
        let effect = horizon_client_core::potions::namespaced(effect);
        // LivingEntity.addEffect: an effect the mob refuses is never added, so it
        // never joins the list the client draws or an effect tick reads.
        // WitherBoss.addEffect returns false for every effect.
        if self.species() == Species::Wither || !self.can_be_affected(effect) {
            return;
        }
        let had = self
            .extra
            .effects
            .iter()
            .find(|e| e.id == effect)
            .map(|e| e.show_icon);
        let ambient = self.extra.ambient_effects.contains(&effect);
        let hidden = self.extra.hidden_effects.contains(&effect);
        self.add_effect_with(effect, ticks, amplifier, false, true);
        // The basic overload changes the chain, not the existing active flags.
        if let Some(show_icon) = had {
            if let Some(e) = self.extra.effects.iter_mut().find(|e| e.id == effect) {
                e.show_icon = show_icon;
            }
            self.extra.ambient_effects.retain(|id| *id != effect);
            if ambient {
                self.extra.ambient_effects.push(effect);
            }
            self.extra.hidden_effects.retain(|id| *id != effect);
            if hidden {
                self.extra.hidden_effects.push(effect);
            }
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
    /// `Entity.playSound(event, 1.0, 1.0)`: at the feet, and nothing while silent.
    pub fn sound(&self, event: &'static str, events: &mut Vec<MobEvent>) {
        let at = [self.position.x, self.position.y, self.position.z];
        events.extend((!self.silent).then_some(MobEvent::Sound(event, at, 1.0, 1.0)));
    }
    /// `Mob.getAmbientSoundInterval`; `HappyGhast` waits six times as long
    /// while it carries a passenger.
    pub fn ambient_interval(&self) -> i32 {
        let ridden = self.rider.is_some() || self.extra.rider.is_some();
        if self.species() == Species::HappyGhast && ridden {
            self.kind.ambient_interval * 6
        } else {
            self.kind.ambient_interval
        }
    }
    /// LivingEntity.getVoicePitch.
    pub fn voice_pitch(&mut self) -> f32 {
        // HappyGhast.getVoicePitch: always 1.0, babies and all.
        if self.species() == Species::HappyGhast {
            return 1.0;
        }
        // Parrot.getVoicePitch is `getPitch(random)`, the plain 1.0 ± 0.2 an
        // adult gets below: a parrot is never a baby (`Parrot.canBeABaby`), so
        // the 1.5 `AgeableMob` gives a baby cannot reach it and the override
        // needs no arm of its own.
        // ZombieVillager.getVoicePitch: babies are 2.0, not 1.5.
        let baby = if self.species() == Species::ZombieVillager {
            2.0
        } else {
            1.5
        };
        let pitch = (self.random.next_float() - self.random.next_float()) * 0.2
            + if self.is_baby() { baby } else { 1.0 };
        // Bat.getVoicePitch: a little lower.
        if self.species() == Species::Bat {
            pitch * 0.95
        } else {
            pitch
        }
    }
    /// Mob.playSound with LivingEntity.getVoicePitch.
    pub fn voice(&mut self, event: &'static str, events: &mut Vec<MobEvent>) {
        // Java draws the pitch before `Entity.playSound` looks at isSilent, so a
        // silent entity draws it and then plays nothing.
        let pitch = self.voice_pitch();
        if self.silent {
            return;
        }
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
#[derive(Clone)]
pub struct EntityModelDocument(Arc<Value>);

impl EntityModelDocument {
    pub fn load(root: &Path) -> crate::Result<Self> {
        Ok(Self(Arc::new(serde_json::from_slice(&fs::read(
            root.join("entity-models.json"),
        )?)?)))
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

    /// The parsed document, for loaders that read several keys from it.
    pub fn value(&self) -> &Value {
        &self.0
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
                    // The armour glint scrolls the model UV in the shader.
                    glint_uv: if overlay == render::ARMOR_GLINT {
                        [v[3], v[4]]
                    } else {
                        [0.0, 0.0]
                    },
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
        self.extra
            .equipment
            .get(horizon_items::equipment::SADDLE)
            .cloned()
    }
    /// Wear (or take off) a saddle stack, keeping its components.
    pub fn set_saddle(&mut self, saddle: Option<crate::items::ItemStack>) {
        self.extra
            .equipment
            .set(horizon_items::equipment::SADDLE, saddle);
    }
}

#[cfg(test)]
#[path = "mobs/lighting_tests.rs"]
mod lighting_tests;

/// `LivingEntity.elytraAnimationState`: eased wing angles and render interpolation.
#[derive(Clone, Copy, Debug, Default)]
pub struct ElytraAnimation {
    pub old: [f32; 3],
    pub angles: [f32; 3],
}
impl ElytraAnimation {
    /// The angles `ElytraAnimationState` eases to when not gliding or crouching.
    pub const RESTING: [f32; 3] = [
        std::f32::consts::PI / 12.0,
        0.0,
        -std::f32::consts::PI / 12.0,
    ];
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
            Self::RESTING
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

impl Mob {
    /// The part of `LivingEntity.tick` and `aiStep` that animates another
    /// player from its synched pose and flags: the glide count, the wings
    /// (`elytraAnimationState`) and the swim amount (`updateSwimAmount`).
    pub fn tick_avatar_pose(&mut self, in_water: bool) {
        let (p, v) = (self.previous, self.velocity);
        let on_ground_alive = self.on_ground && self.alive();
        let e = &mut self.extra;
        e.fall_fly_ticks = if e.fall_flying {
            e.fall_fly_ticks + 1
        } else {
            0
        };
        // LivingEntity.isVisuallySwimming: the swimming pose, or the gliding
        // pose after the glide ended.
        let visually = e.synced_pose == 3 || (!e.fall_flying && e.synced_pose == 1);
        e.elytra.tick(e.fall_flying, e.crouching, self.velocity);
        e.swim.tick(visually, in_water);
        // AbstractClientPlayer.tick's ClientAvatarState.tick (from where the
        // player was before this tick's move), RemotePlayer.aiStep's
        // updateBob and rideTick's resetBob.
        e.avatar.tick([p.x, p.y, p.z]);
        if e.passenger {
            e.avatar.reset_bob();
        } else {
            let swimming = e.synced_pose == 3;
            e.avatar
                .update_bob(on_ground_alive && !swimming, [v.x, v.y, v.z]);
        }
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
    /// An armour stand never glides or crouches: its wings stay at the angles
    /// a standing wearer eases to.
    #[test]
    fn resting_wings_are_the_standing_target() {
        let mut wings = ElytraAnimation::default();
        for _ in 0..80 {
            wings.tick(false, false, Vec3::ZERO);
        }
        for (angle, resting) in wings.angles.iter().zip(ElytraAnimation::RESTING) {
            assert!((angle - resting).abs() < 0.00001);
        }
    }
}
#[cfg(test)]
mod spear_knockback_tests {
    use super::sensor_scan_tests::kind;
    use super::*;
    use horizon_core::geometry::Vec3;

    /// `spear` is in `#no_knockback`: the stab's hurt deals no default
    /// knockback (the spear pushes with `causeExtraKnockback` itself).
    #[test]
    fn a_spear_stab_hurts_without_the_default_knockback() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = Mob::new(1, kind(), Vec3::ZERO, 0.0);
        let health = m.health;
        assert!(m.damage(level, 1.0, Damage::spear([2.0, 0.0, 0.0])));
        assert_eq!(m.health, health - 1.0);
        let v = m.velocity;
        assert_eq!([v.x, v.y, v.z], [0.0; 3]);
    }

    /// a plain player hit still knocks the mob away from the attacker.
    #[test]
    fn a_player_hit_still_knocks_back() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = Mob::new(1, kind(), Vec3::ZERO, 0.0);
        assert!(m.damage(level, 1.0, Damage::player([2.0, 0.0, 0.0])));
        assert!(m.velocity.x < 0.0, "velocity {:?}", m.velocity);
    }
}
#[cfg(test)]
mod teams_tests {
    use super::*;
    use crate::save::uuid_text;
    use crate::scoreboard::{Scoreboard, Team};

    /// A board where every other mob is on the player's team, as with a
    /// datapack that colours its mobs' names.
    fn board(mobs: u32) -> (Scoreboard, HashMap<u32, [i32; 4]>) {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_to_team("Steve", "red");
        let uuids: HashMap<u32, [i32; 4]> = (0..mobs)
            .map(|id| (id, [id as i32, 7, -(id as i32), 11]))
            .collect();
        for (id, uuid) in &uuids {
            if id % 2 == 0 {
                board.add_to_team(&uuid_text(*uuid), "red");
            }
        }
        (board, uuids)
    }
    /// `Game::mob_teams` as built for each mob before: owned clones of the teams.
    fn owned(board: &Scoreboard, uuids: &HashMap<u32, [i32; 4]>) -> HashMap<u32, Team> {
        uuids
            .iter()
            .filter_map(|(id, u)| Some((*id, board.players_team(&uuid_text(*u))?.clone())))
            .collect()
    }
    fn borrowed<'a>(board: &'a Scoreboard, uuids: &HashMap<u32, [i32; 4]>) -> Teams<'a> {
        Teams {
            player: board.players_team("Steve"),
            mobs: uuids
                .iter()
                .filter_map(|(id, u)| Some((*id, board.players_team(&uuid_text(*u))?)))
                .collect(),
        }
    }

    #[test]
    fn borrowed_teams_answer_as_the_clones() {
        let (board, uuids) = board(40);
        let teams = borrowed(&board, &uuids);
        let old = owned(&board, &uuids);
        for id in 0..41 {
            assert_eq!(teams.of(Who::Mob(id)), old.get(&id), "mob {id}");
        }
        assert_eq!(teams.of(Who::Player).map(|t| t.name.as_str()), Some("red"));
    }

    /// One tick's worth of team building: once per mob (before) against once.
    #[test]
    #[ignore]
    fn teams_cost() {
        for mobs in [100u32, 250, 500] {
            let (board, uuids) = board(mobs);
            let t = std::time::Instant::now();
            for _ in 0..mobs {
                std::hint::black_box(owned(&board, &uuids));
            }
            let before = t.elapsed();
            let t = std::time::Instant::now();
            let reps = 20;
            for _ in 0..reps {
                std::hint::black_box(borrowed(&board, &uuids));
            }
            let after = t.elapsed() / reps;
            println!(
                "{mobs} mobs, half on a team: per tick {:.3} ms -> {:.3} ms",
                before.as_secs_f64() * 1e3,
                after.as_secs_f64() * 1e3
            );
        }
    }
}
#[cfg(test)]
pub(crate) mod sensor_scan_tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use serde_json::json;

    pub(crate) fn kind() -> Arc<MobType> {
        Arc::new(MobType {
            shadow: None,
            id: "minecraft:cow".into(),
            species: Species::Cow,
            width: 0.9,
            height: 1.4,
            eye_height: 1.3,
            category: "creature".into(),
            max_health: 10.0,
            speed: 0.2,
            flying_speed: 0.4,
            attack_damage: 0.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 120,
            flags: Flags {
                animal: true,
                ..Default::default()
            },
            model: Arc::new(Part::parse("root", &json!({}))),
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Rabbit,
            sounds: [""; 3],
            sound_overrides: Default::default(),
            food: Arc::default(),
            tempt: Arc::default(),
            passenger_attachments: Vec::new(),
        })
    }
    #[test]
    #[ignore = "needs prepared attributes in HORIZON_TEST_RESOURCES"]
    fn declared_health_spawn_fields_match_computed_values() {
        use horizon_world::attributes::entity::Catalog;
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &horizon_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        // Attributes.MAX_HEALTH is ranged from 1 to 1024 in vanilla.
        for (declared, expected) in [(-5.0, 1.0), (5000.0, 1024.0), (1e300, 1024.0)] {
            let merged = Arc::new(
                catalog
                    .with_entity_row(
                        "ruby:ghoul",
                        "minecraft:zombie",
                        &[("minecraft:max_health".into(), declared)],
                    )
                    .unwrap(),
            );
            let mut mob_type = Arc::unwrap_or_clone(kind());
            mob_type.id = "ruby:ghoul".into();
            mob_type
                .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
            let mut live = merged.create("ruby:ghoul").unwrap();
            let health = merged.id("minecraft:max_health").unwrap();
            assert_eq!(live.base_value(health).unwrap(), declared);
            assert_eq!(mob_type.attributes["minecraft:max_health"], declared);
            assert_eq!(live.value(health).unwrap(), expected);
            for mob in [
                Mob::new(7, Arc::new(mob_type.clone()), Vec3::ZERO, 0.0),
                Mob::new_remote(7, Arc::new(mob_type.clone()), Vec3::ZERO, 0.0),
            ] {
                assert_eq!(f64::from(mob.health), expected, "declared {declared}");
                assert_eq!(
                    f64::from(mob.attrs.max_health),
                    expected,
                    "declared {declared}"
                );
            }
        }
    }

    /// `n` mobs in pens of 20 spread over a square of `side` blocks, as on a
    /// server's farms; their snapshot and its grid.
    fn herd(n: u32, side: f64) -> (Vec<Seen>, seen_grid::SeenGrid) {
        let kind = kind();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut pen = Vec3::ZERO;
        let seen: Vec<Seen> = (0..n)
            .map(|id| {
                if id % 20 == 0 {
                    pen = Vec3::new(next(side), 64.0, next(side));
                }
                let at = Vec3::new(pen.x + next(8.0), 64.0, pen.z + next(8.0));
                Mob::new(id, kind.clone(), at, 0.0).seen(None, &crate::items::Items::empty())
            })
            .collect();
        let grid = seen_grid::SeenGrid::new(seen.iter().map(|s| s.bounds));
        (seen, grid)
    }
    /// `NearestLivingEntitySensor` over every mob, as the brains ran it.
    fn scan(seen: &[Seen], me: &Seen, area: Aabb) -> Vec<u32> {
        seen.iter()
            .filter(|o| o.id != me.id && o.alive && o.bounds.intersects(area))
            .map(|o| o.id)
            .collect()
    }
    fn indexed(seen: &[Seen], grid: &seen_grid::SeenGrid, me: &Seen, area: Aabb) -> Vec<u32> {
        let mut near = Vec::new();
        assert!(grid.candidates(area, &mut near));
        near.iter()
            .map(|&i| &seen[i as usize])
            .filter(|o| o.id != me.id && o.alive && o.bounds.intersects(area))
            .map(|o| o.id)
            .collect()
    }

    #[test]
    fn the_grid_finds_what_the_scan_finds() {
        let (seen, grid) = herd(600, 160.0);
        for me in &seen {
            let area = me.bounds.inflate(Vec3::new(16.0, 16.0, 16.0));
            assert_eq!(indexed(&seen, &grid, me, area), scan(&seen, me, area));
        }
    }

    /// `MobWorld::seen` once per mob (a target, a breeding partner...), by
    /// scan as before and by the tick's id index.
    #[test]
    #[ignore]
    fn lookup_cost() {
        for n in [500u32, 1000, 2000] {
            let (seen, _) = herd(n, 256.0);
            let index: crate::fast_hash::FxHashMap<u32, u32> = seen
                .iter()
                .enumerate()
                .map(|(i, s)| (s.id, i as u32))
                .collect();
            let wanted: Vec<u32> = (0..n).map(|i| (i * 7919) % n).collect();
            let t = std::time::Instant::now();
            let a: usize = wanted
                .iter()
                .filter_map(|&id| seen.iter().find(|s| s.id == id))
                .map(|s| s.id as usize)
                .sum();
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let b: usize = wanted
                .iter()
                .filter_map(|&id| index.get(&id).map(|&i| &seen[i as usize]))
                .map(|s| s.id as usize)
                .sum();
            let after = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(a, b);
            println!(
                "{n} lookups over {n} mobs ({} B each): {before:.3} ms -> {after:.3} ms",
                std::mem::size_of::<Seen>()
            );
        }
    }

    /// Natural spawning's `isUnobstructed` box test against every mob (real
    /// `Mob`s), by scan and by a grid built once for the pass.
    #[test]
    #[ignore]
    fn spawn_box_cost() {
        let kind = kind();
        for n in [500u32, 1000, 2000] {
            let (seen, _) = herd(n, 384.0);
            let mobs: Vec<Mob> = seen
                .iter()
                .map(|s| Mob::new(s.id, kind.clone(), s.position, 0.0))
                .collect();
            let mut seed = 7u64;
            let mut next = |range: f64| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (seed >> 11) as f64 / (1u64 << 53) as f64 * range
            };
            let queries: Vec<Aabb> = (0..100)
                .map(|_| {
                    let (x, z) = (next(384.0), next(384.0));
                    Aabb::new(Vec3::new(x, 64.0, z), Vec3::new(x + 0.9, 65.4, z + 0.9))
                })
                .collect();
            let t = std::time::Instant::now();
            let a: Vec<bool> = queries
                .iter()
                .map(|&q| {
                    mobs.iter()
                        .any(|m| m.alive() && !m.removed && m.bounds().intersects(q))
                })
                .collect();
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let boxes: Vec<Aabb> = mobs.iter().map(|m| m.bounds()).collect();
            let grid = seen_grid::SeenGrid::new(boxes.iter().copied());
            let build = t.elapsed().as_secs_f64() * 1e3;
            let mut near = Vec::new();
            let b: Vec<bool> = queries
                .iter()
                .map(|&q| {
                    assert!(grid.candidates(q, &mut near));
                    near.iter().any(|&i| boxes[i as usize].intersects(q))
                })
                .collect();
            let after = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(a, b);
            println!(
                "{n} mobs, 100 spawn box tests: {before:.3} ms -> {after:.3} ms (grid build {build:.3} ms included)"
            );
        }
    }

    /// One tick of sensing: a twentieth of the mobs fire their sensor.
    #[test]
    #[ignore]
    fn sensor_cost() {
        for (n, side) in [(500u32, 256.0), (1000, 384.0), (2000, 512.0)] {
            let (seen, grid) = herd(n, side);
            let time = |f: &dyn Fn(&Seen, Aabb) -> usize| {
                let t = std::time::Instant::now();
                let mut found = 0;
                for me in &seen {
                    found += f(me, me.bounds.inflate(Vec3::new(16.0, 16.0, 16.0)));
                }
                (t.elapsed().as_secs_f64() * 1e3 / 20.0, found)
            };
            let (before, a) = time(&|me, area| scan(&seen, me, area).len());
            let (after, b) = time(&|me, area| indexed(&seen, &grid, me, area).len());
            assert_eq!(a, b);
            println!("{n} mobs: per tick {before:.3} ms -> {after:.3} ms");
        }
    }
}
#[cfg(test)]
mod push_tests {
    use super::sensor_scan_tests::kind;
    use super::*;
    use horizon_core::geometry::Vec3;

    /// The pending pushes as one list, scanned for each mob, as before.
    fn take_old(events: &mut Vec<MobEvent>, from: usize, pending: &mut Vec<(u32, Vec3)>) {
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
    fn apply_old(velocity: &mut Vec3, id: u32, pending: &mut Vec<(u32, Vec3)>) {
        pending.retain(|(to, v)| {
            if *to != id {
                return true;
            }
            *velocity = Vec3::new(velocity.x + v.x, velocity.y + v.y, velocity.z + v.z);
            false
        });
    }
    fn mobs(n: u32) -> Vec<Mob> {
        let kind = kind();
        (0..n)
            .map(|id| Mob::new(id, kind.clone(), Vec3::ZERO, 0.0))
            .collect()
    }
    /// A crowd tick: each mob takes its pushes, then pushes every other
    /// mob (and the player, and hisses now and then); then all take theirs.
    fn crowd_tick(
        n: u32,
        tick: u32,
        mut on_push: impl FnMut(&mut Vec<MobEvent>, usize),
        mut on_apply: impl FnMut(u32),
    ) -> Vec<MobEvent> {
        let mut events = Vec::new();
        for i in 0..n {
            on_apply(i);
            let from = events.len();
            for j in 0..n {
                if j != i && !(i * 31 + j * 17 + tick).is_multiple_of(5) {
                    let f = f64::from(i * 7 + j + tick) * 0.001;
                    events.push(MobEvent::Push {
                        target: Who::Mob(j),
                        velocity: [f, -f * 0.5, f * 0.25],
                    });
                }
                if j.is_multiple_of(9) {
                    events.push(MobEvent::Hiss { mob: j });
                }
            }
            events.push(MobEvent::Push {
                target: Who::Player,
                velocity: [0.1, 0.0, 0.1],
            });
            on_push(&mut events, from);
        }
        for i in 0..n {
            on_apply(i);
        }
        events
    }

    #[test]
    fn pushes_land_as_with_the_list() {
        for n in [1u32, 3, 8, 20, 60] {
            let mut new_mobs = mobs(n);
            let mut old_v = vec![Vec3::ZERO; n as usize];
            let mut pending = PendingPushes::default();
            let mut old_pending = Vec::new();
            for tick in 0..4 {
                let new_events = {
                    let pending = std::cell::RefCell::new(&mut pending);
                    let mobs = std::cell::RefCell::new(&mut new_mobs);
                    crowd_tick(
                        n,
                        tick,
                        |e, from| take_mob_pushes(e, from, &mut pending.borrow_mut()),
                        |i| {
                            apply_mob_pushes(
                                &mut mobs.borrow_mut()[i as usize],
                                &mut pending.borrow_mut(),
                            )
                        },
                    )
                };
                let old_events = {
                    let pending = std::cell::RefCell::new(&mut old_pending);
                    let v = std::cell::RefCell::new(&mut old_v);
                    crowd_tick(
                        n,
                        tick,
                        |e, from| take_old(e, from, &mut pending.borrow_mut()),
                        |i| {
                            apply_old(
                                &mut v.borrow_mut()[i as usize],
                                i,
                                &mut pending.borrow_mut(),
                            )
                        },
                    )
                };
                assert_eq!(format!("{new_events:?}"), format!("{old_events:?}"));
                for i in 0..n as usize {
                    let (a, b) = (new_mobs[i].velocity, old_v[i]);
                    assert_eq!(
                        (a.x.to_bits(), a.y.to_bits(), a.z.to_bits()),
                        (b.x.to_bits(), b.y.to_bits(), b.z.to_bits()),
                        "{n} mobs, tick {tick}, mob {i}"
                    );
                }
            }
        }
    }

    /// One tick of a pen of `n` mobs pushing each other.
    #[test]
    #[ignore]
    fn push_cost() {
        for n in [10u32, 30, 100, 200] {
            let mut ms = mobs(n);
            let mut v = vec![Vec3::ZERO; n as usize];
            let t = std::time::Instant::now();
            {
                let pending = std::cell::RefCell::new(Vec::new());
                let v = std::cell::RefCell::new(&mut v);
                crowd_tick(
                    n,
                    0,
                    |e, from| take_old(e, from, &mut pending.borrow_mut()),
                    |i| {
                        apply_old(
                            &mut v.borrow_mut()[i as usize],
                            i,
                            &mut pending.borrow_mut(),
                        )
                    },
                );
            }
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            {
                let pending = std::cell::RefCell::new(PendingPushes::default());
                let ms = std::cell::RefCell::new(&mut ms);
                crowd_tick(
                    n,
                    0,
                    |e, from| take_mob_pushes(e, from, &mut pending.borrow_mut()),
                    |i| {
                        apply_mob_pushes(
                            &mut ms.borrow_mut()[i as usize],
                            &mut pending.borrow_mut(),
                        )
                    },
                );
            }
            let after = t.elapsed().as_secs_f64() * 1e3;
            println!("{n} mobs pushing each other: {before:.3} ms -> {after:.3} ms");
        }
    }
}

#[cfg(test)]
mod held_stack_tests {
    use super::*;

    #[test]
    fn replacing_even_the_same_item_clears_its_old_components() {
        let mut extra = Extra::default();
        let mut named = crate::items::ItemStack::new("minecraft:diamond_sword", 1);
        named.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Old Sword".into()));
        extra.set_main_hand_stack(Some(named.clone()));
        assert_eq!(
            extra.main_hand_stack().unwrap().custom_name().as_deref(),
            Some("Old Sword")
        );
        extra.set_main_hand(Some(named.item.clone()));
        assert_eq!(extra.main_hand_stack().unwrap().custom_name(), None);
        extra.set_main_hand_stack(Some(named));
        extra.take_slot_id(horizon_items::equipment::MAINHAND);
        assert!(extra.main_hand_id().is_none());
        assert!(extra.main_hand_stack().is_none());
    }
}

#[cfg(test)]
mod fire_resistance_damage_tests {
    use super::*;

    fn pig() -> Mob {
        let mut kind = super::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:pig".into();
        def.species = Species::Pig;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    #[test]
    fn fire_resistance_rejects_fire_before_health_and_hurt_cooldown() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for kind in [
            DamageKind::Campfire,
            DamageKind::Fireball,
            DamageKind::HotFloor,
            DamageKind::InFire,
            DamageKind::Lava,
            DamageKind::OnFire,
            DamageKind::SulfurCubeHot,
        ] {
            let mut mob = pig();
            mob.add_effect("minecraft:fire_resistance", -1, 0);
            assert!(!mob.damage(level, 1.0, Damage::of(kind)), "{kind:?}");
            assert_eq!(mob.health, 10.0, "{kind:?}");
            assert_eq!(
                (mob.hurt_time, mob.invulnerable, mob.last_hurt),
                (0, 0, 0.0)
            );
            assert!(mob.last_damage.is_none());
            assert!(mob.outbox.is_empty());
            // A refused fire hit must not prevent the next ordinary hit.
            assert!(mob.damage(level, 1.0, Damage::of(DamageKind::Cactus)));
            assert_eq!(mob.health, 9.0);

            let mut bare = pig();
            assert!(bare.damage(level, 1.0, Damage::of(kind)));
            assert_eq!(bare.health, 9.0);
            assert!(
                !bare.damage(level, 1.0, Damage::of(kind)),
                "equal hit during cooldown"
            );
            assert!(
                bare.damage(level, 2.0, Damage::of(kind)),
                "stronger hit during cooldown"
            );
            assert_eq!(bare.health, 8.0);

            let mut resistant = pig();
            resistant.add_effect("minecraft:resistance", -1, 0);
            assert!(resistant.damage(level, 1.0, Damage::of(kind)));
            assert_eq!(resistant.health, 9.2);

            let mut invulnerable = pig();
            invulnerable.invulnerable_flag = true;
            assert!(!invulnerable.damage(level, 1.0, Damage::of(kind)));
            assert_eq!(invulnerable.health, 10.0);
        }
    }

    #[test]
    fn species_hurt_callbacks_still_run_before_the_living_fire_guard() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut mob = pig();
        let kind = Arc::make_mut(&mut mob.kind);
        kind.id = "minecraft:bat".into();
        kind.species = Species::Bat;
        bat::set_resting(&mut mob, true);
        mob.add_effect("minecraft:fire_resistance", -1, 0);
        assert!(!mob.damage(level, 1.0, Damage::of(DamageKind::InFire)));
        assert!(!bat::resting(&mob));
        assert_eq!(mob.health, 10.0);
        assert_eq!(mob.invulnerable, 0);
    }

    #[test]
    fn reflected_ghast_fireball_still_reports_success_when_fire_resistance_refuses_health_damage() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut mob = pig();
        let kind = Arc::make_mut(&mut mob.kind);
        kind.id = "minecraft:ghast".into();
        kind.species = Species::Ghast;
        kind.flags.fire_immune = true;
        mob.add_effect("minecraft:fire_resistance", -1, 0);
        let source = Damage {
            kind: DamageKind::Fireball,
            attacker: Some(Who::Player),
            from: None,
            attacker_species: None,
        };
        assert!(mob.damage(level, 1.0, source));
        assert_eq!(mob.health, 10.0);
        assert_eq!(mob.invulnerable, 0);
    }
}

#[cfg(test)]
mod magic_absorb_tests {
    use super::sensor_scan_tests::kind;
    use super::*;
    use horizon_core::geometry::Vec3;

    fn witch() -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:witch".into();
        k.species = Species::Witch;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `MobEffectInstance.update` for the flags of an effect a mob has: a new instance that is
    /// not ambient takes the ambient flag off an ambient one, one that is ambient does not unless
    /// the effect itself changes, and visibility is always the new instance's.
    #[test]
    fn an_effect_added_again_updates_its_ambient_and_visible_flags_as_vanilla() {
        let mut m = witch();
        let speed = "minecraft:speed";
        // A new effect takes its flags as they are.
        assert!(m.add_effect_with("speed", 600, 0, true, false));
        assert!(
            m.extra.ambient_effects.contains(&speed) && m.extra.hidden_effects.contains(&speed)
        );
        // The same effect, no stronger and no longer, ambient again and visible: ambient stays
        // (nothing changed), the visibility follows.
        assert!(m.add_effect_with("speed", 100, 0, true, true));
        assert_eq!(
            m.extra.effects,
            vec![crate::mobs::StoredEffect::new(speed, 600, 0)]
        );
        assert!(
            m.extra.ambient_effects.contains(&speed) && !m.extra.hidden_effects.contains(&speed)
        );
        // Not ambient: it takes the flag off, though nothing else changed.
        assert!(m.add_effect_with("speed", 100, 0, false, true));
        assert!(!m.extra.ambient_effects.contains(&speed));
        // Ambient over a not ambient one that does not change: not taken; one that changes: taken.
        assert!(!m.add_effect_with("speed", 100, 0, true, true));
        assert!(!m.extra.ambient_effects.contains(&speed));
        assert!(m.add_effect_with("speed", 900, 0, true, true));
        assert!(m.extra.ambient_effects.contains(&speed));
        assert_eq!(
            m.extra.effects,
            vec![crate::mobs::StoredEffect::new(speed, 900, 0)]
        );
    }

    /// `MobEffectInstance.showIcon` on a mob: a new instance keeps its own, `update` always takes
    /// the incoming one (a change by itself counts), and the basic overload leaves it alone.
    #[test]
    fn a_mobs_effect_keeps_its_icon_flag_apart_from_its_particles() {
        let mut m = witch();
        let speed = "minecraft:speed";
        let instance = |ticks, show_icon| {
            let mut e = crate::effects::EffectInstance::new(speed, ticks, 0);
            e.show_icon = show_icon;
            e
        };
        let icon = |m: &Mob| m.extra.effects[0].show_icon;
        assert!(m.add_effect_instance(speed, instance(600, false)));
        assert!(!icon(&m) && !m.extra.hidden_effects.contains(&speed));
        assert!(m.add_effect_instance(speed, instance(100, true)));
        assert!(icon(&m));
        assert!(m.add_effect_instance(speed, instance(100, false)));
        m.add_effect("speed", 900, 0);
        assert!(!icon(&m));
        assert_eq!(m.extra.effects[0].duration, 900);
    }

    #[test]
    fn absorption_refills_on_equal_and_weaker_adds() {
        let mut m = witch();
        m.add_effect_with("absorption", 200, 1, false, true);
        m.absorption = 0.0;
        assert!(!m.add_effect_with("absorption", 100, 1, false, true));
        assert_eq!(m.absorption, 8.0);
        m.absorption = 0.0;
        m.add_effect("minecraft:absorption", 50, 0);
        assert_eq!(m.absorption, 4.0);
        assert_eq!(m.extra.effects[0].amplifier, 1);
        assert_eq!(m.extra.effects[0].duration, 200);
    }

    #[test]
    fn absorption_downgrade_keeps_spent_hearts() {
        // LivingEntity.onEffectUpdated -> refreshDirtyAttributes clamps
        // absorption to the new MAX_ABSORPTION (4 for amplifier 0); it never raises it.
        let mut m = witch();
        m.add_effect_with("absorption", 200, 0, false, true);
        m.add_effect_with("absorption", 90, 1, false, true);
        assert_eq!(m.absorption, 8.0);
        m.absorption = 2.0;
        for _ in 0..90 {
            ai::tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
            );
        }
        assert_eq!(
            m.extra.effects,
            vec![crate::mobs::StoredEffect::new(
                "minecraft:absorption",
                110,
                0
            )]
        );
        assert_eq!(m.absorption, 2.0);
    }

    #[test]
    fn heal_without_a_mod_adds_the_amount_up_to_the_maximum() {
        let context = crate::test_resources::server_loop_context();
        let mut m = witch();
        m.health = m.attrs.max_health - 3.0;
        m.heal(context, 2.0);
        assert_eq!(m.health, m.attrs.max_health - 1.0);
        m.heal(context, 5.0);
        assert_eq!(m.health, m.attrs.max_health);
    }

    #[test]
    fn an_effect_counted_down_leaves_when_no_mod_holds_it() {
        let mut m = witch();
        m.add_effect_with("speed", 3, 0, false, true);
        for _ in 0..4 {
            ai::tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
            );
        }
        assert!(m.extra.effects.is_empty(), "{:?}", m.extra.effects);
    }

    #[test]
    fn stronger_effect_restores_its_hidden_predecessor_after_expiry() {
        let mut m = witch();
        m.add_effect_with("speed", 200, 0, false, true);
        m.add_effect_with("speed", 90, 1, false, true);
        for _ in 0..90 {
            ai::tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
            );
        }
        assert_eq!(
            m.extra.effects,
            vec![crate::mobs::StoredEffect::new("minecraft:speed", 110, 0)]
        );
    }

    #[test]
    fn resolved_instances_preserve_mob_identity_through_hidden_promotion() {
        let mut m = witch();
        let def = horizon_client_core::potions::effect("speed").unwrap();
        let effect = "minecraft:speed";
        assert!(m.add_effect_instance(effect, crate::effects::EffectInstance::from_def(def, 2, 1)));
        assert!(
            !m.add_effect_instance(effect, crate::effects::EffectInstance::from_def(def, 5, 0))
        );
        assert_eq!(m.extra.effect_chains[0].id, effect);
        for _ in 0..2 {
            ai::tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
            );
        }
        let restored = &m.extra.effects[0];
        assert_eq!(restored.id, "minecraft:speed");
        assert_eq!((restored.duration, restored.amplifier), (3, 0));
        assert_eq!(restored.registry_index, Some(0));
        assert!(m.extra.effect_chains.is_empty());
    }

    #[test]
    fn weaker_longer_effect_returns_with_its_flags_and_removal_clears_the_chain() {
        let mut m = witch();
        assert!(m.add_effect_with("speed", 2, 1, false, false));
        assert!(!m.add_effect_with("speed", 5, 0, true, false));
        for _ in 0..2 {
            ai::tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
            );
        }
        assert_eq!(
            m.extra.effects,
            vec![crate::mobs::StoredEffect::new("minecraft:speed", 3, 0)]
        );
        assert!(m.extra.ambient_effects.contains(&"minecraft:speed"));
        assert!(m.extra.hidden_effects.contains(&"minecraft:speed"));
        m.add_effect("speed", 1, 2);
        assert!(m.retain_effects(|_| false));
        assert!(m.extra.effect_chains.is_empty());
        assert!(m.extra.ambient_effects.is_empty());
        assert!(m.extra.hidden_effects.is_empty());
    }

    /// `LivingEntity.playSecondaryHurtSound`: a `thorns` hit that lands whole
    /// sounds `enchant.thorns.hit` where the mob stands, after its own hurt
    /// sound and for a silent mob too (`level.playSound`, not the mob's). The
    /// rest of a stronger hit inside the hurt cooldown makes no sound, and
    /// another kind of hit makes none.
    #[test]
    fn a_thorns_hit_sounds_where_the_mob_stands() {
        let mut fixture = crate::health::TestLevel::new(0);
        let level = &mut fixture.level();
        let thorns = Damage {
            kind: DamageKind::Thorns,
            attacker: Some(Who::Player),
            from: Some([4.0, 2.0, 3.0]),
            attacker_species: None,
        };
        let sounds = |m: &Mob| -> Vec<(&'static str, [f64; 3], f32, f32)> {
            m.outbox
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound(s, at, volume, pitch) => Some((*s, *at, *volume, *pitch)),
                    _ => None,
                })
                .collect()
        };
        let mut m = witch();
        m.position = Vec3::new(1.0, 2.0, 3.0);
        m.silent = true;
        assert!(m.damage(level, 4.0, thorns));
        assert_eq!(
            sounds(&m),
            vec![("enchant.thorns.hit", [1.0, 2.0, 3.0], 1.0, 1.0)]
        );
        m.outbox.clear();
        m.damage(level, 8.0, thorns);
        assert!(sounds(&m).is_empty(), "inside the hurt cooldown");

        let mut loud = witch();
        assert!(loud.damage(level, 4.0, thorns));
        let heard = sounds(&loud);
        assert_eq!(heard.last().map(|s| s.0), Some("enchant.thorns.hit"));
        let mut hit = witch();
        assert!(hit.damage(level, 4.0, Damage::player([4.0, 2.0, 3.0])));
        assert_eq!(heard.len(), sounds(&hit).len() + 1, "{heard:?}");
        assert!(sounds(&hit).iter().all(|s| s.0 != "enchant.thorns.hit"));
    }

    /// `Witch.getDamageAfterMagicAbsorb`: `#witch_resistant_to` (`magic`,
    /// `indirect_magic`, `sonic_boom`, `thorns`) takes 15% of the hit, which
    /// leaves a 10 health witch on 7 of a 20 hit it would not survive whole.
    #[test]
    fn a_witch_takes_15_percent_of_a_witch_resistant_hit() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for resistant in [
            DamageKind::Magic,
            DamageKind::IndirectMagic,
            DamageKind::SonicBoom,
            DamageKind::Thorns,
        ] {
            let mut m = witch();
            assert!(m.damage(level, 20.0, Damage::of(resistant)));
            assert_eq!(m.health, 7.0, "{resistant:?} is in #witch_resistant_to");
        }
    }

    /// A hit outside the tag (`mob_attack`) takes all of it.
    #[test]
    fn a_witch_takes_all_of_a_physical_hit() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = witch();
        m.damage(level, 4.0, Damage::of(DamageKind::MobAttack));
        assert_eq!(m.health, 6.0);
    }

    /// `Witch.getDamageAfterMagicAbsorb`: a source whose entity is the witch
    /// itself takes nothing, so its own harming potion cannot hurt it.
    #[test]
    fn a_witch_takes_nothing_from_its_own_hit() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = witch();
        let own = Damage {
            kind: DamageKind::IndirectMagic,
            attacker: Some(Who::Mob(m.id)),
            from: None,
            attacker_species: Some(Species::Witch),
        };
        m.damage(level, 20.0, own);
        assert_eq!(m.health, 10.0);
    }

    /// `damage_by` with a thrown potion's `indirectMagic(potion, owner)`: the
    /// hit keeps the potion as its direct entity and is the thrower's (a
    /// player's sets `lastHurtByPlayer`, a dispenser's names nobody); the next
    /// hit that names no direct entity forgets it.
    #[test]
    fn a_potion_hit_keeps_its_direct_entity_and_owner() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = Mob::new(1, kind(), Vec3::ZERO, 0.0);
        let thrown = Damage::indirect_magic(Some(Who::Player), [0.0, 1.0, 0.0]);
        assert!(m.damage_by(level, 1.0, thrown, "minecraft:splash_potion"));
        assert_eq!(m.last_direct, Some("minecraft:splash_potion"));
        let (last, _) = m.last_damage.unwrap();
        assert_eq!(last.kind, DamageKind::IndirectMagic);
        assert_eq!(last.attacker, Some(Who::Player));
        assert_eq!(m.ai.last_hurt_by_player, 100);
        m.invulnerable = 0;
        assert!(m.damage(level, 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!(m.last_direct, None);

        let mut m = Mob::new(1, kind(), Vec3::ZERO, 0.0);
        let dispensed = Damage::indirect_magic(None, [0.0, 1.0, 0.0]);
        assert!(m.damage_by(level, 1.0, dispensed, "minecraft:area_effect_cloud"));
        assert_eq!(m.last_direct, Some("minecraft:area_effect_cloud"));
        assert_eq!(m.ai.last_hurt_by_player, 0);
    }

    /// A harming potion's hit on a witch through `damage_by` is cut to 15%
    /// once (`getDamageAfterMagicAbsorb`), not again by the potion code.
    #[test]
    fn a_witch_takes_15_percent_of_a_potion_hit() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = witch();
        let thrown = Damage::indirect_magic(Some(Who::Player), [0.0, 1.0, 0.0]);
        assert!(m.damage_by(level, 20.0, thrown, "minecraft:splash_potion"));
        assert_eq!(m.health, 7.0);
    }
}

#[cfg(test)]
mod armor_hurt_tests {
    use super::sensor_scan_tests::kind;
    use super::*;
    use horizon_core::geometry::Vec3;

    fn horse() -> Mob {
        mob("minecraft:horse", Species::Horse)
    }

    fn mob(id: &str, species: Species) -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = id.into();
        k.species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    #[test]
    fn equipment_stacks_reads_full_offhand() {
        let mut horse = horse();
        let mut stack = crate::items::ItemStack::new("minecraft:shield", 3);
        stack.damage = 17;
        finalize::set_item_stack(&mut horse, finalize::OFFHAND, stack.clone());
        assert_eq!(horse.equipment_stacks()[1], Some(stack));
    }

    #[test]
    fn removing_body_equipment_keeps_the_whole_stack() {
        let mut horse = horse();
        let mut stack = crate::items::ItemStack::new("minecraft:red_carpet", 3);
        stack.damage = 7;
        stack.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
        horse.extra.set_body_stack(Some(stack.clone()));
        assert_eq!(horse.take_body_equipment(), Some(stack));
        assert_eq!(horse.take_body_equipment(), None);
    }

    #[test]
    fn replacing_armor_with_same_item_forgets_old_stack() {
        let mut horse = horse();
        let mut old = crate::items::ItemStack::new("minecraft:iron_helmet", 1);
        old.damage = 8;
        horse.set_equipment_stack(5, Some(old));
        let mut replacement = crate::items::ItemStack::new("minecraft:iron_helmet", 2);
        replacement.damage = 3;
        finalize::set_item_stack(&mut horse, finalize::HEAD, replacement.clone());
        assert_eq!(horse.equipment_stacks()[5], Some(replacement));
    }

    #[test]
    fn saddle_stack_keeps_count() {
        let mut horse = horse();
        let stack = crate::items::ItemStack::new("minecraft:saddle", 3);
        horse.set_saddle(Some(stack.clone()));
        assert_eq!(horse.saddle_item(), Some(stack));
    }

    /// `Entity.hurt` on a client level does nothing: the horse a rider's
    /// client drives takes no fall of its own, and the server's health stays
    /// (#15294: the client's copy died and lay on the ground, still ridable).
    #[test]
    fn a_client_copy_takes_no_fall_damage() {
        let context = crate::test_resources::server_loop_context();
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = horse();
        m.extra.client_side = true;
        let health = m.health;
        let mut events = Vec::new();
        assert!(!m.damage(level, 20.0, Damage::of(DamageKind::Fall)));
        m.cause_fall_damage_at(
            context,
            level,
            30.0,
            1.0,
            &mut events,
            None,
            DamageKind::Fall,
        );
        assert_eq!(m.health, health);
        assert_eq!(m.hurt_time, 0);
        assert!(m.alive());
        // The server's copy of the same fall is hurt.
        let mut server = horse();
        server.cause_fall_damage_at(
            context,
            level,
            30.0,
            1.0,
            &mut events,
            None,
            DamageKind::Fall,
        );
        assert!(server.health < health);
    }

    /// `LivingEntity.hurtArmor` (`doHurtEquipment`): a hit that lands on a mob
    /// wearing body armour leaves the piece owing a wear, which the host
    /// applies with the item's own maximum and fire rules.
    #[test]
    fn a_hurt_horse_reports_the_body_armour_wear() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let hit = Damage {
            kind: DamageKind::MobAttack,
            attacker: None,
            from: None,
            attacker_species: None,
        };
        let mut m = horse();
        m.extra
            .set_body(Some("minecraft:diamond_horse_armor".into()));
        assert!(m.damage(level, 6.0, hit), "the hit landed");
        let owed = m.outbox.iter().find_map(|e| match e {
            MobEvent::ArmorHurt { amount, .. } => Some(*amount),
            _ => None,
        });
        assert!(
            owed.is_some_and(|amount| amount > 0.0),
            "the worn armour owes a wear: {owed:?}"
        );
        // A horse with no body armour owes nothing.
        let mut bare = horse();
        assert!(bare.damage(level, 6.0, hit));
        assert!(
            !bare
                .outbox
                .iter()
                .any(|e| matches!(e, MobEvent::ArmorHurt { .. }))
        );
    }

    /// `Horse` is the only mob that overrides `LivingEntity.hurtArmor` (the
    /// base one is empty, `LivingEntity.java:1887`), so a donkey wearing a body
    /// item — `/item replace entity @e[type=minecraft:donkey,limit=1]
    /// armor.body with minecraft:wolf_armor` — wears nothing.
    #[test]
    fn a_donkey_wearing_a_body_item_wears_nothing() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut donkey = mob("minecraft:donkey", Species::Donkey);
        donkey.extra.set_body(Some("minecraft:wolf_armor".into()));
        assert!(
            donkey.damage(level, 6.0, Damage::of(DamageKind::MobAttack)),
            "the hit landed"
        );
        assert!(
            !donkey
                .outbox
                .iter()
                .any(|e| matches!(e, MobEvent::ArmorHurt { .. })),
            "only a horse's own override wears its body piece"
        );
    }

    /// Vanilla holds a head item and a body item at once on the mobs with a
    /// body slot (`LivingEntity.setItemSlot` over `EntityEquipment`): the
    /// packet keeps the helmet on 5 and the armour on 6, `/item` reads each
    /// back, and the armour value is the body piece's.
    #[test]
    fn a_horse_holds_a_helmet_and_its_body_armour_at_once() {
        use horizon_core::equipment_slot::EquipmentSlot;
        let mut m = horse();
        m.extra.set_slot_id(
            horizon_items::equipment::HEAD,
            Some("minecraft:iron_helmet".into()),
        );
        m.extra
            .set_body(Some("minecraft:diamond_horse_armor".into()));
        let gear = m.equipment_stacks();
        assert_eq!(
            gear[5].as_ref().map(|s| s.item.as_ref().to_owned()),
            Some("minecraft:iron_helmet".to_owned())
        );
        assert_eq!(
            gear[6].as_ref().map(|s| s.item.as_ref().to_owned()),
            Some("minecraft:diamond_horse_armor".to_owned())
        );
        let slot = |s| m.item_by_slot(s).map(|i| i.item.as_ref().to_owned());
        assert_eq!(
            slot(EquipmentSlot::Head),
            Some("minecraft:iron_helmet".to_owned())
        );
        assert_eq!(
            slot(EquipmentSlot::Body),
            Some("minecraft:diamond_horse_armor".to_owned())
        );
        // Death keeps both (`Mob.dropCustomDeathLoot` by slot).
        let body = crate::mobs::drops::slot_item(&m, "body").unwrap();
        let head = crate::mobs::drops::slot_item(&m, "head").unwrap();
        assert_eq!(body.item, "minecraft:diamond_horse_armor");
        assert_eq!(head.item, "minecraft:iron_helmet");
    }

    /// `getDamageAfterArmorAbsorb` asks `hurtArmor` only when the damage type
    /// is not in `#bypasses_armor` (`LivingEntity.java:1907-1909`), so a horse
    /// that falls keeps its body piece whole.
    #[test]
    fn a_fall_does_not_wear_the_body_piece() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = horse();
        m.extra
            .set_body(Some("minecraft:diamond_horse_armor".into()));
        assert!(
            m.damage(level, 6.0, Damage::of(DamageKind::Fall)),
            "the fall landed"
        );
        assert!(
            !m.outbox
                .iter()
                .any(|e| matches!(e, MobEvent::ArmorHurt { .. })),
            "a #bypasses_armor hit asks for no wear"
        );
    }

    /// The wear is measured by the damage `hurtArmor` is handed, which is the
    /// one the armour, toughness and enchantment reductions have not touched
    /// yet: 8 raw damage is `8 / 4 = 2` durability even though plate leaves far
    /// less than 8 through to the health.
    #[test]
    fn the_wear_is_the_damage_before_the_armour_reduction() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = horse();
        let k = Arc::make_mut(&mut m.kind);
        k.armor = 20.0;
        m.extra
            .set_body(Some("minecraft:diamond_horse_armor".into()));
        let before = m.health;
        assert!(m.damage(level, 8.0, Damage::of(DamageKind::MobAttack)));
        assert!(m.health < before, "the hit reached the health");
        let owed = m.outbox.iter().find_map(|e| match e {
            MobEvent::ArmorHurt { amount, .. } => Some(*amount),
            _ => None,
        });
        assert_eq!(
            owed,
            Some(8.0),
            "the pre-reduction damage, not the damage the health took"
        );
    }
}

#[cfg(test)]
mod allay_damage_identity_tests {
    use super::*;

    const ACTUAL: [i32; 4] = [101, 102, 103, 104];
    const OTHER: [i32; 4] = [10, 20, 30, 40];

    fn allay(liked: Option<[i32; 4]>) -> Mob {
        let mut kind = (*super::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:allay".into();
        kind.species = Species::Allay;
        kind.max_health = 20.0;
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.tame = liked.is_some();
        m.extra.owner = liked;
        m
    }

    #[test]
    fn verified_player_damage_compares_live_liked_uuid() {
        let mut test_level = crate::health::TestLevel::new(0);
        let attack = crate::health::Attack {
            attacker: 7,
            player_uuid: Some(ACTUAL),
            ..Default::default()
        };
        let mut level = test_level.level().attacked_by(&attack);
        for liked in [Some(ACTUAL), Some(OTHER), None] {
            let mut m = allay(liked);
            let accepted = m.damage(&mut level, 2.0, Damage::player([0.0; 3]));
            assert_eq!(accepted, liked != Some(ACTUAL), "liked={liked:?}");
            assert_eq!(m.health, if accepted { 18.0 } else { 20.0 });
            assert_eq!(m.hurt_time, if accepted { 10 } else { 0 });
            assert_eq!(m.extra.owner, liked);
        }
        // A UUID context alone must not turn a source without a player into its liked player.
        let mut m = allay(Some(ACTUAL));
        assert!(m.damage(&mut level, 2.0, Damage::of(DamageKind::Generic)));
        assert_eq!(m.health, 18.0);
    }

    #[test]
    fn unverified_player_identity_never_matches_the_liked_player() {
        let mut test_level = crate::health::TestLevel::new(0);
        let mut level = test_level.level();
        for liked in [Some(ACTUAL), Some(OTHER), None] {
            let mut m = allay(liked);
            assert!(m.damage(&mut level, 2.0, Damage::player([0.0; 3])));
            assert_eq!(m.health, 18.0);
            // A projectile without a verified player cannot match a liked UUID either.
            let mut m = allay(liked);
            let arrow = Damage {
                kind: DamageKind::Arrow,
                ..Damage::player([0.0; 3])
            };
            assert!(m.damage_projectile(&mut level, 2.0, arrow, None, [1.0, 0.0]));
            assert_eq!(m.health, 18.0);
            assert!(m.projectile_motion.is_none());
        }
        let mut m = allay(Some(ACTUAL));
        assert!(m.damage(&mut level, 2.0, Damage::of(DamageKind::Generic)));
    }
}

#[cfg(test)]
mod totem_tests {
    use super::*;

    fn fox() -> Mob {
        let mut t = (*crate::mobs::sensor_scan_tests::kind()).clone();
        t.species = Species::Fox;
        t.max_health = 10.0;
        Mob::new(1, Arc::new(t), Vec3::ZERO, 0.0)
    }

    #[test]
    fn death_component_instant_healing_obeys_inversion_cap_and_dead_state() {
        let mut level = crate::health::TestLevel::new(0);
        for (undead, id) in [
            (false, "minecraft:instant_health"),
            (true, "minecraft:instant_damage"),
        ] {
            let mut mob = fox();
            let mut kind = (*mob.kind).clone();
            kind.flags.undead = undead;
            mob.kind = Arc::new(kind);
            mob.health = 1.0;
            let effect = crate::items::ConsumeEffect::ApplyEffects {
                probability: 1.0,
                effects: vec![crate::items::ConsumeEffectSpec {
                    id: id.into(),
                    duration: 1,
                    amplifier: 1,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                }],
            };
            mob.apply_consume_effect(&mut level.level(), &effect);
            assert_eq!(mob.health, 9.0);
            mob.apply_consume_effect(&mut level.level(), &effect);
            assert_eq!(mob.health, 10.0);
            mob.health = 0.0;
            mob.apply_consume_effect(&mut level.level(), &effect);
            assert_eq!(
                mob.health, 0.0,
                "instant healing does not revive a dead mob"
            );
        }
    }

    /// `LivingEntity.checkTotemDeathProtection`: a totem in the mouth takes
    /// the killing blow, the fox stays at 1 health with the totem's effects,
    /// and entity event 35 tells the clients to draw it.
    #[test]
    fn a_totem_in_the_mouth_saves_a_fox() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = fox();
        m.extra
            .set_main_hand(Some("minecraft:totem_of_undying".into()));
        assert!(m.damage(level, 1000.0, Damage::of(DamageKind::Generic)));
        assert!(m.alive(), "the totem kept it alive");
        assert_eq!(m.health, 1.0);
        assert_eq!(m.extra.main_hand_id(), None, "the totem was used up");
        // DeathProtection.TOTEM_EFFECTS.
        assert_eq!(
            m.extra.effects,
            vec![
                crate::mobs::StoredEffect::new("minecraft:regeneration", 900, 1),
                crate::mobs::StoredEffect::new("minecraft:absorption", 100, 1),
                crate::mobs::StoredEffect::new("minecraft:fire_resistance", 800, 0),
            ]
        );
        assert!(
            m.outbox
                .iter()
                .any(|e| matches!(e, MobEvent::EntityEvent { event: 35, .. }))
        );
    }

    /// `getItemInHand` asks the main hand first, then the off hand.
    #[test]
    fn a_totem_in_the_off_hand_saves_a_fox_too() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = fox();
        m.extra.set_slot_id(
            horizon_items::equipment::OFFHAND,
            Some("minecraft:totem_of_undying".into()),
        );
        assert!(m.damage(level, 1000.0, Damage::of(DamageKind::Generic)));
        assert!(m.alive());
        assert_eq!(m.health, 1.0);
        assert_eq!(m.extra.off_hand_id(), None);
    }

    /// Without one in either hand the blow kills it as before.
    #[test]
    fn without_a_totem_the_blow_kills() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = fox();
        m.extra
            .set_main_hand(Some("minecraft:sweet_berries".into()));
        assert!(m.damage(level, 1000.0, Damage::of(DamageKind::Generic)));
        assert!(!m.alive());
    }

    /// `DamageTypeTags.BYPASSES_INVULNERABILITY` (`/kill`, the void) is not
    /// something a totem answers.
    #[test]
    fn a_bypassing_killing_blow_ignores_the_totem() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = fox();
        m.extra
            .set_main_hand(Some("minecraft:totem_of_undying".into()));
        assert!(m.damage(level, 1000.0, Damage::of(DamageKind::GenericKill)));
        assert!(!m.alive());
        assert_eq!(
            m.extra.main_hand_id(),
            Some("minecraft:totem_of_undying"),
            "the totem is still in the mouth"
        );
    }
}
#[cfg(test)]
mod effect_refusal_tests {
    use super::*;

    /// A mob of a real type, from the extracted data (`entity-types.json`), so the
    /// `undead`, `MobCategory` and species facts are the ones the game has.
    fn mob(types: &MobTypes, id: &str) -> Mob {
        let kind = Arc::new(types.types.get(id).expect(id).clone());
        Mob::new(1, kind, Vec3::new(0.0, 64.0, 0.0), 0.0)
    }
    /// `LivingEntity.addEffect` asks `canBeAffected` before it stores anything: an
    /// effect the mob's type refuses never joins the list, so the cloud on its head,
    /// the `/effect` reply and an effect tick never see it. The refused ones are the
    /// `ignores_poison_and_regen` tag (which is `#minecraft:undead`), a silverfish's
    /// infested and a slime's oozing, the classes that override the rule, and the
    /// wither, whose `addEffect` refuses every effect.
    #[test]
    fn an_effect_the_mobs_type_refuses_is_never_added() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let types =
            MobTypes::load_for_server(&root, &MobTypes::root_catalog(&root).unwrap()).unwrap();
        let refused = |id: &str, effect: &'static str| {
            let mut m = mob(&types, id);
            m.add_effect(effect, 200, 0);
            let listed = m.has_effect(effect);
            (listed, m.extra.effects.len())
        };
        // `#minecraft:undead` is the `ignores_poison_and_regen` tag: a zombie, a
        // skeleton and a phantom turn both poison and regeneration away.
        for undead in [
            "minecraft:zombie",
            "minecraft:skeleton",
            "minecraft:phantom",
            "minecraft:zombie_horse",
        ] {
            for effect in ["minecraft:poison", "minecraft:regeneration"] {
                assert_eq!(refused(undead, effect), (false, 0), "{undead} {effect}");
            }
            // Every other effect still lands on it.
            assert_eq!(refused(undead, "minecraft:strength"), (true, 1), "{undead}");
        }
        // The tag's other two effects, each refused by the one type in it.
        assert_eq!(
            refused("minecraft:silverfish", "minecraft:infested"),
            (false, 0)
        );
        assert_eq!(
            refused("minecraft:silverfish", "minecraft:poison"),
            (true, 1)
        );
        assert_eq!(refused("minecraft:slime", "minecraft:oozing"), (false, 0));
        // A magma cube is not in `immune_to_oozing`, so oozing takes.
        assert_eq!(
            refused("minecraft:magma_cube", "minecraft:oozing"),
            (true, 1)
        );
        // Spider.canBeAffected and CaveSpider's inheritance: poison is refused, and a
        // nautilus refuses it too (AbstractNautilus.canBeAffected), the zombie one
        // through the same override, an ordinary mob not at all.
        for spider in [
            "minecraft:spider",
            "minecraft:cave_spider",
            "minecraft:nautilus",
        ] {
            assert_eq!(refused(spider, "minecraft:poison"), (false, 0), "{spider}");
        }
        assert_eq!(refused("minecraft:cow", "minecraft:poison"), (true, 1));
        // WitherBoss.addEffect returns false: the boss takes no effect at all.
        for effect in [
            "minecraft:wither",
            "minecraft:strength",
            "minecraft:speed",
            "minecraft:poison",
        ] {
            assert_eq!(
                refused("minecraft:wither", effect),
                (false, 0),
                "wither {effect}"
            );
        }
        // WitherSkeleton.canBeAffected refuses the wither effect only, which a
        // zombie takes.
        assert_eq!(
            refused("minecraft:wither_skeleton", "minecraft:wither"),
            (false, 0)
        );
        assert_eq!(
            refused("minecraft:wither_skeleton", "minecraft:strength"),
            (true, 1)
        );
        assert_eq!(refused("minecraft:zombie", "minecraft:wither"), (true, 1));
        // Parched.canBeAffected: weakness only, slowness takes.
        assert_eq!(
            refused("minecraft:parched", "minecraft:weakness"),
            (false, 0)
        );
        assert_eq!(
            refused("minecraft:parched", "minecraft:slowness"),
            (true, 1)
        );
    }

    fn check_squid_damage_result(id: &str) {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let types =
            MobTypes::load_for_server(&root, &MobTypes::root_catalog(&root).unwrap()).unwrap();
        let mut attacker = mob(&types, "minecraft:cow");
        attacker.id = 2;
        attacker.position = Vec3::new(4.0, 64.0, 4.0);
        assert!(attacker.alive());
        let by_cow = Damage {
            kind: DamageKind::Generic,
            ..Damage::mob(attacker.id, [4.0, 64.0, 4.0]).with_species(Species::Cow)
        };
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // An unrelated parent's generic acceptance remains true.
        let mut cow = mob(&types, "minecraft:cow");
        assert!(cow.damage(level, 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!(cow.health, 9.0);
        // A parent's existing rejection never reaches the subclass aftermath.
        let mut protected = mob(&types, id);
        protected.invulnerable_flag = true;
        assert!(!protected.damage(level, 1.0, by_cow));
        assert_eq!(protected.health, 10.0);
        if protected.species() == Species::GlowSquid {
            assert_eq!(aquatic::dark_ticks(&protected), 0);
        }
        for no_ai in [false, true] {
            for attributed in [true, false] {
                let mut victim = mob(&types, id);
                victim.no_ai = no_ai;
                assert_eq!(victim.health, 10.0);
                assert!(victim.ai.last_hurt_by.is_none());
                let source = if attributed {
                    by_cow
                } else {
                    Damage::of(DamageKind::Generic)
                };
                let accepted = victim.damage(level, 1.0, source);
                // Check the parent's state first: false must not roll back damage.
                assert_eq!(victim.health, 9.0);
                assert_eq!(
                    victim.ai.last_hurt_by,
                    attributed.then_some(Who::Mob(attacker.id))
                );
                if victim.species() == Species::GlowSquid {
                    assert_eq!(
                        aquatic::dark_ticks(&victim),
                        if attributed { 100 } else { 0 }
                    );
                }
                assert_eq!(accepted, attributed, "{id} NoAI={no_ai}");
            }
        }
    }

    #[test]
    #[ignore = "needs prepared entity types in HORIZON_TEST_RESOURCES"]
    fn glow_squid_damage_result_keeps_parent_health_side_effect() {
        check_squid_damage_result("minecraft:glow_squid");
    }

    #[test]
    #[ignore = "needs prepared entity types in HORIZON_TEST_RESOURCES"]
    fn squid_damage_result_keeps_parent_health_side_effect() {
        check_squid_damage_result("minecraft:squid");
    }

    /// Squid.hurtServer reports a hit only when a living entity dealt it, so
    /// environmental damage leaves a glow squid glowing; a mob's blow dims it.
    #[test]
    fn only_a_mobs_blow_dims_a_glow_squid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let types =
            MobTypes::load_for_server(&root, &MobTypes::root_catalog(&root).unwrap()).unwrap();
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut burned = mob(&types, "minecraft:glow_squid");
        burned.health = 10.0;
        assert!(!burned.damage(level, 1.0, Damage::of(DamageKind::Campfire)));
        assert_eq!(burned.health, 9.0);
        assert_eq!(aquatic::dark_ticks(&burned), 0);
        let mut struck = mob(&types, "minecraft:glow_squid");
        struck.health = 10.0;
        assert!(struck.damage(level, 1.0, Damage::mob(2, [0.0, 64.0, 0.0])));
        assert_eq!(struck.health, 9.0);
        assert_eq!(aquatic::dark_ticks(&struck), 100);
    }
}
#[cfg(test)]
mod model_document_sharing_tests {
    use super::*;

    fn part_dump(part: &Part) -> String {
        format!("{part:?}")
    }

    /// The startup reads `entity-models.json` once and hands the same document
    /// to every model loader; reading it again per loader must build the same
    /// types, model for model and layer for layer.
    #[test]
    fn a_shared_document_builds_the_same_types() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            return;
        }
        let document = EntityModelDocument::load(&root).unwrap();
        let shared = MobTypes::load_with_document(&root, &document).unwrap();
        let independent = MobTypes::load(&root).unwrap();
        assert_eq!(shared.types.len(), independent.types.len());
        for (id, independent) in &independent.types {
            let shared = shared.types.get(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(independent.id, shared.id);
            assert_eq!(independent.species, shared.species);
            assert_eq!(
                (
                    independent.width,
                    independent.height,
                    independent.eye_height,
                ),
                (shared.width, shared.height, shared.eye_height)
            );
            assert_eq!(independent.attributes, shared.attributes);
            assert_eq!(independent.item_stack_limits, shared.item_stack_limits);
            assert_eq!(
                part_dump(&independent.model),
                part_dump(&shared.model),
                "{id} model"
            );
            let mut independent_layers: Vec<_> = independent.layers.iter().collect();
            let mut shared_layers: Vec<_> = shared.layers.iter().collect();
            independent_layers.sort_by_key(|(key, _)| key.as_str());
            shared_layers.sort_by_key(|(key, _)| key.as_str());
            assert_eq!(independent_layers.len(), shared_layers.len(), "{id} layers");
            for ((independent_key, independent_part), (shared_key, shared_part)) in
                independent_layers.into_iter().zip(shared_layers)
            {
                assert_eq!(independent_key, shared_key, "{id} layer key");
                assert_eq!(
                    part_dump(independent_part),
                    part_dump(shared_part),
                    "{id} layer {independent_key}"
                );
            }
        }
    }
}
crate::impl_joined!(Mob);
horizon_mod_runtime::impl_attachment_lifetime!(Mob);

#[cfg(test)]
mod named_damage_tests {
    use super::*;

    fn pig() -> Mob {
        let mut kind = super::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:pig".into();
        def.species = Species::Pig;
        def.armor = 20.0;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }
    fn fire(tags: u32) -> Damage {
        Damage::of(DamageKind::Named(NamedDamage::new(
            "example:fan_fire",
            NamedDamage::IS_FIRE | NamedDamage::BYPASSES_ARMOR | tags,
        )))
    }
    #[test]
    fn named_fire_uses_normal_immunity_and_protection_without_becoming_on_fire() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut resistant = pig();
        resistant.add_effect("minecraft:fire_resistance", -1, 0);
        assert!(!resistant.damage(level, 2.0, fire(0)));
        assert_eq!((resistant.health, resistant.invulnerable), (10.0, 0));
        let mut immune = pig();
        Arc::make_mut(&mut immune.kind).flags.fire_immune = true;
        assert!(!immune.damage(level, 2.0, fire(0)));
        let mut bare = pig();
        assert!(bare.damage(level, 2.0, fire(0)));
        assert_eq!(bare.health, 8.0); // bypasses the 20 armor points
        assert!(bare.velocity.x != 0.0 || bare.velocity.z != 0.0);
        assert_eq!(bare.last_damage.unwrap().0.kind, fire(0).kind);
        let mut protected = pig();
        gear::set(
            &mut protected,
            gear::CHEST,
            vec![("minecraft:fire_protection".into(), 4)],
        );
        assert!(protected.damage(level, 2.0, fire(0)));
        assert!((protected.health - 8.64).abs() < 0.00001);
        let mut still = pig();
        assert!(still.damage(level, 2.0, fire(NamedDamage::NO_KNOCKBACK)));
        assert_eq!(still.velocity, Vec3::ZERO);
    }
    #[test]
    fn named_tags_control_resistance_and_specialist_enchantments() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut mob = pig();
        mob.add_effect("minecraft:resistance", -1, 0);
        assert!(mob.damage(level, 2.0, fire(0)));
        assert!((mob.health - 8.4).abs() < 0.00001);
        let mut bypass = pig();
        bypass.add_effect("minecraft:resistance", -1, 0);
        assert!(bypass.damage(level, 2.0, fire(NamedDamage::BYPASSES_RESISTANCE)));
        assert_eq!(bypass.health, 8.0);
        let mut no_effects = pig();
        no_effects.add_effect("minecraft:resistance", -1, 0);
        gear::set(
            &mut no_effects,
            gear::CHEST,
            vec![("minecraft:fire_protection".into(), 4)],
        );
        assert!(no_effects.damage(level, 2.0, fire(NamedDamage::BYPASSES_EFFECTS)));
        assert_eq!(no_effects.health, 8.0);
        // #bypasses_enchantments (#11308): a warden's sonic boom, or a named
        // type with the tag, is not reduced by Protection, as
        // getDamageAfterMagicAbsorb returns it before the enchantments.
        for kind in [
            DamageKind::SonicBoom,
            DamageKind::Named(NamedDamage::new(
                "example:boom",
                NamedDamage::BYPASSES_ARMOR | NamedDamage::BYPASSES_ENCHANTMENTS,
            )),
        ] {
            let mut plated = pig();
            gear::set(
                &mut plated,
                gear::CHEST,
                vec![("minecraft:protection".into(), 4)],
            );
            assert!(plated.damage(level, 4.0, Damage::of(kind)));
            assert_eq!(plated.health, 6.0, "{kind:?}");
        }
        let mut armor = gear::Gear::default();
        armor.slots[gear::FEET] = vec![("minecraft:feather_falling".into(), 4)];
        armor.slots[gear::CHEST] = vec![("minecraft:blast_protection".into(), 4)];
        let kind = |tags| DamageKind::Named(NamedDamage::new("example:impact", tags));
        assert_eq!(gear::protection_points(&armor, kind(0)), 0.0);
        assert_eq!(
            gear::protection_points(&armor, kind(NamedDamage::IS_FALL)),
            12.0
        );
        assert_eq!(
            gear::protection_points(&armor, kind(NamedDamage::IS_EXPLOSION)),
            8.0
        );
    }
}

#[cfg(test)]
mod silent_voice_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// The sounds `Mob::voice` plays for `ambient_sound` of one mob, with
    /// `silent` set on it, and the random state it leaves behind.
    fn ambient(silent: bool) -> Option<(Vec<(&'static str, f32, f32)>, f32)> {
        let mut out = None;
        let ran = with_test_world(&[], |w, types| {
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:cow"].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            m.silent = silent;
            let mut events = Vec::new();
            crate::mobs::ai::ambient_sound(&mut m, w, &mut events);
            let heard = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound(s, _, volume, pitch) => Some((*s, *volume, *pitch)),
                    _ => None,
                })
                .collect();
            out = Some((heard, m.random.next_float()));
        });
        ran.then_some(out).flatten()
    }

    /// `Mob.playAmbientSound` plays through `Entity.playSound`, which returns
    /// early for a silent entity, so a mob summoned with `Silent:1b` makes no
    /// ambient sound. The pitch is still drawn first, as Java evaluates the
    /// arguments of `playSound(event, getSoundVolume(), getVoicePitch())`
    /// before the call, so the random state is the same either way. Without
    /// the check the silent cow plays `entity.cow.ambient` like the loud one,
    /// and without drawing first its random state falls two draws behind.
    #[test]
    fn a_silent_mob_makes_no_ambient_sound() {
        let Some((loud, loud_next)) = ambient(false) else {
            return;
        };
        let (quiet, quiet_next) = ambient(true).expect("same data");
        // Cow.getSoundVolume is 0.4 and the pitch is the voice pitch.
        assert_eq!(loud.len(), 1);
        assert_eq!(loud[0].0, "entity.cow.ambient");
        assert_eq!(loud[0].1, 0.4);
        assert!((0.8..=1.2).contains(&loud[0].2), "{}", loud[0].2);
        assert!(quiet.is_empty(), "{quiet:?}");
        assert_eq!(quiet_next, loud_next, "the pitch is drawn either way");
    }
}

#[cfg(test)]
mod visibility_percent_tests {
    use super::*;
    /// `LivingEntity.getVisibilityPercent` halves the range for the mob named
    /// on the worn head, and leaves every other mob at full range. A skull
    /// held in the hand does not count: its equippable slot is the head.
    #[test]
    fn worn_mob_heads_halve_that_mobs_detection_range() {
        let see = |head: &str, mob: Species| {
            let mut player = world::test_player(Vec3::ZERO);
            player.head = Some(Arc::from(head));
            player.visibility_percent(mob)
        };
        assert_eq!(see("minecraft:skeleton_skull", Species::Skeleton), 0.5);
        assert_eq!(see("minecraft:zombie_head", Species::Zombie), 0.5);
        assert_eq!(see("minecraft:creeper_head", Species::Creeper), 0.5);
        assert_eq!(see("minecraft:piglin_head", Species::Piglin), 0.5);
        assert_eq!(see("minecraft:piglin_head", Species::PiglinBrute), 0.5);
        assert_eq!(see("minecraft:skeleton_skull", Species::Zombie), 1.0);
        assert_eq!(see("minecraft:piglin_head", Species::ZombifiedPiglin), 1.0);
        assert_eq!(
            see("minecraft:wither_skeleton_skull", Species::WitherSkeleton),
            1.0
        );

        let mut held = world::test_player(Vec3::ZERO);
        held.main_hand = Some(Arc::from("minecraft:skeleton_skull"));
        assert_eq!(held.visibility_percent(Species::Skeleton), 1.0);

        let mut sneaking = world::test_player(Vec3::ZERO);
        sneaking.head = Some(Arc::from("minecraft:skeleton_skull"));
        sneaking.sneaking = true;
        assert_eq!(sneaking.visibility_percent(Species::Skeleton), 0.4);
    }
}

/// Added controls (H248): `LivingEntity.hurtServer` compares a hit inside
/// the hurt cooldown, and keeps `lastHurt`, by its incoming amount; the
/// reductions (`actuallyHurt`) get only the accepted difference.
#[cfg(test)]
mod hurt_cooldown_incoming_tests {
    use super::*;

    fn mob(species: Species) -> Mob {
        let mut kind = (*sensor_scan_tests::kind()).clone();
        kind.species = species;
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.attrs.max_health = 40.0;
        m.health = 40.0;
        m
    }

    #[test]
    fn full_resistance_keeps_the_incoming_amounts_and_takes_a_stronger_hit() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = mob(Species::Pig);
        m.extra
            .effects
            .push(StoredEffect::new("minecraft:resistance", 600, 4));
        assert!(m.damage(level, 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!((m.health, m.invulnerable, m.last_hurt), (40.0, 20, 1.0));
        assert!(!m.damage(level, 1.0, Damage::of(DamageKind::Generic)));
        assert!(m.damage(level, 2.0, Damage::of(DamageKind::Generic)));
        assert_eq!((m.health, m.invulnerable, m.last_hurt), (40.0, 20, 2.0));
        assert!(!m.damage(level, 1.5, Damage::of(DamageKind::Generic)));
        assert_eq!(m.last_hurt, 2.0);
    }

    #[test]
    fn the_armor_effectiveness_callback_runs_only_for_accepted_hits() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = mob(Species::Pig);
        // This damage-boundary fixture supplies effective armor directly;
        // equipment attributes are normally prepared by the equipment tick.
        Arc::make_mut(&mut m.kind).armor = 6.0;
        let mut calls = 0;
        let mut breach = |f: f32, _: &mut LegacyRandom| {
            calls += 1;
            f
        };
        assert!(m.damage_with(
            level,
            4.0,
            Damage::of(DamageKind::Cactus),
            Some(&mut breach)
        ));
        assert!(!m.damage_with(
            level,
            4.0,
            Damage::of(DamageKind::Cactus),
            Some(&mut breach)
        ));
        assert!(m.damage_with(
            level,
            8.0,
            Damage::of(DamageKind::Cactus),
            Some(&mut breach)
        ));
        assert_eq!(calls, 2);
        assert_eq!(m.last_hurt, 8.0);
    }

    #[test]
    fn absorption_soaks_the_accepted_difference() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = mob(Species::Pig);
        // Absorption I gives max_absorption 4, so the 1.5 remainder is valid.
        m.add_effect("absorption", 600, 0);
        m.absorption = 1.5;
        assert!(m.damage(level, 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!((m.health, m.absorption), (40.0, 0.5));
        // 3 after 1: the difference 2, the last 0.5 absorption first.
        assert!(m.damage(level, 3.0, Damage::of(DamageKind::Generic)));
        assert_eq!((m.health, m.absorption, m.last_hurt), (38.5, 0.0, 3.0));
    }

    #[test]
    fn a_horse_armor_wears_by_the_accepted_difference() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = mob(Species::Horse);
        m.extra.set_body(Some("minecraft:iron_horse_armor".into()));
        assert!(m.damage(level, 4.0, Damage::of(DamageKind::Cactus)));
        assert!(m.damage(level, 8.0, Damage::of(DamageKind::Cactus)));
        let worn: Vec<f32> = m
            .outbox
            .iter()
            .filter_map(|e| match e {
                MobEvent::ArmorHurt { amount, .. } => Some(*amount),
                _ => None,
            })
            .collect();
        assert_eq!(worn, [4.0, 4.0]);
    }
}

#[cfg(test)]
mod hidden_speed_mob_tick_tests {
    use super::*;
    fn full_tick_promotes(no_ai: bool) {
        let admitted = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                let stone = registry.default_state("minecraft:stone").unwrap();
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                world.set(2, 199, 4, stone).unwrap();
            },
            |w, types| {
                let mut m = Mob::new(
                    202,
                    Arc::new(types.types["minecraft:pig"].clone()),
                    Vec3::new(2.5, 200.0, 4.5),
                    0.0,
                );
                m.no_ai = no_ai;
                m.no_gravity = true;
                assert!(m.add_effect_with("minecraft:speed", 200, 0, false, true));
                assert!(m.add_effect_with("minecraft:speed", 40, 1, false, true));
                assert_eq!(
                    m.extra
                        .effects
                        .iter()
                        .map(|e| (e.id, e.duration, e.amplifier))
                        .collect::<Vec<_>>(),
                    [("minecraft:speed", 40, 1)]
                );
                let mut test_level = crate::health::TestLevel::new(202);
                let mut events = Vec::new();
                for _ in 0..39 {
                    assert!(m.tick(w, &mut test_level.level(), &mut events));
                }
                assert_eq!(
                    m.extra
                        .effects
                        .iter()
                        .map(|e| (e.id, e.duration, e.amplifier))
                        .collect::<Vec<_>>(),
                    [("minecraft:speed", 1, 1)]
                );
                assert!(m.tick(w, &mut test_level.level(), &mut events));
                assert_eq!(
                    m.extra
                        .effects
                        .iter()
                        .map(|e| (e.id, e.duration, e.amplifier))
                        .collect::<Vec<_>>(),
                    [("minecraft:speed", 160, 0)],
                    "actual Mob tick must promote its counted-down fallback"
                );
                assert_eq!(m.age, 40);
                assert!(m.tick(w, &mut test_level.level(), &mut events));
                assert_eq!(
                    m.extra
                        .effects
                        .iter()
                        .map(|e| (e.id, e.duration, e.amplifier))
                        .collect::<Vec<_>>(),
                    [("minecraft:speed", 159, 0)]
                );
            },
        );
        // Without the vanilla data under .local/horizon-client there is no Mob
        // world to tick: the test has nothing to check.
        let _ = admitted;
    }
    #[test]
    fn noai_full_mob_tick_promotes_speed() {
        full_tick_promotes(true)
    }
    #[test]
    fn ai_full_mob_tick_promotes_speed() {
        full_tick_promotes(false)
    }
    // Added causal controls; the three original old/fixed tests remain above.
    #[test]
    fn hidden_only_updates_copy_and_clear_readd_keep_active_authority() {
        let mut kind = (*sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        let mut m = Mob::new(202, Arc::new(kind), Vec3::ZERO, 0.0);
        m.add_effect("night_vision", 400, 0);
        assert!(m.add_effect_with("speed", 200, 0, true, false));
        assert!(m.add_effect_with("speed", 40, 1, false, true));
        assert_eq!(
            (
                m.extra.effect_chains[0].ambient,
                m.extra.effect_chains[0].visible
            ),
            (true, false)
        );
        assert!(!m.add_effect_with("speed", 300, 0, false, true));
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [
                ("minecraft:night_vision", 400, 0),
                ("minecraft:speed", 40, 1)
            ]
        );
        assert_eq!(m.extra.effect_chains[0].duration, 300);
        assert!(!m.add_effect_with("speed", 40, 1, false, true));
        let mut to = Mob::new(203, m.kind.clone(), Vec3::ZERO, 0.0);
        conversion::single(&m, &mut to, conversion::Params::KEEP_ALL);
        assert_eq!(to.extra.effects, m.extra.effects);
        assert_eq!(to.extra.effect_chains, m.extra.effect_chains);
        to.extra.effect_chains[0].duration = 222;
        assert_eq!(m.extra.effect_chains[0].duration, 300);
        m.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::RemoveEffects(vec!["minecraft:speed".into()]),
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:night_vision", 400, 0)]
        );
        assert!(m.extra.effect_chains.is_empty());
        m.add_effect("speed", 20, 0);
        assert!(m.extra.effect_chains.is_empty());
        m.add_effect("speed", 5, 1);
        assert_eq!(m.extra.effect_chains[0].duration, 20);
        m.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::ClearAllEffects,
        );
        assert!(m.extra.effects.is_empty() && m.extra.effect_chains.is_empty());
        // The basic add has default incoming flags, while preserving its
        // existing active flag policy; a newly retained weaker node gets them.
        assert!(m.add_effect_with("speed", 200, 0, true, false));
        assert!(m.add_effect_with("speed", 40, 1, true, false));
        m.add_effect("speed", 300, 0);
        assert!(m.extra.ambient_effects.contains(&"minecraft:speed"));
        assert!(m.extra.hidden_effects.contains(&"minecraft:speed"));
        assert!(!m.extra.effect_chains[0].ambient);
        assert!(m.extra.effect_chains[0].visible);
    }

    #[test]
    fn health_boost_death_effect_clear_clamps_health() {
        let mut kind = (*sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        let mut m = Mob::new(202, Arc::new(kind), Vec3::ZERO, 0.0);
        finalize::set_base(&mut m, finalize::Attr::MaxHealth, 10.0);
        m.add_effect("health_boost", 200, 1);
        assert_eq!(m.attrs.max_health, 18.0);
        m.health = 18.0;
        m.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::ClearAllEffects,
        );
        assert_eq!(m.attrs.max_health, 10.0);
        assert_eq!(m.health, 10.0);
    }

    #[test]
    fn consume_effect_removal_clamps_absorption_to_remaining_maximum() {
        use horizon_world::attributes::entity::Catalog;
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": horizon_world::DATA_VERSION,
                "attributes": [
                    {"id":"minecraft:max_absorption", "default":bits(0.0), "min":bits(0.0), "max":bits(2048.0), "syncable":true}
                ],
                "entities": {"minecraft:pig":{"minecraft:max_absorption":bits(0.0)}}
            }))
            .unwrap(),
        );
        let key = catalog.id("minecraft:max_absorption").unwrap();

        let mut kind = (*sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        let kind = Arc::new(kind);

        // Case 1: Base 0 mob with Absorption I (granting 4.0 absorption hearts, max_absorption 4.0).
        // On ClearAllEffects, max_absorption becomes 0.0, amount drops to 0.0.
        let mut m0 = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
        m0.add_effect("absorption", 600, 0);
        assert_eq!(armor::max_absorption(&m0), 4.0);
        assert_eq!(m0.absorption, 4.0);
        m0.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::ClearAllEffects,
        );
        assert_eq!(
            m0.absorption, 0.0,
            "base 0 mob loses absorption on ClearAllEffects"
        );

        // Case 2: Base 20 mob with Absorption I (granting 4.0 absorption hearts, max_absorption 24.0).
        // On ClearAllEffects, max_absorption becomes 20.0; amount 4.0 is <= 20.0, so it keeps 4.0.
        let mut m20 = Mob::new(2, kind.clone(), Vec3::ZERO, 0.0);
        let mut attrs = catalog.create("minecraft:pig").unwrap();
        attrs.edit(key, |value, observer| {
            value.set_base_value(20.0, observer).unwrap();
        });
        m20.extra.armor_attributes = Some(Box::new(attrs));
        m20.add_effect("absorption", 600, 0);
        assert_eq!(armor::max_absorption(&m20), 24.0);
        assert_eq!(m20.absorption, 4.0);
        m20.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::ClearAllEffects,
        );
        assert_eq!(
            m20.absorption, 4.0,
            "base 20 mob keeps absorption on ClearAllEffects"
        );

        // Case 3: Base 20 mob with Absorption I and absorption raised to 24.0.
        // On RemoveEffects(["minecraft:absorption"]), remaining max_absorption is 20.0,
        // so amount 24.0 is clamped to 20.0.
        let mut m20_clamp = Mob::new(3, kind.clone(), Vec3::ZERO, 0.0);
        let mut attrs3 = catalog.create("minecraft:pig").unwrap();
        attrs3.edit(key, |value, observer| {
            value.set_base_value(20.0, observer).unwrap();
        });
        m20_clamp.extra.armor_attributes = Some(Box::new(attrs3));
        m20_clamp.add_effect("absorption", 600, 0);
        m20_clamp.absorption = 24.0;
        m20_clamp.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::RemoveEffects(vec!["minecraft:absorption".into()]),
        );
        assert_eq!(
            m20_clamp.absorption, 20.0,
            "base 20 mob clamps absorption to 20.0 on RemoveEffects"
        );

        // Case 4: Base 0 mob with Absorption I (granting 4.0 absorption hearts, max_absorption 4.0).
        // On RemoveEffects(["minecraft:absorption"]), remaining max_absorption is 0.0,
        // so amount drops to 0.0.
        let mut m0_remove = Mob::new(4, kind.clone(), Vec3::ZERO, 0.0);
        m0_remove.add_effect("absorption", 600, 0);
        assert_eq!(m0_remove.absorption, 4.0);
        m0_remove.apply_consume_effect(
            &mut crate::health::TestLevel::new(0).level(),
            &crate::items::ConsumeEffect::RemoveEffects(vec!["minecraft:absorption".into()]),
        );
        assert_eq!(
            m0_remove.absorption, 0.0,
            "base 0 mob loses absorption on RemoveEffects"
        );

        // The shared command/expiry removal path must obey the same contract.
        for (base, amount, expected) in [(20.0, 8.0, 8.0), (20.0, 24.0, 20.0), (0.0, 4.0, 0.0)] {
            let mut m = Mob::new(5, kind.clone(), Vec3::ZERO, 0.0);
            let mut attrs = catalog.create("minecraft:pig").unwrap();
            attrs.edit(key, |value, observer| {
                value.set_base_value(base, observer).unwrap();
            });
            m.extra.armor_attributes = Some(Box::new(attrs));
            m.add_effect("absorption", 600, 1);
            m.absorption = amount;
            m.effects_dirty = false;
            assert!(!m.retain_effects(|_| true));
            assert_eq!(
                m.absorption, amount,
                "retained effect leaves absorption alone"
            );
            assert!(!m.effects_dirty);
            assert!(m.retain_effects(|e| e.id != "minecraft:absorption"));
            assert_eq!(m.absorption, expected, "base {base}, amount {amount}");
            assert!(m.effects_dirty);
            assert!(!m.has_effect("minecraft:absorption"));
        }
    }
}

mod projectile_damage;

pub fn apply_mob_pushes(m: &mut Mob, pending: &mut PendingPushes) {
    // Entity.push(x, y, z) sets needsSync.
    let mut add = |v: Vec3| {
        m.velocity = Vec3::new(m.velocity.x + v.x, m.velocity.y + v.y, m.velocity.z + v.z);
        m.needs_sync = true;
    };
    if !pending.list.is_empty() {
        pending.list.retain(|(id, v)| {
            if *id != m.id {
                return true;
            }
            add(*v);
            false
        });
    }
    if !pending.by_mob.is_empty()
        && let Some(pushes) = pending.by_mob.remove(&m.id)
    {
        pushes.into_iter().for_each(add);
    }
}
