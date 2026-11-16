//! Dimensions a native mod registers: a level with its own build height and
//! generator, as [`register_dimension`](crate::Registrar::register_dimension)
//! hands it over once at startup. Data packs make dimensions with
//! `data/<ns>/dimension` and `dimension_type` files; a registered one becomes
//! those same two entries, below every data pack, so a pack can replace it.

/// The generator every level has without a mod's own: vanilla's
/// `NoiseBasedChunkGenerator`.
/// The built-in noise generator identifier shared with cold registration validation.
pub const NOISE_GENERATOR: &str = "minecraft:noise";

/// Whether `c` is `#rrggbb`.
fn hex_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit())
}

/// How a dimension chooses its biomes (`BiomeSource`): the level stem's `biome_source`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeSourceKind {
    /// The one `biome` of the settings everywhere (`minecraft:fixed`).
    #[default]
    Fixed,
    /// The `biomes` in a checkerboard of cells `2^biome_scale` chunks wide
    /// (`minecraft:checkerboard`).
    Checkerboard,
    /// The Overworld's or the Nether's multi-noise biomes, by `biome_preset`
    /// (`minecraft:multi_noise`).
    MultiNoise,
    /// The End's islands (`minecraft:the_end`).
    End,
    /// The mod's own `biome_points`: each biome where its climate intervals are the nearest to the
    /// level's climate (`minecraft:multi_noise` with an inline `biomes` list).
    Points,
}

/// Where one biome sits in a multi-noise biome source: the climate it is chosen for, as
/// `Climate.ParameterPoint`. Every interval is `(min, max)` inside -2 to 2, with `min` not above
/// `max`; a single value is `(v, v)`. The biome nearest to the level's climate at a position wins,
/// and `offset` (0 to 1) makes this one count as farther away.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods build it themselves as part of a level's `biome_points`
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomePoint {
    /// The biome's id (`mymod:glowing_forest`, or a vanilla or data pack biome's).
    pub biome: String,
    /// `temperature`, -2 to 2.
    pub temperature: (f32, f32),
    /// `humidity`, -2 to 2.
    pub humidity: (f32, f32),
    /// `continentalness`, -2 to 2: low is ocean, high is inland.
    pub continentalness: (f32, f32),
    /// `erosion`, -2 to 2.
    pub erosion: (f32, f32),
    /// `depth`, -2 to 2: 0 is the surface and above 1 is underground.
    pub depth: (f32, f32),
    /// `weirdness`, -2 to 2.
    pub weirdness: (f32, f32),
    /// `offset`, 0 to 1.
    pub offset: f32,
}

impl BiomePoint {
    /// Whether the point is one the codec accepts: finite intervals inside -2 to 2, ordered, and
    /// an offset inside 0 to 1.
    pub(crate) fn is_valid(&self) -> bool {
        let interval = |(min, max): (f32, f32)| {
            (-2.0..=2.0).contains(&min) && (-2.0..=2.0).contains(&max) && min <= max
        };
        !self.biome.is_empty()
            && [
                self.temperature,
                self.humidity,
                self.continentalness,
                self.erosion,
                self.depth,
                self.weirdness,
            ]
            .into_iter()
            .all(interval)
            && (0.0..=1.0).contains(&self.offset)
    }
}

