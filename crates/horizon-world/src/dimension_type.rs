//! The parts of vanilla's `DimensionType` Horizon reads, and the attributes keyed on the level today.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use horizon_core::level_key::LevelKey;
use serde_json::Value;

use crate::worldgen::overrides::LevelHeight;
use crate::worldgen::value_provider::IntProvider;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skybox {
    None,
    Overworld,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardinalLight {
    Default,
    Nether,
}

/// `BedRule.Rule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BedRuleTest {
    Always,
    WhenDark,
    Never,
}

impl BedRuleTest {
    /// `BedRule.Rule.test`.
    pub fn test(self, dark_outside: bool) -> bool {
        match self {
            Self::Always => true,
            Self::WhenDark => dark_outside,
            Self::Never => false,
        }
    }

    /// The name a dimension type's JSON gives it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::WhenDark => "when_dark",
            Self::Never => "never",
        }
    }

    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "always" => Ok(Self::Always),
            "when_dark" => Ok(Self::WhenDark),
            "never" => Ok(Self::Never),
            _ => Err(format!("unknown bed rule {name:?}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BedRule {
    pub can_sleep: BedRuleTest,
    pub can_set_spawn: BedRuleTest,
    pub destroy_on_use: bool,
    pub destroy_on_leave: bool,
}

impl BedRule {
    pub const CAN_SLEEP_WHEN_DARK: Self = Self {
        can_sleep: BedRuleTest::WhenDark,
        can_set_spawn: BedRuleTest::Always,
        destroy_on_use: false,
        destroy_on_leave: false,
    };
    pub const DESTROY_ON_USE: Self = Self {
        can_sleep: BedRuleTest::Never,
        can_set_spawn: BedRuleTest::Never,
        destroy_on_use: true,
        destroy_on_leave: false,
    };
    pub const DESTROY_ON_LEAVE: Self = Self {
        can_sleep: BedRuleTest::WhenDark,
        can_set_spawn: BedRuleTest::Never,
        destroy_on_use: false,
        destroy_on_leave: true,
    };

    fn from_json(v: &Value) -> Result<Self, String> {
        let rule = |k: &str| {
            BedRuleTest::parse(v[k].as_str().ok_or_else(|| format!("bed rule needs {k}"))?)
        };
        Ok(Self {
            can_sleep: rule("can_sleep")?,
            can_set_spawn: rule("can_set_spawn")?,
            destroy_on_use: v["destroy_on_use"].as_bool().unwrap_or(false),
            destroy_on_leave: v["destroy_on_leave"].as_bool().unwrap_or(false),
        })
    }
}

/// A dimension type, with the gameplay attributes `EnvironmentAttributes` defaults fill in.
#[derive(Clone, Debug)]
pub struct DimensionFacts {
    pub id: String,
    pub has_fixed_time: bool,
    pub has_ender_dragon_fight: bool,
    pub coordinate_scale: f64,
    pub height: LevelHeight,
    pub infiniburn: String,
    pub ambient_light: f32,
    pub monster_spawn_light_level: IntProvider,
    pub monster_spawn_block_light_limit: i32,
    pub skybox: Skybox,
    pub cardinal_light: CardinalLight,
    pub default_clock: Option<String>,
    pub timelines: Vec<String>,
    pub water_evaporates: bool,
    pub fast_lava: bool,
    pub respawn_anchor_works: bool,
    pub nether_portal_spawns_piglin: bool,
    pub piglins_zombify: bool,
    pub snow_golem_melts: bool,
    pub can_start_raid: bool,
    pub sky_light_level: f32,
    /// `EnvironmentAttributes.CLOUD_HEIGHT`: where the dimension's clouds start
    /// (its 192.33 default where the dimension type doesn't set one).
    pub cloud_height: f32,
    /// `EnvironmentAttributes.CLOUD_COLOR`'s alpha: 0 where the dimension has
    /// no clouds at all (`Entity.isInClouds`), 0.8 in the overworld.
    pub cloud_color_alpha: f32,
    pub bed_rule: BedRule,
    pub straw_bed_rule: BedRule,
}

const VANILLA_TYPES: [&str; 4] = [
    "minecraft:overworld",
    "minecraft:overworld_caves",
    "minecraft:the_nether",
    "minecraft:the_end",
];

static VANILLA: LazyLock<[DimensionFacts; 4]> = LazyLock::new(|| {
    let overworld = |id: &str, has_ceiling: bool| DimensionFacts {
        id: id.to_owned(),
        has_fixed_time: false,
        has_ender_dragon_fight: false,
        coordinate_scale: 1.0,
        height: LevelHeight {
            min_y: -64,
            height: 384,
            logical_height: 384,
            has_skylight: true,
            has_ceiling,
        },
        infiniburn: "#minecraft:infiniburn_overworld".to_owned(),
        ambient_light: 0.0,
        monster_spawn_light_level: IntProvider::Uniform { min: 0, max: 7 },
        monster_spawn_block_light_limit: 0,
        skybox: Skybox::Overworld,
        cardinal_light: CardinalLight::Default,
        default_clock: Some("minecraft:overworld".to_owned()),
        timelines: [
            "minecraft:villager_schedule",
            "minecraft:day",
            "minecraft:moon",
            "minecraft:early_game",
        ]
        .map(str::to_owned)
        .to_vec(),
        water_evaporates: false,
        fast_lava: false,
        respawn_anchor_works: false,
        nether_portal_spawns_piglin: true,
        piglins_zombify: true,
        snow_golem_melts: false,
        can_start_raid: true,
        sky_light_level: 15.0,
        // `ARGB.vector4fFromARGB32(ARGB.white(0.8F))` and 192.33.
        cloud_height: 192.33,
        cloud_color_alpha: 0.8,
        bed_rule: BedRule::CAN_SLEEP_WHEN_DARK,
        straw_bed_rule: BedRule::DESTROY_ON_LEAVE,
    };
    let overworld_type = overworld("minecraft:overworld", false);
    let caves = overworld("minecraft:overworld_caves", true);
    let nether = DimensionFacts {
        id: "minecraft:the_nether".to_owned(),
        has_fixed_time: true,
        coordinate_scale: 8.0,
        height: LevelHeight {
            min_y: 0,
            height: 256,
            logical_height: 128,
            has_skylight: false,
            has_ceiling: true,
        },
        infiniburn: "#minecraft:infiniburn_nether".to_owned(),
        ambient_light: 0.1,
        monster_spawn_light_level: IntProvider::Constant(7),
        monster_spawn_block_light_limit: 15,
        skybox: Skybox::None,
        cardinal_light: CardinalLight::Nether,
        default_clock: None,
        timelines: vec!["minecraft:villager_schedule".to_owned()],
        water_evaporates: true,
        fast_lava: true,
        respawn_anchor_works: true,
        nether_portal_spawns_piglin: false,
        piglins_zombify: false,
        snow_golem_melts: true,
        can_start_raid: false,
        sky_light_level: 4.0,
        // No cloud colour: no clouds to stand in, though 192.33 stays the height.
        cloud_color_alpha: 0.0,
        bed_rule: BedRule::DESTROY_ON_USE,
        straw_bed_rule: BedRule::DESTROY_ON_USE,
        ..overworld_type.clone()
    };
    let end = DimensionFacts {
        id: "minecraft:the_end".to_owned(),
        has_fixed_time: true,
        has_ender_dragon_fight: true,
        height: LevelHeight {
            min_y: 0,
            height: 256,
            logical_height: 256,
            has_skylight: true,
            has_ceiling: false,
        },
        infiniburn: "#minecraft:infiniburn_end".to_owned(),
        ambient_light: 0.25,
        monster_spawn_light_level: IntProvider::Constant(15),
        skybox: Skybox::End,
        default_clock: Some("minecraft:the_end".to_owned()),
        timelines: vec!["minecraft:villager_schedule".to_owned()],
        nether_portal_spawns_piglin: false,
        cloud_color_alpha: 0.0,
        bed_rule: BedRule::DESTROY_ON_USE,
        straw_bed_rule: BedRule::DESTROY_ON_USE,
        ..overworld_type.clone()
    };
    [overworld_type, caves, nether, end]
});

/// The levels whose dimension type a data pack or a mod changed or added, by level key: set at each
/// world load, so a pack-less world reads none.
static LEVELS: LazyLock<RwLock<HashMap<LevelKey, &'static DimensionFacts>>> =
    LazyLock::new(Default::default);

impl DimensionFacts {
    /// A level's dimension type: the one a data pack or a mod gave it (`set_levels`), else a
    /// built-in level's own, else the Overworld's.
    pub fn of_level(level: LevelKey) -> &'static Self {
        let levels = LEVELS.read().unwrap_or_else(|e| e.into_inner());
        levels
            .get(&level)
            .copied()
            .unwrap_or_else(|| Self::of_builtin_level(level))
    }

    /// The vanilla type `id` names, or the Overworld's for any other id: what a pack's type for `id`
    /// changes.
    pub fn base_for_type(id: &str) -> &'static Self {
        Self::vanilla(id).unwrap_or_else(Self::overworld)
    }

    /// Adds the dimension type of one level opened while the world runs (`set_levels` made the
    /// table at its load). The caller leaks `facts` once and hands the same copy to every open of
    /// the level, so opening and closing it again and again keeps one copy.
    pub fn add_level(level: LevelKey, facts: &'static Self) {
        LEVELS
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(level, facts);
    }

    /// Drops the type of a level opened while the world runs, when it closes.
    pub fn remove_level(level: LevelKey) {
        LEVELS
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&level);
    }

    /// Replaces the levels' own dimension types, at a world's load. The table is leaked, as
    /// `LevelKey`s are: one small entry per level and world load.
    pub fn set_levels(levels: Vec<(LevelKey, Self)>) {
        let table = levels
            .into_iter()
            .map(|(key, facts)| (key, &*Box::leak(Box::new(facts))))
            .collect();
        *LEVELS.write().unwrap_or_else(|e| e.into_inner()) = table;
    }

    /// A vanilla dimension type by id.
    pub fn vanilla(type_id: &str) -> Option<&'static Self> {
        VANILLA_TYPES
            .iter()
            .position(|id| *id == type_id)
            .map(|i| &VANILLA[i])
    }

    /// The type of a built-in level; any other key gets the Overworld's.
    pub fn of_builtin_level(level: LevelKey) -> &'static Self {
        match level {
            LevelKey::NETHER => &VANILLA[2],
            LevelKey::END => &VANILLA[3],
            _ => Self::overworld(),
        }
    }

    pub fn overworld() -> &'static Self {
        &VANILLA[0]
    }

    /// Reads a dimension type's JSON; `timeline_tag` expands a timeline tag id to its timelines.
    pub fn from_json(
        id: &str,
        json: &Value,
        timeline_tag: &dyn Fn(&str) -> Vec<String>,
    ) -> Result<Self, String> {
        let height = LevelHeight::of_json(json).ok_or("invalid level height")?;
        let flag = |k: &str, default: bool| json[k].as_bool().unwrap_or(default);
        let name = |k: &str| json[k].as_str();
        let number = |k: &str| json[k].as_f64().ok_or_else(|| format!("{id} needs {k}"));
        let attributes = &json["attributes"];
        let attribute = |k: &str| &attributes[format!("minecraft:gameplay/{k}")];
        let bool_attr = |k: &str, default: bool| attribute(k).as_bool().unwrap_or(default);
        // `EnvironmentAttributes.CLOUD_HEIGHT` and `CLOUD_COLOR`, the two visual
        // attributes `Entity.isInClouds` reads.
        let visual = |k: &str| &attributes[format!("minecraft:visual/{k}")];
        let cloud_height = visual("cloud_height").as_f64().unwrap_or(192.33) as f32;
        let cloud_color_alpha = match visual("cloud_color") {
            Value::Null => 0.0,
            v => {
                let attribute = crate::environment_attributes::attribute("visual/cloud_color")
                    .ok_or("visual/cloud_color is not registered")?;
                match crate::environment_attributes::Entry::parse(attribute, v)?
                    .apply(&crate::environment_attributes::Value::Argb([0.0; 4]))
                {
                    crate::environment_attributes::Value::Argb(c) => c[3],
                    _ => 0.0,
                }
            }
        };
        let bed = |k: &str, default: BedRule| match attribute(k) {
            Value::Null => Ok(default),
            v => BedRule::from_json(v),
        };
        let timelines = match &json["timelines"] {
            Value::Null => Vec::new(),
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => timeline_tag(tag),
                None => vec![s.clone()],
            },
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => return Err(format!("{id} has invalid timelines")),
        };
        let default_clock = match &json["default_clock"] {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.get("id").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        Ok(Self {
            id: id.to_owned(),
            has_fixed_time: flag("has_fixed_time", false),
            has_ender_dragon_fight: flag("has_ender_dragon_fight", false),
            coordinate_scale: Some(number("coordinate_scale")?)
                .filter(|scale| Self::valid_coordinate_scale(*scale))
                .ok_or_else(|| format!("{id} coordinate_scale is outside 1.0E-5 to 3.0E7"))?,
            height,
            infiniburn: name("infiniburn")
                .ok_or_else(|| format!("{id} needs infiniburn"))?
                .to_owned(),
            ambient_light: number("ambient_light")? as f32,
            monster_spawn_light_level: IntProvider::from_json(
                &json["monster_spawn_light_level"].to_string(),
            )
            .map_err(|e| format!("{id} monster_spawn_light_level: {e:?}"))?,
            monster_spawn_block_light_limit: json["monster_spawn_block_light_limit"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| format!("{id} needs monster_spawn_block_light_limit"))?,
            skybox: match name("skybox").unwrap_or("overworld") {
                "none" => Skybox::None,
                "overworld" => Skybox::Overworld,
                "end" => Skybox::End,
                other => return Err(format!("unknown skybox {other:?}")),
            },
            cardinal_light: match name("cardinal_light").unwrap_or("default") {
                "default" => CardinalLight::Default,
                "nether" => CardinalLight::Nether,
                other => return Err(format!("unknown cardinal light {other:?}")),
            },
            default_clock,
            timelines,
            water_evaporates: bool_attr("water_evaporates", false),
            fast_lava: bool_attr("fast_lava", false),
            respawn_anchor_works: bool_attr("respawn_anchor_works", false),
            nether_portal_spawns_piglin: bool_attr("nether_portal_spawns_piglin", false),
            piglins_zombify: bool_attr("piglins_zombify", true),
            snow_golem_melts: bool_attr("snow_golem_melts", false),
            can_start_raid: bool_attr("can_start_raid", true),
            sky_light_level: attribute("sky_light_level").as_f64().unwrap_or(15.0) as f32,
            cloud_height,
            cloud_color_alpha,
            bed_rule: bed("bed_rule", BedRule::CAN_SLEEP_WHEN_DARK)?,
            straw_bed_rule: bed("straw_bed_rule", BedRule::DESTROY_ON_LEAVE)?,
        })
    }

    /// `base` with the fields `json` names over it: a mod's dimension type names only what it
    /// changes, and a data pack's names them all. Timelines and the clock change when `json` names
    /// `timelines` as a vanilla tag (`#minecraft:in_overworld`, `in_nether` or `in_end`), a timeline
    /// id or a list of ids (any other tag stays `base`'s), and a field the codec refuses is an
    /// error.
    pub fn patched(base: &Self, id: &str, json: &Value) -> Result<Self, String> {
        let mut out = base.clone();
        out.id = id.to_owned();
        out.height = LevelHeight::of_json(json).ok_or("invalid level height")?;
        let flag = |k: &str| json[k].as_bool();
        let attributes = &json["attributes"];
        let attribute = |k: &str| &attributes[format!("minecraft:gameplay/{k}")];
        let bool_attr = |k: &str| attribute(k).as_bool();
        if let Some(v) = flag("has_fixed_time") {
            out.has_fixed_time = v;
        }
        if let Some(v) = flag("has_ender_dragon_fight") {
            out.has_ender_dragon_fight = v;
        }
        if let Some(v) = json["coordinate_scale"].as_f64() {
            if !Self::valid_coordinate_scale(v) {
                return Err(format!("{id} coordinate_scale is outside 1.0E-5 to 3.0E7"));
            }
            out.coordinate_scale = v;
        }
        if let Some(v) = json["infiniburn"].as_str() {
            out.infiniburn = v.to_owned();
        }
        if let Some(v) = json["ambient_light"].as_f64() {
            out.ambient_light = v as f32;
        }
        if !json["monster_spawn_light_level"].is_null() {
            out.monster_spawn_light_level =
                IntProvider::from_json(&json["monster_spawn_light_level"].to_string())
                    .map_err(|e| format!("{id} monster_spawn_light_level: {e:?}"))?;
        }
        if let Some(v) = json["monster_spawn_block_light_limit"]
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
        {
            out.monster_spawn_block_light_limit = v;
        }
        if let Some(name) = json["skybox"].as_str() {
            out.skybox = match name {
                "none" => Skybox::None,
                "overworld" => Skybox::Overworld,
                "end" => Skybox::End,
                other => return Err(format!("unknown skybox {other:?}")),
            };
        }
        if let Some(name) = json["cardinal_light"].as_str() {
            out.cardinal_light = match name {
                "default" => CardinalLight::Default,
                "nether" => CardinalLight::Nether,
                other => return Err(format!("unknown cardinal light {other:?}")),
            };
        }
        let timelines = match &json["timelines"] {
            Value::String(tag) => match tag.as_str() {
                "#minecraft:in_overworld" => Some(VANILLA[0].timelines.clone()),
                "#minecraft:in_nether" => Some(VANILLA[2].timelines.clone()),
                "#minecraft:in_end" => Some(VANILLA[3].timelines.clone()),
                tag if tag.starts_with('#') => None,
                id => Some(vec![id.to_owned()]),
            },
            Value::Array(ids) => Some(
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            ),
            _ => None,
        };
        if let Some(timelines) = timelines {
            out.timelines = timelines;
            // A type with its own timelines has the clock it names, or none.
            out.default_clock = match &json["default_clock"] {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o.get("id").and_then(Value::as_str).map(str::to_owned),
                _ => None,
            };
        }
        for (key, field) in [
            ("water_evaporates", &mut out.water_evaporates),
            ("fast_lava", &mut out.fast_lava),
            ("respawn_anchor_works", &mut out.respawn_anchor_works),
            (
                "nether_portal_spawns_piglin",
                &mut out.nether_portal_spawns_piglin,
            ),
            ("piglins_zombify", &mut out.piglins_zombify),
            ("snow_golem_melts", &mut out.snow_golem_melts),
            ("can_start_raid", &mut out.can_start_raid),
        ] {
            if let Some(v) = bool_attr(key) {
                *field = v;
            }
        }
        if let Some(v) = attribute("sky_light_level").as_f64() {
            out.sky_light_level = v as f32;
        }
        for (key, rule) in [
            ("bed_rule", &mut out.bed_rule),
            ("straw_bed_rule", &mut out.straw_bed_rule),
        ] {
            if !attribute(key).is_null() {
                *rule = BedRule::from_json(attribute(key))?;
            }
        }
        Ok(out)
    }

    pub fn has_timeline(&self, id: &str) -> bool {
        self.timelines.iter().any(|t| t == id)
    }

    /// Whether `scale` is a coordinate scale the codec takes: `Codec.doubleRange(1.0E-5F, 3.0E7)`
    /// (`DimensionType.java:89`, the float widened). Inside it the ratio of two scales is finite.
    pub fn valid_coordinate_scale(scale: f64) -> bool {
        (f64::from(1.0e-5_f32)..=3.0e7).contains(&scale)
    }

    /// `DimensionType.getTeleportationScale`.
    pub fn teleportation_scale(from: &Self, to: &Self) -> f64 {
        from.coordinate_scale / to.coordinate_scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn resources() -> Option<PathBuf> {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        root.join("data/minecraft/dimension_type")
            .is_dir()
            .then_some(root)
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("file")).expect("json")
    }

    fn bounds(p: &IntProvider) -> (i32, i32) {
        match p {
            IntProvider::Constant(n) => (*n, *n),
            IntProvider::Uniform { min, max } => (*min, *max),
            other => panic!("unexpected provider {other:?}"),
        }
    }

    #[test]
    fn vanilla_facts_match_the_prepared_files() {
        let Some(root) = resources() else {
            eprintln!("skipping: no prepared data");
            return;
        };
        let tags = root.join("data/minecraft/tags/timeline");
        fn expand(dir: &Path, tag: &str, out: &mut Vec<String>) {
            let name = tag.strip_prefix("minecraft:").unwrap_or(tag);
            let json = read(&dir.join(format!("{name}.json")));
            for value in json["values"].as_array().expect("values") {
                let value = value.as_str().expect("id");
                match value.strip_prefix('#') {
                    Some(inner) => expand(dir, inner, out),
                    None => out.push(value.to_owned()),
                }
            }
        }
        let timeline_tag = |tag: &str| {
            let mut out = Vec::new();
            expand(&tags, tag, &mut out);
            out
        };
        for id in VANILLA_TYPES {
            let file = id.strip_prefix("minecraft:").unwrap();
            let json = read(&root.join(format!("data/minecraft/dimension_type/{file}.json")));
            let parsed = DimensionFacts::from_json(id, &json, &timeline_tag).expect(id);
            let built = DimensionFacts::vanilla(id).expect(id);
            let (mut a, mut b) = (parsed.clone(), built.clone());
            assert_eq!(
                bounds(&a.monster_spawn_light_level),
                bounds(&b.monster_spawn_light_level),
                "{id}"
            );
            a.monster_spawn_light_level = IntProvider::Constant(0);
            b.monster_spawn_light_level = IntProvider::Constant(0);
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "{id}");
        }
    }

    #[test]
    fn built_in_levels_use_their_own_type() {
        let ty = |k| DimensionFacts::of_builtin_level(k).id.as_str();
        assert_eq!(ty(LevelKey::OVERWORLD), "minecraft:overworld");
        assert_eq!(ty(LevelKey::NETHER), "minecraft:the_nether");
        assert_eq!(ty(LevelKey::END), "minecraft:the_end");
        assert_eq!(
            ty(LevelKey::parse("example:sky").unwrap()),
            "minecraft:overworld"
        );
        assert!(DimensionFacts::vanilla("example:sky").is_none());
    }

    #[test]
    fn portals_scale_by_the_coordinate_scales() {
        let (over, nether) = (
            DimensionFacts::of_builtin_level(LevelKey::OVERWORLD),
            DimensionFacts::of_builtin_level(LevelKey::NETHER),
        );
        assert_eq!(DimensionFacts::teleportation_scale(nether, over), 8.0);
        assert_eq!(DimensionFacts::teleportation_scale(over, nether), 0.125);
    }

    #[test]
    fn bed_rules_test_the_light() {
        assert!(BedRuleTest::WhenDark.test(true));
        assert!(!BedRuleTest::WhenDark.test(false));
        assert!(BedRuleTest::Always.test(false));
        assert!(!BedRuleTest::Never.test(true));
    }

    #[test]
    fn timelines_come_from_the_type() {
        assert!(DimensionFacts::overworld().has_timeline("minecraft:day"));
        let end = DimensionFacts::of_builtin_level(LevelKey::END);
        assert!(!end.has_timeline("minecraft:day"));
        assert!(end.has_timeline("minecraft:villager_schedule"));
    }
}

