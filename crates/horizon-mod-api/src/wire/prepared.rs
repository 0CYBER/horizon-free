//! Decision-scoped guest ownership for prepared borrowed arguments.
use crate::wire::WireError;
use std::rc::Rc;

// no wire: guest-local storage for prepared arguments; only its slot tokens cross the boundary.
pub struct PreparedSlots<T> {
    slots: Vec<Option<(u32, Rc<T>)>>,
}
impl<T> Default for PreparedSlots<T> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}
impl<T> PreparedSlots<T> {
    pub fn prepare(&mut self, slot: u32, generation: u32, value: T) -> Result<(), WireError> {
        let slot = slot as usize;
        if generation == 0 || slot >= 4096 {
            return Err(WireError::Invalid("invalid prepared slot"));
        }
        if self.slots.get(slot).is_some_and(Option::is_some) {
            return Err(WireError::Invalid("prepared slot already occupied"));
        }
        self.slots
            .resize_with(self.slots.len().max(slot + 1), || None);
        self.slots[slot] = Some((generation, Rc::new(value)));
        Ok(())
    }
    pub fn borrow(&self, slot: u32, generation: u32) -> Result<Rc<T>, WireError> {
        self.slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .filter(|(current, _)| *current == generation)
            .map(|(_, value)| Rc::clone(value))
            .ok_or(WireError::Invalid("stale or foreign prepared token"))
    }
    pub fn release(&mut self, slot: u32, generation: u32) -> Result<(), WireError> {
        self.borrow(slot, generation)?;
        self.slots[slot as usize] = None;
        Ok(())
    }
}