/// What a mod's dimension is made of. The default is the Overworld's shape
/// and terrain over plains; the parts of a dimension type not named here are
/// the Overworld's.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods build it themselves as register_dimension's settings
#[derive(horizon_mod_api_macros::Wire)]
pub struct DimensionSettings {
    /// The lowest block's y: a multiple of 16, from -2032.
    pub min_y: i32,
    /// The build height: a multiple of 16, at least 16, reaching no higher than 2031.
    pub height: i32,
    /// How far up portals and chorus fruit reach, at most `height`.
    pub logical_height: i32,
    /// Whether the sky lights the level.
    pub has_skylight: bool,
    /// Whether the level has a bedrock ceiling, as the Nether does.
    pub has_ceiling: bool,
    /// The chunk generator's id: `minecraft:noise`, or one a mod registered
    /// with `register_chunk_generator`, which reads noise settings only for the
    /// vanilla stages it asks for.
    pub generator: String,
    /// The noise settings' id (`minecraft:overworld`, or a data pack's). A
    /// mod's chunk generator takes the surface, the carvers' aquifer and the
    /// structures' base height from them (`ChunkGenerator::vanilla_stages`).
    pub noise_settings: String,
    /// The one biome the level has, with the `Fixed` biome source.
    pub biome: String,
    /// How the level chooses its biomes. Only `Fixed` suits a mod's own chunk generator. Not read
    /// when `mod_biome_source` is set.
    pub biome_source: BiomeSourceKind,
    /// A biome source a mod registered with `register_biome_source` (`mymod:chaos`), which then
    /// chooses the level's biomes in place of `biome_source` and the fields it reads. It suits the
    /// `minecraft:noise` generator and a chunk generator of a mod's own; it may come from a mod
    /// that loads later.
    pub mod_biome_source: Option<String>,
    /// The biomes a `Checkerboard` source alternates, at least one (vanilla sets no maximum).
    pub biomes: Vec<String>,
    /// A `Points` source's biomes and the climate each is chosen for, at least one.
    pub biome_points: Vec<BiomePoint>,
    /// A `Checkerboard`'s cell size as a power of two of chunks, 0 to 62 (vanilla's default is 2).
    pub biome_scale: u32,
    /// A `MultiNoise` source's preset: `minecraft:overworld` or `minecraft:nether`.
    pub biome_preset: String,
    /// `DimensionType.hasFixedTime`: the sky's time does not advance (the Nether's and the End's).
    pub has_fixed_time: bool,
    /// `DimensionType.hasEnderDragonFight`: the level runs the End's dragon fight.
    pub has_ender_dragon_fight: bool,
    /// `DimensionType.coordinateScale`: what a portal divides this level's coordinates by (the
    /// Nether's is 8); above 0 and at most 30,000,000.
    pub coordinate_scale: f64,
    /// `DimensionType.ambientLight`: the light every block gets, 0 to 1.
    pub ambient_light: f32,
    /// `DimensionType.infiniburn`: the block tag (`#minecraft:infiniburn_overworld`) whose blocks
    /// burn forever.
    pub infiniburn: String,
    /// `DimensionType.monsterSpawnLightTest`: the highest light a monster spawns in is drawn from
    /// this range, both ends 0 to 15.
    pub monster_spawn_light_level: (i32, i32),
    /// `DimensionType.monsterSpawnBlockLightLimit`: the block light above which monsters do not
    /// spawn, 0 to 15.
    pub monster_spawn_block_light_limit: i32,
    /// The sky drawn: `overworld`, `end` or `none` (`DimensionType.skybox`).
    pub skybox: String,
    /// How block faces are lit by direction: `default` or `nether` (`DimensionType.cardinalLightType`).
    pub cardinal_light: String,
    /// `minecraft:gameplay/water_evaporates`: water placed here boils away, as in the Nether.
    pub water_evaporates: bool,
    /// `minecraft:gameplay/fast_lava`: lava flows at the Nether's speed.
    pub fast_lava: bool,
    /// `minecraft:gameplay/respawn_anchor_works`: respawn anchors can be charged and used.
    pub respawn_anchor_works: bool,
    /// `minecraft:gameplay/nether_portal_spawns_piglin`: nether portals spawn zombified piglins.
    pub nether_portal_spawns_piglin: bool,
    /// `minecraft:gameplay/piglins_zombify`: piglins turn into zombified piglins here.
    pub piglins_zombify: bool,
    /// `minecraft:gameplay/snow_golem_melts`: snow golems take damage here.
    pub snow_golem_melts: bool,
    /// `minecraft:gameplay/can_start_raid`: a bad omen can start a raid here.
    pub can_start_raid: bool,
    /// `minecraft:gameplay/sky_light_level`: the sky light that daylight is measured against, 0 to 15.
    pub sky_light_level: f32,
    /// `Level.getSeaLevel`: the sea level mob spawning, rain and snow, maps and phantoms use. With
    /// `None` it is the noise settings' own for `minecraft:noise`, and the Overworld's 63 for a
    /// chunk generator a mod registered, which reads none; `Some` is for that generator, whose
    /// terrain the game cannot read a sea level from. A level of `minecraft:noise`, and one whose
    /// chunk generator asks for vanilla stages, keeps its noise settings' sea level, which the
    /// stages are built with.
    pub sea_level: Option<i32>,
    /// When a player can sleep in a bed here (`BedRule.canSleep`): `always`, `when_dark` or `never`.
    pub bed_can_sleep: String,
    /// When a bed sets the player's respawn point here (`BedRule.canSetSpawn`): `always`,
    /// `when_dark` or `never`.
    pub bed_can_set_spawn: String,
    /// Whether a bed explodes when a player tries to use it (`BedRule.explodes`), as in the Nether.
    pub bed_destroy_on_use: bool,
    /// Whether a bed is destroyed when the player leaves it (`BedRule.destroyOnLeave`).
    pub bed_destroy_on_leave: bool,
    /// The same four for a straw bed (`minecraft:gameplay/straw_bed_rule`): when a player can
    /// sleep in one, `always`, `when_dark` or `never`.
    pub straw_bed_can_sleep: String,
    /// When a straw bed sets the respawn point: `always`, `when_dark` or `never`.
    pub straw_bed_can_set_spawn: String,
    /// Whether a straw bed explodes when a player tries to use it.
    pub straw_bed_destroy_on_use: bool,
    /// Whether a straw bed is destroyed when the player leaves it.
    pub straw_bed_destroy_on_leave: bool,
    /// `minecraft:visual/sky_color` as `#rrggbb`: the sky overhead; None is the Overworld's.
    pub sky_color: Option<String>,
    /// `minecraft:visual/fog_color` as `#rrggbb`: the fog at the horizon; None is the Overworld's.
    pub fog_color: Option<String>,
    /// `minecraft:visual/cloud_height`: the clouds' height in blocks; None is the Overworld's.
    pub cloud_height: Option<f32>,
    /// The timelines and default clock the dimension type names: `overworld` (the day and moon
    /// cycles, the villagers' schedule and the Overworld's clock), `nether` (the villagers'
    /// schedule only, no clock), `end` (the End's timeline and clock) or `none`. They set the
    /// level's sky over the day, whether monsters burn in daylight, and the clock `/time` reads.
    pub day_cycle: String,
}