#[cfg(test)]
mod level_type_tests {
    use super::*;

    #[test]
    fn a_partial_type_changes_only_the_fields_it_names() {
        let json = serde_json::json!({
            "min_y": -32, "height": 128, "logical_height": 128,
            "coordinate_scale": 8.0,
            "has_fixed_time": true,
            "monster_spawn_light_level": {"type": "minecraft:uniform", "min_inclusive": 0, "max_inclusive": 15},
            "attributes": {"minecraft:gameplay/fast_lava": true, "minecraft:gameplay/sky_light_level": 4.0},
        });
        let facts =
            DimensionFacts::patched(DimensionFacts::overworld(), "test:pocket", &json).unwrap();
        assert_eq!(facts.id, "test:pocket");
        assert_eq!((facts.height.min_y, facts.height.height), (-32, 128));
        assert_eq!(facts.coordinate_scale, 8.0);
        assert!(facts.has_fixed_time && facts.fast_lava);
        assert_eq!(facts.sky_light_level, 4.0);
        assert!(matches!(
            facts.monster_spawn_light_level,
            IntProvider::Uniform { min: 0, max: 15 }
        ));
        // untouched fields stay the Overworld's
        assert_eq!(facts.infiniburn, "#minecraft:infiniburn_overworld");
        assert!(facts.piglins_zombify && facts.can_start_raid && !facts.respawn_anchor_works);
    }

