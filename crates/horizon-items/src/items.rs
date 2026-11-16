//! Items from vanilla's default-component reports, block<->item mapping,
//! tool mining rules and block loot tables (`LootTable`/`LootPool` semantics
//! for the condition, entry and modifier types used by block drops).
#[path = "ingredient_defaults.rs"]
mod ingredient_defaults;
#[path = "loot_query.rs"]
mod loot_query;
use crate::{Result, blocks::BlockFacts};
use horizon_core::random::RandomSource;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use horizon_world::{
    item_defaults::ItemDefaults,
    loot::{MAX_LOOT_LEVEL, PackOverlay},
    tag_layers::TagLayers,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

mod loot_conditions;

fn tool_number(value: &horizon_core::nbt::Tag) -> Option<f32> {
    horizon_world::datafix::dynamic::as_number(value).map(|n| n.float_value())
}

/// A fire starter's use sound: `SoundEvents.FIRECHARGE_USE` or
/// `FLINTANDSTEEL_USE`.
pub fn fire_starter_sound(fire_charge: bool) -> &'static str {
    if fire_charge {
        "minecraft:item.firecharge.use"
    } else {
        "minecraft:item.flintandsteel.use"
    }
}

/// The pitch of a fire starter's use sound, from the level's random:
/// `FlintAndSteelItem.useOn`'s `nextFloat() * 0.4 + 0.8`, or
/// `FireChargeItem.playSound`'s `(nextFloat() - nextFloat()) * 0.2 + 1.0`.
pub fn fire_starter_pitch(fire_charge: bool, random: &mut impl RandomSource) -> f32 {
    if fire_charge {
        (random.next_float() - random.next_float()) * 0.2 + 1.0
    } else {
        random.next_float() * 0.4 + 0.8
    }
}

