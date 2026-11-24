//! [`MobSlot`]: a mob a list hands out.
use super::Mob;

/// A mob a list hands out: the list's own, or a borrow of one in a larger list (a group of the
/// server's parallel mob loop, `game/mob_islands.rs`, works on its mobs in place).
pub trait MobSlot {
    fn mob(&self) -> &Mob;
    fn mob_mut(&mut self) -> &mut Mob;
}
impl MobSlot for Mob {
    #[inline]
    fn mob(&self) -> &Mob {
        self
    }
    #[inline]
    fn mob_mut(&mut self) -> &mut Mob {
        self
    }
}
impl MobSlot for &mut Mob {
    #[inline]
    fn mob(&self) -> &Mob {
        self
    }
    #[inline]
    fn mob_mut(&mut self) -> &mut Mob {
        self
    }
}

/// A list of mobs that can be searched and edited in place without reordering.
pub trait MobSlots {
    fn iter_mobs(&self) -> impl Iterator<Item = &Mob>;
    fn mob_by_id_mut(&mut self, id: u32) -> Option<&mut Mob>;
}
impl<M: MobSlot> MobSlots for [M] {
    #[inline]
    fn iter_mobs(&self) -> impl Iterator<Item = &Mob> {
        self.iter().map(MobSlot::mob)
    }
    #[inline]
    fn mob_by_id_mut(&mut self, id: u32) -> Option<&mut Mob> {
        self.iter_mut()
            .find(|m| m.mob().id == id)
            .map(MobSlot::mob_mut)
    }
}
impl<M: MobSlot> MobSlots for Vec<M> {
    #[inline]
    fn iter_mobs(&self) -> impl Iterator<Item = &Mob> {
        self.as_slice().iter_mobs()
    }
    #[inline]
    fn mob_by_id_mut(&mut self, id: u32) -> Option<&mut Mob> {
        self.as_mut_slice().mob_by_id_mut(id)
    }
}
impl MobSlots for crate::join_seq::JoinList<Mob> {
    #[inline]
    fn iter_mobs(&self) -> impl Iterator<Item = &Mob> {
        self.iter()
    }
    #[inline]
    fn mob_by_id_mut(&mut self, id: u32) -> Option<&mut Mob> {
        self.iter_mut().find(|m| m.id == id)
    }
}