    /// A type that names `timelines` takes them and its clock; one that does not keeps the base's.
    #[test]
    fn named_timelines_and_a_clock_replace_the_bases() {
        let with = |extra: serde_json::Value| {
            let mut json = serde_json::json!({"min_y": 0, "height": 64});
            json.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            DimensionFacts::patched(DimensionFacts::overworld(), "test:x", &json).unwrap()
        };
        let base = DimensionFacts::overworld();
        let kept = with(serde_json::json!({}));
        assert_eq!(
            (&kept.timelines, &kept.default_clock),
            (&base.timelines, &base.default_clock)
        );
        let nether = with(serde_json::json!({"timelines": "#minecraft:in_nether"}));
        assert!(!nether.has_timeline("minecraft:day"));
        assert!(nether.has_timeline("minecraft:villager_schedule"));
        assert_eq!(nether.default_clock, None);
        let none = with(serde_json::json!({"timelines": []}));
        assert!(none.timelines.is_empty() && none.default_clock.is_none());
        let own = with(serde_json::json!({
            "timelines": ["test:cycle"], "default_clock": "test:clock",
        }));
        assert_eq!(own.timelines, ["test:cycle"]);
        assert_eq!(own.default_clock.as_deref(), Some("test:clock"));
        // a tag this table cannot expand leaves the base's
        let unknown = with(serde_json::json!({"timelines": "#test:cycles"}));
        assert_eq!(unknown.timelines, base.timelines);
    }

