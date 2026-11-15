//! The answers of client `BlockPart` methods the mesher reads, one table per method. The game's
//! per-state resolver builds them on the client thread, beside `WorkerContext::hidden_states`, so
//! the meshing workers read tables and run no mod code. A table is empty when no part subscribes,
//! and the mesher then asks vanilla's tests alone: meshes are the same as with no mod.

use horizon_core::geometry::Direction;
use horizon_world::fluid::FluidKind;

/// One answer per (block state, fluid) pair, row by row: the answer for `state` and the fluid
/// whose [`FluidKind::tick_id`] is `fluid` is at `state * fluids + fluid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidTable<T> {
    fluids: usize,
    answers: Vec<T>,
}
impl<T: Copy> FluidTable<T> {
    /// No table: every lookup is `None`.
    pub const EMPTY: Self = Self {
        fluids: 0,
        answers: Vec::new(),
    };
    /// A table of `fluids` answers per state, as the per-state resolver lays them out.
    pub fn new(fluids: usize, answers: Vec<T>) -> Self {
        assert!(
            fluids > 0 && answers.len().is_multiple_of(fluids),
            "a fluid table has a whole row per state"
        );
        Self { fluids, answers }
    }
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }
    /// The answer for `state` and `fluid`; `None` with no table, or for a state or fluid the
    /// table was built without (one added since).
    pub fn get(&self, state: u32, fluid: FluidKind) -> Option<T> {
        let fluid = fluid.tick_id() as usize;
        if fluid >= self.fluids {
            return None;
        }
        self.answers
            .get(state as usize * self.fluids + fluid)
            .copied()
    }
}
impl<T: Copy> Default for FluidTable<T> {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// The tables, as `WorkerContext::part_tables` holds them and `Mesher::part_tables` reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartTables {
    /// `should_hide_adjacent_fluid_face`: by the state beside a fluid and that fluid, bit
    /// `1 << direction` set when the state hides the face the fluid draws toward it,
    /// `direction` pointing from the fluid to the block.
    pub hidden_fluid_faces: FluidTable<u8>,
    /// `should_display_fluid_overlay`: by the state beside a fluid and that fluid, whether the
    /// fluid's side toward the state uses the fluid's overlay sprite.
    pub fluid_overlay: FluidTable<bool>,
}
impl PartTables {
    /// No tables.
    pub const NONE: Self = Self {
        hidden_fluid_faces: FluidTable::EMPTY,
        fluid_overlay: FluidTable::EMPTY,
    };
    /// Whether every table is empty, as with no subscriber.
    pub fn is_empty(&self) -> bool {
        self.hidden_fluid_faces.is_empty() && self.fluid_overlay.is_empty()
    }
    /// Whether `state`, beside `fluid` in `direction` from it, hides that face of the fluid.
    pub fn hides_fluid_face(&self, state: u32, fluid: FluidKind, direction: Direction) -> bool {
        self.hidden_fluid_faces
            .get(state, fluid)
            .is_some_and(|faces| faces & 1 << direction as u8 != 0)
    }
    /// Whether `fluid`'s side toward `state` uses the overlay sprite, `vanilla` (the state's
    /// `HalfTransparentBlock`/`LeavesBlock` test) when no part answered.
    pub fn fluid_overlay(&self, state: u32, fluid: FluidKind, vanilla: bool) -> bool {
        self.fluid_overlay.get(state, fluid).unwrap_or(vanilla)
    }
}
impl super::MeshTables {
    /// Vanilla's overlay test for a fluid's side beside `state` (`HalfTransparentBlock` or
    /// `LeavesBlock`), where `should_display_fluid_overlay` starts.
    pub fn vanilla_fluid_overlay(&self, state: u32) -> bool {
        self.info
            .get(state as usize)
            .is_some_and(|info| info.overlay)
    }
}
/// No tables, for a mesher built without a worker context.
impl Default for &PartTables {
    fn default() -> Self {
        static NONE: PartTables = PartTables::NONE;
        &NONE
    }
}
