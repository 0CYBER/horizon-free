//! `CarvedPumpkinBlock.trySpawnGolem`: a carved pumpkin on top of snow
//! blocks, an iron T or a copper block brings a golem to life. The block
//! patterns match in every orientation, like vanilla's `BlockPattern`.
use super::{LevelAccess, LevelEvent, Sim, update::rel};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos};

/// The blocks of a matched pattern, rows top to bottom.
struct Match {
    cells: Vec<Vec<BlockPos>>,
}
impl Match {
    fn at(&self, col: usize, row: usize) -> BlockPos {
        self.cells[row][col]
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Body {
    Snow,
    Iron,
    Copper,
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn is_pumpkin(&self, state: BlockStateId) -> bool {
        matches!(
            self.b.table.name(state),
            "minecraft:carved_pumpkin" | "minecraft:jack_o_lantern"
        )
    }
    fn is_golem_body(&self, state: BlockStateId, body: Body) -> bool {
        match body {
            Body::Snow => self.b.table.name(state) == "minecraft:snow_block",
            Body::Iron => self.b.table.name(state) == "minecraft:iron_block",
            Body::Copper => self
                .b
                .tag("minecraft:copper")
                .is_some_and(|set| self.b.in_set(state, set)),
        }
    }
    /// `BlockPattern.find` for a pattern whose only pumpkin is `top`:
    /// `rows` read top to bottom, `^` the pumpkin, `#` the body, `~` air.
    fn find_golem(&self, rows: &[&str], body: Body, top: BlockPos) -> Option<Match> {
        let pumpkin_col = rows[0].find('^')? as i32;
        for up in Direction::ALL {
            for finger in Direction::ALL {
                if finger == up || finger == up.opposite() {
                    continue;
                }
                let step = |origin: BlockPos, col: i32, row: i32| {
                    let mut p = origin;
                    for _ in 0..col {
                        p = rel(p, finger);
                    }
                    for _ in 0..row {
                        p = rel(p, up.opposite());
                    }
                    p
                };
                let mut origin = top;
                for _ in 0..pumpkin_col {
                    origin = rel(origin, finger.opposite());
                }
                let mut cells = Vec::new();
                let mut ok = true;
                'rows: for (r, row) in rows.iter().enumerate() {
                    let mut line = Vec::new();
                    for (c, ch) in row.chars().enumerate() {
                        let pos = step(origin, c as i32, r as i32);
                        let state = self.state(pos);
                        let good = match ch {
                            '^' => self.is_pumpkin(state),
                            '#' => self.is_golem_body(state, body),
                            '~' => self.b.is_air(state),
                            _ => true,
                        };
                        if !good {
                            ok = false;
                            break 'rows;
                        }
                        line.push(pos);
                    }
                    cells.push(line);
                }
                if ok {
                    return Some(Match { cells });
                }
            }
        }
        None
    }
    fn clear_golem_blocks(&mut self, found: &Match) {
        let air = self.b.default_of(self.b.named.air);
        for row in &found.cells {
            for &pos in row {
                let old = self.state(pos);
                self.set_block(pos, air, 2);
                self.level_event(2001, pos, old.0 as i32);
            }
        }
    }
    fn update_golem_blocks(&mut self, found: &Match) {
        let air = self.b.named.air;
        for row in &found.cells {
            for &pos in row {
                self.update_neighbors_at(pos, air);
            }
        }
    }
    /// Called when a carved pumpkin or jack o'lantern is placed at `top`.
    pub(crate) fn carved_pumpkin_try_spawn_golem(&mut self, top: BlockPos) {
        if let Some(found) = self.find_golem(&["^", "#", "#"], Body::Snow, top) {
            let spawn = found.at(0, 2);
            let old = self.state(spawn);
            self.clear_golem_blocks(&found);
            self.emit(LevelEvent::BlockEvent {
                pos: spawn,
                state: old,
                kind: "spawn_snow_golem",
            });
            self.update_golem_blocks(&found);
            return;
        }
        if let Some(found) = self.find_golem(&["~^~", "###", "~#~"], Body::Iron, top) {
            let spawn = found.at(1, 2);
            let old = self.state(spawn);
            self.clear_golem_blocks(&found);
            self.emit(LevelEvent::BlockEvent {
                pos: spawn,
                state: old,
                kind: "spawn_iron_golem",
            });
            self.update_golem_blocks(&found);
            return;
        }
        if let Some(found) = self.find_golem(&["^", "#"], Body::Copper, top) {
            let pumpkin = self.state(found.at(0, 0));
            let copper_pos = found.at(0, 1);
            let copper = self.state(copper_pos);
            let facing = self.b.facing(pumpkin);
            self.clear_golem_blocks(&found);
            self.emit(LevelEvent::BlockEvent {
                pos: found.at(0, 0),
                state: copper,
                kind: "spawn_copper_golem",
            });
            self.update_golem_blocks(&found);
            // CopperChestBlock.getFromCopperBlock.
            let name = self.b.table.name(copper).trim_start_matches("minecraft:");
            let chest = match name.strip_suffix("_block") {
                Some(stem) => format!("{stem}_chest"),
                None => format!("{name}_chest"),
            };
            if let Some(id) = self.b.table.block_by_name(&format!("minecraft:{chest}")) {
                let state = self.b.default_of(id);
                let state = self.b.table.set_dir(state, self.b.props.facing, facing);
                self.set_block(copper_pos, state, 2);
            }
        }
    }
}
