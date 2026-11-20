//! Keys, clicks, attacks, drops and chat the player sends to the server.

use super::*;

impl Game<'_> {
    pub(super) fn request_remote_mining_stop_body(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.stop_mining = true;
        }
    }

    pub(super) fn remote_handle_input_body(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut input = r.input.take_queued();
        let stop = std::mem::take(&mut r.stop_mining);
        // Minecraft.tick runs handleKeybinds only while `gui.screen() == null`: a
        // book, a sign editor, a dialog or a prompt over the world (each a
        // `Screen` in vanilla) stops the attack and use mappings from acting,
        // and `KeyMapping.releaseAll` let go of the buttons when it opened
        // (`track_toggle_screens`), so a dig held into a book ends (#9729).
        let ready = r.loaded_sent
            && self.screen == Screen::Playing
            && self.sleeping.is_none()
            && !self.screen_over_playing();
        // MultiPlayerGameMode.tick, first in Minecraft.tick, screen or not: the slot
        // chosen since the last tick (the wheel, the last tick's number keys, the
        // server's SetHeldSlot) goes out before anything else of this tick.
        self.remote_send_carried();
        if stop || !ready {
            self.stop_remote_mining();
        }
        if !ready {
            // A screen's opening released the keys (`KeyMapping.releaseAll`).
            self.hotbar_presses.clear();
            return;
        }
        let (attack, use_item) = (self.left, self.right);
        input.tick(&mut RemoteInputWorld(self), attack, use_item);
        self.remote_queued_keys();
        if let Some(r) = self.remote.as_mut() {
            input.merge_queued(std::mem::take(&mut r.input));
            r.input = input;
        }
    }

    /// `MultiPlayerGameMode.ensureHasSentCarriedItem`: the server hears of a
    /// new selected slot before anything the slot acts with.
    pub(super) fn remote_send_carried_body(&self) {
        if let Some(r) = &self.remote
            && r.carried.get() != Some(self.selected)
        {
            r.carried.set(Some(self.selected));
            self.send_play(S::SetCarriedItem(play::SetCarriedItem {
                slot: self.selected as i16,
            }));
        }
    }

    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too: its
    /// `isSweepAttack` for a damaging hit on `target` (`living`: a
    /// `LivingEntity`, which may be hit critically), heard by native mods on
    /// both sides. Only the server sweeps.
    pub(super) fn remote_sweep_event_body(&mut self, target: &str, living: bool) {
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let speed = self
            .own_attribute("minecraft:attack_speed")
            .unwrap_or_else(|| self.main_hand_attack_speed());
        let scale = self.hand_strength.scale(0.5, speed);
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            self.own_attribute("minecraft:attack_damage")
                .map_or(1.0, |v| v as f32)
        };
        let magic = scale * (self.enchanted_damage(target, damage, None) - damage);
        if damage <= 0.0 && magic <= 0.0 {
            return;
        }
        let crit = living && self.critical_attack(scale);
        self.sweep_decision(scale, crit, target);
    }
    /// The entity type of the boat, minecart, armor stand or End crystal with
    /// server id `id` that an attack reaches `Player.attack`'s sweep check on,
    /// and whether it is living. A painting or item frame is not: on this
    /// client `skipAttackInteraction` hurts it and stops the attack there.
    pub(super) fn remote_attacked_object_body(&self, id: i32) -> Option<(String, bool)> {
        let id = id as u32;
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            return Some((b.entity_type(), false));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            return Some((c.kind.id().to_owned(), false));
        }
        if self.armor_stands.iter().any(|a| a.id == id) {
            return Some(("minecraft:armor_stand".to_owned(), true));
        }
        self.crystals
            .iter()
            .any(|c| c.id == id)
            .then(|| ("minecraft:end_crystal".to_owned(), false))
    }
    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too.
    /// For a mob `hurtOrSimulate` fails here (`Entity.hurtClient`), so the
    /// hit has no attacker side; but `RemotePlayer.hurtClient` is true, so a
    /// hit on another player with extra knockback runs the attacker's side of
    /// `causeExtraKnockback`: the 0.6 horizontal speed cut and the sprint
    /// stop. Without it the next move outruns the server's player.
    pub(super) fn remote_player_attack_body(&mut self, mob: usize) {
        // Java dispatches this through the target's RemotePlayer type. The
        // level_players map is profile bookkeeping and can lag the entity;
        // mobs must stay out because their hurtClient returns false.
        if self.mobs[mob].kind.id != "minecraft:player" {
            return;
        }
        // `getAttackStrengthScale(0.5)`: here the ticker is `hand_strength`'s
        // (`attack_ticks` only counts in the local world), and the attack
        // speed is the player's synced attribute, which a server may change.
        let speed = self
            .own_attribute("minecraft:attack_speed")
            .unwrap_or_else(|| self.main_hand_attack_speed());
        let strength = self.hand_strength.scale(0.5, speed);
        // Player.attack's damage: a riptide spin's, or this client's own
        // ATTACK_DAMAGE, which the held item and effects don't change here.
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            self.own_attribute("minecraft:attack_damage")
                .map_or(1.0, |v| v as f32)
        };
        let knockback = self
            .own_attribute("minecraft:attack_knockback")
            .map_or(0.0, |v| v as f32);
        let sprinting = self.player.as_ref().is_some_and(|p| p.sprinting);
        // attackVisualEffects: a RemotePlayer's hurtClient is true, so a
        // critical hit (full strength, canCriticalAttack) with damage left
        // after baseDamageScaleFactor runs LocalPlayer.crit, this client's
        // own CRIT emitter beside the one the server's Animate brings.
        // magicBoost is 0 on a client, so no ENCHANTED_HIT one.
        if damage * (0.2 + strength * strength * 0.8) > 0.0 && self.critical_attack(strength) {
            use horizon_client::particles::{ParticleOptions, ParticleType};
            let m = &self.mobs[mob];
            self.particles.tracking_emitter(
                ParticleOptions::simple(ParticleType::Crit),
                TrackingTarget::Entity(m.id as i32),
                mob_tracking_body(m),
            );
        }
        if !horizon_client::attack_strength::attacker_slows_after_hit(
            damage, knockback, strength, sprinting,
        ) {
            return;
        }
        if let Some(p) = self.player.as_mut() {
            let slowdown = horizon_client::attack_strength::ATTACKER_SLOWDOWN;
            p.velocity = Vec3::new(
                p.velocity.x * slowdown,
                p.velocity.y,
                p.velocity.z * slowdown,
            );
        }
        self.stop_sprint_after_extra_knockback();
    }
    /// The latency the server's player list gives this player
    /// (`PlayerInfo.getLatency`), none before the server lists it. A new entry reads 0 until the
    /// server's first latency update (every 600 ticks).
    pub(super) fn own_latency_body(&self) -> Option<i32> {
        let r = self.remote.as_ref()?;
        r.tab_players.get(&r.profile).map(|me| me.latency)
    }

    /// An attribute's value on this client's player, if its map has the attribute.
    pub(super) fn own_attribute_body(&mut self, id: &str) -> Option<f64> {
        let p = self.player.as_mut()?;
        let key = p.attributes.key(id)?;
        p.attributes.lookup(Some(&key))?;
        p.attributes
            .value(Some(key), horizon_world::attribute_map::ValueKind::Computed)
            .ok()
    }

    /// Keys with a multiplayer meaning; returns true when consumed.
    pub(super) fn remote_key_body(&mut self, key: KeyCode) -> Result<bool> {
        if self.screen != Screen::Playing {
            if (key == KeyCode::KeyE || key == KeyCode::Escape) && self.screen == Screen::Inventory
            {
                self.remote_close_container();
                self.set_screen(Screen::Playing);
                self.capture(true);
                return Ok(true);
            }
            return Ok(false);
        }
        match key {
            KeyCode::KeyT | KeyCode::Slash | KeyCode::Enter => {
                // ChatScreen, over the server's command tree.
                self.open_chat_method(if key == KeyCode::Slash { "/" } else { "" });
                // `Minecraft.handleKeybinds` opens it in the call that then
                // releases an active use (Gui.handleKeybinds runs before the
                // isUsingItem branch).
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            KeyCode::KeyQ => {
                if self.spectator() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            // Open the ridden vehicle's own screen (`HasCustomInventoryScreen`)
            // instead of the player's inventory: riding a horse, llama or
            // chest boat, the inventory key sends
            // ServerboundPlayerCommandPacket OPEN_INVENTORY.
            KeyCode::KeyE if self.riding_mount_with_menu() || self.riding_chest_boat() => {
                if self.in_portal_transition() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            KeyCode::KeyE => {
                if self.in_portal_transition() {
                    return Ok(true);
                }
                // The inventory opens without telling the server.
                if self.tutorial.open_inventory() {
                    self.persist_tutorial_step();
                }
                self.recipe_ui.reset_screen();
                if self.creative {
                    self.set_screen(Screen::Creative);
                    self.refresh_creative_tabs();
                    self.creative_screen.reopen();
                    self.refresh_saved_hotbars();
                } else {
                    self.set_screen(Screen::Inventory);
                }
                self.craft_size = 2;
                self.capture(false);
                // `Minecraft.handleKeybinds` opens the inventory in the same
                // call that then releases an active use.
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `Minecraft.handleKeybinds`: a key that opens a screen runs
    /// `Gui.setScreen`, whose `KeyMapping.releaseAll` clears the use key, and
    /// the same invocation then releases an active use
    /// (`player.isUsingItem() && !keyUse.isDown()` → `releaseUsingItem`).
    /// Only the screens opened from those keybinds do this (inventory, chat,
    /// advancements); the pause menu, server containers, mount inventories and
    /// dialogs release on the first tick after closing instead, as vanilla
    /// does.
    pub(super) fn release_use_for_keybind_screen_body(&mut self) {
        // Only a server's client holds a use to release.
        if self.remote.is_none() {
            return;
        }
        // `KeyMapping.releaseAll` clears key.use; without this the button still
        // reads held and a use would start again on the first tick back.
        self.right = false;
        horizon_client::interaction_input::release_on_screen_open(&mut RemoteInputWorld(self));
    }
    /// `MultiPlayerGameMode.releaseUsingItem`: sends `RELEASE_USE_ITEM` and
    /// runs the client half of `Item.releaseUsing` (a Riptide launch moves the
    /// player itself). Vanilla reaches it both when the use key is let go and,
    /// in the same tick, when a keybind opens a screen.
    pub(super) fn release_remote_use_body(&mut self) {
        self.send_action(6);
        // a mod's Stop may skip the item's release, and a mod's own release follows.
        let hand = self.used_hand();
        if let Some((slot, ticks)) = self.using.take()
            && let Some(hand) = hand
            && let Some(stack) = self.stack_in(slot).cloned()
        {
            self.with_hand(hand, |g| {
                if !g.use_item_stopped(&stack, ticks) {
                    return;
                }
                if &*stack.item == "minecraft:trident" {
                    g.release_trident(g.ticks_held(&stack, ticks));
                }
                g.mod_release_using(&stack, ticks);
            });
        }
    }
    /// ClientPacketListener.sendChat / sendCommand, signed with a chat
    /// session (game/network/remote_chat.rs). The Chat option's Hidden and System
    /// levels keep chat out and Hidden keeps the commands out (ChatAbilities).
    pub(super) fn send_signed_chat_input_body(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() || !self.chat_abilities().allows(message) {
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            if self.studio_command(command) {
                return;
            }
            self.send_command(command);
        } else {
            self.send_signed_chat(message);
        }
    }
    /// ChatScreen.handleChatInput fires the client event for chat, not commands.
    pub(super) fn send_chat_from_screen_body(&mut self, message: &str) {
        if message.starts_with('/') || !mod_api::__private::listened(mod_api::HookId::ClientChat) {
            self.send_signed_chat_input(message);
        } else if let Some(uuid) = self.remote.as_ref().map(|r| r.profile) {
            let id = self.own_entity_id();
            let sent = chat_after_event(message, |outgoing, cancel| {
                self.native_client_chat(id, uuid, message, outgoing, cancel);
            });
            if let Some(outgoing) = sent {
                self.send_signed_chat(&outgoing);
            }
        }
    }
    /// MultiPlayerGameMode.dropItem (key.drop): predict the removal, then send
    /// the action; swing only if a nonempty stack was removed.
    pub(super) fn remote_drop_body(&mut self, all: bool) {
        // ensureHasSentCarriedItem: the server drops from the slot just chosen.
        self.remote_send_carried();
        let dropped =
            horizon_client::items::take_held_drop(&mut self.inventory[self.selected], all, false);
        self.send_action(if all { 4 } else { 5 });
        if dropped.is_some() {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
    }
    /// Minecraft.handleKeybinds, key.swapOffhand: the server swaps the hands
    /// (ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND at BlockPos.ZERO, facing down) and
    /// sends both slots back; nothing is predicted.
    pub(super) fn remote_swap_hands_body(&mut self) {
        self.send_action(7);
        // Equal stacks swap with no slot update to wait for.
        let (main, off) = (self.stack_in(self.selected), self.offhand.as_ref());
        let moves = !super::super::hand::matches_for_swap(main, off);
        if let Some(r) = self.remote.as_mut() {
            r.swap_answer = [moves; 2];
        }
    }
    /// The server set inventory `slot` (`shield::OFFHAND_SLOT` for the off
    /// hand), `changed` from the stack this client held there. The stack
    /// that answers a swap (`setItemInHand`) is another object, which the
    /// hand shows after the swap animation (`hand_stack_replaced`); other
    /// updates of a hand's slot count as the held stack changing in place,
    /// as this client doesn't predict every use the way vanilla's does
    /// (a thrown snowball's stack shrinks only when the server says so).
    pub(super) fn remote_hand_slot_set_body(&mut self, slot: usize, changed: bool) {
        let hand = if slot == super::super::shield::OFFHAND_SLOT {
            1
        } else if slot == self.selected {
            0
        } else {
            return;
        };
        let answer = self
            .remote
            .as_mut()
            .is_some_and(|r| std::mem::take(&mut r.swap_answer[hand]));
        if answer && changed {
            self.hand_stack_replaced(slot);
        }
    }
    /// A container click: the server answers with the authoritative slots.
    pub(super) fn remote_click_body(&mut self, right: bool) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        let Some(internal) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let input = if self.has_shift_down() {
            Input::QuickMove
        } else {
            Input::Pickup
        };
        if self.screen == Screen::Creative {
            // CreativeModeInventoryScreen.slotClicked: the Survival Inventory tab clicks the
            // player's own InventoryMenu, and its CreativeInventoryListener sends each slot that
            // changed (SetCreativeModeSlot). No ContainerClick: a server holding the carried
            // stack would give it back after the Destroy Item slot cleared it (arismoko/horizon#3307).
            self.click_open_inventory(Some(internal), i32::from(right), input);
            return;
        }
        self.remote_menu_ops(&[(Some(internal), i32::from(right), input)]);
    }
    /// `AbstractContainerMenu.setItem`/`initializeContents`: the state id a
    /// click answers with is the open menu's, not another window's.
    pub(super) fn remote_menu_state_body(&mut self, container: i32, state: i32) {
        let open = self.remote_menu().0;
        if let Some(r) = self.remote.as_mut().filter(|_| container == open) {
            r.state_id = state;
        }
    }
}
