//! The chunk tickets native mods hold through their ticket controllers
//! (NeoForge's `TicketController`): one ticket per (controller, owner, chunk),
//! so two machines in one chunk don't release
//! each other's. `Streaming` keeps the union with `/forceload`'s chunks in its
//! `forced` set, which is all loading and ticking read.
pub use mod_api::world::mod_chunk_tickets::{
    ControllerTickets, ModTicket, TicketOwner, TicketSet, TicketValidation,
};
use std::collections::{BTreeSet, HashMap};

/// A runtime-only ticket (`ChunkRef::set_transient_forced`): `controller`
/// holds `chunk` for one operation it names, such as finding a teleport
/// destination. Kept apart from [`ModTicket`], so it is never saved, never
/// validated and never listed by `mod_forced_chunks`; it ends with its level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransientTicket {
    pub controller: u32,
    pub operation: u128,
    pub chunk: (i32, i32),
    pub natural_spawning: bool,
}

/// The tickets of one level.
#[derive(Debug, Default)]
pub struct ModTickets {
    held: BTreeSet<ModTicket>,
    /// Runtime-only tickets, held alongside `held` but never saved.
    transient: BTreeSet<TransientTicket>,
    /// Read from the save, not yet validated: they hold nothing until
    /// `take_loaded` hands them out and the survivors are inserted.
    loaded: Vec<ModTicket>,
    /// Per chunk: how many tickets hold it, and how many of those force natural spawning.
    per_chunk: HashMap<(i32, i32), (u32, u32)>,
}

