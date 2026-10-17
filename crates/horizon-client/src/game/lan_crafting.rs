//! Crafting for LAN guests: the inventory's 2x2 grid (window 0) and the
//! crafting table (`minecraft:crafting`) run the host's menu engine
//! (`horizon_client::menu`, `Kind::Inventory` / `Kind::Crafting`) over the
//! guest's inventory, with vanilla's packets: OpenScreen, container clicks
//! and their corrections, and the grid back into the inventory when the menu
//! closes (`AbstractContainerMenu.clearContainer`).
use super::{Game, lan::LanHost};
use horizon_client::{
    containers::BlockEntity,
    items::ItemStack,
    lan::{guest::Guest, menu::Slots},
    menu::{Env, Input, Kind, Menu},
};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// The guest's menus the game runs: kept between clicks, so a drag (quick
/// craft) spans them as in vanilla.
#[derive(Default)]
pub(super) struct GuestCrafting {
    /// The inventory menu; its grid lives in the hub's guest.
    inventory: Option<Box<Menu>>,
    /// The open crafting table: its window, state id and menu.
    table: Option<(i32, i32, Box<Menu>)>,
    /// The open furnace's window and menu (`AbstractFurnaceMenu`); its slots live in
    /// the furnace block entity and the hub's container menu keeps the client in sync.
    furnace: Option<(i32, Box<Menu>)>,
}

/// ServerboundContainerClickPacket's input.
pub(super) fn input(id: i32) -> Input {
    match id {
        0 => Input::Pickup,
        1 => Input::QuickMove,
        2 => Input::Swap,
        3 => Input::Clone,
        4 => Input::Throw,
        5 => Input::QuickCraft,
        _ => Input::PickupAll,
    }
}

/// The armor's player-container slots, head to feet (`Inventory` 39..36).
const ARMOR: [usize; 4] = [39, 38, 37, 36];
const OFFHAND: usize = 40;

/// The guest's inventory, armor, off hand, selection and carried stack into `menu`.
fn load_player(menu: &mut Menu, g: &Guest) {
    for i in 0..36 {
        menu.set_player_item(i, g.inventory[i].clone());
    }
    for (k, slot) in ARMOR.into_iter().enumerate() {
        menu.set_player_item(slot, g.armor[k].clone());
    }
    menu.set_player_item(OFFHAND, g.offhand.clone());
    menu.selected = g.selected;
    menu.creative = g.creative();
    menu.carried = g.carried.clone();
}

/// `load_player` back.
fn store_player(menu: &mut Menu, g: &mut Guest) {
    for i in 0..36 {
        g.inventory[i] = menu.player_item(i).cloned();
    }
    for (k, slot) in ARMOR.into_iter().enumerate() {
        g.armor[k] = menu.player_item(slot).cloned();
    }
    g.offhand = menu.player_item(OFFHAND).cloned();
    g.carried = menu.carried.take();
}

/// Inventory.placeItemBackInInventory for each stack: what doesn't fit is
/// left for Player.drop (creative players keep it too).
fn place_back(menu: &mut Menu, env: &Env, stacks: Vec<ItemStack>) {
    menu.creative = false;
    for mut stack in stacks {
        menu.give(env, &mut stack);
        if stack.count > 0 {
            menu.dropped.push(stack);
        }
    }
}

