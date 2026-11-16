use super::{Game, to_loot};

impl Game<'_> {
    /// The killing player as a loot entity: their worn and held items, read
    /// from the guest who struck the last blow when a server's player did
    /// (on the server thread that is every player, the window's too), so
    /// Looting on their weapon counts.
    pub(in crate::game) fn killer_loot_entity(
        &mut self,
        guest: Option<u32>,
    ) -> horizon_world::loot::LootEntity {
        let own = |g: &Self| {
            let mut e = horizon_world::loot::LootEntity::new("minecraft:player");
            let slots = [
                ("mainhand", g.stack_in(g.selected)),
                ("offhand", g.stack_in(crate::game::shield::OFFHAND_SLOT)),
                ("head", g.armor[0].as_ref()),
                ("chest", g.armor[1].as_ref()),
                ("legs", g.armor[2].as_ref()),
                ("feet", g.armor[3].as_ref()),
            ];
            for (slot, stack) in slots {
                if let Some(stack) = stack {
                    e.equipment.push((slot.into(), to_loot(stack)));
                }
            }
            e
        };
        if let Some(conn) = guest.filter(|_| self.lan_acting.is_none())
            && let Some(entity) = self.as_guest(conn, |g| own(g))
        {
            return entity;
        }
        own(self)
    }
}
