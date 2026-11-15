//! A key press while a world is open (`KeyboardHandler.keyPress` and the gameplay mappings).
use super::inventory_input::drop_whole_stack;
use super::*;

impl Game<'_> {
    /// A key press while a world is open (keyboard or control socket). `key`
    /// is the vanilla default the pressed mapping translates to, `raw` the
    /// physical key as pressed (the same where the caller had no window key):
    /// `Screen.keyPressed` hears the raw one, and only a raw key settles what
    /// the container's hotbar keys are now (`checkHotbarKeyPressed`).
    pub(super) fn key_pressed(&mut self, key: KeyCode, raw: KeyCode) -> Result<()> {
        self.direct_craft_stop();
        if self.screen == Screen::Playing && self.replay.playing() {
            self.studio_key(key);
            return Ok(());
        }
        // `CreativeModeInventoryScreen.keyPressed` clears `ignoreTextInput` on
        // every key, before it decides anything.
        self.creative_ignore_text = false;
        if self.sleeping.is_some()
            && self.screen == Screen::Playing
            && !matches!(
                key,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F11 | KeyCode::KeyT | KeyCode::Slash
            )
        {
            if matches!(key, KeyCode::Escape | KeyCode::ShiftLeft) {
                self.wake_up(false, true);
            }
            return Ok(());
        }
        if self.sort_key_press(key)
            || horizon_client::keybinds::key_name(key)
                .is_some_and(|name| self.hud_editor_key_press(name))
        {
            return Ok(());
        }
        if key == KeyCode::Escape && self.mod_browser_back() {
            return Ok(());
        }
        if self.remote.is_some() && self.remote_key(key)? {
            return Ok(());
        }
        match key {
            KeyCode::Escape => match self.screen {
                Screen::EmoteWheel => self.close_emote_wheel(None),
                Screen::Options => self.options_key(KeyCode::Escape, None)?,
                Screen::Playing => {
                    if self.replay.playing() && self.captured {
                        self.capture(false);
                        return Ok(());
                    }
                    self.pause_focus = None;
                    self.set_screen(Screen::Paused);
                    self.pause_menu = true;
                    self.capture(false);
                }
                Screen::Creative => {
                    self.pad_clear_focus();
                    self.mod_browser_closed();
                    self.carried = None;
                    self.set_screen(Screen::Playing);
                    self.capture(true);
                }
                Screen::Paused | Screen::Inventory => {
                    // RecipeBookComponent.keyPressed: a narrow window's open
                    // book takes Escape to close itself, not the screen.
                    if self.screen == Screen::Inventory && self.recipe_book_escape() {
                        return Ok(());
                    }
                    self.close_container();
                    self.pause_focus = None;
                    self.set_screen(Screen::Playing);
                    self.pause_menu = true;
                    self.capture(true);
                }
                Screen::GameModeSwitcher => self.close_gamemode_switcher(false),
                Screen::Dead => {
                    if self.ui.death_confirm.is_some() {
                        self.death_confirm_answer(false)?;
                    }
                }
                Screen::Stats | Screen::Friends => self.set_screen(Screen::Paused),
                Screen::Social => self.close_social(),
                Screen::ServerLinks => self.set_screen(Screen::Paused),
                // ConfirmScreen.keyPressed: Escape answers no; both return to
                // the Server Links screen they came from.
                Screen::ConfirmLink => self.set_screen(self.confirm_link_back()),
                Screen::DialogWarning => self.set_screen(Screen::ServerLinks),
                Screen::Advancements => self.close_advancements(),
                Screen::WorldOptions => self.world_options_escape(),
                Screen::CommandBlock => self.close_command_block(),
                Screen::BlockEditor => self.close_block_editor(),
                Screen::Chat => self.close_chat(chat::ChatExit::Intentional),
                Screen::Win => self.close_win_screen()?,
                Screen::DebugOptions => self.close_debug_options(),
                Screen::Mod => self.close_mod_screen(),
                Screen::Studio => self.close_studio_screen(),
                Screen::HudEditor => self.close_hud_editor(),
            },
            // `checkHotbarKeyPressed`: the keys the controls bind to the off
            // hand and the hotbar slots, so a rebound key swaps instead of
            // reaching the screen. The list below keeps Q and the defaults.
            // The raw key asks what the bindings are now: the translated
            // `key` is a vanilla default and may itself be another mapping's
            // rebound key (the off hand on Left Alt while hotbar 7 is on F
            // would find that F a second time and swap with slot 7).
            _ if (self.screen == Screen::Inventory
                || (self.screen == Screen::Creative && !self.creative_screen.tab().search))
                && self
                    .settings
                    .keys
                    .container_hotbar_key(raw)
                    .is_some_and(|key| self.inventory_hotkey(key)) => {}
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
            | KeyCode::KeyF
            | KeyCode::KeyQ
                if (self.screen == Screen::Inventory
                    || (self.screen == Screen::Creative && !self.creative_screen.tab().search))
                    && self.inventory_hotkey(key) => {}
            // The search tab's number keys (scripted presses; the window's
            // keys reach search_tab_hotbar_key before the search box).
            _ if self.screen == Screen::Creative
                && self.creative_screen.tab().search
                && self.search_tab_hotbar_key(key) => {}
            KeyCode::KeyT if self.screen == Screen::Inventory => self.recipe_focus_search(),
            KeyCode::KeyT if self.screen == Screen::Playing => self.open_chat_method(""),
            // CreativeModeInventoryScreen.keyPressed: the chat key opens the
            // search tab from another tab, and `ignoreTextInput` drops the
            // letter it would have typed.
            KeyCode::KeyT
                if self.screen == Screen::Creative && !self.creative_screen.tab().search =>
            {
                if let Some(tab) = self.creative_screen.search_tab() {
                    select_creative_tab(&mut self.creative_screen, &mut self.inv_clicks, tab);
                    self.build_screen();
                    self.creative_ignore_text = true;
                }
            }
            // key.advancements.
            KeyCode::KeyL if self.screen == Screen::Playing => {
                self.open_advancements(false);
                // Opened from the keybinds, like the inventory: vanilla releases
                // an active use in the same tick.
                self.release_use_for_keybind_screen();
            }
            // key.socialInteractions (Gui.handleKeybinds): only on a server; in single player
            // the action bar says so, and the narrator.
            KeyCode::KeyP if self.screen == Screen::Playing && !self.is_multiplayer_server() => {
                let message = horizon_client::text::Text::translate(
                    "multiplayer.socialInteractions.not_available",
                    &[],
                );
                self.titles.set_overlay(message.clone(), false);
                self.say_system_now(&message);
            }
            KeyCode::KeyP if self.screen == Screen::Playing => {
                self.open_social(false);
                self.release_use_for_keybind_screen();
            }
            KeyCode::KeyL if self.screen == Screen::Advancements => self.close_advancements(),
            // key.emote (Horizon's): the emote wheel, a tap again closes it.
            KeyCode::KeyB if self.screen == Screen::Playing && super::cosmetics::enabled() => {
                self.open_emote_wheel();
            }
            KeyCode::KeyB if self.screen == Screen::EmoteWheel => self.close_emote_wheel(None),
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
                if self.screen == Screen::EmoteWheel =>
            {
                let slot = (key as u32).wrapping_sub(KeyCode::Digit1 as u32) as usize;
                self.close_emote_wheel(Some(slot));
            }
            // key.quickActions: only open if a datapack provides dialog actions.
            KeyCode::KeyG if self.screen == Screen::Playing => {
                if self.has_quick_actions() {
                    self.open_quick_actions();
                }
            }
            KeyCode::Slash if self.screen == Screen::Playing => self.open_chat_method("/"),
            // key.toggleGui is a KeyMapping: it clicks only with no screen open.
            KeyCode::F1 if self.screen == Screen::Playing => self.toggle_hide_gui(),
            KeyCode::F11 => self.toggle_fullscreen(),
            KeyCode::F5 if self.screen == Screen::Playing && !self.replay.playing() => {
                self.cycle_camera()
            }
            KeyCode::KeyE
                if self.screen == Screen::Creative && self.creative_screen.tab().search => {}
            KeyCode::KeyE => match self.screen {
                Screen::Playing if self.spectator() || self.in_portal_transition() => {}
                Screen::Playing => {
                    // handleKeybinds: the number keys select before key.inventory.
                    self.select_queued_hotbar();
                    // key.inventory while riding a tamed mount opens its inventory.
                    if !self.open_ridden_mount_menu() {
                        if self.tutorial.open_inventory() {
                            self.persist_tutorial_step();
                        }
                        if self.creative {
                            self.set_screen(Screen::Creative);
                            self.refresh_creative_tabs();
                            self.creative_screen.reopen();
                            self.refresh_saved_hotbars();
                        } else {
                            self.set_screen(Screen::Inventory);
                        }
                        self.capture(false);
                    }
                }
                Screen::Creative => {
                    self.pad_clear_focus();
                    if let Some(c) = self.carried.take() {
                        let _ = self.add_stack_to_inventory(c);
                    }
                    self.set_screen(Screen::Playing);
                    self.capture(true);
                }
                Screen::Inventory => {
                    self.close_container();
                    self.set_screen(Screen::Playing);
                    self.capture(true);
                }
                _ => {}
            },
            // key.swapOffhand: on a server, ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND
            // (the server swaps the hands and sends both slots back; spectators can't).
            KeyCode::KeyF if self.screen == Screen::Playing && self.remote.is_some() => {
                if !self.spectator()
                    && let Some(r) = self.remote.as_mut()
                {
                    r.pending_keys.push(key);
                }
            }
            KeyCode::KeyF if self.screen == Screen::Playing => {
                // handleKeybinds: the number keys select before key.swapOffhand.
                self.select_queued_hotbar();
                self.own_swap_hands();
            }
            KeyCode::KeyQ if self.screen == Screen::Playing => {
                // handleKeybinds: the number keys select before key.drop.
                self.select_queued_hotbar();
                let all = drop_whole_stack(self.modifiers) || self.input_takes_stack_drop();
                self.drop_held(all);
            }
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
                if self.screen == Screen::Playing =>
            {
                let slot = match key {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    KeyCode::Digit6 => 5,
                    KeyCode::Digit7 => 6,
                    KeyCode::Digit8 => 7,
                    _ => 8,
                };
                // Minecraft.handleKeybinds: a spectator's number keys drive
                // the spectator menu, not the hotbar; a creative player
                // holding the load or save toolbar key loads or saves a
                // toolbar instead (load first when both are held).
                let load = self.mapped_key_down("key.loadToolbarActivator");
                if self.spectator() {
                    self.spectator_hotbar_key(slot as i32);
                } else if self.creative
                    && (load || self.mapped_key_down("key.saveToolbarActivator"))
                {
                    self.hotbar_load_or_save(slot, load);
                } else {
                    self.hotbar_presses.push(slot);
                }
            }
            _ if self.screen == Screen::Paused && self.pause_menu => {
                self.pause_key(key)?;
            }
            _ if self.screen == Screen::Dead => {
                self.death_key(key)?;
            }
            _ if self.screen == Screen::Stats => self.stats_key(raw),
            _ => {}
        }
        Ok(())
    }
    /// handlePlayerAction SWAP_ITEM_WITH_OFFHAND for the window's own player
    /// with no server: the mods hear it as a server's players
    /// (`player_swap_hands`), and may stop it or change what each hand gets.
    /// Then setItemInHand for the off hand, then the main hand. The stacks are
    /// copied only for a listener: `heard` holds the old main and off hands.
    fn own_swap_hands(&mut self) {
        if self.spectator() {
            return;
        }
        let heard = mod_api::__private::listened(mod_api::HookId::PlayerSwapHands)
            .then(|| (self.inventory[self.selected].clone(), self.offhand.clone()));
        if let Some((old_main, old_off)) = &heard {
            let (mut to_main, mut to_off, mut cancel) = (old_off.clone(), old_main.clone(), false);
            let (uuid, name) = (self.own_uuid(), self.own_profile_name());
            self.native_player_swap_hands(uuid, &name, &mut to_main, &mut to_off, &mut cancel);
            if cancel {
                return;
            }
            (self.inventory[self.selected], self.offhand) = (to_main, to_off);
        } else {
            std::mem::swap(&mut self.offhand, &mut self.inventory[self.selected]);
        }
        self.hand_stack_replaced(self.selected);
        self.hand_stack_replaced(shield::OFFHAND_SLOT);
        let (offhand, main) = (self.offhand.clone(), self.inventory[self.selected].clone());
        let (old_main, old_off) = match &heard {
            Some((old_main, old_off)) => (old_main.as_ref(), old_off.as_ref()),
            None => (offhand.as_ref(), main.as_ref()),
        };
        self.player_on_equip("offhand", old_off, offhand.as_ref());
        self.player_on_equip("mainhand", old_main, main.as_ref());
        self.using = None;
    }
}

