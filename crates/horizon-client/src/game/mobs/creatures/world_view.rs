//! The immutable world view used by mob simulation.
use super::*;

impl Game<'_> {
    pub(in crate::game) fn mob_world_base<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        if self.is_logical_server() {
            self.world.enable_path_type_cache();
        }
        let (rain, thunder) = self.weather_levels(1.0);
        let weather = self.can_have_weather();
        let player = self.player_view();
        MobWorld {
            mob_maps: self.mob_maps(),
            section_writer: None,
            professions: self.stations().professions.as_ref(),
            loop_context: &self.loop_context,
            foreign: None,
            world: &self.world,
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            fluid_tables: &self.motion.fluids,
            attributes: &self.motion.attributes,
            movement: &self.motion.blocks,
            collision: self.collision,
            suffocation: self.mob_suffocation(),
            data: self.res.mob_data(),
            controlling_player_movement: None,
            nearest_player: player.as_ref().filter(|p| !p.spectator).map(|p| p.position),
            player,
            player_identity: self.player_identity(),
            player_selection: None,
            others,
            live_mobs: None,
            dragon: self.dragon_target(),
            leash_boats: &self.leash_boats,
            others_grid: None,
            others_index: None,
            passenger_objects: &[],
            others_villagers: None,
            others_solid: None,
            daylight: self.dim != Dim::Nether && self.sky_darken() < 4,
            // day.json: false from tick 12542, true from 23460 (a step track).
            monsters_burn: self.dimension_type().has_timeline("minecraft:day")
                && !(12542..23460).contains(&self.world.day_time.rem_euclid(24000)),
            game_time: self.world.game_time,
            day_time: self.world.day_time,
            raining: weather && rain > 0.2,
            thundering: weather && thunder > 0.9,
            level: self.level_key(),
            block_tags: &self.items().block_tags,
            landing: &self.motion.landing,
            biomes: world_biomes!(self),
            mob_griefing: self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING),
            universal_anger: self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::UNIVERSAL_ANGER),
            mob_drops: self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_DROPS),
            max_entity_cramming: self
                .ui
                .rules
                .int_at(horizon_client::gamerules::vanilla::MAX_ENTITY_CRAMMING),
            dimension_type: self.dimension_type(),
            world_seed: self.level.seed,
            sea_level: self.dim.sea_level(),
            teams: std::sync::Arc::new(self.mob_teams()),
            items: Some(self.items()),
            lid_openers: &self.lid_openers,
            mod_effects: if self.remote.is_none() {
                Some(self)
            } else {
                None
            },
            mob_hooks: (self.native_authoritative()
                && ([
                    mod_api::HookId::EndermanAnger,
                    mod_api::HookId::EntityMobGriefing,
                    mod_api::HookId::LivingVisibility,
                    mod_api::HookId::LivingChangeTarget,
                ]
                .into_iter()
                .any(mod_api::__private::listened)
                    || [
                        mod_api::HookId::CanEntityDestroy,
                        mod_api::HookId::BlockPathType,
                        mod_api::HookId::AdjacentBlockPathType,
                    ]
                    .into_iter()
                    .any(mod_api::__private::has_scoped_hook)))
            .then_some(self as &dyn horizon_client::mobs::MobHooks),
            goal_host: (self.native_authoritative()
                && (mod_api::__private::has_entity_parts()
                    || mod_api::__private::listened(mod_api::HookId::LivingChangeTarget)))
            .then_some(self as &dyn horizon_client::mobs::part_goals::GoalHost),
            asks_animal_tame: self.remote.is_none()
                && mod_api::__private::listened(mod_api::HookId::AnimalTame),
            asks_living_destroy_block: self.remote.is_none()
                && mod_api::__private::listened(mod_api::HookId::LivingDestroyBlock),
            asks_baby_spawn: self.remote.is_none()
                && mod_api::__private::listened(mod_api::HookId::BabyEntitySpawn),
            // A warden's away suspects are UUIDs: on the server side, where the
            // entity UUIDs live, they resolve back to the mobs that are here.
            who_lookup: if self.remote.is_none() {
                Some(self)
            } else {
                None
            },
            border: Some(self.border.geometry()),
            // The first player in the level: a server has no player of its own.
            difficulty: self.local_difficulty(
                self.level_players_iter()
                    .next()
                    .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]),
            ),
        }
    }
}