impl Default for DimensionSettings {
    fn default() -> Self {
        Self {
            min_y: -64,
            height: 384,
            logical_height: 384,
            has_skylight: true,
            has_ceiling: false,
            generator: NOISE_GENERATOR.into(),
            noise_settings: "minecraft:overworld".into(),
            biome: "minecraft:plains".into(),
            biome_source: BiomeSourceKind::Fixed,
            mod_biome_source: None,
            biomes: Vec::new(),
            biome_points: Vec::new(),
            biome_scale: 2,
            biome_preset: "minecraft:overworld".into(),
            has_fixed_time: false,
            has_ender_dragon_fight: false,
            coordinate_scale: 1.0,
            ambient_light: 0.0,
            infiniburn: "#minecraft:infiniburn_overworld".into(),
            monster_spawn_light_level: (0, 7),
            monster_spawn_block_light_limit: 0,
            skybox: "overworld".into(),
            cardinal_light: "default".into(),
            water_evaporates: false,
            fast_lava: false,
            respawn_anchor_works: false,
            nether_portal_spawns_piglin: true,
            piglins_zombify: true,
            snow_golem_melts: false,
            can_start_raid: true,
            sky_light_level: 15.0,
            sea_level: None,
            bed_can_sleep: "when_dark".into(),
            bed_can_set_spawn: "always".into(),
            bed_destroy_on_use: false,
            bed_destroy_on_leave: false,
            straw_bed_can_sleep: "when_dark".into(),
            straw_bed_can_set_spawn: "never".into(),
            straw_bed_destroy_on_use: false,
            straw_bed_destroy_on_leave: true,
            sky_color: None,
            fog_color: None,
            cloud_height: None,
            day_cycle: "overworld".into(),
        }
    }
}