#[derive(Clone, Debug)]
pub struct ToolRule {
    pub blocks: Vec<String>,
    pub speed: Option<f32>,
    pub correct: Option<bool>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ConsumeEffectSpec {
    pub id: String,
    pub duration: i32,
    pub amplifier: i32,
    /// `MobEffectInstance`'s `ambient` (default false) and `show_particles` (default true).
    pub ambient: bool,
    pub show_particles: bool,
    /// `show_icon`, which defaults to `show_particles`.
    pub show_icon: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConsumeEffect {
    ApplyEffects {
        effects: Vec<ConsumeEffectSpec>,
        probability: f32,
    },
    ClearAllEffects,
    RemoveEffects(Vec<String>),
    /// `TeleportRandomlyConsumeEffect`.
    TeleportRandomly(horizon_world::teleport_consume::TeleportConsume),
    PlaySound(String),
    /// A mod's own kind, kept as the file wrote it (the loader used to drop
    /// it silently): the game hands it to `native_consume_effect`.
    Native {
        kind: String,
        data: Arc<horizon_core::nbt::Tag>,
    },
}

/// Typed consumable shared by item prototypes and stack component patches.
pub type ItemConsumable = horizon_world::consumable::Consumable<String, ConsumeEffect>;

/// Decode `Consumable.CODEC` defaults and ordered effects at the load boundary.
/// Opaque mod payloads are converted once to retained NBT; invalid payloads refuse admission.
pub fn parse_consumable(
    cons: &Value,
) -> std::result::Result<ItemConsumable, horizon_core::json_nbt::JsonNbtError> {
    Ok(ItemConsumable {
        seconds: cons["consume_seconds"].as_f64().map_or(1.6, |s| s as f32),
        animation: crate::use_animation::consumable_animation(cons),
        sound: cons["sound"]
            .as_str()
            .or_else(|| cons["sound"]["sound_id"].as_str())
            .unwrap_or("minecraft:entity.generic.eat")
            .to_owned(),
        particles: cons["has_consume_particles"].as_bool().unwrap_or(true),
        effects: cons["on_consume_effects"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|effect| {
                if let Some(kind) = effect["type"].as_str().filter(|k| {
                    k.contains(':') && !k.starts_with("minecraft:") && !k.starts_with(':')
                }) {
                    return horizon_core::json_nbt::json_to_tag(effect).map(|data| {
                        Some(ConsumeEffect::Native {
                            kind: kind.to_owned(),
                            data: Arc::new(data),
                        })
                    });
                }
                Ok(parse_consume_effect(effect))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
    })
}

/// One `on_consume_effects` entry. A bare type name is minecraft's own, as
/// a bare item id is: `apply_effects` and `:apply_effects` are
/// `minecraft:apply_effects`. A
/// namespaced kind outside `minecraft:` is the mod's own, kept whole for
/// the hook. An unknown `minecraft:` kind and a missing type are dropped.
pub fn parse_consume_effect(effect: &serde_json::Value) -> Option<ConsumeEffect> {
    let kind = effect.get("type")?.as_str()?;
    // `Identifier.bySeparator`: no namespace, or an empty one (`:clear_all_effects`), is minecraft's.
    let kind = match kind.split_once(':') {
        Some(("", path)) => format!("minecraft:{path}"),
        Some(_) => kind.to_owned(),
        None => format!("minecraft:{kind}"),
    };
    match kind.as_str() {
        "minecraft:apply_effects" => {
            let probability = effect
                .get("probability")
                .and_then(|p| p.as_f64())
                .map_or(1.0, |p| p as f32);
            let mut specs = Vec::new();
            if let Some(list) = effect.get("effects").and_then(|l| l.as_array()) {
                specs.extend(list.iter().filter_map(consume_effect_spec));
            }
            Some(ConsumeEffect::ApplyEffects {
                effects: specs,
                probability,
            })
        }
        "minecraft:clear_all_effects" => Some(ConsumeEffect::ClearAllEffects),
        "minecraft:remove_effects" => {
            let mut targets = Vec::new();
            if let Some(eff) = effect.get("effects") {
                if let Some(s) = eff.as_str() {
                    targets.push(s.to_string());
                } else if let Some(arr) = eff.as_array() {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            targets.push(s.to_string());
                        }
                    }
                }
            }
            Some(ConsumeEffect::RemoveEffects(targets))
        }
        "minecraft:teleport_randomly" => Some(ConsumeEffect::TeleportRandomly(
            horizon_world::teleport_consume::TeleportConsume {
                diameter: effect
                    .get("diameter")
                    .and_then(serde_json::Value::as_f64)
                    .map_or(16.0, |d| d as f32),
                directional_particles: effect
                    .get("directional_particles")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
            },
        )),
        "minecraft:play_sound" => effect
            .get("sound")
            .and_then(|s| s.as_str())
            .map(|s| ConsumeEffect::PlaySound(s.to_string())),
        k if !k.starts_with("minecraft:") => Some(ConsumeEffect::Native {
            kind: k.to_owned(),
            data: Arc::new(horizon_core::json_nbt::json_to_tag(effect).ok()?),
        }),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct ItemInfo {
    pub id: Arc<str>,
    pub max_stack: u32,
    pub max_damage: u32,
    /// Prototype damage, cached when default components are loaded.
    pub damage: i32,
    pub block: Option<BlockStateId>,
    pub tool: Vec<ToolRule>,
    pub tool_default_speed: f32,
    /// The `tool` component's `damage_per_block`, when the item has the
    /// component (a mace or trident has one without rules).
    pub tool_damage_per_block: Option<u32>,
    /// `tool.can_destroy_blocks_in_creative` (defaults to true).
    pub can_destroy_blocks_in_creative: bool,
    pub food: Option<(u32, f32)>,
    pub can_always_eat: bool,
    /// `consumable`: its timing, sound, particles and `on_consume_effects`.
    pub consumable: Option<horizon_world::consumable::Consumable<String, ConsumeEffect>>,
    /// Death protection effects decoded at definition load.
    pub death_protection: Option<Vec<ConsumeEffect>>,
    /// Kinetic pose timing decoded once from the shared item prototype.
    pub kinetic_timing: Option<crate::kinetic_weapon::UseTiming>,
    /// `use_remainder`: what a finished use leaves (`UseRemainder`).
    pub use_remainder: Option<String>,
    /// `use_cooldown`: the cooldown a use starts (`UseCooldown`).
    pub use_cooldown: Option<crate::cooldowns::UseCooldown>,
    pub model: String,
    /// Main-hand `attack_damage` / `attack_speed` modifier amounts.
    pub attack_damage: f32,
    pub attack_speed: f32,
    /// `cooking_fuel`: burn-time and speed-multiplier provider references.
    pub fuel: Option<(Value, Value)>,
    /// `brewing_fuel`: uses and speed-multiplier provider references.
    pub brewing_fuel: Option<(Value, Value)>,
    /// `compostable.layers`: the layers provider (`ComposterBlock.addLayer`).
    pub compostable: Option<Value>,
    /// Item left behind after crafting/fuel use (e.g. bucket).
    pub remainder: Option<String>,
    /// `equippable.slot` (head, chest, legs, feet, ...).
    pub equip_slot: Option<String>,
    /// `equippable.asset_id`: the equipment asset drawn when worn.
    pub equip_asset: Option<String>,
    /// Texture overlaid when worn in first person.
    pub equip_camera_overlay: Option<String>,
    /// All `attribute_modifiers`.
    pub modifiers: Vec<crate::item_data::ModifierRow>,
    /// `blocks_attacks` (shields).
    pub blocks_attacks: Option<BlocksAttacks>,
    /// The arm pose a mod item declares for a humanoid holding it (`arm_pose`).
    pub arm_pose: Option<horizon_core::arm_pose::ArmPose>,
    /// `weapon.disable_blocking_for_seconds` (axes).
    pub disable_blocking_seconds: f32,
    /// `weapon.item_damage_per_attack`, when the item is a weapon.
    pub weapon_damage: Option<u32>,
    /// `villager_food.nutrition`: what a villager gets from eating it.
    pub villager_food: Option<i32>,
    /// `use_effects.speed_multiplier` (UseEffects.DEFAULT: 0.2).
    pub use_speed: f32,
    /// `use_effects.can_sprint` (default false): sprinting may start while the
    /// item is in use.
    pub can_sprint: bool,
    /// `use_effects.interact_vibrations` (default true).
    pub interact_vibrations: bool,
    /// `glider` (elytra).
    pub glider: bool,
    /// `equippable.swappable` and `equip_sound`.
    pub swappable: bool,
    /// `equippable.dispensable` (dispensers may put it on an entity).
    pub equip_dispensable: bool,
    /// `equippable.damage_on_hurt`: the item wears when its wearer is hurt
    /// (false for the elytra).
    pub equip_damage_on_hurt: bool,
    /// `equippable.allowed_entities` resolved to entity type ids; None allows all.
    pub equip_allowed: Option<HashSet<String>>,
    pub equip_sound: Option<String>,
    /// `jukebox_playable`: the jukebox song it plays.
    pub jukebox_song: Option<String>,
    /// `crafting_remainder`: what stays in the grid when the item is used in a recipe.
    pub crafting_remainder: Option<String>,
    /// `dye`: the dye colour name (dyes, and anything else with the component).
    pub dye: Option<String>,
    /// `cushion/color`: the DyeColor name of a cushion item.
    pub cushion_color: Option<String>,
    /// `block_transformer`: the registry entry an axe, hoe or shovel uses on blocks.
    pub block_transformer: Option<String>,
    /// Has a default `bundle_contents` (the bundles, `BundleItem`).
    pub bundle: bool,
    /// The ids of the item's default components (its prototype's keys).
    pub components: Vec<String>,
    /// Opaque native component defaults, keyed by their registered name.
    pub custom_components: std::collections::BTreeMap<String, horizon_core::nbt::Tag>,
    /// A content mod's item: the vanilla item it copies (its `template`), whose
    /// behaviour it has where Horizon follows the template (`ItemInfo::is`).
    /// None for vanilla's items.
    pub template: Option<Arc<str>>,
}
impl ItemInfo {
    /// Whether this item is the vanilla item `vanilla`, or a content mod's item
    /// copied from it: what vanilla's `instanceof` on the item's class asks
    /// (a content item copies its template's class, as a NeoForge mod's
    /// `new FishingRodItem(...)` would).
    pub fn is(&self, vanilla: &str) -> bool {
        &*self.id == vanilla || self.template.as_deref() == Some(vanilla)
    }
}
/// `Item.Properties.craftRemainder` (not a data component, so not in the
/// default-component report): what an item leaves in the grid when crafted with.
pub const CRAFT_REMAINDERS: [(&str, &str); 5] = [
    ("minecraft:water_bucket", "minecraft:bucket"),
    ("minecraft:lava_bucket", "minecraft:bucket"),
    ("minecraft:milk_bucket", "minecraft:bucket"),
    ("minecraft:dragon_breath", "minecraft:glass_bottle"),
    ("minecraft:honey_bottle", "minecraft:glass_bottle"),
];
/// In a root built by `mod_content::build`: the content mods' items' crafting
/// remainders, by item. Other roots have none.
pub const MOD_CRAFT_REMAINDERS: &str = "horizon-mod-craft-remainders.json";
/// In a root built by `mod_content::build`: the content mods' items' vanilla
/// templates, by item (`ItemInfo::template`). Other roots have none.
pub const MOD_ITEM_TEMPLATES: &str = "horizon-mod-item-templates.json";
/// The mod items' declared arm poses (`arm_pose`): item id to the pose's name.
pub const MOD_ITEM_ARM_POSES: &str = "horizon-mod-item-arm-poses.json";
/// Explicit item-to-block mappings from content mods (not block drops).
pub const MOD_ITEM_BLOCKS: &str = "horizon-mod-item-blocks.json";
/// A reduction of `blocks_attacks.damage_reductions`.
#[derive(Clone, Debug)]
pub struct BlockReduction {
    pub horizontal_angle: f32,
    /// Damage type ids or `#tags`; None blocks every type.
    pub types: Option<Vec<String>>,
    pub base: f32,
    pub factor: f32,
}
/// The `blocks_attacks` component.
#[derive(Clone, Debug)]
pub struct BlocksAttacks {
    pub delay_seconds: f32,
    pub disable_scale: f32,
    pub reductions: Vec<BlockReduction>,
    /// item_damage: (threshold, base, factor)
    pub item_damage: (f32, f32, f32),
    /// `bypassed_by`: damage type ids and `#tags`, as `RegistryCodecs.holderSet` takes them.
    pub bypassed_by: Option<Vec<String>>,
    pub block_sound: Option<String>,
    pub disabled_sound: Option<String>,
}
/// A holder set's JSON forms: one id (or `#tag`), or a list of them
/// (`ExtraCodecs.compactListCodec`).
fn holder_set(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    }
}
/// `SoundEvent.CODEC`: an id, or the inline `{sound_id, range}`.
fn sound_event(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("sound_id")?.as_str().map(str::to_owned),
        _ => None,
    }
}
impl BlocksAttacks {
    /// The `blocks_attacks` `stack` has (`ItemStack.get(DataComponents.BLOCKS_ATTACKS)`):
    /// its own component when it carries one (`/give @s diamond_sword[blocks_attacks={}]`,
    /// or a server's item), else its item's (`item`, a shield's).
    pub fn of_stack<'a>(
        stack: &ItemStack,
        item: Option<&'a ItemInfo>,
    ) -> Option<std::borrow::Cow<'a, Self>> {
        if let Some(own) = stack
            .data()
            .and_then(|d| d.other("minecraft:blocks_attacks"))
        {
            let json = horizon_client_core::scoreboard::tag_to_json(own);
            return Self::parse(&json).map(std::borrow::Cow::Owned);
        }
        item?
            .blocks_attacks
            .as_ref()
            .map(std::borrow::Cow::Borrowed)
    }
    /// `blocks_attacks={}`: the codec's defaults, what a stack that only a mod's item
    /// ability makes block attacks (`shield_block`) blocks with.
    pub fn bare() -> &'static Self {
        static DEFAULT: std::sync::LazyLock<BlocksAttacks> = std::sync::LazyLock::new(|| {
            BlocksAttacks::parse(&Value::Object(Default::default())).expect("an object parses")
        });
        &DEFAULT
    }
    fn parse(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let f = |v: &Value, d: f64| v.as_f64().unwrap_or(d) as f32;
        let reductions = match v["damage_reductions"].as_array() {
            Some(list) => list
                .iter()
                .map(|r| BlockReduction {
                    horizontal_angle: f(&r["horizontal_blocking_angle"], 90.0),
                    types: holder_set(&r["type"]),
                    base: f(&r["base"], 0.0),
                    factor: f(&r["factor"], 0.0),
                })
                .collect(),
            // BlocksAttacks.CODEC default: block everything within 90 degrees.
            None => vec![BlockReduction {
                horizontal_angle: 90.0,
                types: None,
                base: 0.0,
                factor: 1.0,
            }],
        };
        let d = &v["item_damage"];
        Some(Self {
            delay_seconds: f(&v["block_delay_seconds"], 0.0),
            disable_scale: f(&v["disable_cooldown_scale"], 1.0),
            reductions,
            item_damage: if d.is_object() {
                (
                    f(&d["threshold"], 1.0),
                    f(&d["base"], 0.0),
                    f(&d["factor"], 1.0),
                )
            } else {
                (1.0, 0.0, 1.0)
            },
            bypassed_by: holder_set(&v["bypassed_by"]),
            block_sound: sound_event(&v["block_sound"]),
            disabled_sound: sound_event(&v["disabled_sound"]),
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub item: Arc<str>,
    pub count: u32,
    pub damage: u32,
    /// Components beyond count and damage (see `item_data`); None when default.
    pub extra: Option<Box<crate::item_data::StackData>>,
}
/// `order`'s items in the registry's order (`command-registries.json`'s
/// `items`, exported from the game); `order` itself when that is missing.
fn registry_order(root: &Path, order: &[Arc<str>]) -> Vec<Arc<str>> {
    let listed: Vec<String> = fs::read(root.join("command-registries.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v["items"].clone()).ok())
        .unwrap_or_default();
    if listed.is_empty() {
        return order.to_vec();
    }
    let known: HashSet<&str> = order.iter().map(|i| &**i).collect();
    listed
        .iter()
        .filter(|i| known.contains(i.as_str()))
        .map(|i| Arc::from(i.as_str()))
        .collect()
}
/// Item ids standing for a loot table's `minecraft:dynamic` entries.
const DYNAMIC: &str = "#dynamic:";
/// The item id standing for a chosen `minecraft:loot_table` entry, whose
/// table then rolls in full (`NestedLootTable`).
const NESTED: &str = "#loot_table";
thread_local! {
    /// The nested tables rolling now, outermost first: a table never rolls
    /// inside itself (`LootContext.pushVisitedElement`).
    static NESTED_ROLLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
#[derive(Clone)]
pub struct Items {
    pub items: HashMap<Arc<str>, ItemInfo>,
    /// Shared Block.getItem snapshot, including blocks sharing another block's item.
    pub block_items: Arc<HashMap<String, String>>,
    /// Registry snapshot used to validate ingredient component payloads.
    pub component_registry: Arc<horizon_client_core::registries::Registries>,
    /// Loaded component prototypes for concrete ingredient predicates.
    pub component_defaults: Arc<HashMap<String, horizon_core::nbt::Compound>>,
    /// Default `minecraft:mob_visibility` by item id, with the `equippable`
    /// slot it applies in (`LivingEntity.getVisibilityPercent`).
    mob_visibility: HashMap<Arc<str>, (String, MobVisibility)>,
    pub order: Vec<Arc<str>>,
    /// The same items in item registry (network id) order, as vanilla lists
    /// them; sorted by name when the registry export is missing.
    pub registry_order: Vec<Arc<str>>,
    /// Block tag name (without '#') -> member block names.
    pub block_tags: HashMap<String, HashSet<String>>,
    pub item_tags: HashMap<String, HashSet<String>>,
    /// Fluid tag name -> member fluid names (`minecraft:water` holds `minecraft:water` and
    /// `minecraft:flowing_water`).
    pub fluid_tags: HashMap<String, HashSet<String>>,
    /// Item tags in vanilla's order (`TagLoader`: file order, nested tags in
    /// place, first occurrence kept), the order `TagSlotDisplay` resolves.
    pub item_tag_order: HashMap<String, Vec<String>>,
    /// Block tags in the same order.
    pub block_tag_order: HashMap<String, Vec<String>>,
    /// Fluid tags in the same order.
    pub fluid_tag_order: HashMap<String, Vec<String>>,
    loot: Arc<horizon_world::loot::LootTables>,
    /// `Registries.PREDICATE`: what a table's string condition names, which a
    /// pack replaces or adds to (`LootItemCondition.CODEC`).
    predicates: Arc<HashMap<String, Value>>,
    /// The `jukebox_song` registry.
    pub songs: HashMap<String, Song>,
    /// The `instrument` registry (goat horns).
    pub instruments: HashMap<String, Instrument>,
    /// The enchantment registry: loot functions enchant the items they roll
    /// (`EnchantWithLevelsFunction`, `EnchantRandomlyFunction`), and the
    /// station menus share this copy.
    pub enchantments: Arc<crate::enchant::Enchantments>,
}
impl Items {
    /// This table with a server's block and item tags in place of its own
    /// (`ClientboundUpdateTagsPacket`): each tag's members in the order sent,
    /// which is `TagLoader`'s. `None` keeps this table's tags of that kind.
    pub fn with_tags(
        &self,
        block_tags: Option<HashMap<String, Vec<String>>>,
        item_tags: Option<HashMap<String, Vec<String>>>,
    ) -> Self {
        let mut items = self.clone();
        if let Some(tags) = block_tags {
            items.block_tag_order = tags.clone();
            items.block_tags = tags
                .into_iter()
                .map(|(name, members)| (name, members.into_iter().collect()))
                .collect();
        }
        if let Some(tags) = item_tags {
            items.item_tags = tags
                .iter()
                .map(|(name, members)| (name.clone(), members.iter().cloned().collect()))
                .collect();
            items.item_tag_order = tags;
        }
        items
    }
}
/// A `JukeboxSong`.
#[derive(Clone, Debug)]
pub struct Song {
    pub sound: String,
    pub length_ticks: i64,
    pub comparator: i32,
    pub description: Value,
}
impl Song {
    /// `hasFinished`: the length plus one second.
    pub fn finished(&self, ticks: i64) -> bool {
        ticks >= self.length_ticks + 20
    }
}
/// An `Instrument`: what a goat horn plays.
#[derive(Clone, Debug)]
pub struct Instrument {
    pub sound: String,
    /// Seconds the horn stays up, and its cooldown.
    pub use_duration: f32,
    /// How far it carries, in blocks (`range / 16` is its volume).
    pub range: f32,
}
impl Instrument {
    /// `InstrumentItem.getUseDuration`: `Mth.floor(useDuration * 20)`.
    pub fn use_ticks(&self) -> i32 {
        instrument_use_ticks(self.use_duration)
    }
}
/// `InstrumentItem.getUseDuration`: floor after the float multiplication.
pub fn instrument_use_ticks(seconds: f32) -> i32 {
    (seconds * 20.0).floor() as i32
}
/// The id of a goat horn's instrument (`InstrumentItem.getInstrument`): the
/// stack's own `instrument` component, else the item's `ponder_goat_horn`.
/// None for an inline instrument (see `inline_instrument`).
pub fn instrument_id(stack: &ItemStack) -> Option<String> {
    if &*stack.item != "minecraft:goat_horn" {
        return None;
    }
    match stack.data().and_then(|d| d.other("minecraft:instrument")) {
        Some(horizon_core::nbt::Tag::String(id)) => Some(id.to_string_lossy()),
        Some(horizon_core::nbt::Tag::Compound(_)) => None,
        _ => Some("minecraft:ponder_goat_horn".to_owned()),
    }
}
/// A goat horn's inline `instrument` component (`Instrument.CODEC` also
/// takes the instrument itself instead of a registry id).
pub fn inline_instrument(stack: &ItemStack) -> Option<&horizon_core::nbt::Tag> {
    if &*stack.item != "minecraft:goat_horn" {
        return None;
    }
    match stack.data().and_then(|d| d.other("minecraft:instrument")) {
        Some(tag @ horizon_core::nbt::Tag::Compound(_)) => Some(tag),
        _ => None,
    }
}
/// `Instrument.DIRECT_CODEC`: a `sound_event` that is an id or `{sound_id}`,
/// `use_duration` and `range`.
fn parse_instrument(v: &Value) -> Instrument {
    let sound = match &v["sound_event"] {
        Value::String(s) => s.clone(),
        other => other["sound_id"].as_str().unwrap_or("").to_owned(),
    };
    Instrument {
        sound,
        use_duration: v["use_duration"].as_f64().unwrap_or(0.0) as f32,
        range: v["range"].as_f64().unwrap_or(0.0) as f32,
    }
}
#[path = "condition_files.rs"]
mod condition_files;
pub(crate) use condition_files::data_files;
fn load_instruments(root: &Path, overlay: &PackOverlay) -> HashMap<String, Instrument> {
    data_files(root, "instrument", overlay)
        .into_iter()
        .map(|(id, v)| (id, parse_instrument(&v)))
        .collect()
}
fn load_songs(root: &Path, overlay: &PackOverlay) -> HashMap<String, Song> {
    data_files(root, "jukebox_song", overlay)
        .into_iter()
        .map(|(id, v)| {
            let sound = match &v["sound_event"] {
                Value::String(s) => s.clone(),
                other => other["sound_id"].as_str().unwrap_or("").to_owned(),
            };
            (
                id,
                Song {
                    sound,
                    length_ticks: (v["length_in_seconds"].as_f64().unwrap_or(0.0) as f32 * 20.0)
                        .ceil() as i64,
                    comparator: v["comparator_output"].as_i64().unwrap_or(0) as i32,
                    description: v["description"].clone(),
                },
            )
        })
        .collect()
}
pub fn load_tags(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay,
) -> Result<HashMap<String, HashSet<String>>> {
    Ok(load_tag_lists(root, kind, overlay)?
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}
/// The fluid tags of `root` and `overlay` (`data/<ns>/tags/fluid`): each tag
/// id to the fluids it names, in the sources' order. What a recipe load puts
/// into its context, for the fluid ingredients' `#tags`.
pub fn fluid_tag_list(root: &Path, overlay: &PackOverlay) -> Result<HashMap<String, Vec<String>>> {
    load_tag_lists(root, "fluid", overlay)
}
/// The registries whose tags a mod can list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagRegistry {
    Block,
    Item,
    Fluid,
}
/// Every tag's contents in `TagLoader.tryBuildTag` order (a `LinkedHashSet`).
/// `overlay` adds a world's data packs over the game's own tag files.
pub(crate) fn load_tag_lists(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay,
) -> Result<HashMap<String, Vec<String>>> {
    let mut raw: HashMap<String, TagLayers> = HashMap::new();
    /// Through `json_folder`: from the prepared data file, and read once for
    /// the tables a load builds from the same tags.
    fn walk(dir: &Path, namespace: &str, raw: &mut HashMap<String, TagLayers>) -> Result<()> {
        for file in &horizon_world::json_folder::read(dir)?.files {
            let v: Value = serde_json::from_slice(file.bytes()?)?;
            raw.insert(
                format!("{namespace}:{}", file.name),
                TagLayers::from_file(&v),
            );
        }
        Ok(())
    }
    // Vanilla's tags; with content mods also the convention (`c`) tags and the mods' own.
    for namespace in crate::content_namespaces::tag_namespaces(root) {
        let dir = root.join(format!("data/{namespace}/tags/{kind}"));
        if dir.exists() {
            walk(&dir, &namespace, &mut raw)?;
        }
    }
    // The packs' entries join the game's own, in pack order; a `replace`
    // file drops what the lower sources put in and `remove` takes entries
    // out (`TagLoader.load` walks the resource stack the same way).
    // `tag_kind` reads the packs' keys
    // (`tags/<ns>:<registry>/<path>`) and yields the `ns:path` ids the raw
    // map is keyed by.
    for (id, files) in overlay.tag_kind(kind) {
        for bytes in files {
            let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                continue;
            };
            raw.entry(id.to_owned()).or_default().push(&v);
        }
    }
    fn resolve(name: &str, raw: &HashMap<String, TagLayers>, depth: u32) -> Vec<String> {
        match raw.get(name) {
            Some(layers) if depth <= 32 => layers.build(|tag| resolve(tag, raw, depth + 1)),
            _ => Vec::new(),
        }
    }
    Ok(raw
        .keys()
        .map(|k| (k.clone(), resolve(k, &raw, 0)))
        .collect())
}
/// An entity type id or `#tag` as the set of entity type ids it names.
fn entity_spec(tags: &HashMap<String, HashSet<String>>, spec: &str) -> HashSet<String> {
    match spec.strip_prefix('#') {
        Some(tag) => tags.get(tag).cloned().unwrap_or_default(),
        None => HashSet::from([spec.to_owned()]),
    }
}
/// `MobVisibility`: entity types a worn item hides its wearer from, and the
/// factor `LivingEntity.getVisibilityPercent` multiplies by.
#[derive(Clone, Debug)]
pub struct MobVisibility {
    pub targeting: HashSet<String>,
    pub visibility: f32,
}
/// `minecraft:mob_visibility` (`targeting_entity_types` is one id, a `#tag`,
/// or a list) when the object is present and names at least one type.
fn parse_mob_visibility(
    v: &Value,
    tags: &HashMap<String, HashSet<String>>,
) -> Option<MobVisibility> {
    if !v.is_object() {
        return None;
    }
    let visibility = v.get("visibility")?.as_f64()? as f32;
    let targeting = match v.get("targeting_entity_types")? {
        Value::String(spec) => entity_spec(tags, spec),
        Value::Array(list) => list
            .iter()
            .filter_map(Value::as_str)
            .flat_map(|spec| entity_spec(tags, spec))
            .collect(),
        _ => return None,
    };
    if targeting.is_empty() {
        return None;
    }
    Some(MobVisibility {
        targeting,
        visibility,
    })
}
/// The item's `equippable.slot` and `mob_visibility`, when both are set.
/// `getVisibilityPercent` multiplies only when the stack is in that slot.
fn worn_mob_visibility(
    components: &Value,
    tags: &HashMap<String, HashSet<String>>,
) -> Option<(String, MobVisibility)> {
    let slot = components
        .get("minecraft:equippable")?
        .get("slot")?
        .as_str()?
        .to_owned();
    Some((
        slot,
        parse_mob_visibility(components.get("minecraft:mob_visibility")?, tags)?,
    ))
}

impl Items {
    /// Load item data; bind the world's damage catalog before preparing recipes.
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        facts: &BlockFacts,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
    ) -> Result<Self> {
        let mut block_for_item: HashMap<&str, BlockStateId> = HashMap::new();
        for (i, s) in facts.states.iter().enumerate() {
            let default = registry.default_state(&s.name);
            if default == Some(BlockStateId(i as u32)) && &*s.item != "minecraft:air" {
                block_for_item
                    .entry(&s.item)
                    .or_insert(BlockStateId(i as u32));
            }
        }
        // Block.asItem is not an inverse of BlockItem.getBlock: several items
        // may place the same block. Read their normal compiled bindings too.
        #[derive(Default, serde::Deserialize)]
        struct Bindings {
            #[serde(default)]
            items: HashMap<String, Binding>,
        }
        #[derive(serde::Deserialize)]
        struct Binding {
            #[serde(default)]
            class: Vec<String>,
            #[serde(default)]
            fields: Fields,
        }
        #[derive(Default, serde::Deserialize)]
        struct Fields {
            block: Option<String>,
        }
        let bindings = match fs::read(root.join("block-behaviours.json")) {
            Ok(bytes) => serde_json::from_slice::<Bindings>(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Bindings::default(),
            Err(error) => return Err(error.into()),
        };
        for (item, binding) in &bindings.items {
            if binding.class.iter().any(|name| name == "BlockItem")
                && let Some(state) = binding
                    .fields
                    .block
                    .as_deref()
                    .and_then(|block| registry.default_state(block))
            {
                block_for_item.entry(item).or_insert(state);
            }
        }
        let mut items = Self::load_with(root, &block_for_item, overlay, defaults)?;
        items.block_items = Arc::new(
            facts
                .states
                .iter()
                .map(|state| (state.name.to_string(), state.item.to_string()))
                .collect(),
        );
        let path = root.join(MOD_ITEM_BLOCKS);
        match fs::read(&path) {
            Ok(bytes) => {
                let mappings: HashMap<String, String> = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                for (item, block) in mappings {
                    let state = registry
                        .default_state(&block)
                        .ok_or_else(|| format!("{item}: block {block:?} is not a block"))?;
                    let info = items
                        .items
                        .get_mut(item.as_str())
                        .ok_or_else(|| format!("{item}: not an item"))?;
                    info.block = Some(state);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        }
        Ok(items)
    }
    /// Every item's defaults, tags and block-to-item identities, without
    /// placement state ids (for headless tools that never place blocks).
    pub fn catalog(root: &Path, overlay: &PackOverlay, defaults: &ItemDefaults) -> Result<Self> {
        let mut items = Self::load_with(root, &HashMap::new(), overlay, defaults)?;
        items.block_items = Arc::new(ingredient_defaults::block_items(root)?);
        ingredient_defaults::load_component_damage_registry(root, overlay, &mut items)?;
        Ok(items)
    }
    /// Build a standalone catalog against the owning world's already loaded
    /// damage registry. Reloading recipes must not decode new damage definitions.
    pub fn catalog_with_damage_registry(
        root: &Path,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
        damage: horizon_client_core::registries::RegistryIds,
    ) -> Result<Self> {
        let mut items = Self::load_with(root, &HashMap::new(), overlay, defaults)?;
        items.block_items = Arc::new(ingredient_defaults::block_items(root)?);
        Arc::make_mut(&mut items.component_registry)
            .by_registry
            .insert("minecraft:damage_type".into(), damage);
        Ok(items)
    }
    fn load_with(
        root: &Path,
        block_for_item: &HashMap<&str, BlockStateId>,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
    ) -> Result<Self> {
        let entity_tags = load_tags(root, "entity_type", overlay)?;
        let by_item = |name: &str| -> Result<HashMap<String, String>> {
            let path = root.join(name);
            match fs::read(&path) {
                Ok(bytes) => Ok(serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
                Err(e) => Err(format!("{}: {e}", path.display()).into()),
            }
        };
        let mod_remainders = by_item(MOD_CRAFT_REMAINDERS)?;
        let mod_templates = by_item(MOD_ITEM_TEMPLATES)?;
        let path = root.join(MOD_ITEM_ARM_POSES);
        let mod_arm_poses: HashMap<String, horizon_core::arm_pose::ArmPose> =
            match fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
                Err(e) => return Err(format!("{}: {e}", path.display()).into()),
            };
        let mut items = HashMap::new();
        let mut order = Vec::new();
        let mut mob_visibility = HashMap::new();
        for (name, c) in defaults.entries() {
            let id: Arc<str> = Arc::from(name.as_str());
            let tool = c["minecraft:tool"]["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| ToolRule {
                    blocks: match &r["blocks"] {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(a) => a
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect(),
                        _ => vec![],
                    },
                    speed: r["speed"].as_f64().map(|v| v as f32),
                    correct: r["correct_for_drops"].as_bool(),
                })
                .collect();
            let modifier = |kind: &str| -> f32 {
                c["minecraft:attribute_modifiers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| {
                        m["type"].as_str() == Some(kind)
                            && m["operation"].as_str() == Some("add_value")
                    })
                    .filter_map(|m| m["amount"].as_f64())
                    .sum::<f64>() as f32
            };
            let food = c.get("minecraft:food").map(|f| {
                (
                    f["nutrition"].as_u64().unwrap_or(0) as u32,
                    f["saturation"].as_f64().unwrap_or(0.0) as f32,
                )
            });
            let can_always_eat = c
                .get("minecraft:food")
                .and_then(|f| f.get("can_always_eat"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            items.insert(
                id.clone(),
                ItemInfo {
                    id: id.clone(),
                    max_stack: c["minecraft:max_stack_size"].as_u64().unwrap_or(64) as u32,
                    max_damage: c["minecraft:max_damage"].as_u64().unwrap_or(0) as u32,
                    damage: c["minecraft:damage"].as_i64().unwrap_or(0).max(0) as i32,
                    block: block_for_item.get(name.as_str()).copied(),
                    tool,
                    tool_default_speed: c["minecraft:tool"]["default_mining_speed"]
                        .as_f64()
                        .unwrap_or(1.0) as f32,
                    tool_damage_per_block: c
                        .get("minecraft:tool")
                        .map(|t| t["damage_per_block"].as_u64().unwrap_or(1) as u32),
                    can_destroy_blocks_in_creative:
                        c["minecraft:tool"]["can_destroy_blocks_in_creative"]
                            .as_bool()
                            .unwrap_or(true),
                    food,
                    can_always_eat,
                    kinetic_timing: c
                        .get("minecraft:kinetic_weapon")
                        .and_then(crate::kinetic_weapon::UseTiming::from_json),
                    // Consumable.CODEC's defaults.
                    death_protection: c
                        .get("minecraft:death_protection")
                        .map(parse_death_effects)
                        .transpose()
                        .map_err(|e| format!("{name}: {e}"))?,
                    consumable: c
                        .get("minecraft:consumable")
                        .map(parse_consumable)
                        .transpose()
                        .map_err(|e| format!("{name}: {e}"))?,
                    use_remainder: c["minecraft:use_remainder"]["id"]
                        .as_str()
                        .map(str::to_owned),
                    use_cooldown: c
                        .get("minecraft:use_cooldown")
                        .and_then(crate::cooldowns::UseCooldown::from_json),
                    model: c["minecraft:item_model"]
                        .as_str()
                        .unwrap_or(name)
                        .to_owned(),
                    glider: c["minecraft:glider"].is_object(),
                    swappable: c["minecraft:equippable"]["swappable"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_dispensable: c["minecraft:equippable"]["dispensable"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_damage_on_hurt: c["minecraft:equippable"]["damage_on_hurt"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_allowed: match &c["minecraft:equippable"]["allowed_entities"] {
                        Value::String(spec) => Some(entity_spec(&entity_tags, spec)),
                        Value::Array(list) => Some(
                            list.iter()
                                .filter_map(Value::as_str)
                                .flat_map(|spec| entity_spec(&entity_tags, spec))
                                .collect(),
                        ),
                        _ => None,
                    },
                    equip_sound: c["minecraft:equippable"]["equip_sound"]
                        .as_str()
                        .map(str::to_owned),
                    jukebox_song: c["minecraft:jukebox_playable"].as_str().map(str::to_owned),
                    dye: c["minecraft:dye"].as_str().map(str::to_owned),
                    cushion_color: c["minecraft:cushion/color"].as_str().map(str::to_owned),
                    block_transformer: c["minecraft:block_transformer"].as_str().map(str::to_owned),
                    bundle: c.get("minecraft:bundle_contents").is_some(),
                    components: c
                        .as_object()
                        .map(|c| c.keys().cloned().collect())
                        .unwrap_or_default(),
                    custom_components: c
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(name, _)| !name.starts_with("minecraft:"))
                        .map(|(name, value)| {
                            (
                                name.clone(),
                                horizon_client_core::scoreboard::json_to_tag(value),
                            )
                        })
                        .collect(),
                    crafting_remainder: CRAFT_REMAINDERS
                        .iter()
                        .find(|(item, _)| *item == name)
                        .map(|(_, rest)| (*rest).to_owned())
                        .or_else(|| mod_remainders.get(name).cloned()),
                    template: mod_templates.get(name).map(|t| Arc::from(t.as_str())),
                    villager_food: c["minecraft:villager_food"]["nutrition"]
                        .as_i64()
                        .map(|n| n as i32),
                    use_speed: c["minecraft:use_effects"]["speed_multiplier"]
                        .as_f64()
                        .unwrap_or(0.2) as f32,
                    can_sprint: c["minecraft:use_effects"]["can_sprint"]
                        .as_bool()
                        .unwrap_or(false),
                    interact_vibrations: c["minecraft:use_effects"]["interact_vibrations"]
                        .as_bool()
                        .unwrap_or(true),
                    blocks_attacks: BlocksAttacks::parse(&c["minecraft:blocks_attacks"]),
                    arm_pose: mod_arm_poses.get(name).copied(),
                    disable_blocking_seconds: c["minecraft:weapon"]["disable_blocking_for_seconds"]
                        .as_f64()
                        .unwrap_or(0.0) as f32,
                    weapon_damage: c["minecraft:weapon"].is_object().then(|| {
                        c["minecraft:weapon"]["item_damage_per_attack"]
                            .as_u64()
                            .unwrap_or(1) as u32
                    }),
                    equip_asset: c["minecraft:equippable"]["asset_id"]
                        .as_str()
                        .map(str::to_owned),
                    equip_camera_overlay: c["minecraft:equippable"]["camera_overlay"]
                        .as_str()
                        .map(str::to_owned),
                    equip_slot: c["minecraft:equippable"]["slot"]
                        .as_str()
                        .map(str::to_owned),
                    modifiers: c["minecraft:attribute_modifiers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| {
                            Some(crate::item_data::ModifierRow::new(
                                m["type"].as_str()?,
                                m["id"].as_str()?,
                                m["amount"].as_f64()?,
                                m["operation"].as_str().unwrap_or("add_value"),
                                m["slot"].as_str().unwrap_or("any"),
                            ))
                        })
                        .collect(),
                    fuel: c
                        .get("minecraft:cooking_fuel")
                        .map(|f| (f["burn_time"].clone(), f["speed_multiplier"].clone())),
                    brewing_fuel: c
                        .get("minecraft:brewing_fuel")
                        .map(|f| (f["uses"].clone(), f["speed_multiplier"].clone())),
                    compostable: c.get("minecraft:compostable").map(|f| f["layers"].clone()),
                    remainder: c["minecraft:use_remainder"]["id"]
                        .as_str()
                        .or(c["minecraft:crafting_remainder"]["id"].as_str())
                        .or(c["minecraft:crafting_remainder"].as_str())
                        .map(str::to_owned),
                    attack_damage: modifier("minecraft:attack_damage"),
                    attack_speed: modifier("minecraft:attack_speed"),
                },
            );
            if let Some(parsed) = worn_mob_visibility(c, &entity_tags) {
                mob_visibility.insert(id.clone(), parsed);
            }
            order.push(id);
        }
        // Every loot table, keyed by its id (blocks/stone, entities/sheep/white,
        // gameplay/chicken_lay, shearing/sheep/red...), of every namespace
        // (content mods' tables, docs/modpacks.md), with the packs' over the
        // game's own: one map with the loot data of the same root and packs.
        let loot = horizon_world::loot::shared_tables(root, overlay)?;
        let predicates = horizon_world::loot::load_predicates(root, overlay)?;
        let registry_order = registry_order(root, &order);
        let enchantments = Arc::new(crate::enchant::Enchantments::load_with(
            root, overlay, defaults,
        )?);
        Ok(Self {
            items,
            block_items: Default::default(),
            component_registry: ingredient_defaults::component_registry(
                root,
                &enchantments,
                &entity_tags,
            )?,
            component_defaults: ingredient_defaults::component_defaults(defaults)?,
            mob_visibility,
            order,
            registry_order,
            block_tags: load_tags(root, "block", overlay)?,
            item_tags: load_tags(root, "item", overlay)?,
            fluid_tags: load_tags(root, "fluid", overlay)?,
            item_tag_order: load_tag_lists(root, "item", overlay)?,
            block_tag_order: load_tag_lists(root, "block", overlay)?,
            fluid_tag_order: load_tag_lists(root, "fluid", overlay)?,
            loot,
            predicates: Arc::new(predicates),
            songs: load_songs(root, overlay),
            instruments: load_instruments(root, overlay),
            enchantments,
        })
    }
    /// Use the server's loaded loot-table snapshot for item and block drops.
    pub fn set_loot_tables(&mut self, tables: Arc<horizon_world::loot::LootTables>) {
        self.loot = tables;
    }

    pub fn get(&self, id: &str) -> Option<&ItemInfo> {
        self.items.get(id)
    }
    /// `item`'s `villager_food` nutrition (`DataComponents.VILLAGER_FOOD`) in
    /// this registry: 0 for an item without the component, `None` for an item
    /// the registry lacks.
    pub fn villager_food(&self, item: &str) -> Option<i32> {
        self.get(item).map(|i| i.villager_food.unwrap_or(0))
    }
    /// `ItemStack.get(DataComponents.MOB_VISIBILITY).visibility()` when the
    /// prototype's equippable slot is `slot` and `targeting` is in
    /// `targetingEntityTypes`. None otherwise (`getVisibilityPercent` skips it).
    pub fn worn_visibility(&self, item: &str, slot: &str, targeting: &str) -> Option<f32> {
        let (equip, vis) = self.mob_visibility.get(item)?;
        if equip == slot && vis.targeting.contains(targeting) {
            Some(vis.visibility)
        } else {
            None
        }
    }
    /// Record one prototype's `equippable` and `mob_visibility` the way `load`
    /// reads them from the default-component report.
    #[cfg(any(test, feature = "test-support"))]
    pub fn insert_mob_visibility_component(&mut self, id: &str, components: &Value) {
        if let Some(parsed) = worn_mob_visibility(components, &HashMap::new()) {
            self.mob_visibility.insert(Arc::from(id), parsed);
        }
    }
    /// The instrument a goat horn plays: its `instrument` component's
    /// registry entry or inline instrument.
    pub fn instrument(&self, stack: &ItemStack) -> Option<Instrument> {
        if let Some(tag) = inline_instrument(stack) {
            return Some(parse_instrument(
                &horizon_client_core::scoreboard::tag_to_json(tag),
            ));
        }
        self.instruments.get(&instrument_id(stack)?).cloned()
    }
    /// A table with nothing in it: the tags a client was sent go in with `set_tags`.
    pub fn empty() -> Self {
        Self {
            items: HashMap::new(),
            block_items: Default::default(),
            component_registry: Default::default(),
            component_defaults: Default::default(),
            mob_visibility: HashMap::new(),
            order: Vec::new(),
            registry_order: Vec::new(),
            block_tags: HashMap::new(),
            item_tags: HashMap::new(),
            fluid_tags: HashMap::new(),
            item_tag_order: HashMap::new(),
            block_tag_order: HashMap::new(),
            fluid_tag_order: HashMap::new(),
            loot: Default::default(),
            predicates: Default::default(),
            songs: HashMap::new(),
            instruments: HashMap::new(),
            enchantments: Arc::new(crate::enchant::Enchantments::default()),
        }
    }
    /// An item table with only tags, for unit tests of item rules.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(item_tags: HashMap<String, HashSet<String>>) -> Self {
        Self {
            item_tag_order: item_tags
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
                .collect(),
            item_tags,
            ..Self::empty()
        }
    }
    /// Replace one registry's tags with `tags`, each tag's members in order.
    pub fn set_tags(&mut self, registry: TagRegistry, tags: HashMap<String, Vec<String>>) {
        let (sets, order) = match registry {
            TagRegistry::Block => (&mut self.block_tags, &mut self.block_tag_order),
            TagRegistry::Item => (&mut self.item_tags, &mut self.item_tag_order),
            TagRegistry::Fluid => (&mut self.fluid_tags, &mut self.fluid_tag_order),
        };
        *sets = tags
            .iter()
            .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
            .collect();
        *order = tags;
    }
    /// Whether `block` is in the block tag `tag` (`minecraft:fire`, no `#`).
    pub fn block_in_tag(&self, tag: &str, block: &str) -> bool {
        self.block_tags.get(tag).is_some_and(|s| s.contains(block))
    }
    /// Whether `fluid` is in the fluid tag `tag` (`minecraft:water`, no `#`): `FluidState.is(TagKey)`.
    pub fn fluid_in_tag(&self, tag: &str, fluid: &str) -> bool {
        self.fluid_tags.get(tag).is_some_and(|s| s.contains(fluid))
    }
    /// A loaded tag's members in `TagLoader` order, or `None` for a tag that isn't loaded.
    pub fn tag_members(&self, registry: TagRegistry, tag: &str) -> Option<&[String]> {
        let order = match registry {
            TagRegistry::Block => &self.block_tag_order,
            TagRegistry::Item => &self.item_tag_order,
            TagRegistry::Fluid => &self.fluid_tag_order,
        };
        order.get(tag).map(Vec::as_slice)
    }
    /// Whether an item belongs to a loaded item tag.
    pub fn item_in_tag(&self, tag: &str, item: &str) -> bool {
        let normalized;
        let tag = if !tag.contains(':') || tag.starts_with(':') {
            normalized = format!("minecraft:{}", tag.strip_prefix(':').unwrap_or(tag));
            normalized.as_str()
        } else {
            tag
        };
        self.item_tags.get(tag).is_some_and(|s| s.contains(item))
    }
    fn block_matches(&self, spec: &str, block: &str) -> bool {
        match spec.strip_prefix('#') {
            Some(tag) => self.block_in_tag(tag, block),
            None => spec == block,
        }
    }
    /// `Item.canDestroyBlock`: a tool may forbid block destruction with instabuild.
    pub fn can_destroy_block(&self, held: Option<&ItemStack>, creative: bool) -> bool {
        if !creative {
            return true;
        }
        let Some(held) = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air") else {
            return true;
        };
        if held
            .data()
            .is_some_and(|d| d.other("!minecraft:tool").is_some())
        {
            return true;
        }
        if let Some(tool) = held.data().and_then(|d| d.other("minecraft:tool")) {
            return tool
                .get("can_destroy_blocks_in_creative")
                .and_then(|v| match v {
                    horizon_core::nbt::Tag::Byte(v) => Some(*v),
                    _ => None,
                })
                .is_none_or(|v| v != 0);
        }
        self.get(&held.item)
            .is_none_or(|info| info.can_destroy_blocks_in_creative)
    }
    /// `Tool.getMiningSpeed` for the held item (1.0 without a tool component).
    pub fn mining_speed(&self, held: Option<&ItemStack>, block: &str) -> f32 {
        let held = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        if let Some(stack) = held {
            if stack
                .data()
                .is_some_and(|d| d.other("!minecraft:tool").is_some())
            {
                return 1.0;
            }
            if let Some(tool) = stack.data().and_then(|d| d.other("minecraft:tool")) {
                return self
                    .stack_tool_rule(tool, block, "speed")
                    .and_then(tool_number)
                    .or_else(|| tool.get("default_mining_speed").and_then(tool_number))
                    .unwrap_or(1.0);
            }
        }
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return 1.0;
        };
        for rule in &info.tool {
            if let Some(speed) = rule.speed
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return speed;
            }
        }
        if info.tool.is_empty() {
            1.0
        } else {
            info.tool_default_speed
        }
    }
    /// `Tool.isCorrectForDrops`.
    pub fn correct_for_drops(&self, held: Option<&ItemStack>, block: &str) -> bool {
        let held = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        if let Some(stack) = held {
            if stack
                .data()
                .is_some_and(|d| d.other("!minecraft:tool").is_some())
            {
                return false;
            }
            if let Some(tool) = stack.data().and_then(|d| d.other("minecraft:tool")) {
                return horizon_world::datafix::dynamic::as_bool(
                    self.stack_tool_rule(tool, block, "correct_for_drops"),
                    false,
                );
            }
        }
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return false;
        };
        for rule in &info.tool {
            if let Some(correct) = rule.correct
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return correct;
            }
        }
        false
    }
    fn stack_tool_rule<'a>(
        &self,
        tool: &'a horizon_core::nbt::Tag,
        block: &str,
        field: &str,
    ) -> Option<&'a horizon_core::nbt::Tag> {
        use horizon_core::nbt::Tag;
        let Tag::List(rules) = tool.get("rules")? else {
            return None;
        };
        rules.iter().find_map(|rule| {
            let value = rule.get(field)?;
            let matches = |value: &Tag| match value {
                Tag::String(spec) => spec.to_string().is_ok_and(|s| {
                    let tag = s.starts_with('#');
                    let id = s.strip_prefix('#').unwrap_or(&s);
                    horizon_world::datafix::identifier(id).is_some_and(|(namespace, path)| {
                        let id = format!("{namespace}:{path}");
                        if tag {
                            self.block_in_tag(&id, block)
                        } else {
                            id == block
                        }
                    })
                }),
                _ => false,
            };
            match rule.get("blocks")? {
                Tag::List(blocks) if blocks.iter().any(matches) => Some(value),
                spec if matches(spec) => Some(value),
                _ => None,
            }
        })
    }
    /// The stack's TOOL component, including its default mining wear.
    pub fn mining_damage(&self, stack: &ItemStack) -> Option<u32> {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return None;
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:tool").is_some())
        {
            return None;
        }
        if let Some(tool) = stack.data().and_then(|d| d.other("minecraft:tool")) {
            return Some(
                horizon_world::datafix::dynamic::as_int(tool.get("damage_per_block"), 1).max(0)
                    as u32,
            );
        }
        self.get(&stack.item)?.tool_damage_per_block
    }
    /// `Weapon.disableBlockingForSeconds`: the stack's `weapon` component,
    /// else the item's prototype; zero without one.
    pub fn weapon_disable_blocking(&self, stack: &ItemStack) -> f32 {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return 0.0;
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:weapon").is_some())
        {
            return 0.0;
        }
        if let Some(weapon) = stack.data().and_then(|d| d.other("minecraft:weapon")) {
            return horizon_world::datafix::dynamic::as_float(
                weapon.get("disable_blocking_for_seconds"),
                0.0,
            );
        }
        self.get(&stack.item)
            .map_or(0.0, |i| i.disable_blocking_seconds)
    }

    /// `Weapon.itemDamagePerAttack`: the stack's `weapon` component, else the
    /// item's prototype. A removed component leaves no weapon.
    pub fn weapon_damage(&self, stack: &ItemStack) -> Option<u32> {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return None;
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:weapon").is_some())
        {
            return None;
        }
        if let Some(weapon) = stack.data().and_then(|d| d.other("minecraft:weapon")) {
            return Some(
                horizon_world::datafix::dynamic::as_int(weapon.get("item_damage_per_attack"), 1)
                    .max(0) as u32,
            );
        }
        self.get(&stack.item)?.weapon_damage
    }
    /// The stack's damage capacity (`crate::durability::stack_max_damage`).
    pub fn stack_max_damage(&self, stack: &ItemStack) -> u32 {
        crate::durability::stack_max_damage(self, stack)
    }
    /// The damage capacity mining wear uses (`stack_max_damage`).
    pub fn mining_damage_capacity(&self, stack: &ItemStack) -> u32 {
        self.stack_max_damage(stack)
    }
    /// Whether a loot table (`minecraft:blocks/stone`, ...) exists.
    pub fn has_loot_table(&self, id: &str) -> bool {
        self.loot.contains_key(id)
    }
    /// Every loot table's id: the root's (vanilla's and the content mods')
    /// and the packs' over them, in no order.
    pub fn loot_table_ids(&self) -> impl Iterator<Item = &str> {
        self.loot.ids()
    }
    /// `LootTable.getRandomItems` of an inline table.
    pub fn inline_loot_table<R: RandomSource>(
        &self,
        table: &Value,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.inline_loot_table_with_tool(table, None, random)
    }
    /// `LootCommand.dropFishingLoot`: evaluate an inline table with its TOOL.
    pub fn inline_loot_table_with_tool<R: RandomSource>(
        &self,
        table: &Value,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.inline_loot_table_for_command(table, held, random, false)
    }
    /// `LootCommand.dropFishingLoot`: output-list adapter; replacement keeps emitted empties.
    pub fn inline_loot_table_for_command<R: RandomSource>(
        &self,
        table: &Value,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let empty = horizon_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: horizon_world::loot::query::current_query(),
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player: false,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            location: None,
            keep_empty,
        };
        ctx.evaluate_root(table, random)
    }
    /// The id of the block's loot table (`minecraft:blocks/stone`), when it
    /// has one. `Blocks.wallVariant`: wall signs, banners, heads, fans and
    /// torches use the standing block's table.
    pub fn block_loot_table(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
    ) -> Option<String> {
        let block = registry.state(state)?;
        let (namespace, name) = block
            .name
            .split_once(':')
            .unwrap_or(("minecraft", &block.name));
        let own = format!("{namespace}:blocks/{name}");
        if self.loot.contains_key(&own) {
            return Some(own);
        }
        let standing = name.replacen("_wall_", "_", 1);
        let standing = standing.strip_prefix("wall_").unwrap_or(&standing);
        let standing = standing.replacen("wall_", "", 1);
        let standing = format!("{namespace}:blocks/{standing}");
        self.loot.contains_key(&standing).then_some(standing)
    }
    /// `BlockPredicate` state matching shared with block loot conditions.
    pub fn matches_block_state(
        &self,
        c: &Value,
        block: &str,
        properties: &horizon_world::chunk::BlockProperties,
    ) -> bool {
        let blocks_ok = match &c["blocks"].as_str().or(c["block"].as_str()) {
            Some(spec) => self.block_matches(spec, block),
            None => true,
        };
        blocks_ok
            && c["state"]
                .as_object()
                .into_iter()
                .flatten()
                .chain(c["properties"].as_object().into_iter().flatten())
                .all(|(k, v)| {
                    let have = properties.get(k).map(String::as_str);
                    match v {
                        Value::String(s) => have == Some(s.as_str()),
                        Value::Object(r) => {
                            let have: i64 = have.and_then(|h| h.parse().ok()).unwrap_or(0);
                            r.get("min")
                                .and_then(Value::as_i64)
                                .is_none_or(|m| have >= m)
                                && r.get("max")
                                    .and_then(Value::as_i64)
                                    .is_none_or(|m| have <= m)
                        }
                        _ => true,
                    }
                })
    }
    /// `Block.getDrops`: the block's loot table rolled with `roll`, drawing
    /// from the table's random sequence when it has one (`sequences`: the
    /// world's sequences and seed), else from `random`.
    pub fn block_drops(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        roll: BlockLootRoll<'_>,
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
    ) -> Vec<ItemStack> {
        let drops = self.block_drops_raw(registry, state, roll, random, sequences);
        if !drops.iter().any(|d| d.item.starts_with(DYNAMIC)) {
            return drops;
        }
        drops
            .into_iter()
            .flat_map(|d| {
                if d.item.starts_with(DYNAMIC) {
                    roll.dynamic.to_vec()
                } else {
                    vec![d]
                }
            })
            .collect()
    }

    fn roll_block_table<R: RandomSource>(
        &self,
        table: &Value,
        block: &str,
        properties: &horizon_world::chunk::BlockProperties,
        roll: BlockLootRoll<'_>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.roll_block_table_at(table, block, properties, roll, random, roll.location)
    }
    fn roll_block_table_at<R: RandomSource>(
        &self,
        table: &Value,
        block: &str,
        properties: &horizon_world::chunk::BlockProperties,
        roll: BlockLootRoll<'_>,
        random: &mut R,
        location: Option<&dyn Fn(&Value) -> Option<bool>>,
    ) -> Vec<ItemStack> {
        let ask = |_: &mut R, id: &str, level: u32| roll.levels.map_or(level, |ask| ask(id, level));
        let ctx = LootContext {
            query: horizon_world::loot::query::current_query(),
            items: self,
            block,
            properties,
            held: roll.held,
            killed_by_player: false,
            explosion_radius: roll.explosion,
            this_entity: roll.breaker,
            entity_levels: None,
            block_levels: roll.levels.map(|_| &ask as &LootLevels<'_, R>),
            target: None,
            location,
            keep_empty: roll.keep_empty,
        };
        ctx.evaluate(table, random)
    }
    /// Evaluate an entity's loot table (`minecraft:entities/<name>`).
    /// Evaluate any loot table by id (e.g. `minecraft:gameplay/chicken_lay`).
    pub fn loot_table<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.loot_table_for_command(id, killed_by_player, held, random, false)
    }
    /// `LootCommand.dropChestLoot`: output-list adapter; replacement keeps emitted empties.
    pub fn loot_table_for_command<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        self.table_raw(id, killed_by_player, held, false, random, keep_empty)
            .unwrap_or_default()
    }
    /// `LootTable.getRandomItems(LootParams)` of table `id` with an ORIGIN,
    /// `held` as the TOOL and, when `this_entity`, a THIS_ENTITY: the context
    /// `/loot … fish` gives (`LootContextParamSets.FISHING`). The stacks come
    /// after the stack splitter, each at most its item's stack size, empties
    /// left out. None when there is no such table.
    pub fn roll_loot_table<R: RandomSource>(
        &self,
        id: &str,
        held: Option<&ItemStack>,
        this_entity: bool,
        random: &mut R,
    ) -> Option<Vec<ItemStack>> {
        let raw = self.table_raw(id, false, held, this_entity, random, false)?;
        // LootTable.createStackSplitter.
        let mut out = Vec::with_capacity(raw.len());
        for stack in raw {
            let max = crate::bundle::max_stack(self, &stack);
            let mut count = stack.count;
            while count > max {
                let mut part = stack.clone();
                part.count = max;
                out.push(part);
                count -= max;
            }
            out.push(ItemStack { count, ..stack });
        }
        Some(out)
    }
    /// `LootTable.getRandomItemsRaw` of table `id` with no block.
    fn table_raw<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        this_entity: bool,
        random: &mut R,
        keep_empty: bool,
    ) -> Option<Vec<ItemStack>> {
        let table = self.loot.get(id)?;
        let empty = horizon_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: horizon_world::loot::query::current_query(),
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity,
            entity_levels: None,
            block_levels: None,
            target: None,
            location: None,
            keep_empty,
        };
        // LootTable.getRandomItemsRaw visits the table itself first.
        NESTED_ROLLS.with(|v| v.borrow_mut().push(id.to_owned()));
        let out = ctx.evaluate(table, random);
        ctx.query.finish(Some(id));
        NESTED_ROLLS.with(|v| v.borrow_mut().pop());
        Some(out)
    }
    /// The server random sequence a loot table draws from (`random_sequence`):
    /// vanilla's `LootContext.Builder.create` uses it instead of the level random.
    pub fn loot_sequence(&self, id: &str) -> Option<String> {
        self.loot.get(id)?["random_sequence"]
            .as_str()
            .map(str::to_owned)
    }
    fn interact_roll<R: RandomSource>(
        &self,
        table: &Value,
        block: &str,
        properties: &horizon_world::chunk::BlockProperties,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let ctx = LootContext {
            query: horizon_world::loot::query::current_query(),
            items: self,
            block,
            properties,
            held: None,
            killed_by_player: false,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            location: None,
            keep_empty: false,
        };
        ctx.evaluate(table, random)
    }
    /// `loot_table` drawing from any random (a random sequence).
    pub fn loot_table_from(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut impl RandomSource,
    ) -> Vec<ItemStack> {
        let Some(table) = self.loot.get(id) else {
            return vec![];
        };
        let empty = horizon_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: horizon_world::loot::query::current_query(),
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            location: None,
            keep_empty: false,
        };
        ctx.evaluate_named_root(table, random, id)
    }
    pub fn entity_drops<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.entity_drops_with(entity, killed_by_player, held, random, None, None, false)
    }
    /// Command output retains emitted empty positions when replacing slots.
    pub fn entity_drops_for_command<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            None,
            None,
            keep_empty,
        )
    }
    /// [`Self::entity_drops`] with `level` asked each time
    /// `enchanted_count_increase` or `random_chance_with_enchanted_bonus`
    /// reads the attacker's level.
    pub fn entity_drops_asked<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        level: &mut horizon_world::loot::EnchantedLevel<'_, R>,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let level = std::cell::RefCell::new(level);
        let levels = |random: &mut R, id: &str, read: u32| {
            let read = read.min(i32::MAX as u32) as i32;
            (level.borrow_mut())(random, id, read).max(0) as u32
        };
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            Some(&levels),
            None,
            keep_empty,
        )
    }
    /// Command drops with a live entity predicate context.
    #[allow(clippy::too_many_arguments)]
    pub fn entity_drops_with_target<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        target: &horizon_world::loot::LootEntity,
        matcher: &dyn Fn(&Value, &horizon_world::loot::LootEntity) -> bool,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            None,
            Some((target, matcher)),
            keep_empty,
        )
    }
    /// Command drops with a live target and an answer at each enchanted-level read.
    #[allow(clippy::too_many_arguments)]
    pub fn entity_drops_asked_with_target<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        target: &horizon_world::loot::LootEntity,
        matcher: &dyn Fn(&Value, &horizon_world::loot::LootEntity) -> bool,
        random: &mut R,
        level: &mut horizon_world::loot::EnchantedLevel<'_, R>,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let level = std::cell::RefCell::new(level);
        let levels = |random: &mut R, id: &str, read: u32| {
            let read = read.min(i32::MAX as u32) as i32;
            (level.borrow_mut())(random, id, read).max(0) as u32
        };
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            Some(&levels),
            Some((target, matcher)),
            keep_empty,
        )
    }
    fn entity_drops_with<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        levels: Option<&LootLevels<'_, R>>,
        target: Option<EntityTarget<'_>>,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let name = entity.trim_start_matches("minecraft:");
        let Some(table) = self.loot.get(&format!("minecraft:entities/{name}")) else {
            return vec![];
        };
        let empty = horizon_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: horizon_world::loot::query::current_query(),
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
            entity_levels: levels,
            block_levels: None,
            target,
            location: None,
            keep_empty,
        };
        ctx.evaluate_named_root(table, random, &format!("minecraft:entities/{name}"))
    }
}
/// `DataComponents.BLOCK_STATE`.
const BLOCK_STATE: &str = "minecraft:block_state";
impl<R: RandomSource> LootContext<'_, R> {
    fn evaluate(&self, table: &Value, random: &mut R) -> Vec<ItemStack> {
        let ctx = self;
        let mut out = Vec::new();
        for pool in table["pools"].as_array().into_iter().flatten() {
            if !ctx.conditions(pool, random) {
                continue;
            }
            // LootPool: rolls.getInt (Mth.randomBetweenInclusive for uniform rolls).
            let rolls = Self::int_number(&pool["rolls"], random).max(0) as u32;
            for _ in 0..rolls {
                let mut produced = Vec::new();
                ctx.choose(
                    pool["entries"].as_array().map_or(&[][..], Vec::as_slice),
                    random,
                    &mut produced,
                );
                for (mut stack, emitted) in produced {
                    ctx.modifiers(&pool["modifier"], &mut stack, random);
                    ctx.modifiers(&table["modifier"], &mut stack, random);
                    if stack.count > 0 || (ctx.keep_empty && emitted) {
                        out.push(stack);
                    }
                }
            }
        }
        out
    }
}
/// What a roll asks when its table reads an enchantment level: given the
/// roll's random, the enchantment and the level, the level to use. The random
/// is the roll's own, so whoever answers can keep draws made meanwhile in order.
pub type LootLevels<'a, R> = dyn Fn(&mut R, &str, u32) -> u32 + 'a;
/// The game's say on an enchantment level a block's table reads: the
/// enchantment and the tool's level in, the level to use out. A block's
/// listeners make no draws, so the roll's random isn't theirs.
pub type BlockLootLevels<'a> = dyn Fn(&str, u32) -> u32 + 'a;
/// What a block's table rolls with, beside the block (`LootParams` as
/// `BlockState.getDrops` is given them).
#[derive(Clone, Copy, Default)]
pub struct BlockLootRoll<'a> {
    /// Location predicate reader for command block loot, absent for ordinary breaks.
    pub location: Option<&'a dyn Fn(&Value) -> Option<bool>>,
    /// Retain emitted empty slots for command replacement output.
    pub keep_empty: bool,
    /// `LootContextParams.TOOL`; none is the empty stack.
    pub held: Option<&'a ItemStack>,
    /// What the `minecraft:dynamic` entries become
    /// (`LootParams.withDynamicDrop`: a decorated pot's sherds).
    pub dynamic: &'a [ItemStack],
    /// `LootContextParams.EXPLOSION_RADIUS`: the radius of an explosion that
    /// decays drops (`DESTROY_WITH_DECAY`).
    pub explosion: Option<f32>,
    /// `LootContextParams.THIS_ENTITY` is set (a player broke the block).
    pub breaker: bool,
    /// Asked at each level `ApplyBonusCount` and `BonusLevelTableCondition`
    /// read, in the roll's order; none keeps the tool's.
    pub levels: Option<&'a BlockLootLevels<'a>>,
}
type EntityTarget<'a> = (
    &'a horizon_world::loot::LootEntity,
    &'a dyn Fn(&Value, &horizon_world::loot::LootEntity) -> bool,
);
struct LootContext<'a, R> {
    query: std::rc::Rc<horizon_world::loot::query::LootQuery>,
    items: &'a Items,
    block: &'a str,
    properties: &'a horizon_world::chunk::BlockProperties,
    held: Option<&'a ItemStack>,
    killed_by_player: bool,
    /// `LootContextParams.EXPLOSION_RADIUS`: an explosion that decays drops.
    explosion_radius: Option<f32>,
    /// `LootContextParams.THIS_ENTITY` is set (a player broke the block).
    this_entity: bool,
    /// Asked only at entity enchantment readers; None keeps vanilla's level.
    entity_levels: Option<&'a LootLevels<'a, R>>,
    /// Asked only at block enchantment readers; None keeps vanilla's level.
    block_levels: Option<&'a LootLevels<'a, R>>,
    target: Option<EntityTarget<'a>>,
    /// Bound only for the actual loot-mine location subset.
    location: Option<&'a dyn Fn(&Value) -> Option<bool>>,
    /// Command replacement consumes the emitted list positions, including empty stacks.
    keep_empty: bool,
}
impl<R: RandomSource> LootContext<'_, R> {
    /// Level of an enchantment on the tool / killer's weapon.
    fn enchantment_level(&self, id: &str) -> u32 {
        self.held.map_or(0, |h| h.enchantment_level(id))
    }
    /// The level `EnchantedCountIncreaseFunction` and
    /// `LootItemRandomChanceWithEnchantedBonusCondition` use: the attacker's
    /// (a player's, 0 without one), as `levels` leaves it, up to what a table
    /// reads.
    fn attacker_level(&self, id: &str, random: &mut R) -> u32 {
        let level = if self.killed_by_player && self.block.is_empty() {
            self.enchantment_level(id)
        } else {
            0
        };
        let level = self
            .entity_levels
            .map_or(level, |levels| levels(random, id, level));
        level.min(MAX_LOOT_LEVEL as u32)
    }
    /// The level `ApplyBonusCount.run` and `BonusLevelTableCondition.test`
    /// use: the tool's, as the game leaves it, up to what a table reads.
    fn bonus_level(&self, id: &str, random: &mut R) -> i32 {
        let level = self.enchantment_level(id);
        let level = self
            .block_levels
            .map_or(level, |levels| levels(random, id, level));
        level.min(MAX_LOOT_LEVEL as u32) as i32
    }
    fn number(v: &Value, random: &mut R) -> f32 {
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
            Value::Object(o) => match o
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:uniform")
            {
                "minecraft:constant" => {
                    o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32
                }
                "minecraft:uniform" => {
                    let min = Self::number(o.get("min").unwrap_or(&Value::Null), random);
                    let max = Self::number(o.get("max").unwrap_or(&Value::Null), random);
                    // UniformGenerator.getFloatUnsafe / getIntUnsafe (Mth.nextInt) differ; use int for counts.
                    if max <= min {
                        min
                    } else {
                        min + random.next_float() * (max - min)
                    }
                }
                "minecraft:binomial" => {
                    let n = Self::number(o.get("n").unwrap_or(&Value::Null), random) as u32;
                    let p = Self::number(o.get("p").unwrap_or(&Value::Null), random);
                    (0..n).filter(|_| random.next_float() < p).count() as f32
                }
                // An unknown type gives 0, as in loot.rs `int` and `float`.
                _ => 0.0,
            },
            _ => 0.0,
        }
    }
    fn int_number(v: &Value, random: &mut R) -> i32 {
        if let Value::Object(o) = v
            && o.get("type")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:uniform")
                == "minecraft:uniform"
        {
            let min = Self::int_number(o.get("min").unwrap_or(&Value::Null), random);
            let max = Self::int_number(o.get("max").unwrap_or(&Value::Null), random);
            return if max <= min {
                min
            } else {
                min + random.next_int_bound(max - min + 1).unwrap_or(0)
            };
        }
        Self::number(v, random).round() as i32
    }
    fn condition(&self, c: &Value, random: &mut R) -> bool {
        if let Some(holds) = self.stock_condition(c) {
            return holds;
        }
        let (kind, c) = match c {
            Value::Null => return true,
            Value::Array(list) => return list.iter().all(|c| self.condition(c, random)),
            // A string names a `Registries.PREDICATE` id (`LootItemCondition.CODEC`).
            Value::String(id) => return self.named_condition(id, random),
            Value::Object(o) => (o.get("type").and_then(Value::as_str).unwrap_or(""), c),
            _ => return false,
        };
        match kind {
            // ExplosionCondition: 1 / radius under a decaying explosion.
            "minecraft:survives_explosion" => self
                .explosion_radius
                .is_none_or(|r| random.next_float() <= 1.0 / r),
            "minecraft:inverted" => !self.condition(&c["term"], random),
            "minecraft:any_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| self.condition(t, random)),
            "minecraft:all_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|t| self.condition(t, random)),
            "minecraft:random_chance" => random.next_float() < Self::number(&c["chance"], random),
            "minecraft:table_bonus" => {
                // BonusLevelTableCondition: chances[min(level, len - 1)].
                let level =
                    self.bonus_level(c["enchantment"].as_str().unwrap_or(""), random) as usize;
                let chances = c["chances"].as_array().map_or(&[][..], Vec::as_slice);
                let chance = chances
                    .get(level.min(chances.len().saturating_sub(1)))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32;
                random.next_float() < chance
            }
            "minecraft:match_block" | "minecraft:block_state_property" => self
                .items
                .matches_block_state(c, self.block, self.properties),
            "minecraft:match_tool" => {
                let items = &c["predicate"]["items"];
                let items_ok = match (self.held, items) {
                    (_, Value::Null) => self.held.is_some(),
                    (Some(h), Value::String(s)) => match s.strip_prefix('#') {
                        Some(tag) => self
                            .items
                            .item_tags
                            .get(tag)
                            .is_some_and(|t| t.contains(&*h.item)),
                        None => **s == *h.item,
                    },
                    _ => false,
                };
                // ItemPredicate "minecraft:enchantments": each entry needs a level in range.
                let enchants_ok = c["predicate"]["predicates"]["minecraft:enchantments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|e| {
                        let level =
                            self.enchantment_level(e["enchantments"].as_str().unwrap_or(""));
                        let min = e["levels"]["min"].as_u64().unwrap_or(1) as u32;
                        let max = e["levels"]["max"].as_u64().map_or(u32::MAX, |m| m as u32);
                        level >= min.max(1) && level <= max
                    });
                let custom_ok = c["predicate"]["predicates"]
                    .get("minecraft:custom_data")
                    .is_none_or(|pattern| {
                        horizon_world::loot::custom_data_matches(
                            pattern,
                            self.held.and_then(ItemStack::custom_data),
                        )
                    });
                items_ok && enchants_ok && custom_ok
            }
            "minecraft:killed_by_player" => self.killed_by_player,
            "minecraft:random_chance_with_enchanted_bonus" => {
                let level = self.attacker_level(c["enchantment"].as_str().unwrap_or(""), random);
                let chance = if level > 0 {
                    crate::enchant::level_value(&c["enchanted_chance"], level)
                } else {
                    c["unenchanted_chance"].as_f64().unwrap_or(0.0) as f32
                };
                random.next_float() < chance
            }
            "minecraft:entity_properties"
                if c["entity"].as_str() == Some("this") && self.target.is_some() =>
            {
                let (target, matcher) = self.target.unwrap();
                matcher(&c["predicate"], target)
            }
            // `this` with no checks (snow, chorus flower) holds when an entity
            // broke the block (LootItemEntityPropertyCondition needs the entity).
            "minecraft:entity_properties"
                if self.this_entity
                    && c["entity"].as_str() == Some("this")
                    && c["predicate"].as_object().is_none_or(|p| p.is_empty()) =>
            {
                true
            }
            // Entity flags, equipment and damage-source predicates: none hold
            // for an unenchanted player kill without fire or lightning.
            "minecraft:entity_properties" | "minecraft:damage_source_properties" => false,
            // LocationCheck reads the world at its offset, which this engine
            // does not have: held true (a double plant's other half is taken
            // as there). Unknown types fail, as in loot.rs.
            "minecraft:location_check" => self
                .location
                .and_then(|location| location(c))
                .unwrap_or(true),
            // A mod's own kind: the condition hook answers.
            k if !c.is_null() && k.contains(':') && !k.starts_with("minecraft:") => {
                let ctx = self.native_loot_context(random);
                let mut holds = false;
                horizon_world::loot::native_invoke_loot_condition(
                    k, c, &ctx, &mut holds, self.items,
                );
                holds
            }
            _ => false,
        }
    }
    /// Expand entries; returns whether an entry succeeded (for alternatives).
    /// The last candidate field distinguishes emitted items from empty/deferred
    /// placeholders, without changing existing selection/modifier draws.
    fn expand(
        &self,
        e: &Value,
        random: &mut R,
        out: &mut Vec<(i32, ItemStack, Value, bool)>,
    ) -> bool {
        if !self.conditions(e, random) {
            return false;
        }
        match e["type"].as_str().unwrap_or("") {
            "minecraft:item" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from(e["name"].as_str().unwrap_or("minecraft:air")),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    e["modifier"].clone(),
                    true,
                ));
                true
            }
            "minecraft:alternatives" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if self.expand(child, random, out) {
                        return true;
                    }
                }
                false
            }
            "minecraft:group" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    self.expand(child, random, out);
                }
                true
            }
            "minecraft:sequence" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if !self.expand(child, random, out) {
                        break;
                    }
                }
                true
            }
            // DynamicLoot: what the block entity adds (a cracked decorated
            // pot's sherds), filled in by `block_drops`.
            "minecraft:dynamic" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from(format!("{DYNAMIC}{}", e["name"].as_str().unwrap_or(""))),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    Value::Null,
                    false,
                ));
                true
            }
            // NestedLootTable: chosen by its weight like an item; `choose`
            // then rolls its table (an id or inline) and applies its functions.
            "minecraft:loot_table" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from(NESTED),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    e.clone(),
                    true,
                ));
                true
            }
            "minecraft:empty" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from("minecraft:air"),
                        count: 0,
                        damage: 0,
                        extra: None,
                    },
                    Value::Null,
                    false,
                ));
                true
            }
            _ => false,
        }
    }
    fn choose(&self, entries: &[Value], random: &mut R, produced: &mut Vec<(ItemStack, bool)>) {
        let mut candidates = Vec::new();
        for e in entries {
            self.expand(e, random, &mut candidates);
        }
        if candidates.is_empty() {
            return;
        }
        let pick = if candidates.len() == 1 {
            0
        } else {
            let total: i32 = candidates.iter().map(|c| c.0.max(0)).sum();
            let mut r = random.next_int_bound(total.max(1)).unwrap_or(0);
            let mut chosen = 0;
            for (i, c) in candidates.iter().enumerate() {
                r -= c.0.max(0);
                if r < 0 {
                    chosen = i;
                    break;
                }
            }
            chosen
        };
        let (_, mut stack, modifiers, emitted) = candidates.swap_remove(pick);
        if &*stack.item == NESTED {
            self.nested(&modifiers, random, produced);
            return;
        }
        self.modifiers(&modifiers, &mut stack, random);
        produced.push((stack, emitted));
    }
    /// `NestedLootTable.createItemStack`: the entry's table rolls in full
    /// (its pools and its own functions), then the entry's functions apply to
    /// each stack. A table already rolling further out adds nothing.
    fn nested(&self, entry: &Value, random: &mut R, produced: &mut Vec<(ItemStack, bool)>) {
        let (name, table) = match &entry["value"] {
            Value::String(name) => (name.as_str(), self.items.loot.get(name.as_str())),
            inline @ Value::Object(_) => ("<inline>", Some(inline)),
            _ => return,
        };
        let Some(table) = table else { return };
        let entered = NESTED_ROLLS.with(|v| {
            let mut v = v.borrow_mut();
            let fresh = !v.iter().any(|n| n == name);
            if fresh {
                v.push(name.to_owned());
            }
            fresh
        });
        if !entered {
            return;
        }
        let stacks = self.evaluate(table, random);
        NESTED_ROLLS.with(|v| v.borrow_mut().pop());
        for mut stack in stacks {
            self.modifiers(&entry["modifier"], &mut stack, random);
            produced.push((stack, true));
        }
    }
    /// An enchantment function's `options` (`HolderSet`): a tag, a list or
    /// one id; every enchantment when absent (`registryAccess().listElements`).
    fn enchant_pool(e: &crate::enchant::Enchantments, options: &Value) -> Vec<Arc<str>> {
        match options {
            Value::String(s) if s.starts_with('#') => e.tag(s).to_vec(),
            Value::String(s) => vec![Arc::from(s.as_str())],
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(Arc::from)).collect(),
            _ => e.list.iter().map(|x| Arc::clone(&x.id)).collect(),
        }
    }
    fn modifiers(&self, m: &Value, stack: &mut ItemStack, random: &mut R) {
        let list: Vec<&Value> = match m {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![m],
            _ => return,
        };
        for f in list {
            if !self.conditions(f, random) {
                continue;
            }
            match f["type"].as_str().or(f["function"].as_str()).unwrap_or("") {
                "minecraft:set_count" => {
                    let n = Self::int_number(&f["count"], random);
                    let base = if f["add"].as_bool() == Some(true) {
                        stack.count as i32
                    } else {
                        0
                    };
                    let total = base.saturating_add(n).max(0);
                    stack.count = total as u32;
                }
                // SetItemDamageFunction.run: check damageability before the
                // float provider, so ineligible stacks do not draw randomness.
                "minecraft:set_damage" => {
                    let component = |id| stack.data().and_then(|data| data.other(id));
                    if stack.count == 0
                        || component("minecraft:unbreakable").is_some()
                        || component("!minecraft:max_damage").is_some()
                        || component("!minecraft:damage").is_some()
                    {
                        continue;
                    }
                    let max = match component("minecraft:max_damage") {
                        Some(horizon_core::nbt::Tag::Int(value)) => {
                            u32::try_from(*value).unwrap_or(0)
                        }
                        _ => self
                            .items
                            .get(&stack.item)
                            .map_or(0, |info| info.max_damage),
                    };
                    if max == 0 {
                        continue;
                    }
                    let base = if f["add"].as_bool() == Some(true) {
                        1.0 - stack.damage as f32 / max as f32
                    } else {
                        0.0
                    };
                    let remaining = (Self::number(&f["damage"], random) + base).clamp(0.0, 1.0);
                    stack.damage = ((1.0 - remaining) * max as f32).floor() as u32;
                }
                "minecraft:apply_bonus" => {
                    let level = self.bonus_level(f["enchantment"].as_str().unwrap_or(""), random);
                    let count = stack.count as i32;
                    let params = &f["parameters"];
                    stack.count = match f["formula"].as_str().unwrap_or("") {
                        "minecraft:ore_drops" if level > 0 => {
                            let bound = level.saturating_add(2);
                            let bonus = (random.next_int_bound(bound).unwrap_or(0) - 1).max(0);
                            count.saturating_mul(bonus + 1)
                        }
                        "minecraft:uniform_bonus_count" => {
                            let multiplier = params["bonusMultiplier"].as_i64().unwrap_or(1) as i32;
                            let bound = multiplier.saturating_mul(level).saturating_add(1).max(1);
                            count.saturating_add(random.next_int_bound(bound).unwrap_or(0))
                        }
                        "minecraft:binomial_with_bonus_count" => {
                            let (extra, p) = (
                                params["extra"].as_i64().unwrap_or(0) as i32,
                                params["probability"].as_f64().unwrap_or(0.0) as f32,
                            );
                            let mut n = count;
                            for _ in 0..level.saturating_add(extra) {
                                if random.next_float() < p {
                                    n = n.saturating_add(1);
                                }
                            }
                            n
                        }
                        _ => count,
                    }
                    .max(0) as u32;
                }
                "minecraft:enchanted_count_increase" => {
                    // Attacker's level x count, rounded (looting).
                    let level =
                        self.attacker_level(f["enchantment"].as_str().unwrap_or(""), random);
                    if level > 0 {
                        let add = level as f32 * Self::number(&f["count"], random);
                        // ItemStack.grow: an int sum, which wraps as in Java.
                        let count = (stack.count as i32).wrapping_add(add.round() as i32);
                        stack.count = count.max(0) as u32;
                        if let Some(limit) = f["limit"].as_i64().filter(|l| *l > 0) {
                            stack.count = stack.count.min(limit as u32);
                        }
                    }
                }
                // CopyBlockState: the listed properties of the broken block
                // become the item's `block_state` component.
                "minecraft:copy_state" => {
                    use horizon_core::nbt::Tag;
                    let mut copied = match stack.data().and_then(|d| d.other(BLOCK_STATE)) {
                        Some(Tag::Compound(c)) => c.clone(),
                        _ => Default::default(),
                    };
                    for name in f["properties"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        if let Some(value) = self.properties.get(name) {
                            copied.insert(name.into(), Tag::String(value.as_str().into()));
                        }
                    }
                    if !copied.is_empty() {
                        stack
                            .data_mut()
                            .set_other(BLOCK_STATE, Some(Tag::Compound(copied)));
                    }
                }
                // ApplyExplosionDecay: each item survives with 1 / radius.
                "minecraft:explosion_decay" => {
                    if let Some(radius) = self.explosion_radius {
                        let probability = 1.0 / radius;
                        stack.count = (0..stack.count)
                            .filter(|_| random.next_float() <= probability)
                            .count() as u32;
                    }
                }
                // SetPotionFunction (the barter's fire resistance potions).
                "minecraft:set_potion" => {
                    if let Some(id) = f["id"].as_str() {
                        *stack = std::mem::replace(stack, ItemStack::new("minecraft:air", 0))
                            .with_potion(id);
                    }
                }
                // EnchantWithLevelsFunction: EnchantmentHelper.enchantItem on
                // the context's random (the End cities' gear).
                "minecraft:enchant_with_levels" => {
                    let cost = Self::int_number(&f["levels"], random);
                    let e = &self.items.enchantments;
                    let pool = Self::enchant_pool(e, &f["options"]);
                    let chosen = e.select(self.items, random, stack, cost, &pool);
                    // EnchantmentHelper.enchantItem: a book becomes one
                    // enchanted book (the fresh stack's count).
                    if &*stack.item == "minecraft:book" {
                        *stack = ItemStack::new("minecraft:enchanted_book", 1);
                    }
                    for (id, level) in chosen {
                        stack.enchant(&id, level);
                    }
                }
                // EnchantRandomlyFunction: one enchantment the item accepts,
                // at a level between its min and max.
                "minecraft:enchant_randomly" => {
                    let e = &self.items.enchantments;
                    let book = &*stack.item == "minecraft:book";
                    let only_compatible = f["only_compatible"].as_bool().unwrap_or(true);
                    let options: Vec<Arc<str>> = Self::enchant_pool(e, &f["options"])
                        .into_iter()
                        .filter(|id| {
                            book || !only_compatible || e.can_enchant(self.items, id, &stack.item)
                        })
                        .collect();
                    if options.is_empty() {
                        continue;
                    }
                    let id = options
                        [random.next_int_bound(options.len() as i32).unwrap_or(0) as usize]
                        .clone();
                    let max = e.get(&id).map_or(1, |d| d.max_level).max(1);
                    // `Mth.nextInt(random, 1, max)`: a single-level
                    // enchantment (Mending) returns 1 without drawing, so the
                    // rolls after this one keep vanilla's sequence.
                    let level = if max <= 1 {
                        1
                    } else {
                        1 + random.next_int_bound(max as i32).unwrap_or(0) as u32
                    };
                    if book {
                        *stack = ItemStack::new("minecraft:enchanted_book", 1);
                    }
                    stack.enchant(&id, level);
                }
                "minecraft:limit_count" => {
                    let limit = &f["limit"];
                    if let Some(min) = limit["min"].as_i64() {
                        stack.count = stack.count.max(min as u32);
                    }
                    if let Some(max) = limit["max"].as_i64() {
                        stack.count = stack.count.min(max as u32);
                    }
                }
                // SetNameFunction.run: the component the target names
                // (custom_name by default), set as a chest's roll sets it. As in
                // the main loot engine, an `entity` resolver is not modelled.
                "minecraft:set_name" if !f["name"].is_null() => {
                    let target = f["target"].as_str().unwrap_or("custom_name");
                    let id = format!("minecraft:{target}");
                    stack.data_mut().set_loot_name(&id, &f["name"]);
                    stack.tidy();
                }
                "minecraft:set_components" => apply_components(stack, &f["components"]),
                // A mod's own kind: the function hook changes the stack, as it
                // does in the main loot engine.
                k if k.contains(':') && !k.starts_with("minecraft:") => {
                    let ctx = self.native_loot_context(random);
                    mod_function(k, f, &ctx, stack, |kind, data, ctx, stack| {
                        horizon_world::loot::native_invoke_loot_function(
                            kind, data, ctx, stack, self.items,
                        );
                    });
                }
                _ => {}
            }
        }
    }
}

