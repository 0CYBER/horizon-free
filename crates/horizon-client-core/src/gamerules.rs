//! Game rules (26.3 `GameRules`): the registered rules with their category,
//! type, default and bounds, dense values, and persistence in the world's
//! `data/minecraft/game_rules.dat` saved-data file.
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};
use {
    horizon_core::{
        nbt,
        nbt::{Limits, NamedTag, Tag},
    },
    horizon_world::{DATA_VERSION, compression::Compression},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i32),
}
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub category: String,
    pub default: Value,
    pub min: i32,
    pub max: i32,
}
impl Rule {
    fn matches(&self, id: &str) -> bool {
        self.name == id || (!id.contains(':') && self.name.strip_prefix("minecraft:") == Some(id))
    }
}
struct VanillaRule {
    name: &'static str,
    category: &'static str,
    default: Value,
    min: i32,
    max: i32,
}
impl From<&VanillaRule> for Rule {
    fn from(rule: &VanillaRule) -> Self {
        Self {
            name: format!("minecraft:{}", rule.name),
            category: format!("minecraft:{}", rule.category),
            default: rule.default,
            min: rule.min,
            max: rule.max,
        }
    }
}
const fn b(name: &'static str, category: &'static str, default: bool) -> VanillaRule {
    VanillaRule {
        name,
        category,
        default: Value::Bool(default),
        min: 0,
        max: 0,
    }
}
const fn i(
    name: &'static str,
    category: &'static str,
    default: i32,
    min: i32,
    max: i32,
) -> VanillaRule {
    VanillaRule {
        name,
        category,
        default: Value::Int(default),
        min,
        max,
    }
}
/// Every rule available without experimental features, in registration order.
const RULES: &[VanillaRule] = &[
    b("advance_time", "updates", true),
    b("advance_weather", "updates", true),
    b("allow_entering_nether_using_portals", "misc", true),
    b("block_drops", "drops", true),
    b("block_explosion_drop_decay", "drops", true),
    b("command_blocks_work", "misc", true),
    b("command_block_output", "chat", true),
    b("drowning_damage", "player", true),
    b("elytra_movement_check", "player", true),
    b("ender_pearls_vanish_on_death", "player", true),
    b("entity_drops", "drops", true),
    b("fall_damage", "player", true),
    b("fire_damage", "player", true),
    i(
        "fire_spread_radius_around_player",
        "updates",
        128,
        -1,
        i32::MAX,
    ),
    b("forgive_dead_players", "mobs", true),
    b("freeze_damage", "player", true),
    b("global_sound_events", "misc", true),
    b("immediate_respawn", "player", false),
    b("keep_inventory", "player", false),
    b("lava_source_conversion", "updates", false),
    b("limited_crafting", "player", false),
    b("locator_bar", "player", true),
    b("log_admin_commands", "chat", true),
    i("max_block_modifications", "misc", 32768, 1, i32::MAX),
    i("max_command_forks", "misc", 65536, 0, i32::MAX),
    i("max_command_sequence_length", "misc", 65536, 0, i32::MAX),
    i("max_entity_cramming", "mobs", 24, 0, i32::MAX),
    i("max_snow_accumulation_height", "updates", 1, 0, 8),
    b("mob_drops", "drops", true),
    b("mob_explosion_drop_decay", "drops", true),
    b("mob_griefing", "mobs", true),
    b("natural_health_regeneration", "player", true),
    b("player_movement_check", "player", true),
    i(
        "players_nether_portal_creative_delay",
        "player",
        0,
        0,
        i32::MAX,
    ),
    i(
        "players_nether_portal_default_delay",
        "player",
        80,
        0,
        i32::MAX,
    ),
    i("players_sleeping_percentage", "player", 100, 0, i32::MAX),
    b("projectiles_can_break_blocks", "drops", true),
    b("pvp", "player", true),
    b("raids", "mobs", true),
    i("random_tick_speed", "updates", 3, 0, i32::MAX),
    b("reduced_debug_info", "misc", false),
    i("respawn_radius", "player", 10, 0, i32::MAX),
    b("send_command_feedback", "chat", true),
    b("show_advancement_messages", "chat", true),
    b("show_death_messages", "chat", true),
    b("spawner_blocks_work", "misc", true),
    b("spawn_mobs", "spawning", true),
    b("spawn_monsters", "spawning", true),
    b("spawn_patrols", "spawning", true),
    b("spawn_phantoms", "spawning", true),
    b("spawn_wandering_traders", "spawning", true),
    b("spawn_wardens", "spawning", true),
    b("spectators_generate_chunks", "player", true),
    b("spread_vines", "updates", true),
    b("tnt_explodes", "misc", true),
    b("tnt_explosion_drop_decay", "drops", false),
    b("universal_anger", "mobs", false),
    b("water_source_conversion", "updates", true),
];
/// GameRules.MAX_MINECART_SPEED (26.3), available with minecart improvements.
const MAX_MINECART_SPEED: VanillaRule = i("max_minecart_speed", "misc", 8, 1, 1000);
pub const MINECART_IMPROVEMENTS: &str = "minecraft:minecart_improvements";

