//! A level's world settings through a `LevelRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): the seed,
//! difficulty and game rules on the server, and weather on both sides.
//! Stale handles return `None`.
use super::{Dim, Game};
use horizon_client::command::vanilla::DIFFICULTIES;
use horizon_client::world::World;
use horizon_client_core::gamerules::Value as GameRuleValue;
use horizon_core::position::BlockPos;
use horizon_core::random::LegacyRandom;
use horizon_world::registry::BlockStateId;
use mod_api::{
    BoolGameRuleKey, FeatureFlags, LevelDimensionShape, LevelDimensionType, LevelReadError,
    LevelRef,
};
use std::sync::Arc;
use {horizon_core::dimension::Dimension as LevelDimension, horizon_world::level::LevelAccess};

fn weather_for_level(
    shape: horizon_client::chunk::LevelShape,
    is_end: bool,
    levels: (f32, f32),
) -> (f32, f32) {
    if shape.sky_light && !shape.has_ceiling && !is_end {
        levels
    } else {
        (0.0, 0.0)
    }
}

fn weather_for_world(world: &World, is_end: bool) -> (f32, f32) {
    weather_for_level(world.shape, is_end, world.weather.levels(1.0))
}

/// `Level.isRaining` and `isThundering` from the rain and thunder levels.
fn weather_flags((rain, thunder): (f32, f32)) -> (bool, bool) {
    (rain > 0.2, thunder > 0.9)
}

/// A read-only view of one level's column for `Level.isRainingAt`, so a level parked
/// while the server ticks another answers like the one in place.
struct RainView<'a> {
    world: &'a World,
    facts: &'a horizon_client::blocks::BlockFacts,
    biomes: &'a horizon_client::biome::Biomes,
    dimension: LevelDimension,
    raining: bool,
    /// `LevelAccess::random` asks for a generator; the read draws none.
    random: LegacyRandom,
}

impl horizon_world::level::BlockEntityNeighbors for RainView<'_> {}