/// `SetComponentsFunction.run`: `ItemStack.applyComponents` with the patch
/// (`!id` removes a component), read back as a saved stack's components are,
/// so a modelled one (a potion's contents, a book's pages) lands in its field.
fn apply_components(stack: &mut ItemStack, patch: &Value) {
    use horizon_core::nbt::Tag;
    let mut saved = match crate::item_data::encode_components(stack) {
        Some(Tag::Compound(c)) => c,
        _ => Default::default(),
    };
    for (key, value) in patch.as_object().into_iter().flatten() {
        let (removed, id) = key
            .strip_prefix('!')
            .map_or((false, &**key), |id| (true, id));
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        saved.remove(&id.as_str().into());
        saved.remove(&format!("!{id}").as_str().into());
        if removed {
            saved.insert(
                format!("!{id}").as_str().into(),
                Tag::compound(Vec::<(&str, Tag)>::new()),
            );
        } else {
            saved.insert(
                id.as_str().into(),
                horizon_client_core::scoreboard::json_to_tag(value),
            );
        }
    }
    (stack.damage, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(saved)));
    stack.tidy();
}

/// The items engine's stack as the loot engine's (`LootStack`), for a mod's
/// function hook: the parts of it the hook can change and the engine takes
/// back. Saved components retain their exact NBT types.
fn loot_of(stack: &ItemStack) -> horizon_world::loot::LootStack {
    let mut components: Vec<(String, Option<horizon_core::nbt::Tag>)> = Vec::new();
    if let Some(data) = stack.data() {
        for (id, value) in &data.other {
            components.push((id.clone(), Some(value.clone())));
        }
        if let Some(name) = &data.custom_name {
            components.push(("minecraft:custom_name".to_owned(), Some(name.clone())));
        }
        if let Some(trim) = &data.trim {
            components.push((
                "minecraft:trim".to_owned(),
                Some(crate::item_data::trim::to_nbt(trim)),
            ));
        }
    }
    horizon_world::loot::LootStack {
        item: stack.item.to_string(),
        count: stack.count as i32,
        damage: stack.damage as i32,
        enchantments: stack
            .crafting_enchantments()
            .iter()
            .map(|(id, level)| (id.to_string(), *level as i32))
            .collect(),
        potion: stack.potion().map(str::to_owned),
        map: false,
        exploration: None,
        ominous_amplifier: stack
            .data()
            .and_then(|d| d.ominous_amplifier)
            .map(i32::from),
        stew_effects: stack
            .data()
            .map(|d| {
                d.stew_effects
                    .iter()
                    .map(|(effect, ticks)| (effect.to_string(), *ticks))
                    .collect()
            })
            .unwrap_or_default(),
        names: Vec::new(),
        components,
    }
}

