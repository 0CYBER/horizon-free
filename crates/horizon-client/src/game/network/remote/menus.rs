//! Container menus on a server: clicks, quick moves, opening and closing screens.

use super::*;

impl Game<'_> {
    /// The server's open menu: its window id and menu type (0, "inventory"
    /// when none is open).
    pub(super) fn remote_menu_body(&self) -> (i32, String) {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref().cloned()) {
            Some((id, name)) => (id, name),
            None => (0, "inventory".to_owned()),
        }
    }
    /// `MultiPlayerGameMode.handleContainerInput` for each click of `ops` on
    /// the server's open menu (a quick craft drag is several): the verified
    /// menu engine predicts them on this client's copy, then each goes out as
    /// a `ContainerClick` with the slots it changed and the carried stack
    /// after it; the server corrects anything it sees differently. A menu the
    /// engine doesn't model sends the clicks unpredicted. False when a click's
    /// slot isn't one of the menu's.
    pub(super) fn remote_menu_ops_body(&mut self, ops: &[(Option<usize>, i32, Input)]) -> bool {
        // CrafterScreen.slotClicked: a slot toggle goes out before the click.
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let (container, menu) = self.remote_menu();
        let mut clicks = Vec::with_capacity(ops.len());
        for &(slot, button, input) in ops {
            let slot = match slot {
                None => -999,
                Some(internal) => match Self::menu_slot_index(&menu, internal) {
                    Some(s) => s as i16,
                    None => return false,
                },
            };
            clicks.push((slot, button, input));
        }
        // After the check: only a slot of the menu is read.
        let split = self.split_click(ops);
        // Unpredicted: nothing changed, the carried stack as it is.
        let mut sent: Vec<Predicted> = clicks
            .iter()
            .map(|&(slot, button, input)| (slot, button, input, Vec::new(), self.carried.clone()))
            .collect();
        let mut threw = false;
        // The engine's slot indices are the menu's own, as on the network.
        self.menu_session(|m, env, _| {
            let Some((predicted, dropped)) = predict_clicks(m, env, &clicks) else {
                // The server throws on it too: nothing is predicted.
                return Some(false);
            };
            sent = predicted;
            threw = dropped;
            Some(true)
        });
        self.send_clicks(container, sent);
        // The pad's own click, and only the pad's: the mouse's split doesn't buzz.
        if self.input_made_pointer_event()
            && let Some(split) = split
            && self.split_done(split)
        {
            self.input_feedback(horizon_client::input_source::Feedback::ItemStackSplit);
        }
        // LivingEntity.drop(.., Prediction.PREDICTED): the client swings its own
        // main hand; the server shows the swing only to the other players.
        if threw {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
        true
    }
    /// A QUICK_MOVE on the server's open menu with nothing predicted: the
    /// client's slots stay as they are and every slot it moved comes back from
    /// the server (`broadcastChanges`), the answer the next step waits for.
    /// A prediction the server agrees with gets no answer at all.
    pub(super) fn remote_quick_move_unpredicted_body(&mut self, internal: usize) -> bool {
        let (container, menu) = self.remote_menu();
        let Some(slot) = Self::menu_slot_index(&menu, internal) else {
            return false;
        };
        let carried = self.carried.clone();
        let click = (slot as i16, 0, Input::QuickMove, Vec::new(), carried);
        self.send_clicks(container, vec![click]);
        true
    }
    /// A Shift double click on a server (`AbstractContainerScreen.mouseReleased`):
    /// a predicted QUICK_MOVE for every slot the engine moves, each sent.
    pub(super) fn remote_quick_move_all_body(&mut self, slot: usize, moved: &ItemStack) {
        let (container, _) = self.remote_menu();
        let mut sent: Vec<Predicted> = Vec::new();
        self.menu_session(|m, env, to_menu| {
            let clicked = to_menu(slot)?;
            let mut before = menu_items(m);
            let mut each = |m: &horizon_client::menu::Menu, t: usize| {
                let changed = changed_slots(&before, m);
                sent.push((t as i16, 0, Input::QuickMove, changed, m.carried.clone()));
                before = menu_items(m);
            };
            Some(
                m.quick_move_all_each(env, clicked, moved, &mut each)
                    .is_ok(),
            )
        });
        self.send_clicks(container, sent);
    }
    pub(super) fn send_clicks_body(&mut self, container: i32, clicks: Vec<Predicted>) {
        // A result the engine claims to have changed is the one the server
        // agrees to (`click_agreed`), so it sends none: keep it. A result the
        // engine doesn't claim (a recipe it doesn't know) the server sends.
        if matches!(
            self.remote_menu().1.as_str(),
            "inventory" | "minecraft:crafting"
        ) && let Some(r) = self.remote.as_mut()
        {
            for (.., changed, _) in &clicks {
                if let Some((_, result)) = changed.iter().find(|(slot, _)| *slot == 0) {
                    r.craft_result = result.clone();
                }
            }
        }
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        let hashed = |g: &Self, s: &Option<ItemStack>| g.hashed_stack(s);
        for (slot, button, input, changed, carried) in clicks {
            let changed = changed.iter().map(|(i, s)| (*i, hashed(self, s))).collect();
            let carried = hashed(self, &carried);
            self.send(S::ContainerClick(play::ContainerClick {
                container,
                state,
                slot,
                button: button as i8,
                input: input_ordinal(input),
                changed,
                carried,
            }));
        }
    }
    /// The server's open menu is the inventory or a chest-like container
    /// (chests, barrels, ender chests: `generic_9xN`; shulker boxes).
    pub(super) fn remote_storage_open_body(&self) -> bool {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            None => true,
            Some((_, name)) => {
                name == "inventory"
                    || name.starts_with("minecraft:generic_9x")
                    || name == "minecraft:shulker_box"
            }
        }
    }
    /// A click outside a server's window (slot -999): predicted like
    /// `MultiPlayerGameMode.handleContainerInput` (the carried stack after
    /// `doClick`; the server drops the stack and swings the hand).
    pub(super) fn remote_throw_body(&mut self, right: bool, input: horizon_client::menu::Input) {
        use horizon_client::menu::Input;
        let container = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        if input == Input::Pickup
            && let Some(c) = self.carried.as_mut()
        {
            if right && c.count > 1 {
                c.count -= 1;
            } else {
                self.carried = None;
            }
            self.swing_arm();
        }
        let carried = self.hashed_stack(&self.carried);
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: -999,
            button: i8::from(right),
            input: if input == Input::Throw { 4 } else { 0 },
            changed: Vec::new(),
            carried,
        }));
    }
    /// `CreativeModeInventoryScreen.slotClicked` outside the window: the server knows nothing of
    /// a stack picked from the creative screen, so the carried stack (an item tab's right click:
    /// one of it) is dropped through `handleCreativeModeItemDrop`, a SetCreativeModeSlot to slot
    /// -1, and the hand swings as `Player.drop` predicts.
    pub(super) fn remote_creative_drop_body(&mut self, right: bool) {
        let one = right && !self.creative_inventory_open();
        let Some(carried) = self.carried.as_mut() else {
            return;
        };
        let thrown = if one && carried.count > 1 {
            carried.count -= 1;
            ItemStack {
                count: 1,
                ..carried.clone()
            }
        } else {
            self.carried.take().expect("carried")
        };
        self.remote_creative_throw(thrown);
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemDrop`: a stack thrown out of the creative
    /// screen goes to the server as a SetCreativeModeSlot to slot -1, which drops it from the
    /// player's hand; the client only swings (`Player.drop` on a client).
    pub(super) fn remote_creative_throw_body(&mut self, thrown: ItemStack) {
        if !self.creative {
            return;
        }
        self.swing_arm();
        let item = self.to_network(&Some(thrown));
        self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
            slot: -1,
            item,
        }));
    }
    pub(super) fn remote_close_container_body(&mut self) {
        let id = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().map(|m| m.0))
            .unwrap_or(0);
        self.send(S::ContainerClose(play::ContainerClose { container: id }));
        self.remote_close_local();
    }
    /// `MultiPlayerGameMode.handleInventoryButtonClick`: button `button` of
    /// the menu the server opened goes to it (`ContainerButtonClick`), whose
    /// menu decides what it does (`clickMenuButton`). No sound: that is the
    /// screen's. False while no such menu is open.
    pub(super) fn remote_button_click_body(&self, button: i32) -> bool {
        let Some(container) = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map(|(w, _)| *w)
        else {
            return false;
        };
        self.send(S::ContainerButtonClick(play::ContainerButtonClick {
            container,
            button,
        }));
        true
    }
    pub(super) fn remote_reject_container_body(&mut self, container: i32) {
        self.remote_close_local();
        if self.screen == Screen::Inventory {
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
        self.send(S::ContainerClose(play::ContainerClose { container }));
    }
    pub(super) fn remote_close_local_body(&mut self) {
        // `Gui.setScreen` clears the focus of the screen it takes down: the
        // container's keyboard focus must not outlive it (a Tab bound to the
        // inventory moves the focus and closes the container in one press).
        self.pad_clear_focus();
        self.recipe_ui.reset_screen();
        if let Some(r) = self.remote.as_mut() {
            r.menu.set(None);
            r.set_menu_title(None);
            r.craft_result = None;
        }
        self.mount_menu_reset();
        // The client's menu has no level access: what it holds is the server's.
        self.station = None;
        // AbstractContainerMenu.removed: the server gives the grid and the
        // carried stack back to the inventory and syncs it; the copies here
        // are stale the moment the window closes.
        self.craft_grid.fill(None);
        self.carried = None;
        self.craft_size = 2;
        if let Some(key) = self.container.take() {
            // Server containers stay renderable but lose their copied slots.
            if let Some(entity) = self.world.block_entities.get_mut(&key) {
                for s in entity.slots_mut() {
                    *s = None;
                }
            }
        }
    }
    /// A server's trading window (MerchantScreen over a ClientSideMerchant).
    pub(super) fn remote_open_merchant_body(
        &mut self,
        container: i32,
        title: String,
        text: horizon_client::text::Text,
    ) {
        if let Some(r) = self.remote.as_mut() {
            r.menu
                .set(Some((container, "minecraft:merchant".to_owned())));
            r.set_menu_title(Some(text.clone()));
        }
        let session = horizon_client::merchant::Session::new(Vec::new(), true);
        self.open_remote_merchant(session, (!title.is_empty()).then_some(text));
    }
    /// ClientboundMerchantOffersPacket.
    pub(super) fn remote_merchant_offers_body(&mut self, p: play::MerchantOffers) {
        if self.open_menu_name(p.container) != "minecraft:merchant" {
            return;
        }
        let cost = |g: &Self, c: &play::NetCost| {
            g.to_stack(&Some(c.clone().into()))
                .map(|stack| horizon_client::merchant::ItemCost { stack })
        };
        let offers: Vec<_> = p
            .offers
            .iter()
            .filter_map(|o| {
                let mut offer = horizon_client::merchant::MerchantOffer {
                    base_cost_a: cost(self, &o.cost_a)?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(self, b)?),
                        None => None,
                    },
                    result: self.to_stack(&Some(o.result.clone()))?,
                    uses: o.uses,
                    max_uses: o.max_uses,
                    reward_exp: true,
                    special_price_diff: o.special_price,
                    demand: o.demand,
                    price_multiplier: o.price_multiplier,
                    xp: o.xp,
                };
                // setToOutOfStock.
                if o.out_of_stock {
                    offer.uses = offer.max_uses;
                }
                Some(offer)
            })
            .collect();
        if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
            m.session.offers = offers;
            m.session.level = p.level;
            m.session.xp = p.xp;
            m.session.villager = p.show_progress;
        }
    }
    /// ClientboundOpenScreenPacket for the menus Horizon draws.
    pub(super) fn remote_open_screen_body(&mut self, p: play::OpenScreen) {
        // A new RecipeBookComponent owns fresh ghost slots and an overlay,
        // even when the server replaces a menu without a close packet.
        self.recipe_ui.reset_screen();
        let menu = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:menu")?
                    .name(p.menu)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let target = self
            .remote
            .as_mut()
            .and_then(|r| r.use_target.take())
            .map(|t| (t[0], t[1], t[2]));
        // The server names a mod menu's block itself: it, not the last
        // clicked block, is the one the menu is of. Only for this window:
        // any other's is stale.
        let extra = self
            .remote
            .as_mut()
            .and_then(|r| r.menu_extra.take())
            .filter(|e| e.window == p.container);
        let target = extra
            .as_ref()
            .map_or(target, |e| e.pos.map(|p| (p[0], p[1], p[2])));
        if menu == "minecraft:lectern" {
            // LecternScreen: the book and page come in the menu's content and data.
            if let Some(r) = self.remote.as_mut() {
                r.menu.set(Some((p.container, menu)));
                r.set_menu_title(None);
            }
            // A menu whose screen opens replaces a dialog, as below.
            self.close_dialogs();
            self.remote_open_lectern(target.unwrap_or_default());
            return;
        }
        if let Some(kind) = Self::remote_station_kind(&menu) {
            let title = menu_title(&p.title);
            let pos = target.map_or([0, 0, 0], |t| [t.0, t.1, t.2]);
            self.close_dialogs();
            self.remote_open_station(p.container, menu, kind, title, pos);
            return;
        }
        if menu == "minecraft:merchant" {
            // A trading window: its offers follow in a MerchantOffers packet.
            let title = plain(
                &p.title,
                &self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default(),
            );
            self.close_dialogs();
            self.remote_open_merchant(p.container, title, menu_title(&p.title));
            return;
        }
        let mod_type = horizon_client::containers::mod_menu_type(&menu);
        let entity = match menu.as_str() {
            "minecraft:crafting" => None,
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                let kind = match menu.as_str() {
                    "minecraft:blast_furnace" => FurnaceKind::BlastFurnace,
                    "minecraft:smoker" => FurnaceKind::Smoker,
                    _ => FurnaceKind::Furnace,
                };
                Some(BlockEntity::Furnace(Furnace {
                    kind,
                    items: [None, None, None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: std::collections::HashMap::new(),
                }))
            }
            // HopperMenu, and DispenserMenu (MenuType.GENERIC_3x3: dispensers
            // and droppers).
            "minecraft:hopper" => Some(BlockEntity::Chest(vec![None; 5])),
            // BrewingStandMenu: its times come in ContainerSetData.
            "minecraft:brewing_stand" => Some(BlockEntity::Brewing(Box::new(
                horizon_client::brewing::BrewingStand::new(),
            ))),
            "minecraft:generic_3x3" => Some(BlockEntity::Chest(vec![None; 9])),
            // CrafterMenu: the disabled slots and triggered come in ContainerSetData.
            "minecraft:crafter_3x3" => Some(BlockEntity::Crafter(Default::default())),
            m if m.starts_with("minecraft:generic_9x") || m == "minecraft:shulker_box" => {
                let rows = m
                    .strip_prefix("minecraft:generic_9x")
                    .and_then(|r| r.parse::<usize>().ok())
                    .unwrap_or(3);
                Some(BlockEntity::Chest(vec![None; rows * 9]))
            }
            // A content mod's menu type: its container's slots, which its
            // mod's screen places.
            _ if mod_type.is_some() => Some(BlockEntity::Mod(Box::new(
                horizon_client::containers::ModBlockEntity::new(
                    String::new(),
                    horizon_client::containers::ModBlockEntityFields {
                        items: vec![None; mod_type.map_or(0, |t| t.slots)],
                        ..Default::default()
                    },
                ),
            ))),
            _ => {
                // Unsupported menu: tell the server we closed it.
                self.remote_reject_container(p.container);
                self.add_chat_line(
                    horizon_client::text::Text::literal(format!(
                        "(Horizon cannot show {menu} screens yet)"
                    )),
                    horizon_client::chat::ChatSource::Client,
                );
                return;
            }
        };
        // `MenuScreens.create` ends in `Gui.setScreen`: the menu's screen replaces a dialog
        // (and its waiting or warning screen), it does not open under it.
        self.close_dialogs();
        let title = menu_title(&p.title);
        if let Some(r) = self.remote.as_mut() {
            r.menu.set(Some((p.container, menu.clone())));
            r.set_menu_title(Some(title));
        }
        self.craft_size = if menu == "minecraft:crafting" { 3 } else { 2 };
        self.container = None;
        if let Some(entity) = entity {
            // The clicked block shows the menu when it holds that container;
            // an ender chest's or a double chest's slots are no one block's,
            // so they go under a key no block has.
            let holds = |k: &(i32, i32, i32)| {
                let name = &self.res.facts().get(self.world.block(k.0, k.1, k.2)).name;
                BlockEntity::for_block(name).is_some_and(|e| {
                    e.id() == entity.id() && e.slots().len() == entity.slots().len()
                })
            };
            let key = target.filter(holds).unwrap_or((i32::MIN, 0, 0));
            self.world.block_entities.insert(key, entity);
            self.container = Some(key);
            // A server that negotiated menu-extra sends the block's position
            // when it opens a mod block's screen. Only legacy connections still
            // infer a screen's block from a click when the payload is absent.
            let uses_menu_origins = self
                .remote
                .as_ref()
                .is_some_and(|r| r.registries.menu_extra_revision > 0);
            let layout_target = if uses_menu_origins && extra.is_none() {
                None
            } else {
                target
            };
            self.open_native_screen(key, layout_target, p.container, &menu, extra);
            // A mod menu type shows through its mod's screen only: without one
            // (`MenuScreens.create` finds no screen) the window closes again.
            if self.mod_screen.is_none() && mod_type.is_some() {
                self.world.block_entities.remove(&key);
                self.container = None;
                if let Some(r) = self.remote.as_mut() {
                    r.menu.set(None);
                    r.set_menu_title(None);
                }
                self.send(S::ContainerClose(play::ContainerClose {
                    container: p.container,
                }));
                return;
            }
        }
        self.set_screen(Screen::Inventory);
        self.capture(false);
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a horse's or llama's
    /// inventory; for another entity the window is closed again.
    pub(super) fn remote_open_mount_body(&mut self, p: play::MountScreenOpen) {
        let columns = usize::try_from(p.columns).unwrap_or(0).min(5);
        if !self.open_remote_mount_menu(p.entity as u32, columns) {
            self.remote_reject_container(p.container);
            return;
        }
        // `handleMountScreenOpen` sets the mount's inventory screen, over a dialog too.
        self.close_dialogs();
        if let Some(r) = self.remote.as_mut() {
            r.menu
                .set(Some((p.container, format!("{MOUNT_MENU}{columns}"))));
            r.set_menu_title(None);
        }
    }
}
