//! Engine owners of detached placement and publication use the same frozen plan.
use super::{Kind, Menu, custom_plan::MenuPlacement};
impl MenuPlacement {
    /// Input supply cells; fuel and real machine output are not input stock.
    pub fn input_slots(&self) -> &[usize] {
        &self.inputs
    }
    /// Physical slots whose old contents the placement operation may return.
    pub fn cleared_slots(&self) -> &[usize] {
        &self.clear
    }
    /// Authoritative ingredient predicates, never display examples.
    pub fn requirements(&self) -> &crate::crafting::Placement {
        &self.placement
    }
}
impl Menu {
    /// Rebuild only visibility from the actual current mod menu. A cached
    /// machine may have hidden slots left over from an earlier linked view.
    pub fn reset_mod_visibility(&mut self, hidden: &[usize]) {
        self.touch();
        if matches!(self.state.kind, Kind::Mod(_)) {
            self.state.slots = Menu::new(self.state.kind).state.slots;
            for &slot in hidden {
                self.hide_slot(slot);
            }
        }
    }
}