/// What a mod's loot function left in its stack, applied to the engine's.
/// Only what changed is written, so a field the hook did not touch keeps the
/// engine's own form of it.
fn apply_loot(
    before: &horizon_world::loot::LootStack,
    after: &horizon_world::loot::LootStack,
    stack: &mut ItemStack,
) {
    if after.item != before.item {
        stack.item = Arc::from(after.item.as_str());
    }
    if after.count != before.count {
        stack.count = after.count.max(0) as u32;
    }
    if after.damage != before.damage {
        stack.damage = after.damage.max(0) as u32;
    }
    if after.enchantments != before.enchantments {
        let levels = after
            .enchantments
            .iter()
            .filter(|(_, level)| *level > 0)
            .map(|(id, level)| (Arc::from(id.as_str()), *level as u32))
            .collect();
        stack.set_crafting_enchantments(levels);
    }
    if after.potion != before.potion {
        stack.data_mut().potion = after.potion.as_deref().map(Arc::from);
    }
    if after.ominous_amplifier != before.ominous_amplifier {
        stack.data_mut().ominous_amplifier = after.ominous_amplifier.map(|a| a.clamp(0, 255) as u8);
    }
    if after.stew_effects != before.stew_effects {
        stack.data_mut().stew_effects = after
            .stew_effects
            .iter()
            .map(|(effect, ticks)| (Arc::from(effect.as_str()), *ticks))
            .collect();
    }
    // `SetNameFunction`'s components: the names the hook set.
    for (id, text) in &after.names {
        if !before.names.iter().any(|(i, t)| i == id && t == text) {
            stack.data_mut().set_loot_name(id, text);
        }
    }
    // `SetComponentsFunction`'s patch: the components the hook changed or
    // added are set, and the ones it dropped (gone from the stack) removed.
    for (id, value) in &after.components {
        // Replay changed per-ID sequences in order, not unchanged captured values:
        // an unrelated patch must not undo a separately edited loot name.
        let unchanged = before
            .components
            .iter()
            .filter(|(key, _)| key == id)
            .map(|(_, value)| value)
            .eq(after
                .components
                .iter()
                .filter(|(key, _)| key == id)
                .map(|(_, value)| value));
        if !unchanged {
            stack.data_mut().set_loot_component(id, value);
        }
    }
    for (id, _) in &before.components {
        if !after.components.iter().any(|(i, _)| i == id) {
            stack.data_mut().set_loot_component(id, &None);
        }
    }
    stack.tidy();
}

