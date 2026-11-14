//! One read surface for native and guest equipment predicates.
use crate::{EntityRef, EntityTypeKey, EquipReadRow, EquipmentSlot, ItemStack, LivingRef};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
// no wire: process-local indices over a scoped immutable read snapshot.
pub struct EquipReadSet {
    rows: Vec<EquipReadRow>,
    indices: HashMap<EntityRef, usize>,
}
impl EquipReadSet {
    pub fn row(&self, entity: EntityRef) -> Option<&EquipReadRow> {
        self.rows.get(*self.indices.get(&entity)?)
    }
    pub fn new(rows: Vec<EquipReadRow>) -> Self {
        let indices = rows
            .iter()
            .enumerate()
            .map(|(i, row)| (row.entity, i))
            .collect();
        Self { rows, indices }
    }
}
struct Active {
    set: Rc<EquipReadSet>,
    index: usize,
}
thread_local! { static ACTIVE:RefCell<Option<Active>>=const { RefCell::new(None) }; }
// no wire: a call-scoped transport guard, never serialized.
pub struct EquipReadGuard(Option<Active>);
pub fn enter_equip_reads(set: Rc<EquipReadSet>, entity: EntityRef) -> EquipReadGuard {
    let active = set
        .indices
        .get(&entity)
        .copied()
        .map(|index| Active { set, index });
    EquipReadGuard(ACTIVE.with(|old| old.replace(active)))
}
impl Drop for EquipReadGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.replace(self.0.take());
        });
    }
}
fn read<R>(entity: EntityRef, f: impl FnOnce(&EquipReadRow) -> R) -> Option<R> {
    ACTIVE.with(|active| {
        let active = active.borrow();
        let active = active.as_ref()?;
        let row = &active.set.rows[active.index];
        (row.entity == entity).then(|| f(row))
    })
}
pub fn cached_equip_kind(entity: EntityRef) -> Option<Option<EntityTypeKey>> {
    read(entity, |row| row.kind)
}
pub fn cached_equip_position(entity: EntityRef) -> Option<Option<[f64; 3]>> {
    read(entity, |row| row.position)
}
pub fn cached_equip_health(entity: LivingRef) -> Option<Option<f32>> {
    read(entity.entity(), |row| row.health)
}
pub fn cached_equip_max_health(entity: LivingRef) -> Option<Option<f32>> {
    read(entity.entity(), |row| row.max_health)
}
pub fn cached_equip_alive(entity: LivingRef) -> Option<Option<bool>> {
    read(entity.entity(), |row| row.alive)
}
pub fn cached_equip_living(entity: EntityRef) -> Option<Option<LivingRef>> {
    read(entity, |row| row.living.then_some(LivingRef::new(entity)))
}
pub fn cached_equip_equipment(entity: LivingRef, slot: EquipmentSlot) -> Option<Option<ItemStack>> {
    let index = match slot {
        EquipmentSlot::MainHand => 0,
        EquipmentSlot::OffHand => 1,
        EquipmentSlot::Feet => 2,
        EquipmentSlot::Legs => 3,
        EquipmentSlot::Chest => 4,
        EquipmentSlot::Head => 5,
        EquipmentSlot::Body => 6,
        EquipmentSlot::Saddle => 7,
    };
    read(entity.entity(), |row| row.equipment[index].clone())
}

pub fn cached_equip_hand_item(entity: LivingRef, hand: crate::Hand) -> Option<Option<ItemStack>> {
    cached_equip_equipment(
        entity,
        match hand {
            crate::Hand::Main => EquipmentSlot::MainHand,
            crate::Hand::Off => EquipmentSlot::OffHand,
        },
    )
}
