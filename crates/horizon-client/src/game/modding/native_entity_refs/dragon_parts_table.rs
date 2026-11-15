//! Integer part registry, following the public fastutil javadoc-level description
//! of `Int2ObjectOpenHashMap` (https://fastutil.di.unimi.it/docs/).
//! Keys use multiplicative mixing, linear probing, backward-shift deletion, and
//! descending-slot iteration (with the zero key first). No Mojang implementation
//! is used here.
#[derive(Debug)]
pub(super) struct DragonPartsTable {
    slots: Vec<Option<u32>>,
    zero: bool,
    size: usize,
}
impl Default for DragonPartsTable {
    fn default() -> Self {
        Self {
            slots: vec![None; 32],
            zero: false,
            size: 0,
        }
    }
}
impl DragonPartsTable {
    fn home(key: u32, mask: usize) -> usize {
        let h = key.wrapping_mul(0x9e3779b9);
        ((h ^ (h >> 16)) as usize) & mask
    }
    pub(super) fn insert(&mut self, key: u32) {
        if key == 0 {
            if self.zero {
                return;
            }
            self.zero = true;
        } else {
            let mask = self.slots.len() - 1;
            let mut p = Self::home(key, mask);
            while let Some(old) = self.slots[p] {
                if old == key {
                    return;
                }
                p = (p + 1) & mask;
            }
            self.slots[p] = Some(key);
        }
        self.size += 1;
        if self.size >= self.slots.len() * 3 / 4 {
            let capacity = self.slots.len() * 2;
            let old = std::mem::replace(&mut self.slots, vec![None; capacity]);
            for key in old.into_iter().rev().flatten() {
                let mask = self.slots.len() - 1;
                let mut p = Self::home(key, mask);
                while self.slots[p].is_some() {
                    p = (p + 1) & mask;
                }
                self.slots[p] = Some(key);
            }
        }
    }
    pub(super) fn remove(&mut self, key: u32) {
        if key == 0 {
            if self.zero {
                self.zero = false;
                self.size -= 1;
            }
            return;
        }
        let mask = self.slots.len() - 1;
        let mut hole = Self::home(key, mask);
        loop {
            match self.slots[hole] {
                None => return,
                Some(k) if k == key => break,
                _ => hole = (hole + 1) & mask,
            }
        }
        self.size -= 1;
        let mut scan = (hole + 1) & mask;
        while let Some(k) = self.slots[scan] {
            let home = Self::home(k, mask);
            if (scan.wrapping_sub(home) & mask) >= (scan.wrapping_sub(hole) & mask) {
                self.slots[hole] = Some(k);
                hole = scan;
            }
            scan = (scan + 1) & mask;
        }
        self.slots[hole] = None;
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.zero
            .then_some(0)
            .into_iter()
            .chain(self.slots.iter().rev().filter_map(|k| *k))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // Literal orders were independently calculated from the specified mix/probe
    // algorithm, not obtained from DragonPartsTable. The removal calculation
    // reinserts the affected cluster, independently of our backward-shift code.
    #[test]
    fn one_dragon_descends_slots() {
        let mut t = DragonPartsTable::default();
        for k in 21..=28 {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [25, 27, 28, 24, 26, 23, 22, 21]
        );
    }
    #[test]
    fn two_dragons_collide() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38) {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 35, 25, 27, 37, 31, 32, 28, 34, 24, 38, 36, 26, 23, 22, 21
            ]
        );
    }
    #[test]
    fn removal_repairs_probe_chains() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38) {
            t.insert(k);
        }
        for k in 21..=28 {
            t.remove(k);
        }
        for k in 41..=48 {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 35, 37, 47, 48, 31, 32, 34, 46, 38, 36, 45, 44, 42, 41, 43
            ]
        );
    }
    #[test]
    fn growth_rehashes_in_descending_order() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38).chain(41..=48) {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 25, 37, 28, 24, 26, 23, 22, 21, 35, 27, 47, 48, 31, 32, 34, 36, 38, 46, 41, 45,
                44, 42, 43
            ]
        );
    }
    #[test]
    fn zero_is_first_and_removed() {
        let mut t = DragonPartsTable::default();
        t.insert(21);
        t.insert(0);
        t.insert(0);
        assert_eq!(t.iter().collect::<Vec<_>>(), [0, 21]);
        t.remove(0);
        assert_eq!(t.iter().collect::<Vec<_>>(), [21]);
    }
}

/// The table travels with the dimension-owned singleton, including actor loans.
#[derive(Default)]
pub(in crate::game) struct DragonSlot {
    slot: horizon_client::join_seq::ResidentSlot<crate::game::dragon::Dragon>,
    table: std::cell::RefCell<DragonPartsTable>,
    registered: std::cell::Cell<u32>,
}
impl std::ops::Deref for DragonSlot {
    type Target = horizon_client::join_seq::ResidentSlot<crate::game::dragon::Dragon>;
    fn deref(&self) -> &Self::Target {
        &self.slot
    }
}
impl std::ops::DerefMut for DragonSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slot
    }
}
impl DragonSlot {
    fn register(&self, id: u32) {
        let old = self.registered.replace(id);
        if old == id {
            return;
        }
        let mut table = self.table.borrow_mut();
        if old != 0 {
            for part in old + 1..=old + 8 {
                table.remove(part);
            }
        }
        if id != 0 {
            for part in id + 1..=id + 8 {
                table.insert(part);
            }
        }
    }
    pub(super) fn parts_table(&self) -> std::cell::Ref<'_, DragonPartsTable> {
        self.table.borrow()
    }
    #[cfg(test)]
    fn part_ids(&self) -> Vec<u32> {
        self.table.borrow().iter().collect()
    }
    pub(in crate::game) fn replace(&mut self, dragon: crate::game::dragon::Dragon) {
        self.register(0);
        self.register(dragon.id);
        self.slot.replace(dragon);
    }
    // Preserve ResidentSlot's ownership-returning refusal contract without allocating.
    #[allow(clippy::result_large_err)]
    pub(in crate::game) fn install(
        &mut self,
        dragon: crate::game::dragon::Dragon,
    ) -> Result<(), crate::game::dragon::Dragon> {
        if self.slot.is_some() {
            return Err(dragon);
        }
        self.replace(dragon);
        Ok(())
    }
    pub(in crate::game) fn discard(&mut self) {
        self.register(0);
        self.slot.discard();
    }
    pub(in crate::game) fn id_assigned(&self) {
        self.register(self.slot.as_ref().map_or(0, |d| d.id));
        self.slot.id_assigned();
    }
    pub(in crate::game) fn retire_extracted(
        &self,
        id: u32,
        join: horizon_client::join_seq::JoinSeq,
    ) {
        if self.registered.get() == id {
            self.register(0);
        }
        self.slot.retire_extracted(id, join);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::game::Vec3;
    use crate::game::dragon::Dragon;
    #[test]
    fn dragon_slot_tracks_registration_not_temporary_loans() {
        let mut slot = DragonSlot::default();
        slot.replace(Dragon::new(Vec3::ZERO, 0.0, 0));
        assert!(slot.part_ids().is_empty());
        slot.as_mut().unwrap().id = 20;
        slot.id_assigned();
        assert_eq!(slot.part_ids(), [25, 27, 28, 24, 26, 23, 22, 21]);
        let dragon = slot.extract().unwrap();
        assert_eq!(slot.part_ids().len(), 8);
        slot.put_back(dragon);
        assert_eq!(slot.part_ids(), [25, 27, 28, 24, 26, 23, 22, 21]);
        slot.discard();
        assert!(slot.part_ids().is_empty());
    }
}