impl ModTickets {
    /// Tickets read from a save, whose controllers are registered (the save
    /// drops the others): they wait in the loaded set for
    /// validation and hold nothing yet.
    pub fn from_saved(saved: Vec<ModTicket>) -> Self {
        Self {
            loaded: saved,
            ..Self::default()
        }
    }
    /// Hold `ticket`: true when it was new.
    pub fn insert(&mut self, ticket: ModTicket) -> bool {
        let (chunk, spawning) = (ticket.chunk, ticket.natural_spawning);
        let new = self.held.insert(ticket);
        if new {
            self.count(chunk, spawning);
        }
        new
    }
    /// Release `ticket`: true when it was held.
    pub fn remove(&mut self, ticket: &ModTicket) -> bool {
        let held = self.held.remove(ticket);
        if held {
            self.uncount(ticket.chunk, ticket.natural_spawning);
        }
        held
    }
    /// Hold a runtime-only `ticket`: true when it was new.
    pub fn insert_transient(&mut self, ticket: TransientTicket) -> bool {
        let new = self.transient.insert(ticket);
        if new {
            self.count(ticket.chunk, ticket.natural_spawning);
        }
        new
    }
    /// Release a runtime-only `ticket`: true when it was held.
    pub fn remove_transient(&mut self, ticket: &TransientTicket) -> bool {
        let held = self.transient.remove(ticket);
        if held {
            self.uncount(ticket.chunk, ticket.natural_spawning);
        }
        held
    }
    fn count(&mut self, chunk: (i32, i32), spawning: bool) {
        let counts = self.per_chunk.entry(chunk).or_default();
        counts.0 += 1;
        counts.1 += u32::from(spawning);
    }
    fn uncount(&mut self, chunk: (i32, i32), spawning: bool) {
        if let Some(counts) = self.per_chunk.get_mut(&chunk) {
            counts.0 -= 1;
            counts.1 -= u32::from(spawning);
            if counts.0 == 0 {
                self.per_chunk.remove(&chunk);
            }
        }
    }
    /// Whether any mod holds `chunk`.
    pub fn holds(&self, chunk: (i32, i32)) -> bool {
        self.per_chunk.contains_key(&chunk)
    }
    /// Whether a ticket that forces natural spawning holds `chunk`
    /// (`TicketStorage.shouldForceNaturalSpawning`).
    pub fn forces_natural_spawning(&self, chunk: (i32, i32)) -> bool {
        self.per_chunk
            .get(&chunk)
            .is_some_and(|counts| counts.1 > 0)
    }
    /// The chunks any mod holds.
    pub fn chunks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.per_chunk.keys().copied()
    }
    /// The loaded tickets still to validate, taken out of the store.
    pub fn take_loaded(&mut self) -> Vec<ModTicket> {
        std::mem::take(&mut self.loaded)
    }
    /// Whether tickets loaded from the save wait for validation.
    pub fn has_loaded(&self) -> bool {
        !self.loaded.is_empty()
    }
    /// The tickets `controller` holds, in order.
    pub fn of_controller(&self, controller: u32) -> impl Iterator<Item = &ModTicket> {
        self.held.iter().filter(move |t| t.controller == controller)
    }
    /// Every ticket to save, the loaded ones not yet validated included.
    pub fn to_save(&self) -> impl Iterator<Item = &ModTicket> {
        self.held.iter().chain(&self.loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(controller: u32, owner: TicketOwner, chunk: (i32, i32)) -> ModTicket {
        ModTicket {
            controller,
            owner,
            chunk,
            natural_spawning: false,
        }
    }

    #[test]
    fn a_chunk_is_held_while_any_ticket_holds_it() {
        let mut tickets = ModTickets::default();
        let (a, b) = (TicketOwner::Block([1, 2, 3]), TicketOwner::Entity(9));
        assert!(tickets.insert(ticket(0, a, (0, 0))));
        assert!(!tickets.insert(ticket(0, a, (0, 0))));
        assert!(tickets.insert(ticket(0, b, (0, 0))));
        // The same owner under another controller is another ticket.
        assert!(tickets.insert(ticket(1, a, (0, 0))));
        assert!(tickets.remove(&ticket(0, a, (0, 0))));
        assert!(!tickets.remove(&ticket(0, a, (0, 0))));
        assert!(tickets.holds((0, 0)));
        assert_eq!(tickets.of_controller(0).count(), 1);
        assert!(tickets.remove(&ticket(0, b, (0, 0))));
        assert!(tickets.remove(&ticket(1, a, (0, 0))));
        assert!(!tickets.holds((0, 0)));
        assert_eq!(tickets.chunks().count(), 0);
    }

    #[test]
    fn natural_spawning_is_forced_while_any_such_ticket_holds_the_chunk() {
        let mut tickets = ModTickets::default();
        let owner = TicketOwner::Entity(1);
        let spawning = ModTicket {
            natural_spawning: true,
            ..ticket(0, owner, (2, 2))
        };
        assert!(tickets.insert(ticket(0, owner, (2, 2))));
        assert!(tickets.holds((2, 2)) && !tickets.forces_natural_spawning((2, 2)));
        // The same owner holds both kinds apart.
        assert!(tickets.insert(spawning.clone()));
        assert!(tickets.forces_natural_spawning((2, 2)));
        assert!(tickets.remove(&spawning));
        assert!(tickets.holds((2, 2)) && !tickets.forces_natural_spawning((2, 2)));
    }

    #[test]
    fn a_transient_ticket_holds_its_chunk_but_is_not_listed() {
        let mut tickets = ModTickets::default();
        let block = ticket(0, TicketOwner::Block([1, 2, 3]), (4, 5));
        let entity = ticket(0, TicketOwner::Entity(9), (6, 7));
        let transient = TransientTicket {
            controller: 0,
            operation: 1,
            chunk: (4, 5),
            natural_spawning: true,
        };
        tickets.insert(block.clone());
        tickets.insert(entity.clone());
        assert!(tickets.insert_transient(transient));
        assert!(tickets.forces_natural_spawning((4, 5)));
        assert_eq!(
            tickets.of_controller(0).collect::<Vec<_>>(),
            [&block, &entity]
        );
        assert!(tickets.remove(&block));
        assert!(tickets.holds((4, 5)), "the transient ticket still holds it");
        assert!(tickets.remove_transient(&transient));
        assert!(!tickets.holds((4, 5)));
        assert!(tickets.holds((6, 7)));
    }

    #[test]
    fn saved_tickets_wait_for_validation_and_hold_nothing_yet() {
        let saved = vec![ticket(0, TicketOwner::Entity(1), (4, 4))];
        let mut tickets = ModTickets::from_saved(saved);
        assert!(tickets.has_loaded() && !tickets.holds((4, 4)));
        assert_eq!(tickets.to_save().count(), 1);
        for ticket in tickets.take_loaded() {
            tickets.insert(ticket);
        }
        assert!(tickets.holds((4, 4)));
    }

    fn set(plain: &[[i32; 2]], spawning: &[[i32; 2]]) -> TicketSet {
        TicketSet {
            non_natural_spawning: plain.to_vec(),
            natural_spawning: spawning.to_vec(),
        }
    }

    fn in_mod(id: &'static str) -> crate::crash::ModGuard {
        crate::crash::enter_native_mod(crate::crash::ModCall {
            id,
            hook: "test",
            part: None,
        })
    }

    #[test]
    fn validation_shows_and_removes_the_controllers_tickets_to_its_owner_only() {
        let (block, entity) = (TicketOwner::Block([1, 2, 3]), TicketOwner::Entity(9));
        let mut v = ControllerTickets::new(
            "m",
            vec![
                ticket(0, block, (0, 0)),
                ticket(0, block, (0, 1)),
                ticket(0, entity, (5, 5)),
            ],
        );
        assert!(v.block_tickets().is_empty(), "no caller, no tickets");
        {
            let _other = in_mod("n");
            assert!(v.block_tickets().is_empty() && v.entity_tickets().is_empty());
            assert_eq!(v.remove_block_tickets([1, 2, 3]), 0, "not its controller");
            assert!(!format!("{v:?}").contains("(0, 0)"));
        }
        let _owner = in_mod("m");
        assert_eq!(
            v.block_tickets(),
            vec![([1, 2, 3], set(&[[0, 0], [0, 1]], &[]))]
        );
        assert_eq!(v.entity_tickets(), vec![(9, set(&[[5, 5]], &[]))]);
        assert!(v.remove_block_ticket([1, 2, 3], [0, 1], false));
        assert!(!v.remove_block_ticket([1, 2, 3], [0, 1], false));
        assert_eq!(v.remove_entity_tickets(9), 1);
        assert_eq!(v.remove_block_tickets([1, 2, 3]), 1);
        assert_eq!(v.into_removed().len(), 3);
    }

    #[test]
    fn validation_removes_either_kind_apart_and_remove_all_removes_both() {
        let owner = TicketOwner::Block([4, 5, 6]);
        let spawning = ModTicket {
            natural_spawning: true,
            ..ticket(0, owner, (3, 3))
        };
        let both = vec![ticket(0, owner, (3, 3)), spawning];
        let mut v1 = ControllerTickets::new("m", both.clone());
        let v = &mut v1;
        let _m = in_mod("m");
        assert_eq!(
            v.block_tickets(),
            vec![([4, 5, 6], set(&[[3, 3]], &[[3, 3]]))]
        );
        // A singular removal takes the kind it names and leaves the other.
        assert!(v.remove_block_ticket([4, 5, 6], [3, 3], true));
        assert!(!v.remove_block_ticket([4, 5, 6], [3, 3], true));
        assert_eq!(v.block_tickets(), vec![([4, 5, 6], set(&[[3, 3]], &[]))]);
        assert!(v.remove_block_ticket([4, 5, 6], [3, 3], false));
        assert!(v.block_tickets().is_empty());
        // The other order, and the entity side.
        let mut v2 = ControllerTickets::new("m", both.clone());
        assert!(v2.remove_block_ticket([4, 5, 6], [3, 3], false));
        assert_eq!(v2.block_tickets(), vec![([4, 5, 6], set(&[], &[[3, 3]]))]);
        // Remove-all takes both.
        let mut v3 = ControllerTickets::new("m", both);
        assert_eq!(v3.remove_block_tickets([4, 5, 6]), 2);
        assert_eq!(v1.into_removed().len(), 2);
        assert_eq!(v2.into_removed().len(), 1);
        assert_eq!(v3.into_removed().len(), 2);
        let entity = TicketOwner::Entity(7);
        let mut v = ControllerTickets::new(
            "m",
            vec![
                ticket(0, entity, (1, 1)),
                ModTicket {
                    natural_spawning: true,
                    ..ticket(0, entity, (1, 1))
                },
            ],
        );
        assert!(v.remove_entity_ticket(7, [1, 1], true));
        assert_eq!(v.entity_tickets(), vec![(7, set(&[[1, 1]], &[]))]);
        assert!(v.into_removed()[0].natural_spawning);
    }
}
