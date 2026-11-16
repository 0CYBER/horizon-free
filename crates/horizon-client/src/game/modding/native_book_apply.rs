//! Authoritative placement: detached calculation, journaled writes, then publication.
use super::{Game, native_book_backings::Backings, native_book_scope::MenuSnapshot};
use horizon_client::{
    lan::menu::ModCraft,
    menu::{
        Env, Kind, Menu,
        place::{
            PostPlace,
            custom_plan::{MenuPlacement, PlacementRefusal},
        },
    },
    recipe_book::placement_sync as sync,
};

fn refusal(reason: PlacementRefusal) -> sync::Refusal {
    use sync::Refusal as R;
    match reason {
        PlacementRefusal::NotPlaceable => R::NotPlaceable,
        PlacementRefusal::InvalidLayout => R::Invalid,
        PlacementRefusal::Unavailable => R::Unavailable,
        PlacementRefusal::Denied => R::Denied,
        PlacementRefusal::NoRoom => R::NoRoom,
        PlacementRefusal::Changed => R::Changed,
    }
}
impl Game<'_> {
    pub(super) fn server_mod_book_place(
        &mut self,
        conn: u32,
        selection: &sync::Selection,
        use_max: bool,
    ) -> sync::Outcome {
        match self.apply_mod_book_place(conn, selection, use_max) {
            Ok(post) => {
                if post == PostPlace::Ghost {
                    sync::Outcome::MissingIngredients
                } else {
                    sync::Outcome::Placed
                }
            }
            Err(reason) => sync::Outcome::Refused(reason),
        }
    }
    fn apply_mod_book_place(
        &mut self,
        conn: u32,
        selection: &sync::Selection,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        if !self.transactions.is_empty() {
            return Err(R::Denied);
        }
        let settings = self
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .ok_or(R::Unsupported)?;
        if settings.epoch != selection.catalog.epoch
            || !settings.contains(selection.catalog.book.as_str())
        {
            return Err(R::Stale);
        }
        let authority = self
            .native_net
            .placement
            .servers
            .get_mut(&conn)
            .ok_or(R::Unsupported)?;
        authority.peer.place(selection)?;
        if !authority
            .displays
            .get(selection.display.recipe.as_str())
            .is_some_and(|count| selection.display.source < *count)
        {
            return Err(R::UnknownDisplay);
        }
        self.ensure_mod_book_displays();
        let index = self
            .recipe_tables
            .books
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .ok_or(R::Invalid)?;
        if index.generation() != selection.catalog.generation {
            return Err(R::Stale);
        }
        let plan = index
            .placement(selection.display.recipe.as_str())
            .map_err(refusal)?
            .clone();
        let before = self.mod_book_menu_snapshot(
            conn,
            selection.catalog.window,
            selection.catalog.menu.as_str(),
        )?;
        if before.instance != selection.catalog.instance {
            return Err(R::Stale);
        }
        if !before.known.contains(selection.display.recipe.as_str()) {
            return Err(R::Denied);
        }
        let seal = self.transaction_open();
        let result = self.stage_mod_book_place(&before, selection, &plan, use_max);
        self.transaction_close(&seal, result.is_ok());
        result
    }
    fn mod_book_placement_context(
        &self,
        before: &MenuSnapshot,
        selection: &sync::Selection,
    ) -> bool {
        self.transactions.len() == 1
            && self.mod_book_menu_unchanged(before)
            && self
                .native_net
                .placement
                .servers
                .get(&before.conn)
                .is_some_and(|server| server.peer.owns(&selection.catalog))
            && self
                .native_net
                .book_connections
                .get(&before.conn)
                .and_then(|connection| connection.authority.as_ref())
                .is_some_and(|authority| {
                    authority.epoch == selection.catalog.epoch
                        && authority.contains(selection.catalog.book.as_str())
                })
    }
    fn stage_mod_book_place(
        &mut self,
        before: &MenuSnapshot,
        selection: &sync::Selection,
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        let mut graph = Backings::capture(self, before, plan)?;
        let gi = self
            .lan
            .as_ref()
            .and_then(|host| {
                host.hub
                    .guests
                    .iter()
                    .position(|guest| guest.conn == before.conn)
            })
            .ok_or(R::Changed)?;
        let horizon_client::lan::menu::Slots::Block(pos) = before.slots else {
            return Err(R::Unavailable);
        };
        let sources = self.lan_recipe_sources(gi, before.kind, pos);
        if !self.mod_book_placement_context(before, selection) || !graph.unchanged(self) {
            return Err(R::Changed);
        }
        let stocks = graph.add_sources(self, &sources)?;
        let host = self.lan.as_ref().ok_or(R::Changed)?;
        let guest = host.hub.guest(before.conn).ok_or(R::Changed)?;
        let kind = Kind::Mod(before.kind);
        let mut menu = host
            .menus
            .get(&before.conn)
            .and_then(|play| play.crafting.machine.as_ref())
            .filter(|(window, menu)| *window == before.window && menu.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, menu)| menu.clone());
        menu.reset_mod_visibility(&before.hidden);
        menu.set_registry_id(Some(before.kind.id));
        super::lan_crafting::load_player(&mut menu, guest);
        menu.set_container_limits(graph.limits());
        menu.sources = stocks;
        for (slot, stack) in graph.main.iter().enumerate() {
            menu.load_slot(slot, stack.clone());
        }
        let known = guest.recipe_book.clone();
        let limited = host.hub.limited_crafting;
        let name = guest.name.clone();
        let post = {
            let env = Env {
                player: Some(before.uuid),
                player_entity_id: Some(before.entity_id),
                can_equip: None,
                known_recipes: Some(&known),
                slot_rules: None,
                limited_crafting: limited,
                ..self.menu_env()
            };
            menu.place_custom_recipe(&env, plan, use_max, before.game_mode == 1, || {
                self.mod_book_placement_context(before, selection) && graph.unchanged(self)
            })
            .map_err(refusal)?
        };
        if let Some(crafting) = menu.mod_crafting() {
            let mut result = menu.slot(crafting.result).cloned();
            self.native_prepare_item_craft(
                before.uuid,
                &name,
                Some([pos.0, pos.1, pos.2]),
                &mut result,
            );
            if !self.mod_book_placement_context(before, selection) || !graph.unchanged(self) {
                return Err(R::Changed);
            }
            menu.load_slot(crafting.result, result);
        }
        if !self.native_as_player(before.uuid, |game| game.journal_player(before.uuid)) {
            return Err(R::Changed);
        }
        // The guest's fields are back in the hub before comparing or storing them.
        if !self.mod_book_placement_context(before, selection) || !graph.unchanged(self) {
            return Err(R::Changed);
        }
        let changes = graph.changes(&mut menu);
        for key in changes.keys() {
            self.journal_block_entity(key.pos);
        }
        let mut changed = std::collections::BTreeSet::new();
        for (key, stack) in changes {
            let entity = self
                .world
                .block_entities
                .get_mut(&key.pos)
                .ok_or(R::Changed)?;
            let slot = entity.slots_mut().get_mut(key.slot).ok_or(R::Changed)?;
            *slot = stack;
            changed.insert(key.pos);
        }
        for pos in changed {
            self.after_commit(super::transactions::After::Changed(pos));
        }
        for stack in std::mem::take(&mut menu.dropped) {
            self.drop_after_commit(before.uuid, stack);
        }
        let craft = menu.mod_crafting().map(|crafting| ModCraft {
            grid: crafting
                .grid_slots()
                .map(|slot| menu.slot(slot).cloned())
                .collect(),
            result: menu.slot(crafting.result).cloned(),
        });
        let host = self.lan.as_mut().ok_or(R::Changed)?;
        let gi = host
            .hub
            .guests
            .iter()
            .position(|guest| guest.conn == before.conn)
            .ok_or(R::Changed)?;
        // No callbacks or fallible work follow these live-cache assignments.
        menu.sources.clear();
        host.hub.guests.update(gi, |guest| {
            super::lan_crafting::store_player(&mut menu, guest);
            if let Some(opened) = &mut guest.menu {
                opened.craft = craft;
            }
        });
        *host.menus.entry(before.conn).or_default().crafting.machine = Some((before.window, menu));
        Ok(post)
    }
}
#[cfg(test)]
#[path = "native_book_apply/tests.rs"]
mod tests;