    #[test]
    fn a_refused_field_is_an_error_not_a_default() {
        let bad = |extra: serde_json::Value| {
            let mut json = serde_json::json!({"min_y": 0, "height": 64});
            json.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            DimensionFacts::patched(DimensionFacts::overworld(), "test:x", &json).is_err()
        };
        assert!(bad(serde_json::json!({"skybox": "moon"})));
        assert!(bad(serde_json::json!({"cardinal_light": "up"})));
        // a scale outside the codec's range would make a portal's ratio infinite or NaN
        for scale in [0.0, f64::from_bits(1), 1.0e-6, 3.1e7, -1.0] {
            assert!(
                bad(serde_json::json!({"coordinate_scale": scale})),
                "{scale}"
            );
        }
        assert!(!bad(
            serde_json::json!({"coordinate_scale": f64::from(1.0e-5_f32)})
        ));
        assert!(!bad(serde_json::json!({"coordinate_scale": 3.0e7})));
        assert!(
            DimensionFacts::patched(
                DimensionFacts::overworld(),
                "test:x",
                &serde_json::json!({"min_y": 7, "height": 64}),
            )
            .is_err()
        );
    }

    #[test]
    fn levels_answer_their_own_type_until_the_table_is_replaced() {
        let key = LevelKey::parse("test:facts_pocket").unwrap();
        assert_eq!(DimensionFacts::of_level(key).id, "minecraft:overworld");
        let mut own = DimensionFacts::overworld().clone();
        own.id = "test:facts_pocket".into();
        own.coordinate_scale = 4.0;
        DimensionFacts::set_levels(vec![(key, own)]);
        assert_eq!(DimensionFacts::of_level(key).coordinate_scale, 4.0);
        // a built-in level without its own entry still answers its vanilla type
        assert_eq!(
            DimensionFacts::of_level(LevelKey::NETHER).coordinate_scale,
            8.0
        );
        DimensionFacts::set_levels(Vec::new());
        assert_eq!(DimensionFacts::of_level(key).id, "minecraft:overworld");
    }
}