/// Immutable, ordered rule definitions shared by game-rule consumers.
#[derive(Debug, PartialEq)]
pub struct RuleRegistry {
    rules: Vec<Rule>,
    categories: Vec<String>,
}
/// Failure while validating declarations collected from all mods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleRegistryError {
    /// A rule identifier was declared twice.
    Duplicate(String),
    /// A referenced category does not exist.
    UnknownCategory { rule: String, category: String },
    /// An integer rule's default is outside its `min..=max`.
    InvalidRange(String),
}
impl Default for RuleRegistry {
    fn default() -> Self {
        Self::vanilla()
    }
}
impl RuleRegistry {
    /// Create a fresh vanilla-backed registry for a session.
    pub fn vanilla() -> Self {
        Self {
            categories: [
                "chat", "drops", "misc", "mobs", "player", "spawning", "updates",
            ]
            .into_iter()
            .map(|s| format!("minecraft:{s}"))
            .collect(),
            rules: RULES
                .iter()
                .chain(std::iter::once(&MAX_MINECART_SPEED))
                .map(Rule::from)
                .collect(),
        }
    }
    /// Build a fresh session registry from validated category declarations.
    pub fn with_categories(categories: &[String]) -> Self {
        let mut registry = Self::vanilla();
        registry.categories.extend_from_slice(categories);
        registry
    }
    /// The categories in this session.
    pub fn categories(&self) -> &[String] {
        &self.categories
    }
    /// Add validated mod declarations (bool or int) before the registry is shared.
    pub fn with_mod_rules(
        categories: &[String],
        rules: &[Rule],
    ) -> Result<Self, Vec<RuleRegistryError>> {
        let mut registry = Self::with_categories(categories);
        let mut errors = Vec::new();
        for rule in rules {
            if registry.rule(&rule.name).is_some() {
                errors.push(RuleRegistryError::Duplicate(rule.name.clone()));
                continue;
            }
            if let Value::Int(default) = rule.default
                && !(rule.min..=rule.max).contains(&default)
            {
                errors.push(RuleRegistryError::InvalidRange(rule.name.clone()));
                continue;
            }
            if !registry.categories.contains(&rule.category) {
                errors.push(RuleRegistryError::UnknownCategory {
                    rule: rule.name.clone(),
                    category: rule.category.clone(),
                });
                continue;
            }
            registry.rules.push(rule.clone());
        }
        if errors.is_empty() {
            Ok(registry)
        } else {
            Err(errors)
        }
    }
    /// Resolve an identifier once when loading.
    pub fn position(&self, id: &str) -> Option<u32> {
        self.rules
            .iter()
            .position(|rule| rule.name == id)
            .and_then(|slot| u32::try_from(slot).ok())
    }
    pub fn available_rules(&self, minecart_improvements: bool) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .enumerate()
            .filter(move |(index, _)| minecart_improvements || *index != RULES.len())
            .map(|(_, rule)| rule)
    }
    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.available_rules(true).find(|r| r.matches(name))
    }
}
/// The vanilla-backed registry, shared by all existing consumers.
pub fn rule_registry() -> Arc<RuleRegistry> {
    static REGISTRY: OnceLock<Arc<RuleRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(RuleRegistry::vanilla()))
        .clone()
}
/// A vanilla rule's resolved position in the registration order.
#[derive(Clone, Copy, Debug)]
pub struct RuleIndex(usize);