/// A level's dimension type as the game uses it (`Level.dimensionType()` and the gameplay attributes
/// of its environment), as `level.dimension_type(host)` reads it: the fields of
/// [`DimensionSettings`] that apply, plus the type's id.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct LevelDimensionType {
    /// The dimension type's id (`minecraft:the_nether`, or a mod's or pack's).
    pub id: String,
    /// The lowest buildable block Y.
    pub min_y: i32,
    /// The build height in blocks.
    pub height: i32,
    /// How far up portals and chorus fruit reach.
    pub logical_height: i32,
    /// Whether the sky lights the level.
    pub has_skylight: bool,
    /// Whether the level has a bedrock ceiling.
    pub has_ceiling: bool,
    /// Whether the sky's time does not advance.
    pub has_fixed_time: bool,
    /// Whether the level runs the End's dragon fight.
    pub has_ender_dragon_fight: bool,
    /// What a portal divides this level's coordinates by.
    pub coordinate_scale: f64,
    /// The light every block gets, 0 to 1.
    pub ambient_light: f32,
    /// The block tag whose blocks burn forever.
    pub infiniburn: String,
    /// The range monsters' spawn light is drawn from, when it is a constant or a uniform range.
    pub monster_spawn_light_level: Option<(i32, i32)>,
    /// The block light above which monsters do not spawn.
    pub monster_spawn_block_light_limit: i32,
    /// `overworld`, `end` or `none`.
    pub skybox: String,
    /// `default` or `nether`.
    pub cardinal_light: String,
    /// Whether water placed here boils away.
    pub water_evaporates: bool,
    /// Whether lava flows at the Nether's speed.
    pub fast_lava: bool,
    /// Whether respawn anchors can be used.
    pub respawn_anchor_works: bool,
    /// Whether nether portals spawn zombified piglins.
    pub nether_portal_spawns_piglin: bool,
    /// Whether piglins turn into zombified piglins here.
    pub piglins_zombify: bool,
    /// Whether snow golems take damage here.
    pub snow_golem_melts: bool,
    /// Whether a bad omen can start a raid here.
    pub can_start_raid: bool,
    /// The sky light that daylight is measured against, 0 to 15.
    pub sky_light_level: f32,
    /// The timelines the dimension type names, as ids (`minecraft:day`).
    pub timelines: Vec<String>,
    /// The clock `/time` reads here, if the dimension type has a default one.
    pub default_clock: Option<String>,
    /// When a player can sleep in a bed: `always`, `when_dark` or `never`.
    pub bed_can_sleep: String,
    /// When a bed sets the respawn point: `always`, `when_dark` or `never`.
    pub bed_can_set_spawn: String,
    /// Whether a bed explodes when a player tries to use it.
    pub bed_destroy_on_use: bool,
    /// Whether a bed is destroyed when the player leaves it.
    pub bed_destroy_on_leave: bool,
    /// When a player can sleep in a straw bed: `always`, `when_dark` or `never`.
    pub straw_bed_can_sleep: String,
    /// When a straw bed sets the respawn point: `always`, `when_dark` or `never`.
    pub straw_bed_can_set_spawn: String,
    /// Whether a straw bed explodes when a player tries to use it.
    pub straw_bed_destroy_on_use: bool,
    /// Whether a straw bed is destroyed when the player leaves it.
    pub straw_bed_destroy_on_leave: bool,
}

