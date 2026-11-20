//! Sending packets and turning wire values (effects, item stacks) into the game's.

use super::*;

impl Game<'_> {
    /// `Hud.onDisconnected`, from `Minecraft.disconnect` and `clearClientLevel`
    /// (a proxy's reconfiguration too), but the chat: joining clears it and a
    /// reconfiguration keeps it (`ChatComponent.storeState`).
    pub(super) fn hud_disconnected_body(&mut self) {
        self.tab_list.reset();
        self.boss_overlay.reset();
        self.adv.toasts.clear();
        self.debug_charts.reset();
        self.titles.clear();
        self.titles.reset_times();
    }

    pub(super) fn send_body(&self, packet: S) {
        if self.remote.is_some() {
            // MultiPlayerGameMode's attack, interact, useItem, useItemOn, releaseUsingItem, dropItem and
            // piercingAttack run ensureHasSentCarriedItem before their packet: a slot the number keys
            // chose this tick goes out first. A miss, a dig's start or a swap waits for the next tick.
            if let S::Attack(_) | S::Interact(_) | S::UseItem(_) | S::UseItemOn(_) = &packet {
                self.remote_send_carried();
            } else if let S::PlayerAction(a) = &packet
                && matches!(a.action, 4 | 5 | 6 | 8)
            {
                self.remote_send_carried();
            }
            self.send_play(packet);
        }
    }
    /// `Connection.send` for the game's play packets: a mod may cancel one
    /// (`client_packet_send`), and the server never hears of it. True when queued.
    pub(super) fn send_play_body(&self, packet: S) -> bool {
        let queued = self.remote.is_some() && !self.client_packet_send_canceled(&packet);
        if let (true, Some(r)) = (queued, &self.remote) {
            r.session.send(packet);
        }
        queued
    }
    pub(super) fn send_action_body(&self, action: i32) {
        self.send(S::PlayerAction(play::PlayerAction {
            action,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
    }
    /// `LocalPlayer.isShiftKeyDown` (and so `isSecondaryUseActive`): the input as of the player's
    /// last tick (`input.keyPresses`, what the last ServerboundPlayerInputPacket said), not the key
    /// now: a sneak pressed in the frame of a click acts from the next tick, as on the server.
    pub(super) fn remote_shift_down_body(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.last_input & SHIFT_INPUT != 0)
    }
    pub(super) fn next_sequence_body(&mut self) -> i32 {
        let r = self.remote.as_mut().expect("remote");
        r.sequence += 1;
        r.sequence
    }
    pub(super) fn item_name_body(&self, id: i32) -> Arc<str> {
        let r = self.remote.as_ref().expect("remote");
        Arc::from(
            r.builtin
                .get("minecraft:item")
                .and_then(|i| i.name(id))
                .unwrap_or("minecraft:air"),
        )
    }
    /// The server's network id of a mob effect (`brewing::EFFECTS` id).
    pub(super) fn remote_effect_network_id_body(&self, effect: &str) -> Option<i32> {
        let r = self.remote.as_ref()?;
        r.builtin
            .get("minecraft:mob_effect")?
            .id(&format!("minecraft:{effect}"))
    }
    /// A mob effect's network id as the effect (`brewing::EFFECTS` id).
    pub(super) fn remote_effect_id_body(&self, effect: i32) -> Option<&'static str> {
        self.remote_effect_def(effect).map(|def| def.id)
    }
    pub(super) fn remote_effect_def_body(
        &self,
        effect: i32,
    ) -> Option<&'static horizon_client::brewing::EffectDef> {
        let r = self.remote.as_ref()?;
        let name = r.builtin.get("minecraft:mob_effect")?.name(effect)?;
        horizon_client::brewing::effect(name)
    }
    /// ClientPacketListener.handleUpdateMobEffect for the player itself
    /// (`LivingEntity.forceAddEffect`): the effect replaces any of its kind.
    /// Other entities' effects are not shown.
    pub(super) fn remote_effect_body(&mut self, p: &play::UpdateMobEffect) {
        if self.remote.as_ref().is_none_or(|r| r.entity_id != p.id) {
            return;
        }
        let Some(def) = self.remote_effect_def(p.effect) else {
            return;
        };
        let id = def.id;
        let mut e = horizon_client::effects::EffectInstance::from_def(def, p.duration, p.amplifier);
        e.ambient = p.flags & 1 != 0;
        e.visible = p.flags & 2 != 0;
        e.show_icon = p.flags & 4 != 0;
        let list = &mut self.potions.effects.list;
        match list.iter_mut().find(|x| x.id == id) {
            // forceAddEffect's copyBlendState: an updated effect keeps its fade.
            Some(existing) => {
                e.client_blend = existing.client_blend;
                *existing = e;
            }
            None => {
                // ClientboundUpdateMobEffectPacket.shouldBlend (flag 8).
                let blend = p.flags & 8 != 0;
                if id == "darkness" {
                    self.sculk.darkness = super::super::sculk::darkness_blend_added(&e, blend);
                } else if id == "nausea" {
                    let (o_nausea, nausea) = screen_effects::nausea_blend_added(&e, blend);
                    self.screen_fx.o_nausea = o_nausea;
                    self.screen_fx.nausea = nausea;
                } else if !blend {
                    let has_effect = !e.ends_within(0);
                    e.client_blend = (has_effect, has_effect);
                }
                list.push(e);
            }
        }
    }
    pub(super) fn item_id_body(&self, name: &str) -> i32 {
        let r = self.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }
    /// A stack from the server, decoded as the saved and command item NBT
    /// are, so tooltips and models see every field.
    pub(super) fn to_stack_body(&self, s: &Option<Stack>) -> Option<ItemStack> {
        let r = self.remote.as_ref()?;
        let stack = s.as_ref()?;
        if !horizon_protocol::item::custom::permitted(stack, r.registries.custom_components)
            .unwrap_or(false)
        {
            log::warn!("Dropping a stack with unnegotiated custom components");
            return None;
        }
        Some(horizon_client::item_sync::from_network(
            stack,
            &r.builtin,
            &r.registries,
        ))
    }
    /// A stack to the server (`SetCreativeModeSlot`): the same component
    /// patch a Horizon server sends its clients, with the registries' ids, so
    /// the server's echo doesn't hand back a bare item.
    pub(super) fn to_network_body(&self, s: &Option<ItemStack>) -> Option<Stack> {
        let r = self.remote.as_ref()?;
        // A stack as the server sent it, only its count changed: its own
        // components go back, as a vanilla client keeps them.
        if let Some((stack, patch)) = s.as_ref().and_then(|s| Some((s, s.wire_patch()?))) {
            let mut out = Stack::new(self.item_id(&stack.item), stack.count as i32);
            let safe = r.registries.custom_components
                || horizon_protocol::item::custom::entries().next().is_none()
                || horizon_protocol::item::WirePatch::decode(&patch)
                    .and_then(|(added, removed)| {
                        let decoded = Stack {
                            added,
                            removed,
                            ..Stack::new(out.item, out.count)
                        };
                        horizon_protocol::item::custom::permitted(&decoded, false)
                    })
                    .unwrap_or(false);
            if safe {
                out.wire = horizon_protocol::item::WirePatch(Some(patch));
                return Some(out);
            }
            log::warn!("Cannot send cached custom components to this peer");
        }
        horizon_client::lan::guest::stack_to_network(
            |name| self.item_id(name),
            // Built-in first, as a hub's `SyncData::id` does: potions are one.
            |registry, name| {
                r.builtin
                    .get(registry)
                    .and_then(|ids| ids.id(name))
                    .or_else(|| r.registries.id(registry, name))
                    .or_else(|| {
                        (r.registries.custom_components
                            && registry == "minecraft:data_component_type")
                            .then(|| horizon_protocol::item::custom::id(name))
                            .flatten()
                    })
            },
            s,
        )
    }
    /// `HashedStack.create` with the connection's registries (the client's
    /// `decoratedHashOpsGenerator`).
    pub(super) fn hashed_stack_body(&self, s: &Option<ItemStack>) -> Option<Hashed> {
        let r = self.remote.as_ref()?;
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        self.to_network(s).map(|s| Hashed::create(&s, &names))
    }
}