#[cfg(test)]
mod swap_hands_tests {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::items::ItemStack;
    use mod_api::{Flow, Hooks, PlayerSwapHandsEvent, ServerHost};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    /// Puts a torch in the off hand instead of the sword, or cancels; counts the calls.
    struct Torch(Arc<AtomicBool>, Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Torch {
        fn player_swap_hands(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerSwapHandsEvent<'_>,
        ) -> Flow {
            self.1.fetch_add(1, Ordering::SeqCst);
            if self.0.load(Ordering::SeqCst) {
                return Flow::Cancel;
            }
            assert!(event.to_main_hand.is_none());
            assert_eq!(event.to_off_hand.as_ref().map(|s| s.count()), Some(1));
            let torch = mod_api::__private::item_key("minecraft:torch").unwrap();
            *event.to_off_hand = Some(mod_api::ItemStack::new(torch, 1).unwrap());
            Flow::Continue
        }
    }

    /// The window's own swap contract: a cancel
    /// keeps both hands, and a listener's stack lands in the off hand.
    #[test]
    #[ignore = "daily"]
    fn the_own_player_swap_asks_the_listener_for_each_hand() {
        let (cancel, calls) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        let torch = Torch(Arc::clone(&cancel), Arc::clone(&calls));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("torch", Box::new(torch))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
                game.offhand = None;
                game.set_mode_id(3);
                game.own_swap_hands();
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                assert_eq!(
                    game.inventory[selected].as_ref().map(|s| &*s.item),
                    Some("minecraft:diamond_sword")
                );
                assert!(game.offhand.is_none());
                game.set_mode_id(0);
                game.own_swap_hands();
                assert_eq!(
                    game.inventory[selected].as_ref().map(|s| &*s.item),
                    Some("minecraft:diamond_sword")
                );
                assert!(game.offhand.is_none());
                cancel.store(false, Ordering::SeqCst);
                game.own_swap_hands();
                assert!(game.inventory[selected].is_none());
                assert_eq!(
                    game.offhand.as_ref().map(|s| &*s.item),
                    Some("minecraft:torch")
                );
            });
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// The swap through a LAN host's registered connection: the packet goes
    /// through the hub's inbox and the game's packet pass
    /// (`tick_lan_packets`), and what the guest's client is sent comes off
    /// its sender. The hook is heard once a press; a cancel sends the client
    /// both hands as they were, and a listener's stack reaches its off-hand
    /// slot.
    #[test]
    #[ignore = "daily"]
    fn a_guest_swap_through_its_connection_is_heard_once_and_sends_the_hands() {
        use crate::game::play_ping_tests::{fixture, output};
        use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
        let (cancel, calls) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        let torch = Torch(Arc::clone(&cancel), Arc::clone(&calls));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("torch", Box::new(torch))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let (mut game, receivers) = fixture(&mut resources);
            let selected = game.lan.as_ref().unwrap().hub.guests[0].selected;
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
                g.offhand = None;
            });
            // One press through the connection's inbox; the hands (main, off)
            // in the last inventory the client was sent.
            let press = |game: &mut crate::game::Game<'_>| {
                let swap = S::PlayerAction(play::PlayerAction {
                    action: 7,
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                });
                let host = game.lan.as_mut().unwrap();
                host.hub
                    .guests
                    .update_connection(1, |g| {
                        g.inbox
                            .push_back(horizon_client::lan::guest::InboxItem::Packet(swap, None))
                    })
                    .unwrap();
                game.level_pass.first = false;
                game.level_pass.packets_drained = false;
                game.tick_lan_packets();
                let hub = &game.lan.as_ref().unwrap().hub;
                horizon_client::lan::flush_all(hub.guests.iter().map(|g| &g.sender));
                output(&receivers[0])
                    .into_iter()
                    .filter_map(|p| match p {
                        C::ContainerSetContent(c) if c.container == 0 => {
                            Some((c.items[36 + selected].clone(), c.items[45].clone()))
                        }
                        _ => None,
                    })
                    .next_back()
                    .expect("the client was sent its inventory")
            };
            let hub = &mut game.lan.as_mut().unwrap().hub;
            let sword = hub.stack(&Some(ItemStack::new("minecraft:diamond_sword", 1)));
            let torch = hub.stack(&Some(ItemStack::new("minecraft:torch", 1)));
            assert!(sword.is_some() && torch.is_some());
            // Cancelled: both hands as they were.
            assert_eq!(press(&mut game), (sword.clone(), None));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                g.inventory[selected].as_ref().map(|s| &*s.item),
                Some("minecraft:diamond_sword")
            );
            assert!(g.offhand.is_none());
            // A listener's torch for the off hand: the sword is gone and the
            // client is sent the torch.
            cancel.store(false, Ordering::SeqCst);
            assert_eq!(press(&mut game), (None, torch));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert!(g.inventory[selected].is_none());
            assert_eq!(
                g.offhand.as_ref().map(|s| &*s.item),
                Some("minecraft:torch")
            );
        });
    }
}
