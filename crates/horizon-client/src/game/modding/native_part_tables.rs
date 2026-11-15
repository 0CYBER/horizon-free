//! The client `BlockPart` answers the fluid mesher reads (`mesher::part_tables`), resolved by the
//! per-state resolver where `refresh_render_visibility` resolves the hidden states, so meshing
//! reads tables and never a mod.

use super::{BlockStateId, Game, native_world_read::api_fluid_state};
use horizon_client::mesher::part_tables::{FluidTable, PartTables};
use horizon_core::geometry::Direction;
use horizon_world::fluid::{FluidKind, FluidState, mod_fluids};
use mod_api::HookId;

impl Game<'_> {
    /// Every table, built for the states and fluids of this game; a table that changes nothing
    /// is left empty. The overlay answer starts with vanilla's test, the mesher's `Info::overlay`.
    pub(super) fn resolve_part_tables(&self) -> PartTables {
        let states = self.pack.assets.registry.len();
        let fluids = asked_fluids();
        let hidden_fluid_faces = Self::resolve_per_state_fluid(
            HookId::ShouldHideAdjacentFluidFace,
            states,
            fluids.len(),
            |state, fluid| {
                let Some(fluid) = fluids[fluid] else {
                    return 0;
                };
                Direction::ALL
                    .into_iter()
                    .filter(|&direction| {
                        let mut hidden = false;
                        self.native_should_hide_adjacent_fluid_face(
                            state,
                            direction,
                            fluid,
                            &mut hidden,
                        );
                        hidden
                    })
                    .fold(0, |faces, direction| faces | 1 << direction as u8)
            },
        )
        .filter(|faces| faces.iter().any(|&faces| faces != 0))
        .map_or(FluidTable::EMPTY, |faces| {
            FluidTable::new(fluids.len(), faces)
        });
        let tables = &self.pack.context.tables;
        let vanilla = |state: BlockStateId| tables.vanilla_fluid_overlay(state.0);
        let fluid_overlay = Self::resolve_per_state_fluid(
            HookId::ShouldDisplayFluidOverlay,
            states,
            fluids.len(),
            |state, fluid| {
                let mut overlay = vanilla(state);
                if let Some(fluid) = fluids[fluid] {
                    self.native_should_display_fluid_overlay(state, fluid, &mut overlay);
                }
                overlay
            },
        )
        .filter(|overlay| {
            (overlay.chunks(fluids.len()).enumerate()).any(|(state, row)| {
                row.iter()
                    .any(|&o| o != vanilla(BlockStateId(state as u32)))
            })
        })
        .map_or(FluidTable::EMPTY, |overlay| {
            FluidTable::new(fluids.len(), overlay)
        });
        PartTables {
            hidden_fluid_faces,
            fluid_overlay,
        }
    }
}

/// By [`FluidKind::tick_id`], each fluid as the tables ask about it: a source at amount 8, a
/// flowing fluid at amount 7 and not falling (the answer may not depend on either). `None` for
/// the empty fluid, which draws nothing and is never asked about.
fn asked_fluids() -> Vec<Option<mod_api::FluidState>> {
    let count = FluidKind::ALL.len() + 2 * mod_fluids().len();
    (0..count as u32)
        .map(|id| {
            let kind = FluidKind::from_tick_id(id).filter(|&kind| kind != FluidKind::Empty)?;
            let source = matches!(kind, FluidKind::Water | FluidKind::Lava | FluidKind::Mod(_));
            api_fluid_state(FluidState::new(kind, if source { 8 } else { 7 }, false))
        })
        .collect()
}
