use super::*;

impl Game<'_> {
    /// Runs one interaction and restores the caller's swing policy afterwards, on every return path.
    fn with_interaction_swing_scope<R>(&mut self, action: impl FnOnce(&mut Self) -> R) -> R {
        let previous = self.remote.as_ref().map(|remote| remote.interaction_swing);
        let result = action(self);
        if let (Some(remote), Some(previous)) = (self.remote.as_mut(), previous) {
            remote.interaction_swing = previous;
        }
        result
    }

    /// Minecraft.startUseItem: use one crosshair target, then the item itself.
    fn remote_use_inner(&mut self, attempt: mod_api::InteractionAttempt) {
        // Minecraft.startUseItem returns before resetting rightClickDelay
        // while a survival block break is active.
        if self.remote.as_ref().is_some_and(|r| r.is_destroying) {
            return;
        }
        self.use_delay = 4;
        // Minecraft.startUseItem: nothing is used while the hands paddle a boat.
        if self.hands_busy() {
            return;
        }
        // InputEvent.InteractionKeyMappingTriggered for the main hand: a cancel
        // ends the whole click, the off hand gets no turn.
        if let Some(swing) = self
            .native_interaction(InteractionKey::Use, Hand::Main, attempt)
            .canceled_swing()
        {
            if swing {
                self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
            }
            return;
        }
        // An item a mod claims: its use is the mod's, and the server never hears of it.
        if self.mod_use_start() {
            return;
        }
        // Resolve the crosshair once. A passing entity interaction keeps
        // that target and may only fall back to using the item itself.
        let entity = self.remote_use_targets_entity();
        horizon_client::interaction_input::use_on_target(
            &mut RemoteUseWorld(self, attempt),
            entity,
        );
    }
    pub(in crate::game) fn remote_use(&mut self, attempt: mod_api::InteractionAttempt) {
        self.with_interaction_swing_scope(|game| game.remote_use_inner(attempt));
    }
    fn remote_start_attack_inner(&mut self) -> bool {
        if self.clicks.miss_time > 0 || self.hands_busy() {
            return false;
        }
        if self.spectator() {
            // Minecraft.startAttack: a spectator's click on an entity asks
            // to watch through it (MultiPlayerGameMode.spectate); a click on
            // nothing still tells the server (spectatorNoAction).
            let target = self.spectator_click_target();
            self.send(S::SpectatorAction(play::SpectatorAction { target }));
            return true;
        }
        let spear = self
            .held()
            .and_then(|s| horizon_client::spear::Kinetic::of(&s.item));
        if spear.is_some() && self.cannot_attack_with_item(self.hand_strength.ticker, 0) {
            return false;
        }
        // InputEvent.InteractionKeyMappingTriggered, after the checks that stop
        // an attack: a cancel ends it there, swinging unless the mod said not to.
        if let Some(swing) = self
            .native_interaction(
                InteractionKey::Attack,
                Hand::Main,
                mod_api::InteractionAttempt::Press,
            )
            .canceled_swing()
        {
            if swing {
                let animation = spear.map_or(SwingAnimation::DEFAULT, |k| k.attack_animation());
                self.swing_hand(Hand::Main, animation);
            }
            return false;
        }
        // A spear jabs (`MultiPlayerGameMode.piercingAttack`): this client
        // syncs its look so the server's stab raycasts along it, reports the
        // reach and combo of a mob in range, then the server stabs while this
        // client swings, plays the sound and starts its charge again.
        if let Some(kinetic) = spear {
            self.remote_sync_look();
            if let Some((i, distance, _)) = self.mob_hit()
                && self.before_block(distance)
                && self.held_range_allows(distance)
            {
                self.pvp_attacked(self.mobs[i].id as i32, distance);
            }
            // ServerboundPlayerActionPacket.Action.STAB.
            self.send(S::PlayerAction(play::PlayerAction {
                action: 8,
                pos: [0; 3],
                direction: 0,
                sequence: 0,
            }));
            self.hand_strength.reset_attack();
            self.swing_hand(Hand::Main, kinetic.attack_animation());
            self.play_at_feet(kinetic.attack_sound);
            return true;
        }
        if self.mod_attack_targets() {
            self.remote_swing_packet();
            return false;
        }
        let mut instant = false;
        // The crosshair's entity, of any kind (`Minecraft.hitResult`): the nearest one
        // vanilla picks, before the block aimed at.
        if let Some(pick) = self.crosshair_entity(true) {
            // A ghast's fireball or a wind charge the server turns (game/items/deflect.rs),
            // a vehicle, frame, stand, painting, crystal, falling block or TNT it hurts: the
            // server decides; a mob is the one the held item's attack range must hold.
            self.remote_attack_entity(pick.id, pick.distance);
        } else if !self.entity_targeted()
            && let Some(hit) = self.pick()
        {
            // A block hit on air falls through to MISS. `instant` below reads the
            // block again after the dig, which may have broken it.
            if mod_api::__private::listened(mod_api::HookId::LeftClickEmpty)
                && self
                    .res
                    .facts()
                    .is_air(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
            {
                self.native_left_click_empty();
            }
            if !self.remote_mining(true) {
                return false;
            }
            instant = self
                .res
                .facts()
                .is_air(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        } else {
            if mod_api::__private::listened(mod_api::HookId::LeftClickEmpty)
                && !self.entity_targeted()
            {
                self.native_left_click_empty();
            }
            if !self.creative {
                self.clicks.miss_time = 10;
            }
            self.reset_attack_strength();
        }
        self.remote_swing_packet();
        instant
    }
    pub(in crate::game) fn remote_start_attack(&mut self) -> bool {
        self.with_interaction_swing_scope(Game::remote_start_attack_inner)
    }
    fn remote_continue_attack_inner(&mut self, down: bool) {
        if !down {
            self.clicks.miss_time = 0;
        }
        // A piercing weapon neither breaks blocks nor swings on its own.
        if self.clicks.miss_time > 0
            || self.using.is_some()
            || self
                .held()
                .is_some_and(|s| horizon_client::spear::Kinetic::of(&s.item).is_some())
        {
            return;
        }
        let entity = self.entity_targeted();
        if down
            && !entity
            && let Some(hit) = self.pick()
        {
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let air = self.res.facts().is_air(state);
            // InputEvent.InteractionKeyMappingTriggered, before the dig goes on:
            // a cancel digs nothing and swings unless the mod said not to.
            let canceled = if air {
                None
            } else {
                self.native_interaction(
                    InteractionKey::Attack,
                    Hand::Main,
                    mod_api::InteractionAttempt::Held,
                )
                .canceled_swing()
            };
            if canceled.is_none() && !air {
                // MultiPlayerGameMode.continueDestroyBlock starts with ensureHasSentCarriedItem.
                self.remote_send_carried();
            }
            if let Some(swing) = canceled {
                if swing {
                    self.remote_swing_packet();
                }
            } else if self.remote_break_restricted(hit.pos) {
                // continueDestroyBlock returned false: no swing.
            } else if !air {
                if self.remote_mining(false) {
                    self.remote_swing_packet();
                }
            } else {
                if let Some(r) = self.remote.as_mut() {
                    r.destroying = None;
                    r.is_destroying = false;
                }
                self.mining = None;
            }
        } else {
            // `stopDestroyBlock` returns at once for a break that is not running: a pick that
            // missed once a break stopped ends nothing, so the dig's accumulated progress stays
            // and goes on growing on the jumps that follow (arismoko/horizon#4124).
            if self.stop_remote_mining() {
                self.mining = None;
            }
        }
    }
    pub(in crate::game) fn remote_continue_attack(&mut self, down: bool) {
        self.with_interaction_swing_scope(|game| game.remote_continue_attack_inner(down));
    }
}
