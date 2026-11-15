//! Action state belongs to the existing storage transaction, not a parallel
//! transaction stack. The resource-free harness uses these same journal methods.
use super::*;
use horizon_client::join_seq::visibility::PassLease;

/// A marker lives exactly as long as its transaction level. The existing save
/// admission gate checks passes_settled(), including this lease. In particular
/// commands.save.failed is still produced by the existing save-all command.
pub(in crate::game) struct ActiveAction {
    pub(super) menu: MenuId,
    _save_gate: PassLease,
}

impl ActiveAction {
    pub(super) fn new(menu: MenuId) -> Self {
        Self {
            menu,
            _save_gate: PassLease::enter(),
        }
    }
}

struct KeptMenu {
    menu: Menu,
    state: MenuState,
}

#[derive(Default)]
pub(in crate::game) struct Journal {
    menus: HashMap<MenuId, KeptMenu>,
    pub(super) owners: HashMap<MenuId, engine::KeptOwner>,
    // The harness has no Game::After queue. Production uses that existing queue
    // directly through GameHost::defer, preserving listener/apply effect order.
    #[cfg(test)]
    after: Vec<Effect>,
}

impl Journal {
    pub(super) fn keep(&mut self, menu: &Menu) {
        self.menus.entry(menu.id).or_insert_with(|| KeptMenu {
            menu: menu.clone(),
            state: menu.read(Clone::clone),
        });
    }

    #[cfg(test)]
    pub(super) fn defer(&mut self, effect: Effect) {
        self.after.push(effect);
    }

    /// Keep the oldest snapshot when a nested listener or action commits.
    pub(in crate::game) fn merge(&mut self, child: Self) {
        for (id, kept) in child.menus {
            self.menus.entry(id).or_insert(kept);
        }
        for (id, kept) in child.owners {
            self.owners.entry(id).or_insert(kept);
        }
        #[cfg(test)]
        self.after.extend(child.after);
    }

    pub(in crate::game) fn rollback(self) {
        assert!(self.owners.is_empty(), "game menu undo needs its owner");
        // No success effect, drop, or retirement escapes an aborted action.
        #[cfg(test)]
        drop(self.after);
        for kept in self.menus.into_values() {
            let mut inner = kept.menu.inner.borrow_mut();
            inner.epoch.advance();
            inner.state = kept.state;
        }
    }

    pub(in crate::game) fn rollback_in(mut self, game: &mut crate::game::Game<'_>) {
        for kept in std::mem::take(&mut self.owners).into_values() {
            kept.rollback(game);
        }
        self.rollback();
    }

    /// Only the root commit owner may take the queued effects.
    #[cfg(test)]
    pub(super) fn root_effects(self) -> Vec<Effect> {
        self.after
    }
}

/// This small internal trait adapts the existing transaction owner. It is not
/// a Host service or a second public transaction API. Stage 3 supplies route
/// effects through the existing After variants and root publisher.
pub(in crate::game) trait ActionHost {
    fn identity(&self) -> Identity;
    fn residence(&self, holder: EntityRef) -> Residence;
    fn has_transaction(&self) -> bool;
    fn menu_busy(&self, menu: MenuId) -> bool;
    fn open_action(&mut self, menu: MenuId) -> u64;
    fn action_is_current(&self, savepoint: u64, menu: MenuId) -> bool;
    fn close_action(&mut self, savepoint: u64, commit: bool);
    fn journal(&mut self) -> &mut Journal;
    fn defer(&mut self, effect: Effect);
    fn decided(&mut self) {}
    fn dispatched(&mut self) {}
    fn applied(&mut self) {}
}

/// Stage-2 adapter. No real menu route constructs this yet. Identity is supplied
/// by the action's game-side caller and checked against the live lifecycle on
/// each validation, not recovered from mod-controlled output.
pub(in crate::game) struct GameHost<'a, 'g> {
    pub game: &'a mut super::super::super::Game<'g>,
    pub identity: Identity,
}

impl ActionHost for GameHost<'_, '_> {
    fn identity(&self) -> Identity {
        let mut identity = self.identity;
        if let Some(world) = self.game.native_lifecycle.epoch() {
            identity.world = world;
        }
        identity.side = if self.game.is_logical_server() {
            Side::Server
        } else {
            Side::Client
        };
        identity
    }

    fn residence(&self, holder: EntityRef) -> Residence {
        use horizon_client::join_seq::residency::table::EntityLookup;
        if self.game.native_lifecycle.epoch() != Some(holder.epoch()) {
            return Residence::Missing;
        }
        if self.identity().side == Side::Client {
            return if self.game.native_client_entity_target(holder).is_some() {
                Residence::Found
            } else {
                Residence::Missing
            };
        }
        // The shared entity table includes players and parked dimensions. A
        // missing row refuses the original holder; never fall back to a scan.
        match self.game.server_resident_entity(holder.id()) {
            EntityLookup::Taken => Residence::Taken,
            EntityLookup::Found { .. } => Residence::Found,
            EntityLookup::Missing => Residence::Missing,
        }
    }

    fn has_transaction(&self) -> bool {
        !self.game.transactions.is_empty()
    }

    fn menu_busy(&self, menu: MenuId) -> bool {
        self.game
            .transactions
            .iter()
            .any(|level| level.action.as_ref().is_some_and(|a| a.menu == menu))
    }

    fn open_action(&mut self, menu: MenuId) -> u64 {
        let (token, _) = self.game.open_storage_transaction();
        self.game.transactions.last_mut().unwrap().action = Some(ActiveAction::new(menu));
        token
    }

    fn action_is_current(&self, savepoint: u64, menu: MenuId) -> bool {
        self.game.transactions.last().is_some_and(|level| {
            level.token == savepoint && level.action.as_ref().is_some_and(|a| a.menu == menu)
        })
    }

    fn close_action(&mut self, savepoint: u64, commit: bool) {
        let Some(at) = self
            .game
            .transactions
            .iter()
            .position(|l| l.token == savepoint)
        else {
            return;
        };
        // An abandoned nested transaction is always aborted, never implicitly
        // committed by the action. Use the existing journal close ordering.
        while self.game.transactions.len() > at + 1 {
            self.game.close_storage_transaction(false);
        }
        self.game.close_storage_transaction(commit);
    }

    fn journal(&mut self) -> &mut Journal {
        &mut self
            .game
            .transactions
            .last_mut()
            .expect("write needs transaction")
            .actions
    }

    fn defer(&mut self, effect: Effect) {
        self.game
            .transactions
            .last_mut()
            .expect("effect needs transaction")
            .after
            .push(effect);
    }
}
