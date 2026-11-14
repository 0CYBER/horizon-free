use super::*;

impl Game<'_> {
    /// Whether the mods let the guest open the menu `kind` (`native_container_opened`),
    /// heard with the guest's state swapped in. Asked before the menu that
    /// was open closes.
    pub(in crate::game) fn guest_may_open(
        &mut self,
        conn: u32,
        kind: &str,
        pos: Option<[i32; 3]>,
    ) -> bool {
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return true;
        };
        if !guest.participates_in_player_simulation() {
            return false;
        }
        let (uuid, name) = (guest.uuid, guest.name.clone());
        let mut cancel = false;
        let mut hook =
            |game: &mut Self| game.native_container_opened(uuid, &name, kind, pos, &mut cancel);
        // The guest whose turn it is is in place already.
        if self.lan_acting == Some(conn) {
            hook(self);
        } else {
            self.as_guest(conn, hook);
        }
        !cancel
    }

    /// Hand a container the guest opened over (ServerPlayer.openMenu);
    /// false when it is not one a guest can open.
    pub(in crate::game) fn lan_open_container(&mut self, conn: u32, pos: (i32, i32, i32)) -> bool {
        if pos.1 == crate::game::minecarts::CART_KEY_Y {
            return self.lan_open_vehicle_container(conn, pos);
        }
        let opened = self.open_block_menu_with_opener(conn, pos, true);
        if opened == MenuOpen::Cancelled
            && self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(|guest| !guest.spectator())
        {
            // The click is spent, but no menu took ownership of the use's opener.
            self.container_openers_changed(pos, false);
        }
        !matches!(opened, MenuOpen::Refused | MenuOpen::Stale)
    }

    pub(in crate::game) fn open_block_menu(&mut self, conn: u32, pos: (i32, i32, i32)) -> MenuOpen {
        self.open_block_menu_with_opener(conn, pos, false)
    }

    /// A use handed over from `open_menu` already started its opener; a direct
    /// `BlockEntityRef::open_menu` must start it after its checks succeed.
    pub(super) fn open_block_menu_with_opener(
        &mut self,
        conn: u32,
        pos: (i32, i32, i32),
        opener_started: bool,
    ) -> MenuOpen {
        let generation = self.world.block_entities.generation(&pos);
        let name = &self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name;
        let block = name.to_string();
        let (menu, slots) = if &**name == "minecraft:ender_chest" {
            // EnderChestBlock.useWithoutItem: the player's own inventory.
            ("minecraft:generic_9x3", Slots::Ender)
        } else if let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) {
            // AbstractFurnaceBlock.openContainer: FurnaceMenu, BlastFurnaceMenu, SmokerMenu.
            let menu = match f.kind.recipe_type() {
                "minecraft:blasting" => "minecraft:blast_furnace",
                "minecraft:smoking" => "minecraft:smoker",
                _ => "minecraft:furnace",
            };
            (menu, Slots::Block(pos))
        } else if let Some(BlockEntity::Crafter(_)) = self.world.block_entities.get(&pos) {
            // CrafterBlock.useWithoutItem: CrafterMenu over the block entity.
            ("minecraft:crafter_3x3", Slots::Block(pos))
        } else if let Some(BlockEntity::Brewing(_)) = self.world.block_entities.get(&pos) {
            // BrewingStandBlock.useWithoutItem: BrewingStandMenu.
            ("minecraft:brewing_stand", Slots::Block(pos))
        } else if let Some((other, right)) = self.chest_partner(pos) {
            // ChestBlock.combine: the right half's slots come first.
            let halves = if right {
                Slots::Double(pos, other)
            } else {
                Slots::Double(other, pos)
            };
            ("minecraft:generic_9x6", halves)
        } else if let Some(menu) = self.mod_menu_at(pos) {
            (menu, Slots::Block(pos))
        } else {
            let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&pos) else {
                return MenuOpen::Refused;
            };
            let menu = match items.len() {
                27 => "minecraft:generic_9x3",
                9 => "minecraft:generic_3x3",
                5 => "minecraft:hopper",
                _ => return MenuOpen::Refused,
            };
            (menu, Slots::Block(pos))
        };
        // The menu provider's getDisplayName: a container's custom name
        // (BaseContainerBlockEntity.getName) or its default name.
        let custom = |k: (i32, i32, i32)| self.block_name_plain(k);
        let title = match slots {
            Slots::Ender => horizon_protocol::text::translatable("container.enderchest", &[]),
            Slots::Double(first, second) => horizon_client::containers::double_chest_name(
                custom(first).as_deref(),
                custom(second).as_deref(),
            ),
            Slots::Block(k) => {
                // A mod block's container is named after its block
                // (`Block.getDescriptionId`).
                let key = match horizon_client::containers::default_name_key(name) {
                    Some(key) => key.to_owned(),
                    None if self.mod_menu_at(k).is_some() => {
                        format!("block.{}", name.replace(':', "."))
                    }
                    None => return MenuOpen::Refused,
                };
                horizon_client::containers::display_name(custom(k).as_deref(), &key)
            }
            // Opened by a mod (`open_menu`), never here.
            Slots::Native => return MenuOpen::Refused,
        };
        let Some(host) = self.lan.as_ref() else {
            return MenuOpen::Refused;
        };
        let Some(items) = host
            .hub
            .guest(conn)
            .and_then(|g| slots.read(&self.world, g))
        else {
            return MenuOpen::Refused;
        };
        // The block entity's createMenu: a shulker box opens a ShulkerBoxMenu
        // (not a 9x3 ChestMenu), whose slots refuse shulker boxes.
        let menu = match slots {
            Slots::Block(_) => horizon_client::containers::block_menu_type(name).unwrap_or(menu),
            _ => menu,
        };
        // BaseContainerBlockEntity.createMenu (and a double chest's own menu
        // provider, which asks both halves) refuses a locked container with
        // `sendChestLockedNotifications`: the action bar names it and everyone
        // near hears the locked chest; no menu opens.
        let (spectator, hand) = host.hub.guest(conn).map_or((false, None), |g| {
            (
                g.spectator(),
                g.inventory.get(g.selected).cloned().flatten(),
            )
        });
        let halves = match &slots {
            Slots::Double(first, second) => [Some(*first), Some(*second)],
            Slots::Block(k) => [Some(*k), None],
            _ => [None, None],
        };
        let Some((uuid, name)) = host.hub.guest(conn).map(|g| (g.uuid, g.name.clone())) else {
            return MenuOpen::Refused;
        };
        let mut locked = false;
        for key in halves.iter().flatten().copied() {
            let mut check = self.container_locked_for(key, spectator, hand.as_ref());
            self.native_block_lock_check(uuid, &name, [key.0, key.1, key.2], &mut check);
            locked |= check;
        }
        if locked {
            let Some(host) = self.lan.as_mut() else {
                return MenuOpen::Refused;
            };
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_client::containers::locked_message(&title),
                    overlay: true,
                }),
            );
            // `ChestBlock`: the double chest sounds between its halves, from
            // the first one's centre plus half a block toward the other.
            let at = match &slots {
                Slots::Double(first, second) => [
                    f64::from(first.0) + 0.5 + f64::from((second.0 - first.0).signum()) / 2.0,
                    f64::from(first.1) + 0.5,
                    f64::from(first.2) + 0.5 + f64::from((second.2 - first.2).signum()) / 2.0,
                ],
                Slots::Block(k) => [
                    f64::from(k.0) + 0.5,
                    f64::from(k.1) + 0.5,
                    f64::from(k.2) + 0.5,
                ],
                _ => [0.0; 3],
            };
            let name = "minecraft:block.chest.locked";
            self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), None);
            return MenuOpen::Refused;
        }
        // A mod menu type only for a client that has it: as
        // `ServerPlayer.openMenu` with no menu, nothing opens.
        let Some(host) = self.lan.as_ref() else {
            return MenuOpen::Refused;
        };
        let brand = host.hub.guest(conn).map_or("", |g| g.brand.as_str());
        if !horizon_client::containers::menu_reaches(brand, menu) {
            return MenuOpen::Refused;
        }
        // The mods may stop the menu; the click is spent.
        if !self.guest_may_open(conn, menu, Some([pos.0, pos.1, pos.2])) {
            return MenuOpen::Cancelled;
        }
        let mut cancel = false;
        self.native_inventory_block_start(uuid, &name, [pos.0, pos.1, pos.2], &mut cancel);
        if cancel {
            return MenuOpen::Refused;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        let previous_window = self.lan.as_ref().and_then(|host| {
            super::super::native_menus::open_menu_of(host, conn).map(|(window, ..)| window)
        });
        self.guest_close_all(conn);
        // A hand-off from `open_menu` must not count one player twice. A direct
        // handle open has no earlier `startOpen`; only it starts the block's lid.
        let counts_openers =
            !opener_started && matches!(slots, Slots::Block(_) | Slots::Double(_, _));
        let hidden = self.native_hidden_slots(pos, items.len());
        // A mod menu type's block tells the client where it is, with what its
        // mod writes (`openMenu` with an extra-data writer); a stock menu
        // sends nothing more.
        let extra = match slots {
            Slots::Block(k) if self.mod_menu_at(k).is_some() => {
                let mut data = None;
                self.native_menu_extra_data(uuid, menu, &block, [k.0, k.1, k.2], &mut data);
                let mut data = data.unwrap_or_default();
                if data.len() > horizon_protocol::menu_extra::MAX_DATA {
                    log::warn!(
                        "Horizon: {block}'s menu extra data ({} bytes) is past {} bytes; sent without it",
                        data.len(),
                        horizon_protocol::menu_extra::MAX_DATA
                    );
                    data.clear();
                }
                Some(data)
            }
            _ => None,
        };
        if self.world.block_entities.generation(&pos) != generation {
            if let Some(container) = previous_window
                && let Some(host) = self.lan.as_mut()
                && super::super::native_menus::open_menu_of(host, conn).is_none()
            {
                host.hub
                    .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
            }
            return MenuOpen::Stale;
        }
        let Some(host) = self.lan.as_mut() else {
            return MenuOpen::Refused;
        };
        host.hub
            .open_container(conn, menu, title, pos, slots, items, extra);
        if !hidden.is_empty() {
            host.hub.hide_menu_slots(conn, hidden);
        }
        // `ContainerOpenersCounter.incrementOpeners`: a direct handle open
        // starts the lid once, sounding on its first opener. A spectator's
        // `startOpen` does nothing, so its close must have nothing to decrement.
        if counts_openers && !spectator && self.container_openers_changed(pos, true) {
            self.container_edge_sound(pos, true);
        }
        self.player_menu_opening(conn);
        MenuOpen::Opened
    }

    /// `ContainerEntity.interactWithContainerVehicle` for a guest: the menu
    /// the vehicle's own `MenuProvider.createMenu` builds - three rows of a
    /// `ChestMenu` for a chest boat or a chest minecart, a `HopperMenu` for a
    /// hopper minecart - titled with the vehicle's name.
    pub(super) fn lan_open_vehicle_container(&mut self, conn: u32, pos: (i32, i32, i32)) -> bool {
        let entity_type = if let Some(cart) = self.carts.iter().find(|c| c.id as i32 == pos.2) {
            cart.kind.id().to_owned()
        } else if let Some(boat) = self.boats.iter().find(|b| b.id as i32 == pos.2) {
            if !boat.chest {
                return false;
            }
            boat.entity_type()
        } else {
            return false;
        };
        // `createMenu` (ChestMenu.threeRows or a HopperMenu), and
        // `MenuProvider.getDisplayName`, the vehicle's name key.
        let Some((menu, _)) = horizon_client::containers::vehicle_menu(&entity_type) else {
            return false;
        };
        let title = horizon_protocol::text::translatable(
            &horizon_client::containers::vehicle_menu_name(&entity_type),
            &[],
        );
        let slots = Slots::Block(pos);
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        // The mods may stop the menu; the click is spent. A fake player is
        // stopped here too: the stand-in the open staged leaves the level
        // through the close path, its items back on the vehicle, where a
        // later transfer finds them. A stand-in another guest's menu still
        // backs is not this use's to remove: the close is only for the one
        // nobody is looking at.
        let boat = self
            .boats
            .iter()
            .position(|b| b.id as i32 == pos.2 && b.chest);
        let spectator_pending = boat.is_some_and(|i| {
            host.hub.guest(conn).is_some_and(|g| g.spectator())
                && self.boat_loot_pending(self.boats[i].id)
        });
        if spectator_pending {
            // ServerPlayer.openMenu: the refused spectator's action bar, red.
            let message = horizon_client::text::Text::translate("container.spectatorCantOpen", &[])
                .with_color(horizon_client::text::RED);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_client::lan::data::component_nbt(&message.component()),
                    overlay: true,
                }),
            );
        }
        if spectator_pending || !self.guest_may_open(conn, menu, None) {
            let viewed = self.lan.as_ref().is_some_and(|host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| host.hub.here(g))
                    .any(|g| {
                        g.menu
                            .as_ref()
                            .is_some_and(|m| m.slots == Slots::Block(pos))
                    })
            });
            if !viewed {
                self.close_cart_container(pos);
            }
            return true;
        }
        if let Some(i) = boat
            && self.boat_loot_pending(self.boats[i].id)
        {
            let id = self.boats[i].id;
            self.unpack_boat_loot(i, true);
            // Advancement rewards can remove the boat while generating its loot.
            if !self.boats.iter().any(|boat| boat.id == id) {
                return false;
            }
        }
        let Some(items) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|g| slots.read(&self.world, g))
        else {
            return false;
        };
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        host.hub
            .open_container(conn, menu, title, pos, slots, items, None);
        self.player_menu_opening(conn);
        true
    }

    /// The guest's open container is closing (`AbstractContainerMenu.removed`):
    /// it no longer counts among the openers of the lid, and the lid's close
    /// sound is played for everyone around (`ChestBlockEntity.onClose`, a
    /// `playSound` with no player), the guest included, once the last opener
    /// has left.
    pub(in crate::game) fn guest_container_closed(&mut self, conn: u32) {
        let pos = self.lan.as_ref().and_then(|host| {
            let guest = host.hub.guest(conn)?;
            (!guest.spectator())
                .then(|| guest.menu.as_ref().and_then(|m| m.pos))
                .flatten()
        });
        if let Some(pos) = pos
            && self.container_openers_changed(pos, false)
        {
            self.container_edge_sound(pos, false);
            let post = |game: &mut Self| {
                game.post_block_game_event(
                    "minecraft:container_close",
                    [pos.0, pos.1, pos.2],
                    Some(horizon_client::mobs::Who::Player),
                    None,
                );
            };
            if self.lan_acting == Some(conn) {
                post(self);
            } else {
                self.as_guest(conn, post);
            }
        }
    }

    /// Closes whatever menu the guest has open, the way each `ContainerClose`
    /// does (`ServerPlayer.doCloseContainer`, `AbstractContainerMenu.removed`):
    /// the container's lid, a vehicle's stand-in (its items go back to the
    /// vehicle), the trade (written back to the villager), the crafting table,
    /// station and mount menus, and the cursor stack back to the inventory.
    /// Nothing is sent to the client: this is for a guest who is leaving or
    /// whose server is closing. The guest must be in `host.hub.guests` and the
    /// level they are in must be the one in place.
    pub(in crate::game) fn guest_close_all(&mut self, conn: u32) {
        self.guest_remove_menus(conn, MenuRemoval::Live);
    }

    pub(in crate::game) fn guest_disconnected(&mut self, conn: u32) {
        self.guest_remove_menus(conn, MenuRemoval::Disconnected);
    }

    pub(super) fn guest_remove_menus(&mut self, conn: u32, removal: MenuRemoval) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let open_menu = host.hub.guests[gi]
            .menu
            .as_ref()
            .map(|m| (m.window, m.slots));
        if removal == MenuRemoval::Disconnected {
            self.guest_drop_carried(gi);
        }
        if let Some((window, slots)) = open_menu {
            self.guest_container_closed(conn);
            if let Slots::Block(key) = slots
                && key.1 == crate::game::minecarts::CART_KEY_Y
            {
                self.close_guest_vehicle_menu(conn, key);
            }
            if let Some(host) = self.lan.as_mut() {
                host.hub.container_closed(conn, window);
            }
        }
        self.lan_merchant_closed(conn, None);
        self.lan_crafting_removed(conn, None, removal);
        if removal == MenuRemoval::Disconnected {
            self.lan_crafting_removed(conn, Some(0), removal);
        }
        self.lan_station_closed(conn, None);
        self.lan_mount_closed(conn, None);
        if removal == MenuRemoval::Live {
            self.guest_carried_back(gi);
        }
        // `PlayerContainerEvent.Close`, after the menu's `removed`.
        self.player_menu_closing(conn);
    }

    /// `ContainerEntity`'s `Container.stopOpen` when a vehicle menu closes
    /// (`AbstractChestBoat.stopOpen` / `MinecartChest.stopOpen`): the menu's
    /// slots go back to the vehicle, the shared placeholder leaves
    /// `world.block_entities`, and CONTAINER_CLOSE fires at the vehicle with
    /// the closing player as the source. A hopper minecart keeps
    /// `Container.stopOpen`'s no-op. Called by the close packet and by the
    /// `ServerPlayer.tick` reach/destroyed close.
    pub(super) fn close_guest_vehicle_menu(&mut self, conn: u32, key: (i32, i32, i32)) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let other_viewers =
            Self::vehicle_has_other_viewers(self.container, &host.hub.guests, Some(conn), key);
        self.store_cart_items(key);
        if !other_viewers {
            self.world.block_entities.remove(&key);
        }
        let at = if let Some(b) = self.boats.iter().find(|b| b.id as i32 == key.2) {
            Some([b.position.x, b.position.y, b.position.z])
        } else if let Some(c) = self.carts.iter().find(|c| c.id as i32 == key.2) {
            (c.kind == crate::game::minecarts::CartKind::Chest).then_some([
                c.position.x,
                c.position.y,
                c.position.z,
            ])
        } else {
            None
        };
        if let Some(at) = at {
            let spect = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(|g| g.spectator());
            let ctx = horizon_client::sculk::Context {
                source: Some(horizon_client::mobs::Who::Player.into()),
                spectator: spect,
                targetable: !spect,
                living: true,
                player: true,
                ..Default::default()
            };
            self.post_game_event("minecraft:container_close", at, ctx);
        }
    }

    /// `ServerPlayer.tick`'s `stillValid`: a guest's block container closes
    /// once its block is removed or out of reach. Death removes every kind
    /// of open menu through the same cleanup and synchronous close boundary.
    pub(super) fn lan_containers_tick(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let far: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g))
            .filter(|g| {
                (g.dead && super::super::native_menus::open_menu_of(host, g.conn).is_some())
                    || !self.container_menu_valid(g)
            })
            .map(|g| g.conn)
            .collect();
        for conn in far {
            self.guest_close_menus_and_tell(conn);
        }
    }

    /// `stillValid` of the guest's open container: its blocks are still there
    /// and in reach, or its vehicle exists and is in reach. True with none open.
    pub(super) fn container_menu_valid(&self, g: &horizon_client::lan::guest::Guest) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return true;
        };
        g.menu.as_ref().is_none_or(|m| {
            m.slots.valid(&self.world)
                && m.still_valid(&host.hub.loop_context, &self.world, g)
                && !m.pos.is_some_and(|k| {
                    k.1 == crate::game::minecarts::CART_KEY_Y
                        && !self.chest_vehicle_still_valid(k.2, g)
                })
        })
    }

    /// `AbstractContainerMenu.stillValid` of whatever menu the guest has open
    /// (a container or vehicle, a trade, the crafting table, a station, a
    /// lectern, a mount's inventory). True when none is open.
    pub(super) fn guest_menu_still_valid(&self, conn: u32) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return true;
        };
        let Some(g) = host.hub.guest(conn) else {
            return true;
        };
        if !self.container_menu_valid(g) {
            return false;
        }
        let Some(p) = host.play.get(&conn) else {
            return true;
        };
        let menus = host.menus.get(&conn);
        menus
            .and_then(|m| m.crafting.table.as_ref())
            .is_none_or(|t| self.lan_table_valid(g, t.3))
            && menus
                .and_then(|m| m.station.as_ref())
                .is_none_or(|s| self.lan_station_valid(g, s))
            && p.lectern
                .as_ref()
                .is_none_or(|l| self.lan_lectern_valid(g, l.key))
            && p.merchant
                .as_ref()
                .and_then(|m| m.merchant.mob)
                .is_none_or(|id| self.lan_merchant_valid(g, id))
            && p.mount_window
                .as_ref()
                .is_none_or(|m| self.lan_mount_valid(g, m.mob))
    }

    /// The check before a menu click: a menu that is no longer valid closes
    /// (returning what it held, as `ContainerClose` does) and the client's
    /// window with it, so the click lands on nothing
    /// (`ServerGamePacketListenerImpl.handleContainerClick` and the others
    /// ask `stillValid` first).
    pub(super) fn guest_validate_menu(&mut self, conn: u32) {
        if self.guest_menu_still_valid(conn) {
            return;
        }
        self.guest_close_menus_and_tell(conn);
    }

    /// `ServerPlayer.closeContainer`: the client is told to close each window first, and
    /// then every menu the guest has open closes through `guest_close_all` (the lid,
    /// stand-ins, the cursor stack back to the inventory, `PlayerContainerEvent.Close`), once.
    pub(in crate::game) fn guest_close_menus_and_tell(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let mut windows: Vec<i32> = host
            .hub
            .guest(conn)
            .and_then(|g| g.menu.as_ref().map(|m| m.window))
            .into_iter()
            .collect();
        if let Some(p) = host.play.get(&conn) {
            let menus = host.menus.get(&conn);
            windows.extend(menus.and_then(|m| m.crafting.table.as_ref()).map(|t| t.0));
            windows.extend(menus.and_then(|m| m.station.as_ref()).map(|s| s.window));
            windows.extend(p.lectern.as_ref().map(|l| l.window));
            windows.extend(p.merchant.as_ref().map(|m| m.window));
            windows.extend(p.mount_window.as_ref().map(|m| m.window));
        }
        if let Some(host) = self.lan.as_mut() {
            for container in windows {
                host.hub
                    .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
            }
        }
        self.guest_close_all(conn);
    }

    /// `ContainerEntity.isChestVehicleStillValid`: the chest minecart or chest
    /// boat is still there and within the guest's entity interaction range
    /// plus 4 of its box.
    pub(super) fn chest_vehicle_still_valid(
        &self,
        id: i32,
        g: &horizon_client::lan::guest::Guest,
    ) -> bool {
        let bounds = self
            .carts
            .iter()
            .find(|c| c.id as i32 == id)
            .map(|c| c.bounds())
            .or_else(|| {
                self.boats
                    .iter()
                    .find(|b| b.id as i32 == id)
                    .map(|b| b.bounds())
            });
        bounds.is_some_and(|b| guest_within_entity_range(g, &b, 4.0))
    }
}