/// `native_invoke_loot_function` with the items engine's stack: the stack as the
/// loot engine's, the hook, and back what it changed. `hook` is the hook
/// point (`horizon_world::loot::native_invoke_loot_function`); a test passes its own.
fn mod_function(
    kind: &str,
    data: &Value,
    ctx: &horizon_world::loot::LootContext<'_>,
    stack: &mut ItemStack,
    hook: impl FnOnce(
        &str,
        &Value,
        &horizon_world::loot::LootContext<'_>,
        &mut horizon_world::loot::LootStack,
    ),
) {
    let before = loot_of(stack);
    let mut after = before.clone();
    hook(kind, data, ctx, &mut after);
    apply_loot(&before, &after, stack);
}

/// `DispensibleContainerItem`: shared contents for hand use and dispensers.
pub fn bucket_contents(
    item: &str,
) -> Option<(horizon_world::level::BucketContent, &str, Option<&str>)> {
    use horizon_world::{fluid::FluidKind, level::BucketContent as C};
    // A bucket of a fluid empties with that fluid's sound.
    let fluid = |kind| {
        Some((
            C::Fluid(kind),
            horizon_world::level::fluid_empty_sound(kind)?,
            None,
        ))
    };
    Some(match item {
        _ if !item.starts_with("minecraft:") => fluid(FluidKind::Mod(
            horizon_world::fluid::mod_fluid_of_bucket(item)?,
        ))?,
        "minecraft:water_bucket" => fluid(FluidKind::Water)?,
        "minecraft:lava_bucket" => fluid(FluidKind::Lava)?,
        "minecraft:powder_snow_bucket" => {
            (C::Powder, "minecraft:item.bucket.empty_powder_snow", None)
        }
        "minecraft:cod_bucket" => (C::MobWater, "minecraft:item.bucket.empty_fish", Some("cod")),
        "minecraft:salmon_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("salmon"),
        ),
        "minecraft:pufferfish_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("pufferfish"),
        ),
        "minecraft:tropical_fish_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("tropical_fish"),
        ),
        "minecraft:axolotl_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_axolotl",
            Some("axolotl"),
        ),
        "minecraft:tadpole_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_tadpole",
            Some("tadpole"),
        ),
        "minecraft:sulfur_cube_bucket" => (
            C::Nothing,
            "minecraft:item.bucket.empty_sulfur_cube",
            Some("sulfur_cube"),
        ),
        _ => return None,
    })
}

/// One `MobEffectInstance` of an `apply_effects` consume effect (its codec's defaults).
/// `DeathProtection.CODEC`: the `death_effects` of a `minecraft:death_protection`
/// component, in order (`applyEffects` runs each one). A missing or empty
/// list, as in `death_protection={}`, has none.
pub fn parse_death_effects(
    component: &Value,
) -> std::result::Result<Vec<ConsumeEffect>, horizon_core::json_nbt::JsonNbtError> {
    parse_consumable(&serde_json::json!({"on_consume_effects": component.get("death_effects").cloned().unwrap_or_else(|| serde_json::json!([]))})).map(|value| value.effects)
}

#[cfg(test)]
mod death_effects_tests {
    use super::*;
    #[test]
    fn the_totems_component_reads_clear_then_three_effects() {
        let totem = serde_json::json!({"death_effects": [
            {"type": "minecraft:clear_all_effects"},
            {"type": "minecraft:apply_effects", "effects": [
                {"amplifier": 1, "duration": 900, "id": "minecraft:regeneration", "show_icon": true},
                {"amplifier": 1, "duration": 100, "id": "minecraft:absorption", "show_icon": true},
                {"duration": 800, "id": "minecraft:fire_resistance", "show_icon": true}
            ]}
        ]});
        let effects = parse_death_effects(&totem).unwrap();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0], ConsumeEffect::ClearAllEffects);
        let ConsumeEffect::ApplyEffects {
            effects,
            probability,
        } = &effects[1]
        else {
            panic!("second effect is apply_effects");
        };
        assert_eq!(*probability, 1.0);
        let ids: Vec<_> = effects
            .iter()
            .map(|e| (e.id.as_str(), e.duration, e.amplifier))
            .collect();
        assert_eq!(
            ids,
            [
                ("minecraft:regeneration", 900, 1),
                ("minecraft:absorption", 100, 1),
                ("minecraft:fire_resistance", 800, 0)
            ]
        );
    }
    #[test]
    fn an_empty_component_has_no_effects() {
        assert!(
            parse_death_effects(&serde_json::json!({}))
                .unwrap()
                .is_empty()
        );
    }
}

fn consume_effect_spec(e: &Value) -> Option<ConsumeEffectSpec> {
    let flag = |key: &str, default: bool| e.get(key).and_then(|v| v.as_bool()).unwrap_or(default);
    Some(ConsumeEffectSpec {
        id: e.get("id")?.as_str()?.to_string(),
        duration: e.get("duration").and_then(|d| d.as_i64()).unwrap_or(0) as i32,
        amplifier: e.get("amplifier").and_then(|a| a.as_i64()).unwrap_or(0) as i32,
        ambient: flag("ambient", false),
        show_particles: flag("show_particles", true),
        show_icon: flag("show_icon", flag("show_particles", true)),
    })
}

#[cfg(test)]
mod bucket_use_tests {
    use super::*;
    #[test]
    fn every_filled_bucket_has_the_native_content_and_sound() {
        use horizon_world::level::BucketContent as C;
        assert_eq!(
            bucket_contents("minecraft:powder_snow_bucket"),
            Some((C::Powder, "minecraft:item.bucket.empty_powder_snow", None))
        );
        assert_eq!(
            bucket_contents("minecraft:tadpole_bucket"),
            Some((
                C::MobWater,
                "minecraft:item.bucket.empty_tadpole",
                Some("tadpole")
            ))
        );
        assert_eq!(
            bucket_contents("minecraft:sulfur_cube_bucket"),
            Some((
                C::Nothing,
                "minecraft:item.bucket.empty_sulfur_cube",
                Some("sulfur_cube")
            ))
        );
        for id in ["cod", "salmon", "pufferfish", "tropical_fish", "axolotl"] {
            assert!(bucket_contents(&format!("minecraft:{id}_bucket")).is_some());
        }
        assert!(bucket_contents("minecraft:bucket").is_none());
        assert!(bucket_contents("minecraft:milk_bucket").is_none());
    }
}

/// The items of the prepared resources, when they are on this machine, for
/// tests: the block registry and facts as the client's `Assets::load` reads them.
#[cfg(test)]
pub(crate) fn prepared_items() -> Option<Items> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client/26.3");
    if !root.join("items.json").exists() {
        return None;
    }
    let (registry, _) =
        BlockRegistry::from_facts_json_with_flags(fs::File::open(root.join("facts.json")).ok()?)
            .ok()?;
    let facts = BlockFacts::load(&root, &registry).ok()?;
    let defaults = horizon_world::item_defaults::read(&root).ok()?;
    Items::load(&root, &registry, &facts, &PackOverlay::NONE, &defaults).ok()
}

#[cfg(test)]
mod consume_effect_tests {
    use super::*;

    #[test]
    fn effects_keep_their_ambient_and_particle_flags() {
        let quiet =
            serde_json::json!({"id": "minecraft:haste", "duration": 3600, "show_particles": false});
        let s = consume_effect_spec(&quiet).unwrap();
        assert_eq!(
            (s.id.as_str(), s.duration, s.amplifier),
            ("minecraft:haste", 3600, 0)
        );
        assert!(!s.ambient && !s.show_particles);
        let plain =
            consume_effect_spec(&serde_json::json!({"id": "minecraft:regeneration"})).unwrap();
        assert!(!plain.ambient && plain.show_particles);
    }

    #[test]
    fn invalid_native_consume_payload_refuses_component() {
        let bad =
            serde_json::json!({"on_consume_effects": [{"type": "mymod:charge", "data": null}]});
        assert!(parse_consumable(&bad).is_err());
        let good =
            serde_json::json!({"on_consume_effects": [{"type": "mymod:charge", "amount": 42}]});
        let value = parse_consumable(&good).unwrap();
        let copy = value.clone();
        let (ConsumeEffect::Native { data: a, .. }, ConsumeEffect::Native { data: b, .. }) =
            (&value.effects[0], &copy.effects[0])
        else {
            panic!("native payload")
        };
        assert!(Arc::ptr_eq(a, b));
    }