const fn vanilla_index(name: &str) -> RuleIndex {
    let mut i = 0;
    while i < RULES.len() {
        let a = RULES[i].name.as_bytes();
        let b = name.as_bytes();
        let mut n = 0;
        while n < a.len() && n < b.len() && a[n] == b[n] {
            n += 1;
        }
        if n == a.len() && n == b.len() {
            return RuleIndex(i);
        }
        i += 1;
    }
    panic!("unknown vanilla rule")
}

/// Resolved vanilla rules used by the simulation.
pub mod vanilla {
    use super::{RuleIndex, vanilla_index};
    /// The `spawn_wandering_traders` rule.
    pub const SPAWN_WANDERING_TRADERS: RuleIndex = vanilla_index("spawn_wandering_traders");
    /// The `spawn_wardens` rule.
    pub const SPAWN_WARDENS: RuleIndex = vanilla_index("spawn_wardens");
    /// The `spawn_patrols` rule.
    pub const SPAWN_PATROLS: RuleIndex = vanilla_index("spawn_patrols");
    /// The `raids` rule.
    pub const RAIDS: RuleIndex = vanilla_index("raids");
    /// The `entity_drops` rule.
    pub const ENTITY_DROPS: RuleIndex = vanilla_index("entity_drops");
    /// The `spectators_generate_chunks` rule.
    pub const SPECTATORS_GENERATE_CHUNKS: RuleIndex = vanilla_index("spectators_generate_chunks");
    /// The `keep_inventory` rule.
    pub const KEEP_INVENTORY: RuleIndex = vanilla_index("keep_inventory");
    /// The `show_death_messages` rule.
    pub const SHOW_DEATH_MESSAGES: RuleIndex = vanilla_index("show_death_messages");
    /// The `players_sleeping_percentage` rule.
    pub const PLAYERS_SLEEPING_PERCENTAGE: RuleIndex = vanilla_index("players_sleeping_percentage");
    /// The `allow_entering_nether_using_portals` rule.
    pub const ALLOW_ENTERING_NETHER_USING_PORTALS: RuleIndex =
        vanilla_index("allow_entering_nether_using_portals");
    /// The `elytra_movement_check` rule.
    pub const ELYTRA_MOVEMENT_CHECK: RuleIndex = vanilla_index("elytra_movement_check");
    /// The `global_sound_events` rule.
    pub const GLOBAL_SOUND_EVENTS: RuleIndex = vanilla_index("global_sound_events");
    /// The `immediate_respawn` rule.
    pub const IMMEDIATE_RESPAWN: RuleIndex = vanilla_index("immediate_respawn");
    /// The `limited_crafting` rule.
    pub const LIMITED_CRAFTING: RuleIndex = vanilla_index("limited_crafting");
    /// The `locator_bar` rule.
    pub const LOCATOR_BAR: RuleIndex = vanilla_index("locator_bar");
    /// The `player_movement_check` rule.
    pub const PLAYER_MOVEMENT_CHECK: RuleIndex = vanilla_index("player_movement_check");
    /// The `pvp` rule.
    pub const PVP: RuleIndex = vanilla_index("pvp");
    /// The `respawn_radius` rule.
    pub const RESPAWN_RADIUS: RuleIndex = vanilla_index("respawn_radius");
    /// The `advance_weather` rule.
    pub const ADVANCE_WEATHER: RuleIndex = vanilla_index("advance_weather");
    /// The `spawn_phantoms` rule.
    pub const SPAWN_PHANTOMS: RuleIndex = vanilla_index("spawn_phantoms");
    /// The `command_block_output` rule.
    pub const COMMAND_BLOCK_OUTPUT: RuleIndex = vanilla_index("command_block_output");
    /// The `command_blocks_work` rule.
    pub const COMMAND_BLOCKS_WORK: RuleIndex = vanilla_index("command_blocks_work");
    /// The `log_admin_commands` rule.
    pub const LOG_ADMIN_COMMANDS: RuleIndex = vanilla_index("log_admin_commands");
    /// The `max_block_modifications` rule.
    pub const MAX_BLOCK_MODIFICATIONS: RuleIndex = vanilla_index("max_block_modifications");
    /// The `max_command_forks` rule.
    pub const MAX_COMMAND_FORKS: RuleIndex = vanilla_index("max_command_forks");
    /// The `max_command_sequence_length` rule.
    pub const MAX_COMMAND_SEQUENCE_LENGTH: RuleIndex = vanilla_index("max_command_sequence_length");
    /// The `send_command_feedback` rule.
    pub const SEND_COMMAND_FEEDBACK: RuleIndex = vanilla_index("send_command_feedback");
    /// The feature-gated `max_minecart_speed` rule.
    pub const MAX_MINECART_SPEED: RuleIndex = RuleIndex(super::RULES.len());
    /// The `reduced_debug_info` rule.
    pub const REDUCED_DEBUG_INFO: RuleIndex = vanilla_index("reduced_debug_info");
    /// The `show_advancement_messages` rule.
    pub const SHOW_ADVANCEMENT_MESSAGES: RuleIndex = vanilla_index("show_advancement_messages");
    /// The `players_nether_portal_creative_delay` rule.
    pub const PLAYERS_NETHER_PORTAL_CREATIVE_DELAY: RuleIndex =
        vanilla_index("players_nether_portal_creative_delay");
    /// The `players_nether_portal_default_delay` rule.
    pub const PLAYERS_NETHER_PORTAL_DEFAULT_DELAY: RuleIndex =
        vanilla_index("players_nether_portal_default_delay");
    /// The `mob_explosion_drop_decay` rule.
    pub const MOB_EXPLOSION_DROP_DECAY: RuleIndex = vanilla_index("mob_explosion_drop_decay");
    /// The `tnt_explosion_drop_decay` rule.
    pub const TNT_EXPLOSION_DROP_DECAY: RuleIndex = vanilla_index("tnt_explosion_drop_decay");
    /// The `block_explosion_drop_decay` rule.
    pub const BLOCK_EXPLOSION_DROP_DECAY: RuleIndex = vanilla_index("block_explosion_drop_decay");
    /// The `advance_time` rule.
    pub const ADVANCE_TIME: RuleIndex = vanilla_index("advance_time");
    /// The `water_source_conversion` rule.
    pub const WATER_SOURCE_CONVERSION: RuleIndex = vanilla_index("water_source_conversion");
    /// The `lava_source_conversion` rule.
    pub const LAVA_SOURCE_CONVERSION: RuleIndex = vanilla_index("lava_source_conversion");
    /// The `random_tick_speed` rule.
    pub const RANDOM_TICK_SPEED: RuleIndex = vanilla_index("random_tick_speed");
    /// The `block_drops` rule.
    pub const BLOCK_DROPS: RuleIndex = vanilla_index("block_drops");
    /// The `mob_griefing` rule.
    pub const MOB_GRIEFING: RuleIndex = vanilla_index("mob_griefing");
    /// The `max_snow_accumulation_height` rule.
    pub const MAX_SNOW_ACCUMULATION_HEIGHT: RuleIndex =
        vanilla_index("max_snow_accumulation_height");
    /// The `spawn_mobs` rule.
    pub const SPAWN_MOBS: RuleIndex = vanilla_index("spawn_mobs");
    /// The `spawn_monsters` rule.
    pub const SPAWN_MONSTERS: RuleIndex = vanilla_index("spawn_monsters");
    /// The `fire_spread_radius_around_player` rule.
    pub const FIRE_SPREAD_RADIUS_AROUND_PLAYER: RuleIndex =
        vanilla_index("fire_spread_radius_around_player");
    /// The `spread_vines` rule.
    pub const SPREAD_VINES: RuleIndex = vanilla_index("spread_vines");
    /// The `natural_health_regeneration` rule.
    pub const NATURAL_HEALTH_REGENERATION: RuleIndex = vanilla_index("natural_health_regeneration");
    /// The `drowning_damage` rule.
    pub const DROWNING_DAMAGE: RuleIndex = vanilla_index("drowning_damage");
    /// The `fall_damage` rule.
    pub const FALL_DAMAGE: RuleIndex = vanilla_index("fall_damage");
    /// The `fire_damage` rule.
    pub const FIRE_DAMAGE: RuleIndex = vanilla_index("fire_damage");
    /// The `freeze_damage` rule.
    pub const FREEZE_DAMAGE: RuleIndex = vanilla_index("freeze_damage");
    /// The `tnt_explodes` rule.
    pub const TNT_EXPLODES: RuleIndex = vanilla_index("tnt_explodes");
    /// The `spawner_blocks_work` rule.
    pub const SPAWNER_BLOCKS_WORK: RuleIndex = vanilla_index("spawner_blocks_work");
    /// The `universal_anger` rule.
    pub const UNIVERSAL_ANGER: RuleIndex = vanilla_index("universal_anger");
    /// The `mob_drops` rule.
    pub const MOB_DROPS: RuleIndex = vanilla_index("mob_drops");
    /// The `max_entity_cramming` rule.
    pub const MAX_ENTITY_CRAMMING: RuleIndex = vanilla_index("max_entity_cramming");
    /// The `forgive_dead_players` rule.
    pub const FORGIVE_DEAD_PLAYERS: RuleIndex = vanilla_index("forgive_dead_players");
    /// The `ender_pearls_vanish_on_death` rule.
    pub const ENDER_PEARLS_VANISH_ON_DEATH: RuleIndex =
        vanilla_index("ender_pearls_vanish_on_death");
    /// The `projectiles_can_break_blocks` rule.
    pub const PROJECTILES_CAN_BREAK_BLOCKS: RuleIndex =
        vanilla_index("projectiles_can_break_blocks");
}