impl LevelAccess for RainView<'_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }
    fn set_raw(&mut self, _pos: BlockPos, _state: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    fn max_y(&self) -> i32 {
        self.world.shape.max_y()
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
}

impl Game<'_> {
    /// The attribute stack a level reads (`Level.environmentAttributes`): its environment, its
    /// rain and thunder levels and its day clock's time.
    fn level_environment_in(
        &self,
        world: &World,
        dimension: Dim,
    ) -> (&horizon_client::environment::Environment, (f32, f32), i64) {
        // A client of a server has the environment its server's type made for a mod's level.
        let environment = match self
            .remote
            .as_ref()
            .and_then(|r| r.level_environment.as_ref())
        {
            Some(own) => own,
            None => match dimension {
                Dim::Overworld => self.res.environment(),
                Dim::Extra(_) => horizon_client::environment::Environment::for_level(
                    dimension.id(),
                    self.res.environment(),
                ),
                Dim::Nether => self.res.nether_environment(),
                Dim::End => self.res.end_environment(),
            },
        };
        let weather = weather_for_level(
            world.shape,
            dimension == Dim::End,
            self.world.weather.levels(1.0),
        );
        let day_time = self.command_clock_time(&self.level_clock_name(dimension));
        (environment, weather, day_time)
    }

    /// `Level.getSkyDarken`: the same dimension, time and weather calculation used by gameplay.
    pub(super) fn sky_darken_in(&self, world: &World, dimension: Dim) -> i32 {
        let (environment, (rain, thunder), day_time) = self.level_environment_in(world, dimension);
        horizon_client::environment::sky_darken(environment, day_time, rain, thunder)
    }

    /// `level.environmentAttributes().getValue(attribute, pos)` of a server level, active or
    /// parked, for a float attribute: the level's stack with the one biome at `pos` (no
    /// interpolator). None on a client, for a stale level, an unknown id or another type.
    pub(super) fn level_environment_float(
        &self,
        level: LevelRef,
        attribute: &str,
        pos: [f64; 3],
    ) -> Option<f32> {
        if !self.is_logical_server() {
            return None;
        }
        let world = self.native_level_read_world(level)?;
        let (environment, weather, _) =
            self.level_environment_in(world, Dim::from_id(level.dimension_id()));
        // `Mth.floor` of each coordinate, as the biome layer reads the position.
        let block = pos.map(|v| v.floor() as i32);
        let biomes = world_biomes!(self);
        let map = |zoomed: bool| {
            let info = biomes.get(self.native_biome_index(level, block, zoomed)?);
            Some(environment.biome(&info.name).unwrap_or(&info.attributes))
        };
        let id = attribute.strip_prefix("minecraft:").unwrap_or(attribute);
        // Each timeline reads the clock it names, 0 for one the world has not met.
        let clock = |name: &str| self.command_clock_time(name);
        match environment.value_at(id, &clock, weather, map(false), map(true))? {
            horizon_world::environment_attributes::Value::Float(value) => Some(value),
            _ => None,
        }
    }

    /// `Level.getSkyDarken` for an active or parked level, without switching dimensions.
    pub(super) fn level_get_sky_darken(&self, level: LevelRef) -> Option<i32> {
        let world = self.native_level_read_world(level)?;
        if level.dimension() == self.native_level_key() {
            return Some(self.sky_darken());
        }
        Some(self.sky_darken_in(world, Dim::from_id(level.dimension_id())))
    }

    /// The enabled feature flags as this side knows them: a client's are the server's, sent at
    /// join; a server's are its world's `enabled_features`.
    pub(super) fn enabled_feature_ids(&self) -> &Arc<[String]> {
        self.remote
            .as_ref()
            .map_or(&self.level.enabled_features, |remote| &remote.features)
    }
    /// Dimension shape of a loaded server level, including parked levels.
    pub(super) fn level_dimension_shape(&self, level: LevelRef) -> Option<LevelDimensionShape> {
        let shape = self.native_level_world(level)?.shape;
        Some(LevelDimensionShape::new(
            shape.min_y,
            shape.height,
            shape.logical_height,
            shape.sky_light,
            shape.has_ceiling,
        ))
    }

    /// The level's dimension type with its gameplay attributes, from the facts the game uses.
    pub(super) fn level_dimension_type(&self, level: LevelRef) -> Option<LevelDimensionType> {
        use horizon_world::dimension_type::{CardinalLight, Skybox};
        use horizon_world::worldgen::value_provider::IntProvider;
        self.native_level_read_world(level)?;
        let key = horizon_core::LevelKey::parse(level.dimension_id()).ok()?;
        let facts = self.dimension_facts_for(key);
        Some(LevelDimensionType {
            id: facts.id.clone(),
            min_y: facts.height.min_y,
            height: facts.height.height,
            logical_height: facts.height.logical_height,
            has_skylight: facts.height.has_skylight,
            has_ceiling: facts.height.has_ceiling,
            has_fixed_time: facts.has_fixed_time,
            has_ender_dragon_fight: facts.has_ender_dragon_fight,
            coordinate_scale: facts.coordinate_scale,
            ambient_light: facts.ambient_light,
            infiniburn: facts.infiniburn.clone(),
            monster_spawn_light_level: match facts.monster_spawn_light_level {
                IntProvider::Constant(n) => Some((n, n)),
                IntProvider::Uniform { min, max } => Some((min, max)),
                _ => None,
            },
            monster_spawn_block_light_limit: facts.monster_spawn_block_light_limit,
            skybox: match facts.skybox {
                Skybox::None => "none".to_owned(),
                Skybox::Overworld => "overworld".to_owned(),
                Skybox::End => "end".to_owned(),
            },
            cardinal_light: match facts.cardinal_light {
                CardinalLight::Default => "default".to_owned(),
                CardinalLight::Nether => "nether".to_owned(),
            },
            water_evaporates: facts.water_evaporates,
            fast_lava: facts.fast_lava,
            respawn_anchor_works: facts.respawn_anchor_works,
            nether_portal_spawns_piglin: facts.nether_portal_spawns_piglin,
            piglins_zombify: facts.piglins_zombify,
            snow_golem_melts: facts.snow_golem_melts,
            can_start_raid: facts.can_start_raid,
            sky_light_level: facts.sky_light_level,
            timelines: facts.timelines.clone(),
            default_clock: facts.default_clock.clone(),
            bed_can_sleep: facts.bed_rule.can_sleep.name().to_owned(),
            bed_can_set_spawn: facts.bed_rule.can_set_spawn.name().to_owned(),
            bed_destroy_on_use: facts.bed_rule.destroy_on_use,
            bed_destroy_on_leave: facts.bed_rule.destroy_on_leave,
            straw_bed_can_sleep: facts.straw_bed_rule.can_sleep.name().to_owned(),
            straw_bed_can_set_spawn: facts.straw_bed_rule.can_set_spawn.name().to_owned(),
            straw_bed_destroy_on_use: facts.straw_bed_rule.destroy_on_use,
            straw_bed_destroy_on_leave: facts.straw_bed_rule.destroy_on_leave,
        })
    }

    /// The sea level the game uses for the level (`Dim::sea_level`). None as the host call says.
    pub(super) fn level_sea_level(&self, level: LevelRef) -> Option<i32> {
        self.native_level_world(level)?;
        Some(Dim::sea_level_of_id(level.dimension_id()))
    }

    /// The world's seed (`WorldOptions.seed`), as `/seed` shows it.
    pub(super) fn level_seed(&self, level: LevelRef) -> Option<i64> {
        self.native_level_world(level)?;
        Some(self.level.seed)
    }
    /// The difficulty (`Level.getDifficulty`): `peaceful`, `easy`, `normal`
    /// or `hard`, as `/difficulty` names it.
    pub(super) fn level_difficulty(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        DIFFICULTIES.get(usize::from(self.ui.difficulty)).copied()
    }
    /// `ServerLevel.getCurrentDifficultyAt(pos)` in the level the handle names, in place or
    /// parked.
    pub(super) fn level_local_difficulty(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<mod_api::Difficulty> {
        let world = self.native_level_world(level)?;
        let at = pos.map(f64::from);
        Some(super::creatures::local_difficulty_in(world, at).into())
    }
    /// Read a registered boolean game rule.
    pub(super) fn level_game_rule_bool(
        &self,
        level: LevelRef,
        key: BoolGameRuleKey,
    ) -> Option<bool> {
        self.native_level_world(level)?;
        self.ui
            .rules
            .bool_slot(mod_api::__private::game_rule_bool_slot(key)?)
    }
    /// Read a registered integer game rule.
    pub(super) fn level_game_rule_int(
        &self,
        level: LevelRef,
        key: mod_api::IntGameRuleKey,
    ) -> Option<i32> {
        self.native_level_world(level)?;
        self.ui
            .rules
            .int_slot(mod_api::__private::game_rule_int_slot(key)?)
    }
    /// Game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`), as
    /// `/gamerule` queries it: None for an unknown rule.
    pub(super) fn level_game_rule(
        &self,
        level: LevelRef,
        rule: &str,
    ) -> Result<Option<GameRuleValue>, LevelReadError> {
        self.native_level_world_result(level)?;
        Ok(self.ui.rules.get(rule))
    }
    /// `/gamerule <rule> <value>` from a mod: the world's rule changes, `game_rule_changed`
    /// fires with no actor and the clients get the rules. The rule already at `value` is true
    /// with nothing fired; an unknown rule, a value of the other kind or out of range is false.
    pub(super) fn level_set_game_rule(
        &mut self,
        level: LevelRef,
        rule: &str,
        value: GameRuleValue,
    ) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        let Some(rule) = self.ui.rules.rule(rule).cloned() else {
            return false;
        };
        let fits = match (rule.default, value) {
            (GameRuleValue::Bool(_), GameRuleValue::Bool(_)) => true,
            (GameRuleValue::Int(_), GameRuleValue::Int(v)) => (rule.min..=rule.max).contains(&v),
            _ => false,
        };
        if fits && self.ui.rules.get(&rule.name) != Some(value) {
            self.set_game_rule(&rule, value, None);
        }
        fits
    }
    /// `/time set <time>` in the level a handle names, on its default clock: `time_skip`
    /// (cause `mod`) may keep the old time; the clock's partial tick restarts.
    pub(super) fn level_set_clock_time(&mut self, level: LevelRef, time: i64) -> bool {
        if time < 0 || self.native_level_world(level).is_none() {
            return false;
        }
        let dim = Dim::from_id(level.dimension_id());
        if self.dimension_type_of(dim).default_clock.is_none() {
            return false;
        }
        let clock = self.level_clock_name(dim);
        let total = self.command_clock_time(&clock);
        if total == time {
            return true;
        }
        let mut cancel = false;
        self.native_time_skip(level.dimension_id(), total, time, "mod", &mut cancel);
        if cancel {
            return false;
        }
        self.set_command_clock_time(&clock, time);
        self.command_clock_state(&clock).partial = 0.0;
        true
    }
    /// Every game rule the world has (`GameRules`) with its current value, by `minecraft:` id
    /// (`max_minecart_speed` only with minecart improvements): None for a stale handle and on
    /// a client.
    pub(super) fn level_game_rules(
        &self,
        level: LevelRef,
    ) -> Result<Vec<(String, GameRuleValue)>, LevelReadError> {
        self.native_level_world_result(level)?;
        let rules = &self.ui.rules;
        Ok(rules
            .available_rules()
            .filter_map(|r| Some((r.name.clone(), rules.get(&r.name)?)))
            .collect())
    }
    /// The rain and thunder levels, each 0.0 to 1.0 (`Level.getRainLevel`,
    /// `getThunderLevel` at partial tick 1.0): both 0 in a level with no weather.
    /// Reads any open server level or the client's current synchronized level;
    /// None for a stale handle, an unavailable level or a client not logged in.
    pub(super) fn level_weather(&self, level: LevelRef) -> Option<(f32, f32)> {
        let world = self.native_level_read_world(level)?;
        Some(weather_for_world(
            world,
            level.dimension() == Dim::End.key(),
        ))
    }
    /// Whether it rains (`Level.isRaining`: a rain level above 0.2).
    pub(super) fn level_is_raining(&self, level: LevelRef) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(
            weather_flags(weather_for_world(
                world,
                level.dimension() == Dim::End.key(),
            ))
            .0,
        )
    }
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    pub(super) fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        let world = self.native_level_world(level)?;
        if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return None;
        }
        let view = RainView {
            world,
            facts: self.res.facts(),
            biomes: world_biomes!(self),
            dimension: Dim::from_id(level.dimension_id()).level(),
            raining: self.level_is_raining(level)?,
            random: LegacyRandom::new(0),
        };
        Some(
            self.res
                .behaviours()
                .is_raining_at(&view, BlockPos::new(pos[0], pos[1], pos[2])),
        )
    }
    /// `LevelReader.enabledFeatures` of the level a handle names. Every level of a world shares
    /// the world's flags. None for a stale handle and on a client.
    pub(super) fn level_enabled_features(&self, level: LevelRef) -> Option<FeatureFlags> {
        self.native_level_world(level)?;
        Some(FeatureFlags::new(self.enabled_feature_ids().clone()))
    }
    /// Whether it thunders (`Level.isThundering`: a thunder level above 0.9).
    pub(super) fn level_is_thundering(&self, level: LevelRef) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(
            weather_flags(weather_for_world(
                world,
                level.dimension() == Dim::End.key(),
            ))
            .1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::chunk::Chunk;
    use horizon_client::gamerules;
    use mod_api::{
        FeatureFlags, GameRuleChangedEvent, GameRuleValue, Hooks, LevelHost, ModEntry, ServerHost,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "daily"]
    fn dimension_shape_reads_current_parked_and_stale_levels() {
        use super::super::Dim;
        use horizon_client::chunk::LevelShape;
        use mod_api::{Authority, Host, LevelDimensionShape};

        with_game(|game| {
            game.world.shape = LevelShape::new(-32, 320, 256, true, false).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            let expected = LevelDimensionShape::new(-32, 320, 256, true, false);
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), Some(expected));
            game.enter_level(Dim::Nether).unwrap();
            game.world.shape = LevelShape::new(0, 256, 128, false, true).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), Some(expected));
            assert_eq!(
                nether.dimension_shape(host),
                Some(LevelDimensionShape::new(0, 256, 128, false, true))
            );
            game.native_lifecycle
                .begin_open(Authority::IntegratedServer);
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), None);
            assert_eq!(nether.dimension_shape(host), None);
        });
    }

    /// A chunk of air with `biome` in every cell: full sky light everywhere.
    fn chunk_in(game: &super::Game<'_>, x: i32, biome: &str) -> Arc<Chunk> {
        let id = game.res.biomes().id(biome);
        let mut chunk = Chunk::empty(x, 0);
        for qy in 0..chunk.sections.len() as i32 * 4 {
            for (qx, qz) in (0..4).flat_map(|qx| (0..4).map(move |qz| (qx, qz))) {
                chunk.set_biome_cell(qx, qy, qz, id);
            }
        }
        Arc::new(chunk)
    }

    #[test]
    #[ignore = "daily"]
    fn rain_at_a_position_checks_sky_roof_and_biome() {
        with_game(|game| {
            game.install_live_features();
            for (x, biome) in [
                (0, "minecraft:plains"),
                (1, "minecraft:desert"),
                (2, "minecraft:snowy_plains"),
            ] {
                let chunk = chunk_in(game, x, biome);
                game.world.chunks.insert((x, 0), chunk);
            }
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(3, 100, 3, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let chunks = game.world.chunks.len();
            let plains = [1, 70, 1];
            let roofed = [3, 70, 3];
            let desert = [17, 70, 1];
            let snowy = [33, 70, 1];
            let unloaded = [16 * 9, 70, 1];

            game.world.weather.rain_level = (1.0, 1.0);
            for (pos, rains) in [
                (plains, true),
                (roofed, false),
                (desert, false),
                (snowy, false),
            ] {
                assert_eq!(game.level_is_raining_at(level, pos), Some(rains), "{pos:?}");
            }
            assert_eq!(game.level_is_raining_at(level, unloaded), None);
            assert_eq!(game.level_is_raining_at(level, [1, 4000, 1]), None);
            game.with_level(|_, host, _| {
                assert!(host.is_raining_at(plains));
                assert!(!host.is_raining_at(roofed));
                assert!(!host.is_raining_at(desert));
                assert!(!host.is_raining_at(snowy));
                assert!(!host.is_raining_at(unloaded));
            });

            game.world.weather.rain_level = (0.0, 0.0);
            assert_eq!(game.level_is_raining_at(level, plains), Some(false));
            game.with_level(|_, host, _| assert!(!host.is_raining_at(plains)));
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn enabled_features_default_to_vanilla_and_follow_the_world() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let flags = game.level_enabled_features(level).unwrap();
            assert!(flags.contains("minecraft:vanilla"));
            assert!(flags.contains("vanilla"));
            assert!(!flags.contains("minecraft:redstone_experiments"));
            game.level.enabled_features = [
                "minecraft:vanilla".to_string(),
                "minecraft:redstone_experiments".to_string(),
            ]
            .into();
            let flags = game.level_enabled_features(level).unwrap();
            assert!(flags.contains("minecraft:redstone_experiments"));
            assert_eq!(flags.iter().count(), 2);
            assert_eq!(
                FeatureFlags::new(Arc::default()).iter().collect::<Vec<_>>(),
                ["minecraft:vanilla"]
            );
            let ids: Vec<String> = flags.iter().map(str::to_owned).collect();
            game.with_level(|_, host, _| {
                assert_eq!(host.enabled_features().iter().collect::<Vec<_>>(), ids);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn native_mod_reads_registered_bool_rule_by_key() {
        use std::sync::OnceLock;
        static KEY: OnceLock<mod_api::BoolGameRuleKey> = OnceLock::new();
        struct RuleMod;
        #[mod_api::__subscriptions]
        impl Hooks for RuleMod {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar.game_rule_category("machines").unwrap();
                let key = registrar
                    .game_rule_bool("enabled", "example:machines", true)
                    .unwrap();
                let _ = KEY.set(key);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("example", Box::new(RuleMod))].into();
        with_mods(mods, || {
            with_game(|game| {
                let registry = Arc::new(crate::game::api_values::game_rule_registry());
                game.level.rule_registry = registry.clone();
                game.ui.rules = gamerules::GameRules::with_registry(registry, &[]);
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(level.game_rule_bool(game, *KEY.get().unwrap()), Some(true));
            })
        });
    }

    /// arismoko/horizon#3316's case: `example:cap` (8, 0..=64) refuses `/gamerule example:cap 70`, takes 12, and
    /// keeps 12 through a save and load of the rules.
    #[test]
    #[ignore = "daily"]
    fn a_mod_int_rule_is_range_checked_by_gamerule_and_saved() {
        use std::sync::OnceLock;
        static KEY: OnceLock<mod_api::IntGameRuleKey> = OnceLock::new();
        struct RuleMod;
        #[mod_api::__subscriptions]
        impl Hooks for RuleMod {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar.game_rule_category("machines").unwrap();
                let key = registrar
                    .game_rule_int("cap", "example:machines", 8, 0, 64)
                    .unwrap();
                let _ = KEY.set(key);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("example", Box::new(RuleMod))].into();
        with_mods(mods, || {
            with_game(|game| {
                let registry = Arc::new(crate::game::api_values::game_rule_registry());
                game.level.rule_registry = registry.clone();
                game.ui.rules = gamerules::GameRules::with_registry(registry.clone(), &[]);
                game.load_world_state().unwrap();
                game.set_allow_commands(true);
                game.build_registries();
                let key = *KEY.get().unwrap();
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(level.game_rule_int(game, key), Some(8));
                assert!(!game.run_command("gamerule example:cap 70"));
                assert_eq!(level.game_rule_int(game, key), Some(8));
                assert!(game.run_command("gamerule example:cap 12"));
                assert_eq!(level.game_rule_int(game, key), Some(12));
                let saved = game.ui.rules.to_nbt().tag;
                game.ui.rules = gamerules::GameRules::from_nbt_with_registry(&saved, &[], registry);
                assert_eq!(level.game_rule_int(game, key), Some(12));
            })
        });
    }

    /// What a `game_rule_changed` listener read of the whole rule set: the changed rule, whether a
    /// player caused it, and `keep_inventory`, `random_tick_speed` and the rule count it saw.
    type RuleRead = (
        String,
        bool,
        Option<GameRuleValue>,
        Option<GameRuleValue>,
        usize,
    );
    struct RuleReader(Arc<Mutex<Vec<RuleRead>>>);
    #[mod_api::__subscriptions]
    impl Hooks for RuleReader {
        fn game_rule_changed(
            &self,
            host: &mut dyn ServerHost,
            event: &mut GameRuleChangedEvent<'_>,
        ) {
            let rules = host
                .level("minecraft:overworld")
                .and_then(|level| level.game_rules(host).ok())
                .unwrap_or_default();
            let value = |id: &str| rules.iter().find(|(rule, _)| rule == id).map(|(_, v)| *v);
            self.0.lock().unwrap().push((
                event.rule.to_owned(),
                event.actor.is_some(),
                value("minecraft:keep_inventory"),
                value("minecraft:random_tick_speed"),
                rules.len(),
            ));
        }
    }

    /// A listener of a change no player made reads every server rule through its host: the new
    /// value and the others as they stand.
    #[test]
    #[ignore = "daily"]
    fn a_game_rule_listener_reads_the_whole_rule_set() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "rule-reader",
            Box::new(RuleReader(seen.clone())),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                assert!(
                    game.ui
                        .rules
                        .set("random_tick_speed", gamerules::Value::Int(7))
                );
                let keep_inventory = gamerules::rule_registry()
                    .rule("keep_inventory")
                    .unwrap()
                    .clone();
                game.set_game_rule(&keep_inventory, gamerules::Value::Bool(true), None);
            });
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [(
                "minecraft:keep_inventory".to_owned(),
                false,
                Some(GameRuleValue::Bool(true)),
                Some(GameRuleValue::Int(7)),
                // Vanilla 26.3 `GameRules` has 59, `max_minecart_speed` behind minecart improvements.
                58,
            )]
        );
    }

    /// `level_set_game_rule` writes what `/gamerule` accepts, refuses what it refuses, and
    /// `level_set_clock_time` moves the level's clock as `/time set` does.
    #[test]
    #[ignore = "daily"]
    fn a_mod_sets_game_rules_and_the_clock() {
        use mod_api::WorldEpoch;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert!(level.set_game_rule(game, "keep_inventory", GameRuleValue::Bool(true)));
            assert_eq!(
                game.ui.rules.get("keep_inventory"),
                Some(gamerules::Value::Bool(true))
            );
            assert!(game.level_set_game_rule(
                level,
                "minecraft:random_tick_speed",
                gamerules::Value::Int(9)
            ));
            assert_eq!(
                game.ui.rules.get("random_tick_speed"),
                Some(gamerules::Value::Int(9))
            );
            assert!(game.level_set_game_rule(level, "random_tick_speed", gamerules::Value::Int(9)));
            assert!(!game.level_set_game_rule(level, "missing:rule", gamerules::Value::Bool(true)));
            assert!(!game.level_set_game_rule(level, "keep_inventory", gamerules::Value::Int(1)));
            assert!(!game.level_set_game_rule(
                level,
                "random_tick_speed",
                gamerules::Value::Int(-1)
            ));
            assert_eq!(
                game.ui.rules.get("random_tick_speed"),
                Some(gamerules::Value::Int(9))
            );

            assert!(game.level_set_clock_time(level, 13_000));
            assert_eq!(game.level_clock_time(level), Some(13_000));
            assert!(game.level_set_clock_time(level, 13_000));
            assert!(!game.level_set_clock_time(level, -1));
            assert_eq!(game.level_clock_time(level), Some(13_000));
            let stale =
                mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert!(!game.level_set_clock_time(stale, 1));
            assert!(!game.level_set_game_rule(
                stale,
                "keep_inventory",
                gamerules::Value::Bool(false)
            ));
        });
    }

    /// What a mod's `game_rule_changed` and `time_skip` listeners were told.
    #[derive(Default)]
    struct Told {
        rules: Mutex<Vec<(String, bool)>>,
        skips: Mutex<Vec<(String, i64, i64, String)>>,
    }
    struct Teller(Arc<Told>, bool);
    #[mod_api::__subscriptions]
    impl Hooks for Teller {
        fn game_rule_changed(&self, _: &mut dyn ServerHost, event: &mut GameRuleChangedEvent<'_>) {
            self.0
                .rules
                .lock()
                .unwrap()
                .push((event.rule.to_owned(), event.actor.is_some()));
        }
        fn time_skip(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::TimeSkipEvent<'_>,
        ) -> mod_api::Flow {
            self.0.skips.lock().unwrap().push((
                event.dimension.to_owned(),
                event.from,
                event.to,
                event.cause.to_owned(),
            ));
            if self.1 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// A rule a mod sets reaches `game_rule_changed` once, with no actor; the same value again
    /// reaches nothing.
    #[test]
    #[ignore = "daily"]
    fn a_mod_set_game_rule_has_no_actor() {
        let told = Arc::new(Told::default());
        let mods: mod_api::Mods = vec![ModEntry::new(
            "teller",
            Box::new(Teller(told.clone(), false)),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                assert!(level.set_game_rule(game, "keep_inventory", GameRuleValue::Bool(true)));
                assert!(level.set_game_rule(game, "keep_inventory", GameRuleValue::Bool(true)));
            });
        });
        assert_eq!(
            *told.rules.lock().unwrap(),
            [("minecraft:keep_inventory".to_owned(), false)]
        );
    }

    /// A `time_skip` listener that cancels keeps the old time: the call says `false`, the
    /// listener heard the jump with cause `mod`, and the clock has not moved.
    #[test]
    #[ignore = "daily"]
    fn a_canceled_time_skip_keeps_the_clock() {
        let told = Arc::new(Told::default());
        let mods: mod_api::Mods = vec![ModEntry::new(
            "teller",
            Box::new(Teller(told.clone(), true)),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let start = game.level_clock_time(level).unwrap();
                assert_ne!(start, 13_000);
                assert!(!game.level_set_clock_time(level, 13_000));
                assert_eq!(game.level_clock_time(level), Some(start));
                assert_eq!(
                    *told.skips.lock().unwrap(),
                    [(game.dim.id().to_owned(), start, 13_000, "mod".to_owned())]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn rule_reads_distinguish_unknown_stale_and_unavailable_and_snapshot_values() {
        use super::super::execution_role::ExecutionRole;
        use mod_api::{LevelReadError, WorldEpoch};
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert!(
                game.ui
                    .rules
                    .set("keep_inventory", gamerules::Value::Bool(true))
            );
            assert!(
                game.ui
                    .rules
                    .set("random_tick_speed", gamerules::Value::Int(7))
            );
            assert_eq!(level.game_rule(game, "missing:rule"), Ok(None));
            assert_eq!(
                level.game_rule(game, "keep_inventory"),
                Ok(Some(GameRuleValue::Bool(true)))
            );
            assert_eq!(
                level.game_rule(game, "random_tick_speed"),
                Ok(Some(GameRuleValue::Int(7)))
            );
            let rules = level.game_rules(game).unwrap();
            assert!(rules.contains(&("minecraft:random_tick_speed".into(), GameRuleValue::Int(7))));
            let stale =
                mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let gone = mod_api::LevelRef::new(
                level.epoch(),
                mod_api::__private::dimension_key("missing:level"),
            );
            for handle in [stale, gone] {
                assert_eq!(
                    handle.game_rule(game, "keep_inventory"),
                    Err(LevelReadError::Stale)
                );
                assert_eq!(handle.game_rules(game), Err(LevelReadError::Stale));
                assert!(game.native_level_world(handle).is_none());
            }
            {
                let _scope =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Setup);
                assert_eq!(
                    level.game_rule(game, "keep_inventory"),
                    Err(LevelReadError::Unavailable)
                );
                assert_eq!(level.game_rules(game), Err(LevelReadError::Unavailable));
            }
            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(
                level.game_rule(game, "keep_inventory"),
                Err(LevelReadError::Unavailable)
            );
            assert_eq!(level.game_rules(game), Err(LevelReadError::Unavailable));
            assert!(game.native_level_world(level).is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn weather_read_resolves_levels_and_refuses_stale_or_unavailable_handles() {
        use super::super::execution_role::ExecutionRole;
        use mod_api::{LevelRef, WorldEpoch};

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.world.weather.rain_level = (0.0, 0.5);
            game.world.weather.thunder_level = (0.0, 0.75);
            assert_eq!(level.weather(game), Some((0.5, 0.375)));
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.weather(game), None);
            let absent = LevelRef::new(
                level.epoch(),
                mod_api::__private::dimension_key("missing:level"),
            );
            assert_eq!(absent.weather(game), None);
            // The current world's dimension type, not its id alone, gates weather.
            game.world.shape.has_ceiling = true;
            assert_eq!(level.weather(game), Some((0.0, 0.0)));
            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(level.weather(game), None, "no logged-in remote level");
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "WeatherTest").unwrap();
            assert_eq!(level.weather(game), None, "login has not completed");
            game.remote.as_mut().unwrap().logged_in = true;
            game.world.weather.rain_level = (0.0, 0.5);
            game.world.weather.thunder_level = (0.0, 0.75);
            assert_eq!(level.weather(game), Some((0.5, 0.375)));
            let other = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(
                other.weather(game),
                None,
                "client shows only its current level"
            );
            assert_eq!(stale.weather(game), None);
            // Only the weather pair is a both-side read; the server queries keep their guard.
            assert_eq!(game.level_is_raining(level), None);
            assert_eq!(game.level_is_thundering(level), None);
        });
    }

    #[test]
    fn weather_requires_skylight_no_ceiling_and_not_end() {
        use horizon_client::chunk::LevelShape;
        let rain = (1.0, 0.5);
        assert_eq!(
            super::weather_for_level(LevelShape::OVERWORLD, false, rain),
            rain
        );
        assert_eq!(
            super::weather_for_level(LevelShape::NETHER, false, rain),
            (0.0, 0.0)
        );
        assert_eq!(
            super::weather_for_level(LevelShape::END, true, rain),
            (0.0, 0.0)
        );
        let mut skyless = LevelShape::OVERWORLD;
        skyless.sky_light = false;
        assert_eq!(super::weather_for_level(skyless, false, rain), (0.0, 0.0));
        assert_eq!(
            super::weather_for_level(LevelShape::OVERWORLD.with_ceiling(true), false, rain),
            (0.0, 0.0)
        );
    }

    #[test]
    fn rain_and_thunder_start_above_vanillas_thresholds() {
        assert_eq!(super::weather_flags((0.2, 0.0)), (false, false));
        assert_eq!(super::weather_flags((0.21, 0.9)), (true, false));
        assert_eq!(super::weather_flags((1.0, 0.95)), (true, true));
    }
}
