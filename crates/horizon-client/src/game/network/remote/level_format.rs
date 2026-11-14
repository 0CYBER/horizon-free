//! A level's shape, chunk format and facts, from the dimension type its server synced: what the
//! player's level takes on entering it (`enter_remote_dimension`), and a viewed level's world on
//! its `open` (`viewed_worlds`).
use super::Remote;
use crate::game::{Dim, remote_chunks::ChunkFormat};
use horizon_client::{chunk::LevelShape, environment::Environment};
use horizon_protocol::{packets::play, registry::DimensionType};
use horizon_world::dimension_type::DimensionFacts;
use std::sync::Arc;

/// What a level of the server is, from its synced dimension type.
pub(in crate::game) struct LevelFormat {
    pub(in crate::game) dimension: DimensionType,
    pub(in crate::game) shape: LevelShape,
    /// The format its chunk packets decode with.
    pub(in crate::game) chunk_format: Arc<ChunkFormat>,
    /// Its facts (what it does to fluids, mobs, beds and time); `None` when the server sent no
    /// type the game takes.
    pub(in crate::game) facts: Option<Arc<DimensionFacts>>,
    /// The sky, fog and cloud attributes of a level of a mod or a pack; `None` for a built-in
    /// level, or when the type is refused.
    pub(in crate::game) environment: Option<Environment>,
}

impl crate::game::Game<'_> {
    /// The client's table of the server's levels on entering `entered`: a level past vanilla's
    /// three that the process does not know yet is added, so that [`Dim::from_id`] finds it, and
    /// the level left is dropped when the server opened it after Login (a runtime level). Such a
    /// level is not in Login's `levels`, so it is never a viewed world, and a long session through
    /// many of them keeps none. Only a level this connection added leaves the table: in single
    /// player the integrated server shares the table, and a runtime level it opened is its own
    /// until it closes it (`runtime_levels.rs`).
    pub(in crate::game) fn track_remote_levels(&mut self, entered: &str) {
        if let Some(r) = self.remote.as_mut()
            && r.level != entered
            && !Dim::is_vanilla(&r.level)
            && !r.levels.contains(&r.level)
        {
            if r.added_levels.remove(&r.level) {
                Dim::remove_extra(&r.level);
            }
            r.facts_cache.remove(&r.level);
        }
        if !Dim::is_vanilla(entered)
            && !Dim::exists(entered)
            && let Some(r) = self.remote.as_mut()
        {
            Dim::add_extra(entered);
            r.added_levels.insert(entered.to_owned());
        }
    }
}

impl Remote {
    /// The format and facts of the level `spawn` names, from the server's dimension type; `base`
    /// is the game's own environment. The facts are cached by level and named type.
    pub(in crate::game) fn level_format(
        &mut self,
        spawn: &play::SpawnInfo,
        base: &Environment,
    ) -> LevelFormat {
        let dimension = self.registries.dimension(spawn.dimension_type);
        // The server's dimension type gives the level's shape, and its
        // chunks the shape's column.
        let shape = LevelShape::new(
            dimension.min_y,
            dimension.height,
            dimension.logical_height,
            dimension.has_skylight,
            dimension.has_ceiling,
        )
        .unwrap_or_else(|_| Dim::from_id(&dimension.name).shape());
        let chunk_format = Arc::new(
            self.chunk_format
                .with_dimension(&dimension)
                .in_column(shape.column()),
        );
        // The server's dimension type gives the level's facts (what it does to fluids, mobs,
        // beds and time), not only its shape: a level of a mod the server runs is no
        // longer taken for the Overworld.
        let mut facts = None;
        let mut environment = None;
        if let Some((name, json)) = self.registries.dimension_json(spawn.dimension_type) {
            if !matches!(
                spawn.dimension.as_str(),
                "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
            ) {
                environment = Environment::with_type(base, &json);
            }
            let cached = self
                .facts_cache
                .get(&spawn.dimension)
                .filter(|(old_name, old, _)| *old_name == name && *old == json)
                .map(|(_, _, facts)| facts.clone());
            facts = cached.or_else(|| {
                let base = DimensionFacts::base_for_type(&name);
                let facts = (|| {
                    for field in ["coordinate_scale", "ambient_light"] {
                        if json.get(field).is_some()
                            && !json[field].as_f64().is_some_and(f64::is_finite)
                        {
                            return Err(format!("invalid {field}"));
                        }
                    }
                    if json.get("monster_spawn_block_light_limit").is_some()
                        && json["monster_spawn_block_light_limit"].as_i64().is_none()
                    {
                        return Err("invalid monster_spawn_block_light_limit".into());
                    }
                    DimensionFacts::patched(base, &name, &json)
                })()
                .map_err(|why| log::warn!("the server's dimension type {name} is refused ({why})"))
                .unwrap_or_else(|_| base.clone());
                if !sane_facts(&facts) {
                    log::warn!(
                        "the server's dimension type {name} is refused (a value out of range)"
                    );
                    return Some(Arc::new(base.clone()));
                }
                let facts = Arc::new(facts);
                self.facts_cache.insert(
                    spawn.dimension.clone(),
                    (name.clone(), json.clone(), facts.clone()),
                );
                Some(facts)
            });
        }
        LevelFormat {
            dimension,
            shape,
            chunk_format,
            facts,
            environment,
        }
    }
}

/// Whether a synchronized dimension type's values are inside the codec's ranges
/// (`DimensionType.java`: a coordinate scale of `1.0E-5F` to `3.0E7`, a monster spawn block light
/// limit of 0 to 15; the ambient light has no range), so a portal's ratio of two scales is finite.
fn sane_facts(facts: &DimensionFacts) -> bool {
    (f64::from(1.0e-5_f32)..=3.0e7).contains(&facts.coordinate_scale)
        && (0..=15).contains(&facts.monster_spawn_block_light_limit)
        && facts.ambient_light.is_finite()
}

#[cfg(test)]
mod tests {
    use super::sane_facts;
    use horizon_world::dimension_type::DimensionFacts;

    /// A type outside the codec's ranges is not taken; the vanilla ones are.
    #[test]
    fn a_type_outside_the_codecs_ranges_is_refused() {
        let mut facts = DimensionFacts::overworld().clone();
        assert!(sane_facts(&facts));
        assert!(sane_facts(DimensionFacts::of_builtin_level(
            horizon_core::LevelKey::NETHER
        )));
        for scale in [0.0, f64::from_bits(1), -1.0, 3.1e7, f64::NAN] {
            facts.coordinate_scale = scale;
            assert!(!sane_facts(&facts), "{scale}");
        }
        facts.coordinate_scale = 1.0;
        // the codec sets no range on the ambient light
        facts.ambient_light = 1.5;
        assert!(sane_facts(&facts));
        facts.monster_spawn_block_light_limit = 16;
        assert!(!sane_facts(&facts));
    }
}
