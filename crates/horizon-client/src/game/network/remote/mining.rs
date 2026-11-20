//! Mining a block on a server, and using an item on a block.

use super::*;

impl Game<'_> {
    /// `MultiPlayerGameMode.stopDestroyBlock`: aborts the break and starts the attack strength
    /// again, if one is running (`if (!isDestroying) return;`), and says whether it did. Only the
    /// progress goes: `destroyBlockPos` and `destroyingItem` keep the target, so a dig resumed on
    /// the same block sends no START and the server's clock runs on (arismoko/horizon#4124).
    pub(super) fn stop_remote_mining_body(&mut self) -> bool {
        let kept = self.remote.as_mut().and_then(|r| {
            if !std::mem::take(&mut r.is_destroying) {
                return None;
            }
            let (pos, face, _) = r.destroying?;
            r.destroying = Some((pos, face, 0.0));
            r.destroying
        });
        let Some((pos, _, _)) = kept else {
            return false;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.tutorial_destroy(state, -1.0);
        self.send(S::PlayerAction(play::PlayerAction {
            action: 2,
            pos,
            direction: 0,
            sequence: 0,
        }));
        self.reset_attack_strength();
        true
    }

    /// MultiPlayerGameMode.startDestroyBlock / continueDestroyBlock.
    pub(super) fn remote_mining_body(&mut self, start: bool) -> bool {
        let debug_stick = self
            .held()
            .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
        if !self
            .res
            .items()
            .can_destroy_block(self.held(), self.creative)
        {
            self.stop_remote_mining();
            self.mining = None;
            return true;
        }
        let Some(r) = self.remote.as_mut() else {
            return true;
        };
        if !start && r.destroy_delay > 0 {
            r.destroy_delay -= 1;
            return true;
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return true;
        };
        let face = face_direction(hit.face);
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.res.facts().is_air(state) {
            return true;
        }
        // startDestroyBlock refuses a restricted break before predicting it.
        if self.remote_break_restricted(hit.pos) {
            self.mining = None;
            return true;
        }
        if self.creative {
            if self.veto_mining_prediction(hit.pos, hit.face, true, 1.0) {
                return false;
            }
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock never lets the client break the
            // block; the server only selects the property on this START.
            if !debug_stick {
                self.predict_break(hit.pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                r.destroy_delay = 5;
            }
            return true;
        }
        // sameDestroyTarget: the same block and still the item the break began with.
        let same = self.remote.as_ref().is_some_and(|r| {
            r.destroying.is_some_and(|(p, _, _)| p == hit.pos)
                && self.same_destroying_item(&r.destroying_item)
        });
        // startDestroyBlock leaves only a break that is still running alone (`isDestroying &&
        // sameDestroyTarget`); a click on the block a stopped break still names starts it again,
        // so the server re-anchors its clock. A held button carries the same target on, and that
        // one keeps the clock (arismoko/horizon#4124).
        let running = self.remote.as_ref().is_some_and(|r| r.is_destroying);
        if start && same && running {
            return true;
        }
        // The server's own getDestroyProgress: under water, off the ground,
        // with haste, fatigue and efficiency, so the client stops digging
        // when the server is done too (arismoko/horizon#1254).
        let progress = self.destroy_progress(hit.pos);
        if !same || start {
            let proposed = if progress >= 1.0 { 1.0 } else { 0.0 };
            if self.veto_mining_prediction(hit.pos, hit.face, true, proposed) {
                return false;
            }
            // startDestroyBlock aborts only a break that is running; a target left by a stop that
            // ended one is just replaced by the START below.
            let aborted = self.remote.as_mut().and_then(|r| {
                if !std::mem::take(&mut r.is_destroying) {
                    return None;
                }
                r.destroying.take()
            });
            if let Some((old, _, _)) = aborted {
                let old_state = self.world.block(old[0], old[1], old[2]);
                self.tutorial_destroy(old_state, -1.0);
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 2,
                    pos: old,
                    direction: face,
                    sequence: 0,
                }));
            }
            let seq = self.next_sequence();
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            self.tutorial_destroy(state, 0.0);
            // startDestroyBlock: BlockState.attack on the client, so a redstone
            // ore's dust (RedStoneOreBlock.attack) comes before the break.
            self.redstone_ore_particles(hit.pos);
            if progress >= 1.0 {
                // startDestroyBlock: instant survival breaks have no destroy delay.
                if !debug_stick {
                    self.predict_break(hit.pos, state, seq);
                }
                return true;
            }
            let held = self.main_hand_key();
            if let Some(r) = self.remote.as_mut() {
                (r.destroying, r.destroying_item) = (Some((hit.pos, face, 0.0)), held);
                r.is_destroying = true;
            }
            // startDestroyBlock plays the first hit and crack itself; the
            // server's level events 2019 and 2020 do the rest.
            self.breaking_block_effects(hit.pos, hit.face, true);
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                per_tick: 0.0,
                ticks: 0,
            });
            return true;
        }
        let Some((pos, last_face, done)) = self.remote.as_ref().and_then(|r| r.destroying) else {
            return true;
        };
        let done = done + progress;
        if self.veto_mining_prediction(pos, hit.face, false, done) {
            return false;
        }
        self.tutorial_destroy(state, done.clamp(0.0, 1.0));
        let ticks = self.mining.as_ref().map_or(0, |m| m.ticks) + 1;
        if done >= 1.0 {
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock: the server selects the property
            // and sends the block update instead; never predict a break.
            if !debug_stick {
                self.predict_break(pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 3,
                pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                // `destroyBlockPos` remains the last target after the break;
                // if that position is filled again before the next pick, a
                // held attack resumes it without sending a second START.
                r.destroying = Some((pos, face, 0.0));
                r.is_destroying = false;
                r.destroy_delay = 5;
            }
            self.mining = None;
        } else {
            if last_face != face {
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 1,
                    pos,
                    direction: face,
                    sequence: 0,
                }));
            }
            if let Some(r) = self.remote.as_mut() {
                r.destroying = Some((pos, face, done));
            }
            self.mining = Some(Mining {
                pos,
                progress: done,
                per_tick: 0.0,
                ticks,
            });
        }
        true
    }

    pub(super) fn veto_mining_prediction_body(
        &self,
        pos: [i32; 3],
        face: [i32; 3],
        starting: bool,
        progress: f32,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientMiningPredict) {
            return false;
        }
        let mut canceled = false;
        self.native_client_mining_predict(
            mod_api::ClientMiningPrediction {
                pos,
                face,
                starting,
                progress: progress.clamp(0.0, 1.0),
            },
            &mut canceled,
        );
        canceled
    }
    /// `Player.blockActionRestricted` for the block at `pos`: a spectator
    /// never breaks, an adventure player only with a main-hand item whose
    /// `can_break` matches the block (`ItemStack.canBreakBlockInAdventureMode`).
    /// A predicate this client could not decode (exact components) only
    /// tells that the server gave one, and the server sends the block back
    /// if it does not match.
    pub(super) fn remote_break_restricted_body(&self, pos: [i32; 3]) -> bool {
        use horizon_protocol::item::{WirePatch, component::CAN_BREAK};
        if self
            .held()
            .and_then(|s| s.data())
            .is_some_and(|d| d.other("minecraft:can_break").is_some())
        {
            return self.block_action_restricted(pos);
        }
        self.spectator()
            || (!self.may_build()
                && !self
                    .held()
                    .and_then(|s| s.wire_patch())
                    .is_some_and(|p| WirePatch::adds(&p, CAN_BREAK)))
    }

    /// `Minecraft.startUseItem`'s block turn for `hand`: a use that swung drops
    /// the hand (`LocalPlayer.itemUsed`) when it spent the held item or the
    /// player has infinite materials.
    pub(super) fn remote_use_block_body(&mut self, hand: Hand) -> bool {
        let before = self.with_hand(hand, |g| g.held().map(|s| s.count));
        let swings = self.swings_started;
        let done = self.remote_use_item_on(hand);
        if done
            && self.swings_started != swings
            && let Some(after) = self.with_hand(hand, |g| g.held().map(|s| s.count))
            && (Some(after) != before || self.creative)
        {
            self.heights.item_used(hand);
        }
        done
    }

    /// `Item.useOn` on this client's level for the items whose effects are
    /// the clicker's own (`Level.levelEvent(player, ...)` and
    /// `playSound(player, ...)`: the server sends them to the others only):
    /// honeycomb waxing copper, and an axe, hoe or shovel transforming a
    /// block (stripping, scraping, removing wax, tilling, making a path).
    /// A cushion succeeds where it may be placed and spawns on the server, a
    /// firework rocket wherever it may be used, and the server launches it.
    /// Whether the item's use succeeded, which ends the click.
    /// The caller has selected Item.useOn after cooldown and adventure gates.
    pub(super) fn predict_item_on_block_body(
        &mut self,
        stack: &ItemStack,
        hit: &super::super::Hit,
    ) -> bool {
        // FireworkRocketItem.useOn: SUCCESS unless gliding, when Item.use
        // boosts instead. The phase's cooldown/adventure gates already ran.
        if &*stack.item == "minecraft:firework_rocket" {
            return !self.player.as_ref().is_some_and(|p| p.fall_flying);
        }
        let at = [
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.5,
            f64::from(hit.pos[2]) + 0.5,
        ];
        if &*stack.item == "minecraft:honeycomb" {
            // HoneycombItem.useOn.
            let registry = self.pack.assets.registry;
            let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let old = registry.state(clicked);
            let waxes = old
                .and_then(|old| self.res.transformers().waxed(registry, old))
                .is_some();
            if waxes {
                self.level_event_at(3003, hit.pos, 0);
                self.play_client_local_sound(
                    LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                        .in_source(mod_api::SoundSource::Blocks),
                );
                // The other half of a double chest: its particles, and a second
                // sound from the clicked block.
                if let Some(other) =
                    old.and_then(|old| super::super::storage::chest_connected_pos(old, hit.pos))
                {
                    self.level_event_at(3003, other, 0);
                    self.play_client_local_sound(
                        LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                            .in_source(mod_api::SoundSource::Blocks),
                    );
                }
            }
            waxes
        } else if let Some((_, sound, _, _, particle, _)) = self.block_transform(stack, hit) {
            self.transform_particle(&particle, hit.pos);
            self.play_client_local_sound(
                LevelSound::new(&sound, at, 1.0, 1.0).in_source(mod_api::SoundSource::Blocks),
            );
            true
        } else if self
            .res
            .items()
            .get(&stack.item)
            .is_some_and(|i| i.cushion_color.is_some())
        {
            // CushionItem.useOn on a client level: SUCCESS once
            // `canBePlacedAt` holds, before the server's own checks.
            self.cushion_spawn(hit).is_some()
        } else if &*stack.item == "minecraft:painting" {
            // HangingEntityItem.useOn on a client level: SUCCESS once
            // `Painting.create` finds a surviving variant, before the
            // server spawns it.
            self.painting_fits(hit, stack)
        } else {
            false
        }
    }

    /// `MapItem.useOn` on this client: a map used on a banner is SUCCESS
    /// whatever its data says (the server toggles the marker, or fails
    /// quietly), so this hand swings and the click ends. The caller applies
    /// ItemStack.useOn's adventure predicate before selecting this phase.
    pub(super) fn predict_map_on_banner_body(
        &self,
        stack: &ItemStack,
        hit: &super::super::Hit,
    ) -> bool {
        super::super::maps::map_id(stack).is_some()
            && self.named_block_in_tag("minecraft:banners", &self.block_name(hit.pos))
    }

    /// `BoneMealItem.useOn` with the acting hand's `stack` on this client's
    /// level (a copy: nothing grows here): whether it succeeds.
    pub(super) fn remote_bone_meal_succeeds_body(
        &mut self,
        hit: &super::super::Hit,
        stack: &ItemStack,
    ) -> bool {
        let pos = horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let face = horizon_core::geometry::Direction::ALL[face_direction(hit.face) as usize];
        let (ask, _) = self.held_bonemeal(pos, stack);
        self.with_level(|b, level, _| {
            b.bone_meal_succeeds_on_client(
                &mut DryLevel::new(level),
                pos,
                face,
                ask.crop,
                ask.check,
            )
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction: the blocks the
    /// use changed on this client's level at once, each kept as predicted
    /// under the use's sequence until the server acknowledges it (a use the
    /// server refused goes back). Only the blocks a hand opens or flips: a
    /// wooden or copper door, trapdoor or fence gate, a lever or a button;
    /// whatever else the use changes waits for the server.
    pub(super) fn predict_use_body(
        &mut self,
        (changes, sounds): (&DryChanges, &Vec<DrySound>),
        seq: i32,
    ) {
        let facts = self.res.facts();
        let predicted = |state: BlockStateId| hand_operated(&facts.get(state).name);
        if changes.is_empty()
            || !changes
                .iter()
                .all(|(p, s)| predicted(*s) && predicted(self.world.block(p.x, p.y, p.z)))
        {
            return;
        }
        self.predict_blocks(changes, seq);
        // `level.playSound(player, ...)`: the clicker hears it from its own use; the
        // server sends it to the others only.
        for (pos, sound, volume, pitch) in sounds {
            let at = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            let name = if sound.contains(':') {
                sound.to_string()
            } else {
                format!("minecraft:{sound}")
            };
            self.play_client_local_sound(
                LevelSound::new(&name, at, *volume, *pitch).in_source(mod_api::SoundSource::Blocks),
            );
        }
    }
}
