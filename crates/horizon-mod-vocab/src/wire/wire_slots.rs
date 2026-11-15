//! The table a resource crosses the boundary through: the value stays on the side that owns it
//! and the other side holds an [`Id`]. One table type serves every resource; nothing in it
//! knows what it holds.
//!
//! An id is a slot index and that slot's generation in one `u64`. Freeing a slot bumps its
//! generation, so an id kept past its value's life is [`Stale`], never another value and never
//! a panic. A value kept for one call is cleared by [`Slots::end_frame`]; one that outlives it
//! (a provider) is inserted with [`Slots::insert_long`].

// exhaustive: wasm boundary plumbing that generated code builds field by field
/// A resource's id: generation in the high half, slot in the low.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id(pub u64);

// exhaustive: wasm boundary plumbing that generated code builds field by field
/// The id names no live value: it was freed, its call ended, or it was never given out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: local slot-table lookup diagnostic.
pub struct Stale;

struct Slot<T> {
    generation: u32,
    value: Option<T>,
    /// Cleared when the call that put it there returns.
    framed: bool,
}

// exhaustive: wasm boundary plumbing that generated code builds field by field
/// A generational slot map. Generation zero is reserved for protocol control values;
/// exhausted generations retire their slot rather than wrapping and reviving an old id.
// no wire: process-local generational slot storage.
pub struct Slots<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    framed: Vec<u32>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            framed: Vec::new(),
        }
    }
}

impl<T> Slots<T> {
    fn put(&mut self, value: T, framed: bool) -> Id {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot {
                    generation: 1,
                    value: None,
                    framed: false,
                });
                (self.slots.len() - 1) as u32
            }
        };
        let slot = &mut self.slots[index as usize];
        slot.value = Some(value);
        slot.framed = framed;
        if framed {
            self.framed.push(index);
        }
        Id(u64::from(slot.generation) << 32 | u64::from(index))
    }

    /// A value that lives until the call returns.
    pub fn insert(&mut self, value: T) -> Id {
        self.put(value, true)
    }

    /// A value that lives until [`remove`](Self::remove).
    pub fn insert_long(&mut self, value: T) -> Id {
        self.put(value, false)
    }

    fn slot(&self, id: Id) -> Result<&Slot<T>, Stale> {
        self.slots
            .get((id.0 & 0xffff_ffff) as usize)
            .filter(|slot| slot.value.is_some() && u64::from(slot.generation) == id.0 >> 32)
            .ok_or(Stale)
    }

    /// The value, or [`Stale`].
    pub fn get(&self, id: Id) -> Result<&T, Stale> {
        Ok(self
            .slot(id)?
            .value
            .as_ref()
            .expect("a live slot holds a value"))
    }

    /// The value to change, or [`Stale`].
    pub fn get_mut(&mut self, id: Id) -> Result<&mut T, Stale> {
        self.slot(id)?;
        let slot = &mut self.slots[(id.0 & 0xffff_ffff) as usize];
        Ok(slot.value.as_mut().expect("a live slot holds a value"))
    }

    /// Frees the slot and returns its value; the id is stale from now on.
    pub fn remove(&mut self, id: Id) -> Result<T, Stale> {
        self.slot(id)?;
        let index = (id.0 & 0xffff_ffff) as u32;
        let slot = &mut self.slots[index as usize];
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(index);
        }
        Ok(slot.value.take().expect("a live slot holds a value"))
    }

    /// Frees every value of this call; long-lived ones stay.
    pub fn end_frame(&mut self) {
        for index in std::mem::take(&mut self.framed) {
            let slot = &mut self.slots[index as usize];
            if slot.framed && slot.value.is_some() {
                slot.value = None;
                if let Some(next) = slot.generation.checked_add(1) {
                    slot.generation = next;
                    self.free.push(index);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_zero_and_exhausted_generations_never_become_live() {
        for framed in [false, true] {
            let mut slots = Slots::default();
            let old = if framed {
                slots.insert("old")
            } else {
                slots.insert_long("old")
            };
            assert!(slots.get(Id(0)).is_err());
            // Reach the generation boundary without billions of release operations.
            let index = (old.0 & 0xffff_ffff) as usize;
            slots.slots[index].generation = u32::MAX;
            let last = Id(u64::from(u32::MAX) << 32 | index as u64);
            if framed {
                slots.end_frame()
            } else {
                assert_eq!(slots.remove(last), Ok("old"))
            }
            let next = slots.insert_long("next");
            assert!(slots.get(Id(0)).is_err());
            assert_eq!(slots.remove(next), Ok("next"));
            let live = slots.insert_long("live");
            assert!(slots.get(old).is_err());
            assert!(slots.get(last).is_err());
            assert_eq!(slots.get(live), Ok(&"live"));
        }
    }
    #[test]
    fn a_freed_id_is_stale_even_when_its_slot_is_reused() {
        let mut slots = Slots::default();
        let a = slots.insert_long("a");
        assert_eq!(slots.remove(a), Ok("a"));
        let b = slots.insert_long("b");
        assert_eq!(slots.get(a), Err(Stale));
        assert_eq!(slots.get(b), Ok(&"b"));
        assert_ne!(a, b);
    }

    #[test]
    fn a_call_ends_only_its_own_values() {
        let mut slots = Slots::default();
        let kept = slots.insert_long(1);
        let framed = slots.insert(2);
        slots.end_frame();
        assert_eq!(slots.get(framed), Err(Stale));
        assert_eq!(slots.get(kept), Ok(&1));
        assert_eq!(slots.remove(framed), Err(Stale));
    }

    #[test]
    fn invented_ids_are_stale() {
        let mut slots: Slots<u8> = Slots::default();
        slots.insert_long(0);
        // The first slot's live id is generation 1, slot 0 (`1 << 32`); generation 0 is reserved.
        for raw in [u64::MAX, 0, 2 << 32, 7, 1 << 40 | 1] {
            assert_eq!(slots.get(Id(raw)), Err(Stale));
            assert_eq!(slots.get_mut(Id(raw)), Err(Stale));
        }
    }
}