impl DimensionSettings {
    /// What the biome source's codec refuses, if anything. A mod's biome source is checked once
    /// every mod has registered.
    pub fn invalid_biome_source(&self) -> Option<String> {
        if self.mod_biome_source.is_some() {
            return None;
        }
        let reason = match self.biome_source {
            BiomeSourceKind::Fixed => return None,
            BiomeSourceKind::Checkerboard if self.biomes.is_empty() => {
                "a checkerboard needs at least one biome"
            }
            BiomeSourceKind::Checkerboard if self.biome_scale > 62 => "biome_scale must be 0 to 62",
            BiomeSourceKind::Points if self.biome_points.is_empty() => {
                "a points biome source needs at least one biome point"
            }
            BiomeSourceKind::Points if !self.biome_points.iter().all(BiomePoint::is_valid) => {
                "a biome point needs a biome, six climate intervals from -2 to 2 with min not above max, and an offset from 0 to 1"
            }
            BiomeSourceKind::MultiNoise
                if !matches!(
                    self.biome_preset.as_str(),
                    "minecraft:overworld" | "minecraft:nether"
                ) =>
            {
                "biome_preset must be minecraft:overworld or minecraft:nether"
            }
            _ if self.generator != NOISE_GENERATOR => {
                "only the fixed biome source suits a chunk generator of a mod's own"
            }
            _ => return None,
        };
        Some(reason.to_owned())
    }
    /// What the dimension type's codec refuses in the fields besides the build height, if
    /// anything: a value out of its range, or a name the game has no such sky or lighting for.
    pub fn invalid_field(&self) -> Option<String> {
        let light = |n: i32| (0..=15).contains(&n);
        let (low, high) = self.monster_spawn_light_level;
        // The codec's range: `DimensionFacts::valid_coordinate_scale`.
        let reason = if !(f64::from(1.0e-5_f32)..=3.0e7).contains(&self.coordinate_scale) {
            "coordinate_scale must be 0.00001 to 30000000"
        } else if !self.ambient_light.is_finite() {
            "ambient_light must be a number"
        } else if !(0.0..=15.0).contains(&self.sky_light_level) {
            "sky_light_level must be 0 to 15"
        } else if !(light(low) && light(high) && low <= high) {
            "monster_spawn_light_level must be a range inside 0 to 15"
        } else if !light(self.monster_spawn_block_light_limit) {
            "monster_spawn_block_light_limit must be 0 to 15"
        } else if !matches!(self.skybox.as_str(), "overworld" | "end" | "none") {
            "skybox must be overworld, end or none"
        } else if !matches!(self.cardinal_light.as_str(), "default" | "nether") {
            "cardinal_light must be default or nether"
        } else if ![
            &self.bed_can_sleep,
            &self.bed_can_set_spawn,
            &self.straw_bed_can_sleep,
            &self.straw_bed_can_set_spawn,
        ]
        .iter()
        .all(|rule| matches!(rule.as_str(), "always" | "when_dark" | "never"))
        {
            "bed_can_sleep, bed_can_set_spawn, straw_bed_can_sleep and straw_bed_can_set_spawn must be always, when_dark or never"
        } else if !self.infiniburn.starts_with('#') || self.infiniburn.len() < 2 {
            "infiniburn must be a block tag, such as #minecraft:infiniburn_overworld"
        } else if !self
            .sky_color
            .iter()
            .chain(&self.fog_color)
            .all(|c| hex_color(c))
        {
            "sky_color and fog_color must be #rrggbb"
        } else if self.cloud_height.is_some_and(|h| !h.is_finite()) {
            "cloud_height must be a number"
        } else if !matches!(
            self.day_cycle.as_str(),
            "overworld" | "nether" | "end" | "none"
        ) {
            "day_cycle must be overworld, nether, end or none"
        } else {
            return None;
        };
        Some(reason.to_owned())
    }
}

/// A registered dimension: its namespaced id (`modid:pocket`), the mod that
/// registered it and its settings.
pub(crate) struct DimensionInfo {
    pub(crate) mod_id: &'static str,
    pub(crate) id: Box<str>,
    pub(crate) settings: DimensionSettings,
}

/// The dimensions registered at startup, for the game's pack-entry adapter.
// hidden: the game's pack-entry adapter reads the registered dimensions; mods declare them through the registrar.
#[doc(hidden)]
pub fn dimension_declarations() -> Vec<(String, DimensionSettings)> {
    crate::parts::installed_dimension_list()
        .iter()
        .map(|dimension| (dimension.id.to_string(), dimension.settings.clone()))
        .collect()
}