/// Current values (defaults unless set).
#[derive(Clone, Debug, PartialEq)]
pub struct GameRules {
    registry: Arc<RuleRegistry>,
    values: Vec<Value>,
    // Keep entries that cannot be interpreted by the current registry verbatim.
    // A temporarily absent mod must not lose its saved rule values.
    unknown: nbt::Compound,
    minecart_improvements: bool,
}
impl Default for GameRules {
    fn default() -> Self {
        Self::with_registry(rule_registry(), &[])
    }
}
impl GameRules {
    pub fn with_features(features: &[String]) -> Self {
        Self::with_registry(rule_registry(), features)
    }
    /// Build a rule set from a session's immutable definitions.
    pub fn with_registry(registry: Arc<RuleRegistry>, features: &[String]) -> Self {
        Self {
            values: registry
                .available_rules(true)
                .map(|rule| rule.default)
                .collect(),
            registry,
            unknown: nbt::Compound::new(),
            minecart_improvements: features.iter().any(|f| f == MINECART_IMPROVEMENTS),
        }
    }
    /// Reconcile values with installed definitions, retaining matching selections.
    pub fn rebind_registry(&mut self, registry: Arc<RuleRegistry>) {
        if Arc::ptr_eq(&self.registry, &registry) {
            return;
        }
        let mut next = Self::with_registry(registry, &[]);
        next.minecart_improvements = self.minecart_improvements;
        for rule in self.available_rules() {
            if let Some(value) = self.get(&rule.name) {
                next.set(&rule.name, value);
            }
        }
        self.unknown.clone_into(&mut next.unknown);
        *self = next;
    }
    /// The immutable definitions used by these values.
    pub fn registry(&self) -> &Arc<RuleRegistry> {
        &self.registry
    }
    /// GameRules.copy(enabledFeatures): keep available values, add defaults for
    /// newly enabled rules and discard rules absent from the target feature set.
    pub fn copy_with_features(&self, features: &[String]) -> Self {
        let mut rules = Self::with_registry(self.registry.clone(), features);
        rules.unknown = self.unknown.clone();
        for (rule, &value) in self.registry.available_rules(true).zip(&self.values) {
            rules.set(&rule.name, value);
        }
        rules
    }
    pub fn available_rules(&self) -> impl Iterator<Item = &Rule> {
        self.registry.available_rules(self.minecart_improvements)
    }
    /// Look up a definition in this rule set's registry.
    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.available_rules().find(|rule| rule.matches(name))
    }
    pub fn get(&self, name: &str) -> Option<Value> {
        let index = self
            .registry
            .rules
            .iter()
            .enumerate()
            .find_map(|(index, rule)| {
                ((rule.matches(name)) && (index != RULES.len() || self.minecart_improvements))
                    .then_some(index)
            })?;
        Some(self.values[index])
    }
    /// Read a prevalidated boolean slot.
    pub fn bool_slot(&self, slot: u32) -> Option<bool> {
        match self.values.get(slot as usize)? {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }
    /// Read a prevalidated integer slot.
    pub fn int_slot(&self, slot: u32) -> Option<i32> {
        match self.values.get(slot as usize)? {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }
    /// Read a resolved vanilla boolean rule without a name lookup.
    pub fn bool_at(&self, index: RuleIndex) -> bool {
        matches!(self.values[index.0], Value::Bool(true))
    }
    /// Read a resolved vanilla integer rule without a name lookup.
    pub fn int_at(&self, index: RuleIndex) -> i32 {
        match self.values.get(index.0) {
            Some(Value::Int(value)) => *value,
            _ => 0,
        }
    }
    /// Parse a network rule value, retaining type and range validation.
    pub fn set_string(&mut self, name: &str, text: &str) -> bool {
        let value = match self.get(name) {
            Some(Value::Bool(_)) => text.parse::<bool>().ok().map(Value::Bool),
            Some(Value::Int(_)) => text.parse::<i32>().ok().map(Value::Int),
            None => None,
        };
        value.is_some_and(|v| self.set(name, v))
    }
    /// Set a rule; false if the name or value type is wrong.
    pub fn set(&mut self, name: &str, value: Value) -> bool {
        let Some((index, r)) = self.registry.rules.iter().enumerate().find(|(index, r)| {
            (*index != RULES.len() || self.minecart_improvements) && (r.matches(name))
        }) else {
            return false;
        };
        let ok = match (r.default, value) {
            (Value::Bool(_), Value::Bool(_)) => true,
            (Value::Int(_), Value::Int(v)) => (r.min..=r.max).contains(&v),
            _ => false,
        };
        if ok {
            self.values[index] = value;
        }
        ok
    }
    /// Every rule's value by name without namespace, as `save::Level` holds
    /// them (booleans as 0 or 1).
    pub fn by_name(&self) -> BTreeMap<String, i32> {
        self.registry
            .rules
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != RULES.len() || self.minecart_improvements)
            .map(|(index, r)| {
                let v = match self.values[index] {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                };
                (r.name.to_owned(), v)
            })
            .collect()
    }
    /// GameRuleMap codec: `{data: {"minecraft:<rule>": value...}, DataVersion}`.
    pub fn to_nbt(&self) -> NamedTag {
        let mut data = self.unknown.clone();
        data.extend(
            self.registry
                .rules
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != RULES.len() || self.minecart_improvements)
                .map(|(index, r)| {
                    let v = self.values[index];
                    let tag = match v {
                        Value::Bool(b) => Tag::Byte(i8::from(b)),
                        Value::Int(i) => Tag::Int(i),
                    };
                    (horizon_core::nbt::JavaString::from(r.name.as_str()), tag)
                }),
        );
        NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("data", Tag::Compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        }
    }
    pub fn from_nbt(root: &Tag) -> Self {
        Self::from_nbt_with_features(root, &[])
    }
    pub fn from_nbt_with_features(root: &Tag, features: &[String]) -> Self {
        Self::from_nbt_with_registry(root, features, rule_registry())
    }
    /// Decode saved values against session definitions.
    pub fn from_nbt_with_registry(
        root: &Tag,
        features: &[String],
        registry: Arc<RuleRegistry>,
    ) -> Self {
        let mut rules = Self::with_registry(registry, features);
        let Some(Tag::Compound(data)) = root.get("data") else {
            return rules;
        };
        rules.unknown = data.clone();
        let registry = rules.registry.clone();
        for r in registry.available_rules(rules.minecart_improvements) {
            let key = &r.name;
            let value = data.get(&key.as_str().into()).or_else(|| {
                r.name
                    .strip_prefix("minecraft:")
                    .and_then(|bare| data.get(&bare.into()))
            });
            match (r.default, value) {
                (Value::Bool(_), Some(Tag::Byte(v))) => {
                    rules.set(&r.name, Value::Bool(*v != 0));
                }
                (Value::Int(_), Some(Tag::Int(v))) => {
                    rules.set(&r.name, Value::Int(*v));
                }
                _ => {}
            }
            rules.unknown.remove(&key.as_str().into());
            if let Some(bare) = r.name.strip_prefix("minecraft:") {
                rules.unknown.remove(&bare.into());
            }
        }
        rules
    }
    pub fn load(world: &Path) -> Self {
        Self::load_with_features(world, &[])
    }
    pub fn load_with_features(world: &Path, features: &[String]) -> Self {
        Self::load_with_registry(world, features, rule_registry())
    }
    /// Load saved values with the session registry.
    pub fn load_with_registry(
        world: &Path,
        features: &[String],
        registry: Arc<RuleRegistry>,
    ) -> Self {
        let Ok(bytes) = fs::read(world.join("data/minecraft/game_rules.dat")) else {
            return Self::with_registry(registry, features);
        };
        let Ok(raw) = Compression::Gzip.decode(&bytes, 16 << 20) else {
            return Self::with_registry(registry, features);
        };
        match nbt::read_named(&raw, Limits::default()) {
            Ok((root, _)) => Self::from_nbt_with_registry(&root.tag, features, registry),
            Err(_) => Self::with_registry(registry, features),
        }
    }
    pub fn save(&self, world: &Path) -> crate::Result<()> {
        let dir = world.join("data/minecraft");
        fs::create_dir_all(&dir)?;
        let bytes =
            nbt::write_named(&self.to_nbt(), Limits::default()).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("game_rules.dat"),
            Compression::Gzip.encode(&bytes)?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_rule(default: i32) -> Rule {
        Rule {
            name: "example:cap".into(),
            category: "example:machines".into(),
            default: Value::Int(default),
            min: 0,
            max: 64,
        }
    }

    #[test]
    fn mod_int_rules_keep_their_range_through_set_and_save() {
        let categories = ["example:machines".to_owned()];
        assert_eq!(
            RuleRegistry::with_mod_rules(&categories, &[int_rule(70)]).err(),
            Some(vec![RuleRegistryError::InvalidRange("example:cap".into())])
        );
        let registry = Arc::new(RuleRegistry::with_mod_rules(&categories, &[int_rule(8)]).unwrap());
        let slot = registry.position("example:cap").unwrap();
        let mut rules = GameRules::with_registry(registry.clone(), &[]);
        assert_eq!(rules.int_slot(slot), Some(8));
        assert_eq!(rules.bool_slot(slot), None);
        assert!(!rules.set_string("example:cap", "70"));
        assert!(!rules.set_string("example:cap", "true"));
        assert!(rules.set_string("example:cap", "12"));
        let saved = rules.to_nbt().tag;
        let loaded = GameRules::from_nbt_with_registry(&saved, &[], registry.clone());
        assert_eq!(loaded.int_slot(slot), Some(12));
        let out_of_range =
            Tag::compound([("data", Tag::compound([("example:cap", Tag::Int(70))]))]);
        let loaded = GameRules::from_nbt_with_registry(&out_of_range, &[], registry);
        assert_eq!(loaded.int_slot(slot), Some(8));
    }

    #[test]
    fn unknown_rules_survive_save_and_feature_copy() {
        let source = Tag::compound([(
            "data",
            Tag::compound([
                ("missing:example", Tag::Long(123)),
                ("minecraft:keep_inventory", Tag::Byte(1)),
            ]),
        )]);
        let rules = GameRules::from_nbt(&source).copy_with_features(&[]);
        assert_eq!(rules.get("keep_inventory"), Some(Value::Bool(true)));
        assert_eq!(
            rules
                .to_nbt()
                .tag
                .get("data")
                .unwrap()
                .get("missing:example"),
            Some(&Tag::Long(123))
        );
    }

    #[test]
    fn vanilla_registry_is_shared_with_rule_consumers() {
        let registry = rule_registry();
        let rules = GameRules::default();
        assert!(Arc::ptr_eq(&registry, &rules.registry));
        assert!(std::ptr::eq(
            registry.rule("minecraft:keep_inventory").unwrap(),
            rules.rule("keep_inventory").unwrap()
        ));
        assert_eq!(
            registry.available_rules(false).count() + 1,
            registry.available_rules(true).count()
        );
    }

    #[test]
    fn session_registry_survives_load_and_feature_copy() {
        let first = Arc::new(RuleRegistry::vanilla());
        let second = Arc::new(RuleRegistry::vanilla());
        let rules = GameRules::with_registry(first.clone(), &[]);
        let loaded = GameRules::from_nbt_with_registry(&rules.to_nbt().tag, &[], first.clone());
        let copied = loaded.copy_with_features(&[MINECART_IMPROVEMENTS.to_owned()]);
        assert!(Arc::ptr_eq(&copied.registry, &first));
        assert!(!Arc::ptr_eq(&copied.registry, &second));
    }

    #[test]
    fn disabled_feature_does_not_shift_later_dense_positions() {
        let mut registry = RuleRegistry::vanilla();
        registry.rules.push(Rule {
            name: "example:later".into(),
            category: "minecraft:misc".into(),
            default: Value::Int(4),
            min: 0,
            max: 9,
        });
        let registry = Arc::new(registry);
        let mut rules = GameRules::with_registry(registry, &[]);
        assert_eq!(rules.get("example:later"), Some(Value::Int(4)));
        assert!(rules.set("example:later", Value::Int(7)));
        assert_eq!(rules.get("example:later"), Some(Value::Int(7)));
        assert_eq!(rules.get("max_minecart_speed"), None);
        let enabled = rules.copy_with_features(&[MINECART_IMPROVEMENTS.to_owned()]);
        assert_eq!(enabled.get("example:later"), Some(Value::Int(7)));
        assert_eq!(enabled.int_at(vanilla::MAX_MINECART_SPEED), 8);
    }

    #[test]
    fn resolved_reads_follow_mutations() {
        let mut rules = GameRules::default();
        assert!(rules.bool_at(vanilla::SPAWN_MOBS));
        assert_eq!(rules.int_at(vanilla::RANDOM_TICK_SPEED), 3);
        assert!(rules.set("spawn_mobs", Value::Bool(false)));
        assert!(rules.set("random_tick_speed", Value::Int(12)));
        assert!(!rules.bool_at(vanilla::SPAWN_MOBS));
        assert_eq!(rules.int_at(vanilla::RANDOM_TICK_SPEED), 12);
    }

    #[test]
    fn network_values_validate_rule_names_types_and_bounds() {
        let mut rules = GameRules::default();
        assert!(rules.set_string("minecraft:keep_inventory", "true"));
        assert!(rules.bool_at(vanilla::KEEP_INVENTORY));
        assert!(!rules.set_string("minecraft:keep_inventory", "1"));
        assert!(rules.bool_at(vanilla::KEEP_INVENTORY));
        assert!(rules.set_string("minecraft:random_tick_speed", "7"));
        assert_eq!(rules.int_at(vanilla::RANDOM_TICK_SPEED), 7);
        assert!(!rules.set_string("minecraft:random_tick_speed", "-1"));
        assert!(!rules.set_string("minecraft:random_tick_speed", "999999999999"));
        assert!(!rules.set_string("minecraft:unknown", "true"));
        assert!(!rules.set_string("other:keep_inventory", "false"));
        assert_eq!(rules.int_at(vanilla::RANDOM_TICK_SPEED), 7);
    }
    #[test]
    fn defaults_bounds_and_round_trip() {
        let mut rules = GameRules::default();
        assert!(rules.bool_at(vanilla::ADVANCE_TIME));
        assert_eq!(rules.int_at(vanilla::RANDOM_TICK_SPEED), 3);
        assert!(!rules.set("random_tick_speed", Value::Bool(true)));
        assert!(!rules.set("max_snow_accumulation_height", Value::Int(9)));
        assert!(rules.set("keep_inventory", Value::Bool(true)));
        assert!(rules.set("minecraft:random_tick_speed", Value::Int(10)));
        let back = GameRules::from_nbt(&rules.to_nbt().tag);
        assert_eq!(back, rules);
        assert!(back.bool_at(vanilla::KEEP_INVENTORY));
        let dir = std::env::temp_dir().join(format!("horizon-gamerules-{}", std::process::id()));
        rules.save(&dir).unwrap();
        assert_eq!(GameRules::load(&dir), rules);
        let _ = fs::remove_dir_all(dir);
    }
}
