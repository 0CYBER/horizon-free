//! Explosion resistance reads for server mods: what a blast a mod could set off would meet in one
//! cell or many, by the rule its rays use (`Game::cell_explosion_resistance`), without exploding
//! anything.
use super::{Game, explosion::PartAnswers};
use horizon_client::mobs::DamageKind;
use mod_api::{
    EntityRef, ExplosionInteraction, ExplosionResistance, LevelRef, ResistanceCells,
    ResistanceCellsError,
};

impl Game<'_> {
    /// The resistance the blast `level.explode(source, center, radius, fire, interaction)` would
    /// meet in the cell at `pos`. `Unavailable` for a stale level or source, a chunk that isn't
    /// loaded or an unknown state.
    pub(super) fn level_explosion_resistance(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Option<ExplosionResistance> {
        let blast = Blast {
            source,
            center,
            radius,
            fire,
            interaction,
        };
        Some(self.explosion_resistances_of(level, [pos].into_iter(), 1, &blast, None)[0])
    }

    /// `level_explosion_resistance` for every cell of `cells`, in its order. Refused, with
    /// nothing read, for an inverted area or more than `ResistanceCells::MAX` cells.
    pub(super) fn level_explosion_resistances(
        &mut self,
        level: LevelRef,
        cells: &ResistanceCells,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Result<Vec<ExplosionResistance>, ResistanceCellsError> {
        let count = cell_count(cells)?;
        let blast = Blast {
            source,
            center,
            radius,
            fire,
            interaction,
        };
        let mut answers = PartAnswers::default();
        let started = std::time::Instant::now();
        let out = match cells {
            ResistanceCells::Box { min, max } => {
                let positions = (min[1]..=max[1]).flat_map(move |y| {
                    (min[2]..=max[2]).flat_map(move |z| (min[0]..=max[0]).map(move |x| [x, y, z]))
                });
                self.explosion_resistances_of(level, positions, count, &blast, Some(&mut answers))
            }
            ResistanceCells::Columns {
                columns,
                min_y,
                max_y,
            } => {
                let (min_y, max_y) = (*min_y, *max_y);
                let positions = columns
                    .iter()
                    .flat_map(move |&(x, z)| (min_y..=max_y).map(move |y| [x, y, z]));
                self.explosion_resistances_of(level, positions, count, &blast, Some(&mut answers))
            }
            _ => unreachable!("ResistanceCells: unhandled variant"),
        };
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!("Horizon: slow mod explosion resistance read {ms:.1} ms ({count} cells)");
        }
        Ok(out)
    }

    /// The resistance `blast` meets in each of the `count` cells of `positions`. The blast's
    /// source and cause are set as `native_explode` sets them while the cells are read, and the
    /// enclosing blast's are put back after.
    fn explosion_resistances_of(
        &mut self,
        level: LevelRef,
        positions: impl Iterator<Item = [i32; 3]>,
        count: usize,
        blast: &Blast,
        mut answers: Option<&mut PartAnswers>,
    ) -> Vec<ExplosionResistance> {
        let unavailable = || vec![ExplosionResistance::Unavailable; count];
        if !self.level_in_place(level) {
            return unavailable();
        }
        let source = match blast.source {
            Some(entity) => match self.native_explosion_source(entity) {
                Some(source) => Some(source),
                None => return unavailable(),
            },
            None => None,
        };
        let id = source.map(|(id, _)| id);
        let interaction = self.native_blast_interaction(blast.interaction, id);
        let outer = self.take_blast_state();
        if let Some((id, target)) = source {
            self.attribute_native_blast(id, target);
            self.native_blast_calculator(id, target);
        }
        // The calculator `level.explode` would have: the same selection, read the same way.
        let rules = self.blast_rules(None, false);
        let damage_type =
            super::creatures::damage_type(DamageKind::Explosion, self.blast_attacker().is_some());
        let start = self.explosion_start(
            blast.center,
            blast.radius,
            interaction,
            blast.fire,
            damage_type,
        );
        let (min_y, max_y) = (self.world.bottom_y(), self.world.top_y());
        let mut out = Vec::with_capacity(count);
        for pos in positions {
            // Reads never load a column.
            if !self.world.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4)) {
                out.push(ExplosionResistance::Unavailable);
                continue;
            }
            // Above and below the level is void air: no resistance.
            if pos[1] < min_y || pos[1] >= max_y {
                out.push(ExplosionResistance::NoResistance);
                continue;
            }
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if self.pack.assets.registry.state(state).is_none() {
                out.push(ExplosionResistance::Unavailable);
                continue;
            }
            let (resistance, _) =
                self.cell_explosion_resistance(pos, &rules, &start, answers.as_deref_mut());
            out.push(resistance.map_or(
                ExplosionResistance::NoResistance,
                ExplosionResistance::Resists,
            ));
        }
        self.restore_blast_state(outer);
        out
    }
}

/// The blast a read describes, as `LevelRef::explode` takes it.
struct Blast {
    source: Option<EntityRef>,
    center: [f64; 3],
    radius: f32,
    fire: bool,
    interaction: ExplosionInteraction,
}

/// How many cells `cells` covers, or why it is refused.
fn cell_count(cells: &ResistanceCells) -> Result<usize, ResistanceCellsError> {
    let span = |low: i32, high: i32| {
        if high < low {
            Err(ResistanceCellsError::Inverted)
        } else {
            Ok(u64::from(high.abs_diff(low)) + 1)
        }
    };
    let count = match cells {
        ResistanceCells::Box { min, max } => span(min[0], max[0])?
            .saturating_mul(span(min[1], max[1])?)
            .saturating_mul(span(min[2], max[2])?),
        ResistanceCells::Columns {
            columns,
            min_y,
            max_y,
        } => (columns.len() as u64).saturating_mul(span(*min_y, *max_y)?),
        _ => unreachable!("ResistanceCells: unhandled variant"),
    };
    if count > ResistanceCells::MAX {
        return Err(ResistanceCellsError::TooMany(count));
    }
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_counts_its_cells_and_refuses_past_the_cap() {
        let cells = |min: [i32; 3], max: [i32; 3]| cell_count(&ResistanceCells::Box { min, max });
        assert_eq!(cells([0, 0, 0], [0, 0, 0]), Ok(1));
        assert_eq!(cells([-1, 60, -1], [1, 61, 1]), Ok(18));
        assert_eq!(cells([0, 0, 0], [63, 63, 63]), Ok(262_144));
        assert_eq!(
            cells([0, 0, 0], [63, 63, 64]),
            Err(ResistanceCellsError::TooMany(266_240))
        );
        assert_eq!(
            cells([0, 5, 0], [0, 4, 0]),
            Err(ResistanceCellsError::Inverted)
        );
        assert_eq!(
            cells([i32::MIN; 3], [i32::MAX; 3]),
            Err(ResistanceCellsError::TooMany(u64::MAX))
        );
        let columns = ResistanceCells::Columns {
            columns: vec![(0, 0), (0, 0), (5, -3)],
            min_y: -64,
            max_y: 319,
        };
        assert_eq!(cell_count(&columns), Ok(1152));
    }
}