impl Game<'_> {
    fn menu_env(&self) -> Env<'_> {
        Env {
            items: &self.res.items,
            recipes: &self.res.recipes,
            brewing: &self.res.stations.brewing,
            banners: &self.res.stations.banners,
            trade: None,
        }
    }

    /// Runs `op` on the guest's menu with their inventory copied in and back.
    fn guest_menu_op<R>(
        &mut self,
        host: &mut LanHost,
        gi: usize,
        menu: &mut Menu,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> R {
        let env = self.menu_env();
        load_player(menu, &host.hub.guests[gi]);
        let result = op(menu, &env);
        let dropped = std::mem::take(&mut menu.dropped);
        menu.drops.clear();
        menu.crafted.clear();
        store_player(menu, &mut host.hub.guests[gi]);
        self.drop_guest_items(host, gi, dropped);
        result
    }

    /// A click on the guest's inventory menu (window 0): `InventoryMenu`,
    /// crafting grid included. False for other windows.
    pub(super) fn lan_inventory_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        if p.container != 0 {
            return false;
        }
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        if host.hub.guests[gi].spectator() {
            return true;
        }
        let crafting = host.play.entry(conn).or_default();
        let mut menu = crafting
            .crafting
            .inventory
            .take()
            .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
        let grid = host.hub.guests[gi].craft_grid.clone();
        let changed = self.guest_menu_op(host, gi, &mut menu, |menu, env| {
            for (k, s) in grid.into_iter().enumerate() {
                menu.set_slot(env, 1 + k, s);
            }
            let before: Vec<_> = (0..menu.slot_count())
                .map(|s| menu.slot(s).cloned())
                .collect();
            let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
            (0..menu.slot_count())
                .filter(|&s| menu.slot(s) != before[s].as_ref())
                .collect::<Vec<_>>()
        });
        let g = &mut host.hub.guests[gi];
        for k in 0..4 {
            g.craft_grid[k] = menu.slot(1 + k).cloned();
        }
        g.craft_result = menu.slot(0).cloned();
        host.play.entry(conn).or_default().crafting.inventory = Some(menu);
        host.hub.inventory_clicked(conn, p, &self.world, &changed);
        true
    }

    /// A click on the guest's open furnace, blast furnace or smoker: the host's
    /// `AbstractFurnaceMenu` (fuel slot, take-only result, quick moves by recipe and
    /// fuel) over the block entity's slots. False for other windows.
    pub(super) fn lan_furnace_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let Some(pos) = host.hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| p.container != 0 && m.window == p.container)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some(pos),
                _ => None,
            })
        else {
            return false;
        };
        let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) else {
            return false;
        };
        let recipe = match f.kind.recipe_type() {
            "minecraft:blasting" => "blasting",
            "minecraft:smoking" => "smoking",
            _ => "smelting",
        };
        let items = f.items.clone();
        if host.hub.guests[gi].spectator() {
            return true;
        }
        let mut menu = host
            .play
            .entry(conn)
            .or_default()
            .crafting
            .furnace
            .take()
            .filter(|(window, _)| *window == p.container)
            .map_or_else(|| Box::new(Menu::new(Kind::Furnace(recipe))), |(_, m)| m);
        let changed = self.guest_menu_op(host, gi, &mut menu, |menu, env| {
            for (k, s) in items.iter().enumerate() {
                menu.set_slot(env, k, s.clone());
            }
            let before: Vec<_> = (0..menu.slot_count())
                .map(|s| menu.slot(s).cloned())
                .collect();
            let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
            (0..menu.slot_count())
                .filter(|&s| menu.slot(s) != before[s].as_ref())
                .collect::<Vec<_>>()
        });
        if let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get_mut(&pos) {
            for k in 0..3 {
                f.items[k] = menu.slot(k).cloned();
            }
            // AbstractFurnaceBlockEntity.setItem: only a new input restarts cooking.
            f.input_clicked(&self.res.recipes, items[0].clone());
        }
        host.play.entry(conn).or_default().crafting.furnace = Some((p.container, menu));
        host.hub.inventory_clicked(conn, p, &self.world, &changed);
        true
    }

    /// ServerGamePacketListenerImpl.handlePlaceRecipe: the guest's recipe
    /// book places a known recipe in their inventory grid or crafting table
    /// (`ServerPlaceRecipe`), and answers with a ghost recipe when the items
    /// are missing.
    pub(super) fn lan_place_recipe(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &horizon_protocol::packets::recipes::PlaceRecipe,
    ) {
        use horizon_client::menu::place::PostPlace;
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        if g.spectator() {
            return;
        }
        let Some(id) = self
            .res
            .stations
            .recipe_index
            .recipe_of_display(p.recipe)
            .map(str::to_owned)
        else {
            return;
        };
        // An unknown or impossible recipe is refused outright.
        if !g.recipe_book.knows(&id)
            || self
                .res
                .recipes
                .placement(&id)
                .is_none_or(|placement| placement.impossible())
        {
            return;
        }
        let creative = g.creative();
        let placed = if p.container == 0 {
            let crafting = host.play.entry(conn).or_default();
            let mut menu = crafting
                .crafting
                .inventory
                .take()
                .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
            let grid = host.hub.guests[gi].craft_grid.clone();
            let placed = self.guest_menu_op(host, gi, &mut menu, |menu, env| {
                for (k, s) in grid.into_iter().enumerate() {
                    menu.set_slot(env, 1 + k, s);
                }
                menu.place_recipe(env, &id, p.use_max, creative, None)
            });
            let g = &mut host.hub.guests[gi];
            for k in 0..4 {
                g.craft_grid[k] = menu.slot(1 + k).cloned();
            }
            g.craft_result = menu.slot(0).cloned();
            host.play.entry(conn).or_default().crafting.inventory = Some(menu);
            host.hub.send_inventory(gi);
            placed
        } else {
            let Some((window, state, mut menu)) = host
                .play
                .get_mut(&conn)
                .and_then(|play| play.crafting.table.take_if(|(w, _, _)| *w == p.container))
            else {
                return;
            };
            let placed = self.guest_menu_op(host, gi, &mut menu, |menu, env| {
                menu.place_recipe(env, &id, p.use_max, creative, None)
            });
            host.play.entry(conn).or_default().crafting.table = Some((window, state, menu));
            self.lan_send_table_content(host, conn);
            placed
        };
        if placed == PostPlace::Ghost {
            host.hub.send_ghost_recipe(conn, p.container, &id);
        }
    }

    /// The guest opened a crafting table (`CraftingTableBlock.getMenuProvider`).
    pub(super) fn lan_open_crafting(&mut self, host: &mut LanHost, conn: u32) {
        self.lan_crafting_closed(host, conn, None);
        // The container it replaces loses its opener.
        self.guest_container_closed(host, conn);
        let title = horizon_protocol::text::translatable("container.crafting", &[]);
        let Some(window) = host.hub.open_window(conn, "minecraft:crafting", title) else {
            return;
        };
        let menu = Box::new(Menu::new(Kind::Crafting));
        host.play.entry(conn).or_default().crafting.table = Some((window, 0, menu));
        self.lan_send_table_content(host, conn);
    }

    /// A click on the guest's crafting table window; false for other windows.
    pub(super) fn lan_crafting_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let Some(play) = host.play.get_mut(&conn) else {
            return false;
        };
        let Some((window, state, mut menu)) = play
            .crafting
            .table
            .take_if(|(window, _, _)| *window == p.container)
        else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && !host.hub.guests[gi].spectator()
        {
            self.guest_menu_op(host, gi, &mut menu, |menu, env| {
                let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
            });
        }
        host.play.entry(conn).or_default().crafting.table = Some((window, state, menu));
        self.lan_send_table_content(host, conn);
        true
    }

    /// The whole crafting table menu for the client (`sendAllDataToRemote`).
    fn lan_send_table_content(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some((window, state, menu)) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.crafting.table.as_mut())
        else {
            return;
        };
        *state = (*state + 1) & 0x7FFF;
        let (window, state) = (*window, *state);
        let mut items: Vec<Option<ItemStack>> = (0..10).map(|s| menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let items = items.iter().map(|s| host.hub.stack(s)).collect();
        let carried = host.hub.stack(&carried);
        host.hub.send_to(
            conn,
            &C::ContainerSetContent(play::ContainerSetContent {
                container: window,
                state,
                items,
                carried,
            }),
        );
    }

    /// `AbstractContainerMenu.removed` of a crafting menu: the closed window
    /// (`window`; None closes the table whichever it is, and 0 the inventory
    /// menu) gives its grid back to the inventory, what doesn't fit is
    /// dropped. False when no such window is open.
    pub(super) fn lan_crafting_closed(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let crafting = &mut host.play.entry(conn).or_default().crafting;
        let grid: Vec<ItemStack> = if window == Some(0) {
            crafting.inventory = None;
            let g = &mut host.hub.guests[gi];
            g.craft_result = None;
            g.craft_grid.iter_mut().filter_map(Option::take).collect()
        } else {
            match crafting
                .table
                .take_if(|(w, _, _)| window.is_none_or(|window| *w == window))
            {
                Some((_, _, menu)) => (1..10).filter_map(|s| menu.slot(s).cloned()).collect(),
                None => return false,
            }
        };
        if grid.is_empty() {
            return true;
        }
        let mut menu = Menu::new(Kind::Inventory);
        self.guest_menu_op(host, gi, &mut menu, |menu, env| place_back(menu, env, grid));
        true
    }

    /// A guest left: what their crafting grids held goes back into their
    /// inventory before it is saved (dropped where they stood if full).
    pub(super) fn lan_crafting_left(&mut self, host: &mut LanHost, g: &mut Guest) {
        let table = host.play.remove(&g.conn).and_then(|p| p.crafting.table);
        let mut grid: Vec<ItemStack> = g.craft_grid.iter_mut().filter_map(Option::take).collect();
        g.craft_result = None;
        if let Some((_, _, menu)) = table {
            grid.extend((1..10).filter_map(|s| menu.slot(s).cloned()));
        }
        if grid.is_empty() {
            return;
        }
        let mut menu = Menu::new(Kind::Inventory);
        load_player(&mut menu, g);
        place_back(&mut menu, &self.menu_env(), grid);
        let dropped = std::mem::take(&mut menu.dropped);
        store_player(&mut menu, g);
        for stack in dropped {
            self.spawn_item(stack, g.position, [0.0, 0.1, 0.0], 40);
        }
    }
}