    #[test]
    fn a_mods_own_consume_effect_kind_is_kept() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        // A mod's own kind: Native with its JSON.
        let native =
            parse_consume_effect(&v(r#"{"type":"mymod:energy_boost","amount":42}"#)).unwrap();
        assert!(matches!(&native, ConsumeEffect::Native { kind, data }
            if kind == "mymod:energy_boost" && data.get("amount") == Some(&horizon_core::nbt::Tag::Byte(42))));
        // A bare vanilla name and its namespaced form are the same kind.
        let bare = parse_consume_effect(&v(
            r#"{"type":"apply_effects","effects":[{"id":"minecraft:speed","duration":100}]}"#,
        ))
        .unwrap();
        assert!(matches!(bare, ConsumeEffect::ApplyEffects { .. }));
        let named = parse_consume_effect(
            &v(r#"{"type":"minecraft:apply_effects","effects":[{"id":"minecraft:speed","duration":100}]}"#),
        ).unwrap();
        assert!(matches!(named, ConsumeEffect::ApplyEffects { .. }));
        // An empty namespace is minecraft's too (`Identifier.bySeparator`).
        let empty = parse_consume_effect(&v(r#"{"type":":clear_all_effects"}"#)).unwrap();
        assert!(matches!(empty, ConsumeEffect::ClearAllEffects));
        // An unknown minecraft: kind and a missing type are dropped.
        assert!(parse_consume_effect(&v(r#"{"type":"minecraft:future"}"#)).is_none());
        assert!(parse_consume_effect(&v(r#"{"amount":42}"#)).is_none());
    }
}

#[cfg(test)]
mod tests {
    mod enchant_select_tests {
        include!("enchant_select_tests.rs");
    }
    use super::*;
    use horizon_core::random::LegacyRandom;
    use serde_json::json;

    /// The ids `/loot ... loot <id>` takes: a content mod's table in the
    /// root and a world pack's table over it.
    #[test]
    fn weapon_damage_comes_from_the_stack() {
        let items = Items::for_tests(HashMap::new());
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.data_mut().set_other(
            "minecraft:weapon",
            Some(horizon_core::nbt::Tag::compound([(
                "item_damage_per_attack",
                horizon_core::nbt::Tag::Int(3),
            )])),
        );
        assert_eq!(items.weapon_damage(&sword), Some(3));
        sword.data_mut().set_other(
            "!minecraft:weapon",
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new())),
        );
        assert_eq!(items.weapon_damage(&sword), None);
    }

    #[test]
    fn weapon_disable_blocking_comes_from_the_stack() {
        let root = std::env::temp_dir().join(format!(
            "horizon-weapon-disable-blocking-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{},"minecraft:diamond_axe":{"minecraft:weapon":{"disable_blocking_for_seconds":5.0}}}"#,
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let mut axe = ItemStack::new("minecraft:diamond_axe", 1);
        assert_eq!(items.weapon_disable_blocking(&axe), 5.0);
        for item in ["minecraft:stick", "test:unknown", "minecraft:air"] {
            assert_eq!(items.weapon_disable_blocking(&ItemStack::new(item, 1)), 0.0);
        }

        // A replacement component uses the codec's zero default, not the axe's.
        let empty = crate::item_data::command_components("[weapon={}]").unwrap();
        (axe.damage, axe.extra) = crate::item_data::decode_components(Some(&empty));
        assert_eq!(items.weapon_disable_blocking(&axe), 0.0);

        let custom =
            crate::item_data::command_components("[weapon={disable_blocking_for_seconds:1.5f}]")
                .unwrap();
        (axe.damage, axe.extra) = crate::item_data::decode_components(Some(&custom));
        assert_eq!(items.weapon_disable_blocking(&axe), 1.5);
        let mut stick = ItemStack::new("minecraft:stick", 1);
        (stick.damage, stick.extra) = crate::item_data::decode_components(Some(&custom));
        assert_eq!(items.weapon_disable_blocking(&stick), 1.5);
        stick.count = 0;
        assert_eq!(items.weapon_disable_blocking(&stick), 0.0);
        stick.count = 1;
        stick.item = "minecraft:air".into();
        assert_eq!(items.weapon_disable_blocking(&stick), 0.0);

        axe.data_mut().set_other(
            "!minecraft:weapon",
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new())),
        );
        assert_eq!(items.weapon_disable_blocking(&axe), 0.0);
        let removal = crate::item_data::command_components("[!weapon]").unwrap();
        (axe.damage, axe.extra) = crate::item_data::decode_components(Some(&removal));
        let restored = crate::item_nbt::item_from_nbt(&crate::item_nbt::item_to_nbt(&axe)).unwrap();
        assert_eq!(items.weapon_disable_blocking(&restored), 0.0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loot_table_ids_list_the_roots_and_the_packs_tables() {
        let root = std::env::temp_dir().join(format!("horizon-loot-ids-{}", std::process::id()));
        fs::create_dir_all(root.join("data/mymod/loot_table/rooms")).unwrap();
        fs::write(root.join("items.json"), "{}").unwrap();
        fs::write(
            root.join("data/mymod/loot_table/rooms/a.json"),
            r#"{"pools":[]}"#,
        )
        .unwrap();
        let pack = [(
            "loot_table/mypack:b".to_owned(),
            vec![br#"{"pools":[]}"#.to_vec()],
        )];
        let items = Items::catalog(
            &root,
            &PackOverlay::new(&pack),
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let mut ids: Vec<&str> = items.loot_table_ids().collect();
        ids.sort();
        assert_eq!(ids, ["mymod:rooms/a", "mypack:b"]);
        fs::remove_dir_all(root).unwrap();
    }

    /// Songs load under their own namespace beside vanilla's through the resource overlay.
    #[test]
    fn content_mod_songs_load_under_their_namespace() {
        let root = std::env::temp_dir().join(format!("horizon-songs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for ns in ["minecraft", "mymod"] {
            fs::create_dir_all(root.join(format!("data/{ns}/jukebox_song"))).unwrap();
        }
        fs::write(root.join("items.json"), "{}").unwrap();
        let song = json!({"sound_event": "mymod:music_disc.tune", "length_in_seconds": 5.0, "comparator_output": 3, "description": "tune"});
        fs::write(
            root.join("data/mymod/jukebox_song/tune.json"),
            song.to_string(),
        )
        .unwrap();
        fs::write(
            root.join("data/minecraft/jukebox_song/cat.json"),
            json!({"sound_event": "minecraft:music_disc.cat", "length_in_seconds": 185.0, "comparator_output": 2, "description": "cat"}).to_string(),
        )
        .unwrap();

        let songs = load_songs(&root, &PackOverlay::NONE);
        assert!(songs.contains_key("minecraft:cat"));
        assert!(songs.contains_key("mymod:tune"));
        let tune = &songs["mymod:tune"];
        assert_eq!(tune.sound, "mymod:music_disc.tune");
        assert_eq!((tune.length_ticks, tune.comparator), (100, 3));
        assert!(songs.contains_key("minecraft:cat"));
        assert!(!songs.contains_key("minecraft:tune"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn server_json_conditions_filter_selected_songs_instruments_and_enchantments() {
        let root = std::env::temp_dir().join(format!("horizon-server-json-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut files = Vec::new();
        for kind in ["jukebox_song", "instrument", "enchantment"] {
            let dir = root.join("data/demo").join(kind);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("entry.json"), "{}").unwrap();
            files.push((format!("{kind}/demo:entry"), vec![
                b"{}".to_vec(),
                json!({"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"provider"}],"description":"winning","max_level":7}).to_string().into_bytes(),
            ]));
            files.push((
                format!("{kind}/demo:unknown"),
                vec![
                    json!({"neoforge:conditions":[{"type":"unsupported"}]})
                        .to_string()
                        .into_bytes(),
                ],
            ));
        }
        let off = PackOverlay {
            files: &files,
            ..PackOverlay::NONE
        };
        let enabled = horizon_world::conditions::LoadConditions::new(["provider"]);
        let on = PackOverlay {
            files: &files,
            conditions: &enabled,
        };
        for kind in ["jukebox_song", "instrument", "enchantment"] {
            assert!(data_files(&root, kind, &off).is_empty());
            let selected = data_files(&root, kind, &on);
            assert_eq!(selected.len(), 1);
            assert_eq!(selected["demo:entry"]["description"], "winning");
        }
        assert!(load_songs(&root, &off).is_empty());
        assert_eq!(
            load_songs(&root, &on)["demo:entry"].description,
            json!("winning")
        );
        assert!(load_instruments(&root, &off).is_empty());
        assert!(load_instruments(&root, &on).contains_key("demo:entry"));
        fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        assert!(
            crate::enchant::Enchantments::load_with(&root, &off, &defaults)
                .unwrap()
                .list
                .is_empty()
        );
        let enchantments = crate::enchant::Enchantments::load_with(&root, &on, &defaults).unwrap();
        assert_eq!(enchantments.list.len(), 1);
        assert_eq!(enchantments.list[0].max_level, 7);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tag_conditions_use_enabled_server_ids_before_each_replace_and_reload() {
        use horizon_mod_package::enabled::EnabledMods;
        use mod_vocab::Side;
        let root = std::env::temp_dir().join(format!(
            "horizon-conditional-tags-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join("data/visuals/tags/block");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            root.join(crate::content_namespaces::MARKER),
            r#"["visuals"]"#,
        )
        .unwrap();
        let condition = json!([{"type":"neoforge:mod_loaded","modid":"code_only"}]);
        fs::write(
            dir.join("base.json"),
            serde_json::to_vec(&json!({
                "neoforge:conditions": condition,
                "values": ["minecraft:stone"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("namespace.json"),
            serde_json::to_vec(&json!({
                "neoforge:conditions": [{"type":"neoforge:mod_loaded","modid":"visuals"}],
                "values": ["minecraft:stone"]
            }))
            .unwrap(),
        )
        .unwrap();
        let bytes = |value: Value| serde_json::to_vec(&value).unwrap();
        let files = vec![
            (
                "tags/visuals:block/layers".to_owned(),
                vec![
                    bytes(json!({"values":["minecraft:stone"]})),
                    bytes(json!({"neoforge:conditions": condition, "replace":true,
                    "values":["minecraft:dirt"]})),
                    bytes(json!({"neoforge:conditions":[{"type":"unknown:condition"}],
                    "replace":true,"values":["minecraft:iron_block"]})),
                    bytes(json!({"values":["minecraft:gold_block"]})),
                ],
            ),
            (
                "tags/visuals:block/absent".to_owned(),
                vec![bytes(json!({
                    "neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:true"}}],
                    "replace":true,"values":["minecraft:iron_block"]
                }))],
            ),
        ];
        let overlay = PackOverlay::new(&files);
        for side in [None, Some(Side::Client), Some(Side::Both), None] {
            let inventory = EnabledMods::new(side.map(|side| ("code_only".into(), side)));
            fs::write(
                root.join(horizon_mod_package::enabled::MARKER),
                serde_json::to_vec(&inventory).unwrap(),
            )
            .unwrap();
            let loaded = load_tag_lists(&root, "block", &overlay).unwrap();
            assert_eq!(
                loaded["visuals:layers"],
                ["minecraft:iron_block", "minecraft:gold_block"]
            );
            assert_eq!(loaded["visuals:base"], ["minecraft:stone"]);
            assert!(
                loaded.contains_key("visuals:namespace"),
                "tag conditions are not evaluated"
            );
            assert!(
                loaded.contains_key("visuals:absent"),
                "tag conditions do not skip layers"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_higher_packs_remove_takes_entries_out_of_a_tag() {
        let root = std::env::temp_dir().join(format!("horizon-tag-remove-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let bytes = |value: Value| serde_json::to_vec(&value).unwrap();
        let files = vec![
            (
                "tags/test:item/things".to_owned(),
                vec![
                    bytes(json!({"values": ["minecraft:stone", "minecraft:dirt", "#test:other"]})),
                    bytes(json!({"values": [], "remove": ["minecraft:dirt", "#test:other"]})),
                    bytes(json!({"neoforge:conditions": [{"type": "neoforge:not",
                    "value": {"type": "neoforge:true"}}], "values": [], "remove": ["minecraft:stone"]})),
                ],
            ),
            (
                "tags/test:item/other".to_owned(),
                vec![bytes(json!({"values": ["minecraft:sand"]}))],
            ),
        ];
        let loaded = load_tag_lists(&root, "item", &PackOverlay::new(&files)).unwrap();
        assert!(loaded["test:things"].is_empty());
        assert_eq!(loaded["test:other"], ["minecraft:sand"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attribute_modifiers_come_from_the_stack() {
        let items = Items::for_tests(HashMap::new());
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        let attack = horizon_core::nbt::Tag::compound([
            (
                "type",
                horizon_core::nbt::Tag::String("minecraft:attack_damage".into()),
            ),
            (
                "id",
                horizon_core::nbt::Tag::String("minecraft:test".into()),
            ),
            ("amount", horizon_core::nbt::Tag::Double(0.1f64.to_bits())),
            (
                "operation",
                horizon_core::nbt::Tag::String("add_value".into()),
            ),
            ("slot", horizon_core::nbt::Tag::String("mainhand".into())),
        ]);
        sword.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(horizon_core::nbt::Tag::List(vec![attack])),
        );
        let modifiers = sword.default_attribute_modifiers(&items);
        assert_eq!(modifiers.len(), 1);
        assert_eq!(modifiers[0].attribute, "minecraft:attack_damage");
        // A double, as `AttributeModifier.amount`: not rounded through a float.
        assert_eq!(modifiers[0].amount, 0.1);
        drop(modifiers);
        sword.data_mut().set_other(
            "!minecraft:attribute_modifiers",
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new())),
        );
        assert!(sword.default_attribute_modifiers(&items).is_empty());
    }

    #[test]
    fn admitted_tool_rules_override_defaults_and_removals_survive_saving() {
        let root =
            std::env::temp_dir().join(format!("horizon-tool-components-{}", std::process::id()));
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        fs::write(root.join("items.json"), r#"{"minecraft:stick":{},"minecraft:diamond_pickaxe":{"minecraft:max_damage":1561,"minecraft:tool":{"rules":[{"blocks":"minecraft:stone","speed":8.0,"correct_for_drops":true}],"damage_per_block":1}}}"#).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        assert_eq!(items.mining_speed(Some(&pick), "minecraft:stone"), 8.0);
        assert!(items.correct_for_drops(Some(&pick), "minecraft:stone"));
        let removal = crate::item_data::command_components("[!tool]").unwrap();
        (pick.damage, pick.extra) = crate::item_data::decode_components(Some(&removal));
        let restored =
            crate::item_nbt::item_from_nbt(&crate::item_nbt::item_to_nbt(&pick)).unwrap();
        assert_eq!(items.mining_speed(Some(&restored), "minecraft:stone"), 1.0);
        assert!(!items.correct_for_drops(Some(&restored), "minecraft:stone"));
        assert_eq!(items.mining_damage(&restored), None);
        let mut stick = ItemStack::new("minecraft:stick", 1);
        let tool = crate::item_data::command_components(r#"[tool={rules:[{blocks:"stone",correct_for_drops:false},{blocks:["stone","dirt"],speed:7.0f,correct_for_drops:true},{blocks:"stone",speed:90.0f}],default_mining_speed:2.0f,damage_per_block:3}]"#).unwrap();
        (stick.damage, stick.extra) = crate::item_data::decode_components(Some(&tool));
        assert_eq!(items.mining_speed(Some(&stick), "minecraft:stone"), 7.0);
        assert!(!items.correct_for_drops(Some(&stick), "minecraft:stone"));
        assert_eq!(items.mining_speed(Some(&stick), "minecraft:cobweb"), 2.0);
        assert_eq!(items.mining_damage(&stick), Some(3));
        assert_eq!(items.mining_damage_capacity(&stick), 0);
        stick.data_mut().set_other(
            "minecraft:max_damage",
            Some(horizon_core::nbt::Tag::Int(20)),
        );
        assert_eq!(
            items.mining_damage_capacity(&stick),
            0,
            "MAX_DAMAGE alone does not supply DAMAGE"
        );
        stick.count = 0;
        assert_eq!(items.mining_speed(Some(&stick), "minecraft:stone"), 1.0);
        assert!(!items.correct_for_drops(Some(&stick), "minecraft:stone"));
        assert_eq!(items.mining_damage(&stick), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_loot_keeps_zero_positions_without_changing_filtered_rolls() {
        let mut items = Items::for_tests(HashMap::new());
        let zero = json!({"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot",
            "modifier":{"type":"minecraft:set_count","count":0}}]});
        let positive =
            json!({"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot"}]});
        let flat = json!({"pools":[zero, positive]});
        let empty = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:empty"}]}]});
        let nested =
            json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":flat}]}]});
        let random = json!({"pools":[zero, {"rolls":{"type":"minecraft:uniform","min":1,"max":3},
            "entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot"},
                       {"type":"minecraft:item","name":"minecraft:gold_ingot"}]}]});
        let summary = |stacks: Vec<ItemStack>| {
            stacks
                .into_iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(items.inline_loot_table_for_command(
                &flat,
                None,
                &mut LegacyRandom::new(39),
                true
            )),
            vec![
                ("minecraft:iron_ingot".into(), 0),
                ("minecraft:iron_ingot".into(), 1)
            ]
        );
        assert!(
            items
                .inline_loot_table_for_command(&empty, None, &mut LegacyRandom::new(39), true)
                .is_empty(),
            "EmptyLootItem emits nothing, rather than an empty item position"
        );
        let deferred = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:dynamic","name":"minecraft:contents"}],
            "modifier":{"type":"minecraft:set_count","count":0}}]});
        assert!(
            items
                .inline_loot_table_for_command(&deferred, None, &mut LegacyRandom::new(39), true)
                .is_empty(),
            "deferred dynamic placeholders are not emitted items"
        );
        for table in [flat, empty, nested, random, deferred] {
            items.loot = Arc::new(
                [
                    ("test:table".into(), table.clone()),
                    ("minecraft:entities/pig".into(), table.clone()),
                ]
                .into_iter()
                .collect(),
            );
            for seed in 0..32 {
                let mut natural_rng = LegacyRandom::new(seed);
                let natural =
                    summary(items.inline_loot_table_with_tool(&table, None, &mut natural_rng));
                let continuation = natural_rng.next_long();
                let mut command_rng = LegacyRandom::new(seed);
                let command =
                    items.inline_loot_table_for_command(&table, None, &mut command_rng, true);
                assert_eq!(
                    summary(command.into_iter().filter(|s| s.count > 0).collect()),
                    natural
                );
                assert_eq!(
                    command_rng.next_long(),
                    continuation,
                    "output policy consumes no randomness"
                );
                let mut named_rng = LegacyRandom::new(seed);
                let named =
                    items.loot_table_for_command("test:table", false, None, &mut named_rng, true);
                assert_eq!(
                    summary(named.into_iter().filter(|s| s.count > 0).collect()),
                    natural
                );
                assert_eq!(named_rng.next_long(), continuation);
                let mut entity_rng = LegacyRandom::new(seed);
                let entity = items.entity_drops_for_command(
                    "minecraft:pig",
                    false,
                    None,
                    &mut entity_rng,
                    true,
                );
                assert_eq!(
                    summary(entity.into_iter().filter(|s| s.count > 0).collect()),
                    natural
                );
                assert_eq!(entity_rng.next_long(), continuation);
            }
        }
    }

    /// `LootCommand.dropFishingLoot` carries TOOL even for an inline holder.
    #[test]
    fn inline_fishing_keeps_the_actual_tool_context() {
        let mut items = Items::for_tests(HashMap::new());
        let table = |tool| {
            json!({"type":"minecraft:fishing","pools":[{"rolls":1,"entries":[{
                "type":"minecraft:item","name":"minecraft:diamond",
                "condition":{"type":"minecraft:match_tool","predicate":{"items":tool}}
            }]}]})
        };
        let rod_table = table("minecraft:fishing_rod");
        let stick_table = table("minecraft:stick");
        let unconditioned = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
        items.loot = Arc::new(
            [
                ("test:rod".to_owned(), rod_table.clone()),
                ("test:stick".to_owned(), stick_table.clone()),
                ("test:plain".to_owned(), unconditioned.clone()),
            ]
            .into_iter()
            .collect(),
        );
        let rod = ItemStack::new("minecraft:fishing_rod", 1);
        let stick = ItemStack::new("minecraft:stick", 1);
        let summary = |drops: Vec<ItemStack>| {
            drops
                .into_iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect::<Vec<_>>()
        };
        for (id, table, matches) in [
            ("test:rod", &rod_table, [false, true, false]),
            ("test:stick", &stick_table, [false, false, true]),
            ("test:plain", &unconditioned, [true, true, true]),
        ] {
            for (held, matches) in [None, Some(&rod), Some(&stick)].into_iter().zip(matches) {
                let expected = if matches {
                    vec![("minecraft:diamond".to_owned(), 1)]
                } else {
                    vec![]
                };
                let mut named_rng = LegacyRandom::new(41);
                assert_eq!(
                    summary(items.loot_table(id, false, held, &mut named_rng)),
                    expected
                );
                let mut inline_rng = LegacyRandom::new(41);
                assert_eq!(
                    summary(items.inline_loot_table_with_tool(table, held, &mut inline_rng)),
                    expected
                );
                let mut command_rng = LegacyRandom::new(41);
                assert_eq!(
                    summary(items.inline_loot_table_for_command(
                        table,
                        held,
                        &mut command_rng,
                        true
                    )),
                    expected
                );
                let continuation = named_rng.next_long();
                assert_eq!(
                    inline_rng.next_long(),
                    continuation,
                    "same evaluator draw order"
                );
                assert_eq!(
                    command_rng.next_long(),
                    continuation,
                    "command output keeps TOOL and draw order"
                );
            }
            let mut random = LegacyRandom::new(41);
            let legacy = summary(items.inline_loot_table(table, &mut random));
            assert_eq!(
                legacy,
                if matches[0] {
                    vec![("minecraft:diamond".to_owned(), 1)]
                } else {
                    vec![]
                }
            );
        }
    }

    #[test]
    fn loaded_block_and_item_tags_answer_membership() {
        let mut items = super::Items::for_tests(std::collections::HashMap::from([(
            "test:items".into(),
            ["test:apple".into()].into_iter().collect(),
        )]));
        items.block_tags.insert(
            "test:blocks".into(),
            ["test:stone".into()].into_iter().collect(),
        );
        assert!(items.block_in_tag("test:blocks", "test:stone"));
        assert!(!items.block_in_tag("test:blocks", "test:dirt"));
        assert!(!items.block_in_tag("test:missing", "test:stone"));
        assert!(items.item_in_tag("test:items", "test:apple"));
        assert!(!items.item_in_tag("test:items", "test:stone"));
        assert!(!items.item_in_tag("test:missing", "test:apple"));
    }

    /// `NestedLootTable`: a `loot_table` entry, by id or inline, rolls that
    /// table in full and applies its own functions to each stack; a table
    /// never rolls inside itself.
    #[test]
    fn loot_table_entries_roll_their_table() {
        let mut items = Items::for_tests(HashMap::new());
        let diamond = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
        items.loot = Arc::new(
            [
                ("test:inner", diamond.clone()),
                ("test:outer", json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table",
                    "value":"test:inner","modifier":[{"type":"minecraft:set_count","count":3}]}]}]})),
                ("test:inline", json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table",
                    "value":diamond}]}]})),
                ("test:loop", json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table",
                    "value":"test:loop"}]},{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]})),
            ]
            .map(|(id, t)| (id.to_owned(), t))
            .into_iter()
            .collect(),
        );
        let roll = |id: &str| {
            items
                .loot_table(id, false, None, &mut LegacyRandom::new(1))
                .into_iter()
                .map(|s| format!("{} {}", s.item, s.count))
                .collect::<Vec<_>>()
        };
        assert_eq!(roll("test:outer"), ["minecraft:diamond 3"]);
        assert_eq!(roll("test:inline"), ["minecraft:diamond 1"]);
        // `test:loop` is already rolling: its own entry adds nothing.
        assert_eq!(roll("test:loop"), ["minecraft:stick 1"]);
    }

    /// `Blocks.wallVariant`: a wall block without a table of its own drops
    /// from its standing block's (vanilla's tables: `blocks/torch` for
    /// `wall_torch`, `blocks/oak_sign` for `oak_wall_sign`...).
    #[test]
    fn wall_blocks_use_the_standing_blocks_table() {
        let facts = br#"{"defaults":{"minecraft:air":0,"minecraft:wall_torch":1,
            "minecraft:oak_wall_sign":2,"minecraft:skeleton_wall_skull":3,
            "minecraft:dead_brain_coral_wall_fan":4,"test:wall_lamp":5,"minecraft:cobblestone_wall":6},
            "states":[
            ["air",true,false,"","minecraft:air"],
            ["a",false,false,"","minecraft:wall_torch"],
            ["b",false,false,"","minecraft:oak_wall_sign"],
            ["c",false,false,"","minecraft:skeleton_wall_skull"],
            ["d",false,false,"","minecraft:dead_brain_coral_wall_fan"],
            ["e",false,false,"","test:wall_lamp"],
            ["f",false,false,"","minecraft:cobblestone_wall"]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new(
            [
                "minecraft:blocks/torch",
                "minecraft:blocks/oak_sign",
                "minecraft:blocks/skeleton_skull",
                "minecraft:blocks/dead_brain_coral_fan",
                "test:blocks/wall_lamp",
                "test:blocks/lamp",
                "minecraft:blocks/cobblestone_wall",
            ]
            .map(|id| (id.to_owned(), json!({"pools": []})))
            .into_iter()
            .collect(),
        );
        let table = |state| items.block_loot_table(&registry, BlockStateId(state));
        assert_eq!(table(1).as_deref(), Some("minecraft:blocks/torch"));
        assert_eq!(table(2).as_deref(), Some("minecraft:blocks/oak_sign"));
        assert_eq!(table(3).as_deref(), Some("minecraft:blocks/skeleton_skull"));
        assert_eq!(
            table(4).as_deref(),
            Some("minecraft:blocks/dead_brain_coral_fan")
        );
        // A table of its own wins over the standing block's.
        assert_eq!(table(5).as_deref(), Some("test:blocks/wall_lamp"));
        assert_eq!(
            table(6).as_deref(),
            Some("minecraft:blocks/cobblestone_wall")
        );
        assert_eq!(table(0), None);
    }

    #[test]
    fn content_mod_blocks_keep_their_namespace_in_loot_table_ids() {
        let facts = br#"{"defaults":{"minecraft:air":0,"loot_repro:glowing_amethyst":1},
            "states":[
            ["air",true,false,"","minecraft:air"],
            ["amethyst",false,false,"","loot_repro:glowing_amethyst"]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new(
            [(
                "loot_repro:blocks/glowing_amethyst".to_owned(),
                json!({"pools": []}),
            )]
            .into_iter()
            .collect(),
        );

        assert_eq!(
            items
                .block_loot_table(&registry, BlockStateId(1))
                .as_deref(),
            Some("loot_repro:blocks/glowing_amethyst")
        );
    }

    #[test]
    fn entity_target_context_leaves_unaffected_rolls_and_rng_continuation_unchanged() {
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new([("minecraft:entities/test".into(), json!({"pools":[{
            "rolls":{"type":"minecraft:uniform","min":1,"max":3},
            "condition":{"type":"minecraft:all_of","terms":[
                {"type":"minecraft:killed_by_player"},
                {"type":"minecraft:match_tool","predicate":{"items":"minecraft:stick"}}]},
            "entries":[{"type":"minecraft:item","name":"minecraft:diamond","weight":1},
                       {"type":"minecraft:item","name":"minecraft:iron_ingot","weight":3}],
            "modifier":[{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":4}}]
        }]}))].into_iter().collect());
        let target = horizon_world::loot::LootEntity::new("minecraft:sheep");
        let held = ItemStack::new("minecraft:stick", 1);
        let unused = |_: &Value, _: &horizon_world::loot::LootEntity| -> bool {
            panic!("no entity predicate on this unaffected path")
        };
        for seed in [0, 1, 99, i64::MAX] {
            for by_player in [false, true] {
                let mut old = LegacyRandom::new(seed);
                let mut bound = old.clone();
                let expected =
                    items.entity_drops("minecraft:test", by_player, Some(&held), &mut old);
                let actual = items.entity_drops_with_target(
                    "minecraft:test",
                    by_player,
                    Some(&held),
                    &target,
                    &unused,
                    &mut bound,
                    false,
                );
                assert_eq!(actual, expected);
                assert_eq!(bound.next_long(), old.next_long());
                assert_eq!(bound.next_int_bound(137), old.next_int_bound(137));
            }
        }
    }

    #[test]
    fn entity_target_context_reuses_the_loaded_matcher_only_for_this() {
        use horizon_world::loot::{LootData, LootEntity};
        let root = std::env::temp_dir().join(format!("h036-items-matcher-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let data = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let matcher = |p: &Value, e: &LootEntity| data.entity_matches(p, Some(e));
        let mut items = Items::for_tests(HashMap::new());
        for who in ["this", "attacker"] {
            items.loot = Arc::new(
                [(
                    "minecraft:entities/sheep".into(),
                    json!({"pools":[{"rolls":1,"entries":[{
                        "type":"minecraft:item","name":"minecraft:white_wool",
                        "condition":{"type":"minecraft:entity_properties","entity":who,"predicate":{
                            "minecraft:components":{"minecraft:sheep/color":"white"},
                            "minecraft:type_specific/sheep":{"sheared":false}}}
                    }]}]}),
                )]
                .into_iter()
                .collect(),
            );
            for (color, sheared) in [("white", false), ("white", true), ("red", false)] {
                let mut target = LootEntity::new("minecraft:sheep");
                target.sheep = Some((color.into(), sheared));
                let mut random = LegacyRandom::new(36);
                let actual = items.entity_drops_with_target(
                    "minecraft:sheep",
                    true,
                    None,
                    &target,
                    &matcher,
                    &mut random,
                    false,
                );
                assert_eq!(
                    !actual.is_empty(),
                    who == "this" && color == "white" && !sheared
                );
                assert!(
                    items
                        .entity_drops("minecraft:sheep", true, None, &mut LegacyRandom::new(36))
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn location_context_preserves_unaffected_rolls_and_rng_continuation() {
        let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new([("minecraft:blocks/stone".into(), json!({"pools":[{
            "rolls":{"type":"minecraft:uniform","min":1,"max":3},
            "condition":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stick"}},
            "entries":[{"type":"minecraft:item","name":"minecraft:diamond","weight":1},
                       {"type":"minecraft:item","name":"minecraft:emerald","weight":3}],
            "modifier":[{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":4}}]
        }]}))].into_iter().collect());
        let unused = |_: &Value| -> Option<bool> {
            panic!("unaffected conditions must not invoke location reader")
        };
        for seed in [0, 1, 79, i64::MAX] {
            for held in [None, Some(ItemStack::new("minecraft:stick", 1))] {
                let mut old = LegacyRandom::new(seed);
                let mut contextual = old.clone();
                assert_eq!(
                    items.block_drops(
                        &registry,
                        BlockStateId(0),
                        BlockLootRoll {
                            held: held.as_ref(),
                            ..Default::default()
                        },
                        &mut old,
                        None
                    ),
                    items.block_drops(
                        &registry,
                        BlockStateId(0),
                        BlockLootRoll {
                            held: held.as_ref(),
                            location: Some(&unused),
                            ..Default::default()
                        },
                        &mut contextual,
                        None,
                    )
                );
                assert_eq!(old.next_long(), contextual.next_long());
                assert_eq!(old.next_int_bound(137), contextual.next_int_bound(137));
            }
        }
    }

    #[test]
    fn location_context_keeps_unbound_and_unsupported_legacy_behavior() {
        let items = Items::for_tests(HashMap::new());
        let properties = horizon_world::chunk::BlockProperties::new();
        let condition = json!({"type":"minecraft:location_check","predicate":{"block":{"blocks":"minecraft:stone"}}});
        let table = json!({"pools":[{"rolls":1,"condition":condition,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
        let unbound = ctx(&items, None, &properties, "minecraft:stone");
        assert_eq!(
            unbound.evaluate(&table, &mut LegacyRandom::new(79)).len(),
            1
        );
        for value in [Some(false), Some(true), None] {
            let reader = |_: &Value| value;
            let bound = LootContext {
                location: Some(&reader),
                ..ctx(&items, None, &properties, "minecraft:stone")
            };
            assert_eq!(
                bound.evaluate(&table, &mut LegacyRandom::new(79)).len(),
                usize::from(value.unwrap_or(true))
            );
        }
    }

    fn ctx<'a>(
        items: &'a Items,
        held: Option<&'a ItemStack>,
        props: &'a horizon_world::chunk::BlockProperties,
        block: &'a str,
    ) -> LootContext<'a, LegacyRandom> {
        LootContext {
            query: horizon_world::loot::query::current_query(),
            items,
            block,
            properties: props,
            held,
            killed_by_player: block.is_empty(),
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            location: None,
            keep_empty: false,
        }
    }

    /// A mod's kind nobody claims fails; a reference by name isn't a kind,
    /// and fails too.
    #[test]
    fn an_unclaimed_condition_of_a_mods_kind_fails() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let table = |condition: Value| {
            json!({"pools":[{"rolls":1,"entries":[
                {"type":"minecraft:item","name":"minecraft:stone","condition":condition}]}]})
        };
        let mut random = LegacyRandom::new(7);
        let mut drops = |condition: Value, block: &str| {
            ctx(&items, None, &props, block)
                .evaluate(&table(condition), &mut random)
                .len()
        };
        assert_eq!(drops(json!({"type":"mymod:always"}), "minecraft:stone"), 0);
        assert_eq!(drops(json!({"type":"mymod:always"}), ""), 0);
        assert_eq!(drops(json!("mymod:always"), "minecraft:stone"), 0);
        assert_eq!(drops(Value::Null, "minecraft:stone"), 1);
    }

    /// A mod's function kind changes the stack the items engine drops: what
    /// the hook leaves in the stack comes back (item, count, a component),
    /// through the conversion the block-drop path uses.
    #[test]
    fn a_mod_function_changes_the_stack_the_items_engine_drops() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let mut random = LegacyRandom::new(7);
        let block = ctx(&items, None, &props, "minecraft:stone");
        let native = block.native_loot_context(&mut random);
        let mut stack = ItemStack::new("minecraft:stone", 1);
        mod_function(
            "mymod:seven",
            &json!({"type": "mymod:seven"}),
            &native,
            &mut stack,
            |_, _, _, loot| {
                loot.item = "minecraft:diamond".to_owned();
                loot.count = 7;
                loot.components.push((
                    "minecraft:custom_data".to_owned(),
                    Some(
                        horizon_core::json_nbt::json_to_tag(&json!({"machine": "furnace"}))
                            .unwrap(),
                    ),
                ));
            },
        );
        assert_eq!(&*stack.item, "minecraft:diamond");
        assert_eq!(stack.count, 7);
        assert_eq!(
            stack.data().and_then(|d| d.other("minecraft:custom_data")),
            Some(&horizon_client_core::scoreboard::json_to_tag(
                &json!({"machine": "furnace"})
            ))
        );
    }

    #[test]
    fn loot_patches_keep_nbt_types_removals_and_order() {
        use horizon_core::nbt::Tag;
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack
            .data_mut()
            .set_other("test:array", Some(Tag::ByteArray(vec![7])));
        stack.data_mut().set_other("test:byte", Some(Tag::Byte(7)));
        let before = loot_of(&stack);
        assert!(
            before
                .components
                .contains(&("test:array".into(), Some(Tag::ByteArray(vec![7]))))
        );
        assert!(
            before
                .components
                .contains(&("test:byte".into(), Some(Tag::Byte(7))))
        );
        let mut after = before.clone();
        after.components.extend([
            ("test:array".into(), Some(Tag::List(vec![Tag::Int(7)]))),
            ("test:array".into(), None),
            ("test:array".into(), Some(Tag::ByteArray(vec![7]))),
            ("test:byte".into(), None),
        ]);
        apply_loot(&before, &after, &mut stack);
        assert_eq!(
            stack.data().unwrap().other("test:array"),
            Some(&Tag::ByteArray(vec![7]))
        );
        assert_eq!(stack.data().unwrap().other("test:byte"), None);
    }

    #[test]
    fn loot_trim_patch_decodes_materializes_captures_and_removes() {
        use horizon_core::nbt::Tag;
        let mut stack = ItemStack::new("minecraft:iron_chestplate", 1);
        let before = loot_of(&stack);
        let mut after = before.clone();
        let trim = horizon_core::json_nbt::json_to_tag(
            &json!({"material": "minecraft:gold", "pattern": "minecraft:sentry"}),
        )
        .unwrap();
        after.components.push(("minecraft:trim".into(), Some(trim)));
        apply_loot(&before, &after, &mut stack);
        assert_eq!(
            stack.data().unwrap().trim.as_ref().cloned(),
            Some((
                crate::item_data::trim::Holder::Named("minecraft:gold".into()),
                crate::item_data::trim::Holder::Named("minecraft:sentry".into())
            ))
        );
        let captured = loot_of(&stack);
        assert_eq!(
            captured.components,
            vec![(
                "minecraft:trim".into(),
                Some(Tag::Compound(
                    [
                        ("material".into(), Tag::String("minecraft:gold".into())),
                        ("pattern".into(), Tag::String("minecraft:sentry".into())),
                    ]
                    .into()
                ))
            )]
        );
        let mut removed = captured.clone();
        removed.components.push(("minecraft:trim".into(), None));
        apply_loot(&captured, &removed, &mut stack);
        assert!(stack.data().is_none_or(|data| data.trim.is_none()));
    }

    #[test]
    fn an_unrelated_loot_patch_does_not_restore_a_captured_name() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &json!("Old"));
        let before = loot_of(&stack);
        let mut after = before.clone();
        after
            .names
            .push(("minecraft:custom_name".into(), json!("New")));
        after
            .components
            .push(("test:byte".into(), Some(horizon_core::nbt::Tag::Byte(7))));
        apply_loot(&before, &after, &mut stack);
        assert_eq!(
            stack.data().unwrap().custom_name,
            Some(horizon_core::nbt::Tag::String("New".into()))
        );
    }

    /// A function that leaves the stack alone keeps its own form: the round
    /// trip through the loot engine's stack changes nothing.
    #[test]
    fn a_mod_function_that_changes_nothing_keeps_the_stacks_form() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let mut random = LegacyRandom::new(7);
        let block = ctx(&items, None, &props, "minecraft:stone");
        let native = block.native_loot_context(&mut random);
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.enchant("minecraft:sharpness", 3);
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &json!("Excalibur"));
        let before = stack.clone();
        mod_function(
            "mymod:nothing",
            &json!({"type": "mymod:nothing"}),
            &native,
            &mut stack,
            |_, _, _, _| {},
        );
        assert_eq!(stack, before);
    }

    /// The items engine reaches the function hook: a pool whose function kind
    /// is a mod's draws the hook's one `next_long` where a vanilla pool draws
    /// none. The hook is unclaimed here, so the drop keeps its count.
    #[test]
    fn a_mod_function_kind_reaches_the_items_engines_roll() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let vanilla = json!({"pools":[{"rolls":1,"entries":[
            {"type":"minecraft:item","name":"minecraft:stone"}]}]});
        let modded = json!({"pools":[{"rolls":1,"entries":[
            {"type":"minecraft:item","name":"minecraft:stone"}],
            "modifier":[{"type":"mymod:duplicate","factor":2}]}]});
        let mut a = LegacyRandom::new(100);
        let mut b = LegacyRandom::new(100);
        let _ = ctx(&items, None, &props, "minecraft:stone").evaluate(&vanilla, &mut a);
        let expected: Vec<i64> = (0..5).map(|_| b.next_long()).collect();
        let got: Vec<i64> = (0..5).map(|_| a.next_long()).collect();
        assert_eq!(got, expected, "a vanilla table shifted the random");

        let mut c = LegacyRandom::new(100);
        let mut d = LegacyRandom::new(100);
        let drops = ctx(&items, None, &props, "minecraft:stone").evaluate(&modded, &mut c);
        let _ = d.next_long(); // the hook's seed
        let expected: Vec<i64> = (0..5).map(|_| d.next_long()).collect();
        let got: Vec<i64> = (0..5).map(|_| c.next_long()).collect();
        assert_eq!(got, expected, "the mod function did not draw its seed");
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].count, 1);
    }

    #[test]
    fn fortune_silk_touch_and_looting() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pick.enchant("minecraft:fortune", 3);
        let ore = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:alternatives","children":[
            {"type":"minecraft:item","condition":"minecraft:tool/can_silk_touch","name":"minecraft:diamond_ore"},
            {"type":"minecraft:item","name":"minecraft:diamond","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:ore_drops"}]}]}]}]});
        let mut random = LegacyRandom::new(7);
        let n = 20000;
        let total: u32 = (0..n)
            .map(|_| {
                ctx(&items, Some(&pick), &props, "minecraft:diamond_ore")
                    .evaluate(&ore, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        // Fortune III ore drops average (1 + 1 + 2 + 3 + 4) / 5 = 2.2 diamonds.
        let mean = f64::from(total) / f64::from(n);
        assert!((mean - 2.2).abs() < 0.05, "{mean}");
        let mut silk = ItemStack::new("minecraft:diamond_pickaxe", 1);
        silk.enchant("minecraft:silk_touch", 1);
        let drops =
            ctx(&items, Some(&silk), &props, "minecraft:diamond_ore").evaluate(&ore, &mut random);
        assert_eq!(&*drops[0].item, "minecraft:diamond_ore");
        let plain = ItemStack::new("minecraft:diamond_pickaxe", 1);
        let drops =
            ctx(&items, Some(&plain), &props, "minecraft:diamond_ore").evaluate(&ore, &mut random);
        assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
        // Looting III adds level x uniform(0, 1) on player kills.
        let flesh = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:rotten_flesh","modifier":[
            {"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":0.0,"max":2.0}},
            {"type":"minecraft:enchanted_count_increase","count":{"type":"minecraft:uniform","min":0.0,"max":1.0},"enchantment":"minecraft:looting"}]}]}]});
        let mut sword = ItemStack::new("minecraft:iron_sword", 1);
        sword.enchant("minecraft:looting", 3);
        let looted: u32 = (0..n)
            .map(|_| {
                ctx(&items, Some(&sword), &props, "")
                    .evaluate(&flesh, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        let base: u32 = (0..n)
            .map(|_| {
                ctx(&items, None, &props, "")
                    .evaluate(&flesh, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        assert!(f64::from(looted) / f64::from(n) > f64::from(base) / f64::from(n) + 1.0);
        // Table bonus uses the chance for the tool's level.
        let leaves = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]}],"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]});
        assert_eq!(
            ctx(&items, Some(&pick), &props, "minecraft:oak_leaves")
                .evaluate(&leaves, &mut random)
                .len(),
            1
        );
        assert!(
            ctx(&items, Some(&plain), &props, "minecraft:oak_leaves")
                .evaluate(&leaves, &mut random)
                .is_empty()
        );
    }

    #[test]
    fn set_potion_gives_the_potion_its_contents() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:splash_potion",
            "modifier":{"type":"minecraft:set_potion","id":"minecraft:fire_resistance"}}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(1));
        assert_eq!(drops.len(), 1);
        assert_eq!(&*drops[0].item, "minecraft:splash_potion");
        assert_eq!(drops[0].potion(), Some("minecraft:fire_resistance"));
    }

    /// What `levels` answers is what `/loot ... kill`'s two readers use, with
    /// no player at all; a level left alone rolls as if nobody were asked.
    #[test]
    fn the_answered_level_drives_a_kills_count_and_chance() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let reads = std::cell::RefCell::new(Vec::new());
        let levels = |_: &mut LegacyRandom, id: &str, level: u32| {
            reads.borrow_mut().push((id.to_owned(), level));
            3
        };
        let answered = LootContext {
            killed_by_player: false,
            entity_levels: Some(&levels),
            ..ctx(&items, None, &props, "")
        };
        let count = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":2.0,"enchantment":"minecraft:looting"}]}]}]});
        let got = answered.evaluate(&count, &mut LegacyRandom::new(7));
        assert_eq!(got[0].count, 7);
        let chance = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:random_chance_with_enchanted_bonus","enchantment":"minecraft:looting","unenchanted_chance":0.0,"enchanted_chance":1.0}],"entries":[{"type":"minecraft:item","name":"minecraft:bone"}]}]});
        assert_eq!(
            answered.evaluate(&chance, &mut LegacyRandom::new(7)).len(),
            1
        );
        let looting = ("minecraft:looting".to_owned(), 0);
        assert_eq!(*reads.borrow(), [looting.clone(), looting]);

        let mut sword = ItemStack::new("minecraft:iron_sword", 1);
        sword.enchant("minecraft:looting", 2);
        let same = |_: &mut LegacyRandom, _: &str, level: u32| level;
        let table = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:random_chance_with_enchanted_bonus","enchantment":"minecraft:looting","unenchanted_chance":0.5,"enchanted_chance":0.5}],"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":{"type":"minecraft:uniform","min":0.0,"max":3.0},"enchantment":"minecraft:looting"}]}]}]});
        for seed in 0..16 {
            let (mut plain, mut asked) = (LegacyRandom::new(seed), LegacyRandom::new(seed));
            let expected = ctx(&items, Some(&sword), &props, "").evaluate(&table, &mut plain);
            let got = LootContext {
                entity_levels: Some(&same),
                ..ctx(&items, Some(&sword), &props, "")
            }
            .evaluate(&table, &mut asked);
            assert_eq!(
                got.iter().map(|s| s.count).collect::<Vec<_>>(),
                expected.iter().map(|s| s.count).collect::<Vec<_>>()
            );
            assert_eq!(asked.next_long(), plain.next_long());
        }
    }

    /// A draw the answer makes on the roll's random is a draw of the roll.
    #[test]
    fn a_kills_answer_draws_on_the_rolls_own_random() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let drawing = |random: &mut LegacyRandom, _: &str, level: u32| {
            random.next_int();
            level
        };
        let context = LootContext {
            entity_levels: Some(&drawing),
            ..ctx(&items, None, &props, "")
        };
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":1.0,"enchantment":"minecraft:looting"}]}]}]});
        let mut random = LegacyRandom::new(7);
        context.evaluate(&table, &mut random);
        let mut expected = LegacyRandom::new(7);
        expected.next_int();
        assert_eq!(random.next_long(), expected.next_long());
    }

    /// A mod may answer any level, and it counts as `MAX_LOOT_LEVEL`.
    #[test]
    fn a_kills_largest_level_counts_as_the_most_a_table_reads() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let huge = |_: &mut LegacyRandom, _: &str, _: u32| u32::MAX;
        let context = LootContext {
            entity_levels: Some(&huge),
            ..ctx(&items, None, &props, "")
        };
        let count = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":2.0,"enchantment":"minecraft:looting"}]}]}]});
        let got = context.evaluate(&count, &mut LegacyRandom::new(7));
        assert_eq!(got[0].count, 1 + 2 * MAX_LOOT_LEVEL as u32);
        let squared = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:random_chance_with_enchanted_bonus","enchantment":"minecraft:looting","unenchanted_chance":0.0,"enchanted_chance":{"type":"minecraft:levels_squared","added":0.0}}],"entries":[{"type":"minecraft:item","name":"minecraft:bone"}]}]});
        assert_eq!(
            context.evaluate(&squared, &mut LegacyRandom::new(7)).len(),
            1
        );
    }

    #[test]
    fn unrelated_entity_reader_does_not_ask_the_block_listener() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let reads = std::cell::Cell::new(0);
        let levels = |_: &str, _: u32| {
            reads.set(reads.get() + 1);
            3
        };
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":2.0}]}]}]});
        let drops = items.roll_block_table(
            &table,
            "minecraft:stone",
            &props,
            BlockLootRoll {
                levels: Some(&levels),
                ..Default::default()
            },
            &mut LegacyRandom::new(7),
        );
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].count, 1);
        assert_eq!(reads.get(), 0);
    }

    #[test]
    fn unrelated_block_reader_does_not_ask_the_entity_listener() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let reads = std::cell::Cell::new(0);
        let levels = |_: &mut LegacyRandom, _: &str, _: u32| {
            reads.set(reads.get() + 1);
            3
        };
        let context = LootContext {
            entity_levels: Some(&levels),
            ..ctx(&items, None, &props, "")
        };
        let table = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]}],"entries":[{"type":"minecraft:item","name":"minecraft:bone"}]}]});
        let mut random = LegacyRandom::new(7);
        assert!(context.evaluate(&table, &mut random).is_empty());
        assert_eq!(reads.get(), 0);
        let mut expected = LegacyRandom::new(7);
        expected.next_float();
        assert_eq!(random.next_long(), expected.next_long());
    }

    /// The game's answer is the level a block table's count and condition
    /// use; the tool keeps its own, and the table's draws stay the formula's.
    #[test]
    fn a_game_level_replaces_the_tools_in_a_block_table() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let mut pick = ItemStack::new("minecraft:iron_pickaxe", 1);
        pick.enchant("minecraft:fortune", 1);
        let reads = std::cell::RefCell::new(Vec::new());
        let levels = |_: &mut LegacyRandom, id: &str, level: u32| {
            reads.borrow_mut().push((id.to_owned(), level));
            3
        };
        let answered = |pick| LootContext {
            block_levels: Some(&levels),
            target: None,
            ..ctx(&items, pick, &props, "minecraft:stone")
        };
        let count = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:uniform_bonus_count","parameters":{"bonusMultiplier":2}}]}]}]});
        let mut random = LegacyRandom::new(7);
        let got = answered(Some(&pick)).evaluate(&count, &mut random);
        let mut expected = LegacyRandom::new(7);
        let bonus = expected.next_int_bound(7).unwrap() as u32;
        assert_eq!(got[0].count, 1 + bonus);
        assert_eq!(random.next_int(), expected.next_int());
        assert_eq!(*reads.borrow(), [("minecraft:fortune".to_owned(), 1)]);
        assert_eq!(pick.enchantment_level("minecraft:fortune"), 1);

        let chance = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]}],"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]});
        assert!(
            ctx(&items, None, &props, "minecraft:stone")
                .evaluate(&chance, &mut LegacyRandom::new(7))
                .is_empty()
        );
        let got = answered(None).evaluate(&chance, &mut LegacyRandom::new(7));
        assert_eq!(&*got[0].item, "minecraft:apple");
        assert_eq!(reads.borrow()[1], ("minecraft:fortune".to_owned(), 0));
    }

    /// A mod can answer any level, and it counts as `MAX_LOOT_LEVEL`: no
    /// formula overflows, and `binomial_with_bonus_count` draws once a level
    /// up to there, so its loop stays short.
    #[test]
    fn the_largest_level_counts_as_the_most_a_table_reads() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let levels = |_: &mut LegacyRandom, _: &str, _: u32| u32::MAX;
        let context = LootContext {
            block_levels: Some(&levels),
            target: None,
            ..ctx(&items, None, &props, "minecraft:stone")
        };
        for bonus in [
            json!({"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:ore_drops"}),
            json!({"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:uniform_bonus_count","parameters":{"bonusMultiplier":2}}),
        ] {
            let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","modifier":[bonus, bonus, bonus]}]}]});
            for seed in 0..32 {
                let drops = context.evaluate(&table, &mut LegacyRandom::new(seed));
                assert!(drops.iter().all(|stack| stack.count <= i32::MAX as u32));
            }
        }
        let binomial = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:wheat_seeds","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:binomial_with_bonus_count","parameters":{"extra":3,"probability":1.0}}]}]}]});
        let mut random = LegacyRandom::new(7);
        let drops = context.evaluate(&binomial, &mut random);
        assert_eq!(drops[0].count, 1 + MAX_LOOT_LEVEL as u32 + 3);
        let mut expected = LegacyRandom::new(7);
        for _ in 0..MAX_LOOT_LEVEL + 3 {
            expected.next_float();
        }
        assert_eq!(random.next_long(), expected.next_long());
    }

    /// Vanilla's levels stop at 255, below the bound: such a level reads and
    /// draws in full.
    #[test]
    fn the_largest_vanilla_level_is_read_in_full() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let mut pick = ItemStack::new("minecraft:iron_pickaxe", 1);
        pick.enchant("minecraft:fortune", 255);
        let binomial = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:wheat_seeds","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:binomial_with_bonus_count","parameters":{"extra":3,"probability":1.0}}]}]}]});
        let context = ctx(&items, Some(&pick), &props, "minecraft:stone");
        let drops = context.evaluate(&binomial, &mut LegacyRandom::new(7));
        assert_eq!(drops[0].count, 1 + 255 + 3);
    }
    /// `/loot give` and every other table this engine rolls: set_name and
    /// set_components reach the stack, as they do in a chest's roll.
    #[test]
    fn set_name_and_set_components_reach_the_rolled_item() {
        let items = Items::for_tests(HashMap::new());
        let props = horizon_world::chunk::BlockProperties::new();
        let effects = json!({"custom_effects":[{"id":"minecraft:absorption","duration":2400}]});
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:potion",
            "modifier":[
                {"type":"minecraft:set_components","components":{"minecraft:potion_contents":effects}},
                {"type":"minecraft:set_name","target":"item_name","name":{"text":"Potion of 1-Up","italic":false}},
                {"type":"minecraft:set_name","name":"Lucky"},
                {"type":"minecraft:set_name"}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(1));
        let saved = crate::item_data::encode_components(&drops[0]).expect("components");
        let saved = horizon_client_core::scoreboard::tag_to_json(&saved);
        assert_eq!(saved["minecraft:custom_name"], "Lucky");
        assert_eq!(saved["minecraft:item_name"]["text"], "Potion of 1-Up");
        let effect = &saved["minecraft:potion_contents"]["custom_effects"][0];
        assert_eq!(effect["id"], "minecraft:absorption");
        assert_eq!(effect["duration"], 2400);
        // The potion's effect is where drinking reads it, not only saved.
        assert_eq!(
            drops[0].custom_potion_effects(),
            [("minecraft:absorption".to_owned(), 2400, 0)]
        );
        // `!id` takes away a component the item has by default.
        let apple = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple",
            "modifier":{"type":"minecraft:set_components","components":{"!food":{}}}}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&apple, &mut LegacyRandom::new(1));
        let saved = crate::item_data::encode_components(&drops[0]).expect("components");
        assert!(saved.get("!minecraft:food").is_some());
    }

    /// `/loot give` and every other table this dispatcher rolls: the two
    /// enchantment functions (`EnchantWithLevelsFunction`,
    /// `EnchantRandomlyFunction`) put their enchantments on the stack.
    #[test]
    fn loot_enchant_functions_enchant_the_rolled_item() {
        let Some(items) = super::prepared_items() else {
            return;
        };
        let props = horizon_world::chunk::BlockProperties::new();
        let mut random = LegacyRandom::new(11);
        // A sword at levels 20..39 comes out enchanted, as in an End city.
        let levels = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:diamond_sword","modifier":[
            {"type":"minecraft:enchant_with_levels","levels":{"type":"minecraft:uniform","min":20.0,"max":39.0}}]}]}]});
        let mut enchanted = 0;
        for _ in 0..40 {
            let drops = ctx(&items, None, &props, "").evaluate(&levels, &mut random);
            if drops[0].enchantments().is_empty() {
                continue;
            }
            enchanted += 1;
            assert_eq!(&*drops[0].item, "minecraft:diamond_sword");
            // Levels stay within the enchantment's range.
            for (id, level) in drops[0].enchantments() {
                let max = items
                    .enchantments
                    .get(id)
                    .expect("a known enchantment")
                    .max_level;
                assert!((1..=max).contains(level), "{id} at {level}");
            }
        }
        assert!(
            enchanted > 30,
            "only {enchanted} of 40 rolls were enchanted"
        );
        // `enchant_randomly` on a book makes one enchanted book, its levels
        // stored; the pool is the enchantment tag it names.
        let book = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:book","modifier":[
            {"type":"minecraft:enchant_randomly","options":"#minecraft:on_random_loot"}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&book, &mut random);
        assert_eq!(&*drops[0].item, "minecraft:enchanted_book");
        assert_eq!(drops[0].count, 1);
        assert!(drops[0].enchantments().is_empty());
        let stored = drops[0].stored_enchantments();
        assert_eq!(stored.len(), 1, "{stored:?}");
        assert!(
            items
                .enchantments
                .in_tag("minecraft:on_random_loot", &stored[0].0)
        );
        // An item no enchantment accepts keeps none of them.
        let dirt = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:dirt","modifier":[
            {"type":"minecraft:enchant_with_levels","levels":{"type":"minecraft:constant","value":30.0}}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&dirt, &mut random);
        assert!(drops[0].enchantments().is_empty());
    }

    /// `Mth.nextInt(random, 1, max)` returns `1` without drawing when the
    /// enchantment has one level (`EnchantRandomlyFunction.enchantItem`), so
    /// the rolls after it must not shift. Vanilla with `LegacyRandom(11)`: a
    /// Mending book, then `set_count` uniform 1..64, is 46.
    #[test]
    fn a_single_level_enchantment_draws_no_level() {
        let Some(items) = super::prepared_items() else {
            return;
        };
        let props = horizon_world::chunk::BlockProperties::new();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:book","modifier":[
            {"type":"minecraft:enchant_randomly","options":"minecraft:mending"},
            {"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1.0,"max":64.0}}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(11));
        assert_eq!(drops.len(), 1);
        assert_eq!(&*drops[0].item, "minecraft:enchanted_book");
        assert_eq!(
            drops[0].stored_enchantments(),
            [("minecraft:mending".to_owned().into(), 1)],
            "the book keeps its single-level enchantment"
        );
        // 46 is vanilla's count for this seed; an extra level draw gives 28.
        assert_eq!(drops[0].count, 46);
    }

    /// A content root's remainders reach the mods' items; vanilla's stay its own.
    #[test]
    fn mod_items_take_their_crafting_remainders() {
        let root =
            std::env::temp_dir().join(format!("horizon-craft-remainders-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = r#"{"minecraft:bowl": {}, "minecraft:milk_bucket": {}, "ruby:stew": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let catalog = |root: &Path| Items::catalog(root, &PackOverlay::NONE, &defaults).unwrap();
        let remainder = |items: &Items, id: &str| items.items[id].crafting_remainder.clone();
        let vanilla = catalog(&root);
        assert_eq!(remainder(&vanilla, "ruby:stew"), None);
        fs::write(
            root.join(MOD_CRAFT_REMAINDERS),
            r#"{"ruby:stew": "minecraft:bowl"}"#,
        )
        .unwrap();
        let modded = catalog(&root);
        assert_eq!(
            remainder(&modded, "ruby:stew").as_deref(),
            Some("minecraft:bowl")
        );
        assert_eq!(
            remainder(&modded, "minecraft:milk_bucket").as_deref(),
            Some("minecraft:bucket")
        );
        assert_eq!(remainder(&modded, "minecraft:bowl"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    /// A content root's templates make the mods' items what they copy (`ItemInfo::is`).
    #[test]
    fn mod_items_are_their_templates() {
        let root =
            std::env::temp_dir().join(format!("horizon-item-templates-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = r#"{"minecraft:fishing_rod": {}, "minecraft:stick": {}, "ruby:rod": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let catalog = |root: &Path| Items::catalog(root, &PackOverlay::NONE, &defaults).unwrap();
        let vanilla = catalog(&root);
        assert!(!vanilla.items["ruby:rod"].is("minecraft:fishing_rod"));
        fs::write(
            root.join(MOD_ITEM_TEMPLATES),
            r#"{"ruby:rod": "minecraft:fishing_rod"}"#,
        )
        .unwrap();
        let modded = catalog(&root);
        let rod = &modded.items["ruby:rod"];
        assert_eq!(rod.template.as_deref(), Some("minecraft:fishing_rod"));
        assert!(rod.is("minecraft:fishing_rod"));
        assert!(rod.is("ruby:rod"));
        assert!(!rod.is("minecraft:stick"));
        assert!(modded.items["minecraft:fishing_rod"].is("minecraft:fishing_rod"));
        assert!(!modded.items["minecraft:stick"].is("minecraft:fishing_rod"));
        assert_eq!(modded.items["minecraft:fishing_rod"].template, None);
        fs::remove_dir_all(&root).unwrap();
    }
}

/// Minecraft.handleKeybinds: spectators cannot drop from the selected slot.
pub fn take_held_drop(
    slot: &mut Option<ItemStack>,
    all: bool,
    spectator: bool,
) -> Option<ItemStack> {
    if spectator {
        return None;
    }
    let stack = slot.as_mut()?;
    if stack.count == 0 {
        return None;
    }
    let count = if all { stack.count } else { 1 };
    let thrown = ItemStack {
        count,
        ..stack.clone()
    };
    stack.count -= count;
    if stack.count == 0 {
        *slot = None;
    }
    Some(thrown)
}
#[cfg(test)]
#[path = "blocks_attacks_tests.rs"]
mod blocks_attacks_tests;

#[cfg(test)]
mod held_drop_tests {
    use super::*;
    #[test]
    fn one_all_and_empty_drop() {
        let mut slot = Some(ItemStack::new("minecraft:stone", 4));
        assert_eq!(take_held_drop(&mut slot, false, false).unwrap().count, 1);
        assert_eq!(slot.as_ref().unwrap().count, 3);
        assert_eq!(take_held_drop(&mut slot, true, false).unwrap().count, 3);
        assert!(slot.is_none());
        assert!(take_held_drop(&mut slot, false, false).is_none());
    }
    #[test]
    fn spectator_keeps_selected_stack_for_single_and_whole_stack_drops() {
        for all in [false, true] {
            let mut slot = Some(ItemStack::new("minecraft:stone", 4));
            assert!(take_held_drop(&mut slot, all, true).is_none());
            assert_eq!(slot.as_ref().unwrap().count, 4);
            // Leaving spectator allows dropping the same selected stack again.
            assert_eq!(
                take_held_drop(&mut slot, all, false).unwrap().count,
                if all { 4 } else { 1 }
            );
        }
    }
}

#[cfg(test)]
mod villager_food_tests {
    use super::*;

    /// A villager's food is the item's `villager_food` component, read from the registry:
    /// vanilla's four (bread 4, the rest 1) and nothing else.
    #[test]
    fn the_registry_tells_a_villagers_food() {
        let Some(items) = prepared_items() else {
            return;
        };
        let food: Vec<(&str, Option<i32>)> = [
            "minecraft:bread",
            "minecraft:carrot",
            "minecraft:potato",
            "minecraft:beetroot",
            "minecraft:apple",
        ]
        .map(|i| (i, items.get(i).and_then(|i| i.villager_food)))
        .into();
        assert_eq!(
            food,
            [
                ("minecraft:bread", Some(4)),
                ("minecraft:carrot", Some(1)),
                ("minecraft:potato", Some(1)),
                ("minecraft:beetroot", Some(1)),
                ("minecraft:apple", None),
            ]
        );
        // The registry hands its villagers the same table.
        assert_eq!(items.villager_food("minecraft:bread"), Some(4));
        assert_eq!(items.villager_food("minecraft:apple"), Some(0));
    }

    /// A catalog over a content root of its own (`name`) whose bread feeds a villager `bread`
    /// points and whose apple feeds none.
    fn bread_catalog(name: &str, bread: i32) -> Items {
        let root = std::env::temp_dir().join(format!(
            "horizon-villager-food-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = format!(
            r#"{{"minecraft:bread": {{"minecraft:villager_food": {{"nutrition": {bread}}}}}, "minecraft:apple": {{}}}}"#
        );
        fs::write(root.join("items.json"), items).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let catalog = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        fs::remove_dir_all(&root).unwrap();
        catalog
    }

    /// Each registry answers from its own table: catalogs built after it, or while it is read
    /// on another thread, never change its answers (the last catalog built used to replace the
    /// table every villager read).
    #[test]
    fn other_catalogs_never_change_a_registrys_villager_food() {
        let vanilla = bread_catalog("vanilla", 4);
        let starved = bread_catalog("starved", 0);
        let answers = |items: &Items| {
            [
                items.villager_food("minecraft:bread"),
                items.villager_food("minecraft:apple"),
                items.villager_food("minecraft:carrot"),
            ]
        };
        assert_eq!(answers(&vanilla), [Some(4), Some(0), None]);
        assert_eq!(answers(&starved), [Some(0), Some(0), None]);
        std::thread::scope(|s| {
            for i in 0..4 {
                s.spawn(move || bread_catalog(&format!("fixture-{i}"), 9 + i));
            }
            for _ in 0..200 {
                assert_eq!(answers(&vanilla), [Some(4), Some(0), None]);
                assert_eq!(answers(&starved), [Some(0), Some(0), None]);
            }
        });
        assert_eq!(answers(&vanilla), [Some(4), Some(0), None]);
        assert_eq!(answers(&starved), [Some(0), Some(0), None]);
    }
}
