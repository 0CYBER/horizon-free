//! Predicting what a use does before the server answers: placing, buckets, held uses.

use super::*;

impl Game<'_> {
    /// Another player of this level, whose copy passes every click (`Player.interactOn`).
    pub(super) fn is_level_player_body(&self, id: i32) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.level_players.contains_key(&id))
    }

    /// Whether a main-hand right click with `held` would end on the block hit, for the gameplay
    /// guide: what `remote_block_reacts` answers on its dry level.
    pub(super) fn block_takes_click_body(
        &mut self,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
    ) -> bool {
        self.remote_block_reacts(hit, held).0.consumes_action()
    }
    /// `InteractionResult.CONSUME` for this click: the block keeps it and does nothing, so the
    /// item in hand never gets it either (an empty flower pot, a pressed button).
    pub(super) fn block_swallows_click_body(
        &mut self,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
    ) -> bool {
        matches!(
            self.remote_block_reacts(hit, held).0,
            InteractionResult::Consume
        )
    }

    /// `remote_block_reacts` for one hand.
    pub(super) fn remote_block_reacts_body(
        &mut self,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
    ) -> (InteractionResult, DryUse) {
        self.remote_block_reacts_hand_body(Hand::Main, hit, held)
    }

    /// Whether the block hit takes a right click with `held` in `hand`
    /// (`BlockState.useItemOn`, then `useWithoutItem` for the main hand only):
    /// a door, a button, a container, and the blocks it changed. Vanilla's
    /// client runs it on its own level first; this runs it on a copy, and
    /// `predict_use` keeps the changes it predicts.
    pub(super) fn remote_block_reacts_hand_body(
        &mut self,
        hand: Hand,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
    ) -> (InteractionResult, DryUse) {
        // AbstractCauldronBlock.useItemOn (CauldronInteractions): a bucket,
        // bottle or dye the cauldron takes fills or lowers it, and the click
        // ends there instead of falling through to Item.use (which would
        // empty a water bucket into the face of the cauldron).
        if let Some(reacts) = self.remote_cauldron_reacts(hit, held) {
            return reacts;
        }
        if self.jukebox_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        // RespawnAnchorBlock.useItemOn: PASS when the main hand's turn leaves the
        // off hand's glowstone to charge the anchor, so useWithoutItem doesn't run
        // and the off hand's turn comes (glowstone in this hand charges it in
        // `use_item_on`).
        if hand == Hand::Main
            && !self.anchor_takes(hit.pos, held)
            && self.anchor_takes(hit.pos, self.offhand.as_ref())
        {
            return (InteractionResult::Pass, DryUse::default());
        }
        // BeehiveBlock.useItemOn: shears or a glass bottle on a full hive
        // (honey level 5) is SUCCESS, so the click ends after UseItemOn and
        // no UseItem follows (the server does the harvest, see `use_beehive`).
        if held.is_some_and(|s| matches!(&*s.item, "minecraft:shears" | "minecraft:glass_bottle"))
            && self
                .hive_state(hit.pos)
                .is_some_and(|(honey, _)| honey == Some(5))
        {
            return (InteractionResult::Success, DryUse::default());
        }
        // ComposterBlock.useItemOn: a compostable item on a composter below
        // level 8 is SUCCESS on the client too (only the server adds the
        // layer), so the other hand (a torch) is never tried.
        if self.composter_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        self.remote_block_template_hand_body(hand, hit, held, false)
    }

    /// `remote_block_template` for one hand.
    #[allow(dead_code)] // the mod API's point; the hand-aware one is what the game calls
    pub(super) fn remote_block_template_body(
        &mut self,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
        without_item: bool,
    ) -> (InteractionResult, DryUse) {
        self.remote_block_template_hand_body(Hand::Main, hit, held, without_item)
    }

    /// The shared dry-level boundary for a normal block use or the empty
    /// method requested by a canceled `BLOCK` phase's `TryWithEmptyHand`, for
    /// one hand.
    pub(super) fn remote_block_template_hand_body(
        &mut self,
        hand: Hand,
        hit: &super::super::Hit,
        held: Option<&ItemStack>,
        without_item: bool,
    ) -> (InteractionResult, DryUse) {
        use horizon_core::geometry::Direction;
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let player = self.use_context();
        self.with_block_use_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            let hit = horizon_core::geometry::BlockHit {
                block: horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                direction: face,
                location: horizon_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                ),
                inside: hit.inside,
            };
            let mut ticks = horizon_world::level::Ticks::default();
            let result = if without_item {
                use horizon_world::level::LevelAccess as _;
                let state = level.block_state(hit.block);
                b.use_without_item(&mut level, &mut ticks, state, hit, player)
            } else if hand == Hand::Main {
                b.use_item_on(&mut level, &mut ticks, hit, player, held.map(|s| &*s.item))
            } else {
                // `ServerPlayerGameMode.useItemOn`: only the main hand follows a
                // TRY_WITH_EMPTY_HAND with `useWithoutItem`.
                use horizon_world::level::LevelAccess as _;
                let state = level.block_state(hit.block);
                b.block_use_item_on(
                    &mut level,
                    &mut ticks,
                    state,
                    hit,
                    player,
                    held.map(|s| &*s.item),
                )
                .unwrap_or(InteractionResult::Pass)
            };
            (result, (level.changed.into_iter().collect(), level.sounds))
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction, for a block item:
    /// the blocks `BlockItem.place` set at `pos` are placed on this client's
    /// level at once, so the next click of a fast bridge aims at them rather
    /// than past them (the server would refuse it). Its place sound is the
    /// placer's own (`playSound(player, ...)`: the server sends it to the
    /// others only).
    pub(super) fn predict_place_body(
        &mut self,
        (pos, changes): (&horizon_core::position::BlockPos, &DryChanges),
        seq: i32,
    ) {
        let ticks = self.ticks;
        if let Some(r) = self.remote.as_mut() {
            // set_block plays the place sound of the block at `placing`.
            r.placing = Some(([pos.x, pos.y, pos.z], ticks));
        }
        self.predict_blocks(changes, seq);
    }
    /// The blocks a use changed, set on this client's level and kept as
    /// predicted under the use's sequence until the server acknowledges it
    /// (`BlockStatePredictionHandler.retainKnownServerState`): a use the
    /// server refused goes back.
    pub(super) fn predict_blocks_body(&mut self, changes: &DryChanges, seq: i32) {
        for &(p, state) in changes {
            let pos = [p.x, p.y, p.z];
            let before = self.world.block(p.x, p.y, p.z);
            let here = self.player_position();
            if let Some(r) = self.remote.as_mut() {
                r.predictions
                    .entry(pos)
                    .and_modify(|e| e.sequence = seq)
                    .or_insert(Prediction {
                        sequence: seq,
                        state: before.0 as i32,
                        player_pos: here,
                    });
            }
            self.set_block(pos, state.0 as i32, true);
        }
    }

    /// Run BlockItem.place on a disposable level: where it placed and the
    /// blocks it set. The real level supplies collision shapes and
    /// building-blocking entities; random draws, scheduled ticks and events
    /// stay in the preview. `ItemStack.useOn` runs it in adventure mode only
    /// when the used stack's `can_place_on` names the clicked block.
    pub(super) fn remote_can_place_body(
        &mut self,
        hit: &super::super::Hit,
        stack: &ItemStack,
    ) -> Option<DryPlace> {
        if !self.may_build() && !self.stack_can_place_on_in_adventure(stack, hit.pos) {
            return None;
        }
        let ctx = horizon_world::level::PlaceContext {
            clicked_pos: horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face: horizon_core::geometry::Direction::ALL[face_direction(hit.face) as usize],
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: hit.inside,
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.remote_shift_down(),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        self.with_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            match b.place_block(
                &mut level,
                &mut horizon_world::level::Ticks::default(),
                &ctx,
            ) {
                horizon_world::level::PlaceOutcome::Placed { pos, .. } => {
                    Some((pos, level.changed.into_iter().collect()))
                }
                horizon_world::level::PlaceOutcome::Failed => None,
            }
        })
    }

    /// `MultiPlayerGameMode.useItem`'s local `ItemStack.use` for a bucket:
    /// `BucketItem.use` picks its own target and runs `pickupBlock` or
    /// `emptyContents` on this client's level inside `startPrediction`, so
    /// its success (which decides `itemUsed`) and the blocks it changes are
    /// the client's own. A fill into a waterloggable block changes nothing
    /// here: `SimpleWaterloggedBlock.placeLiquid` tests `isClientSide` and
    /// leaves the block update to the server.
    pub(super) fn remote_use_bucket_body(
        &mut self,
        stack: &ItemStack,
    ) -> (bool, Option<DryChanges>, Vec<DrySound>) {
        // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
        // `Player.mayUseItemAt` tests the clicked block against `can_place_on`.
        if !self.may_build() {
            let aim = if &*stack.item == "minecraft:bucket" {
                self.bucket_hit()
            } else {
                self.pick()
            };
            if !aim.is_some_and(|hit| self.can_place_on_in_adventure(hit.pos)) {
                return (false, None, Vec::new());
            }
        }
        if &*stack.item == "minecraft:bucket" {
            let Some(hit) = self.bucket_hit() else {
                return (false, None, Vec::new());
            };
            let pos = horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            return self.with_level(|b, level, ticks| {
                let mut level = DryLevel::new(level);
                let picked = b.bucket_pickup(&mut level, ticks, pos, creative).is_some();
                (
                    picked,
                    Some(level.changed.into_iter().collect()),
                    std::mem::take(&mut level.sounds),
                )
            });
        }
        let Some((content, sound, _)) = self.predicted_bucket_contents(stack) else {
            return (false, None, Vec::new());
        };
        let Some(hit) = self.pick() else {
            return (false, None, Vec::new());
        };
        let target = self.bucket_target(content, &hit);
        let pos = horizon_core::position::BlockPos::new(target[0], target[1], target[2]);
        let [x, y, z] = hit.beside();
        let beside = Some(horizon_core::position::BlockPos::new(x, y, z));
        let sneaking = self.remote_shift_down();
        let creative = self.creative;
        self.with_level(|b, level, ticks| {
            let mut level = DryLevel::new(level);
            let emptied = b.bucket_empty(
                &mut level, ticks, pos, beside, content, sound, sneaking, creative,
            );
            // Water poured into a liquid container (where it went, which may
            // be the block beside a refusing one) fills it server-side only.
            let client_change = !(content.is_water()
                && emptied.is_some_and(|at| {
                    use horizon_world::level::LevelAccess as _;
                    b.is_liquid_container(level.block_state(at))
                }));
            (
                emptied.is_some(),
                client_change.then(|| level.changed.into_iter().collect()),
                std::mem::take(&mut level.sounds),
            )
        })
    }

    /// Whether `held`'s `Item.use` needs a target this client sees none of
    /// (not SUCCESS): BucketItem.use (a fluid source or powder snow to pick
    /// up through `ClipContext.Fluid.SOURCE_ONLY`, a block to empty a full
    /// one against, `mayUseItemAt`), BottleItem.use (water to fill from) and
    /// BoatItem.use (a block or fluid surface with room for the boat).
    pub(super) fn use_finds_no_target_body(&self, held: &ItemStack) -> bool {
        let id = &*held.item;
        let facts = |pos: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(pos[0], pos[1], pos[2]))
        };
        let source = |want: Option<Fluid>| {
            let fluid_hit = self.pick_fluid();
            // pick_fluid steps through waterlogged blocks, but their water is
            // a source the SOURCE_ONLY ray stops at (and a BucketPickup).
            if let Some(hit) = self.pick()
                && facts(hit.pos).waterlogged
            {
                let eye = self.aim().map_or(hit.point, |(eye, _)| eye);
                let d = |p: [f64; 3]| (0..3).map(|a| (p[a] - eye[a]).powi(2)).sum::<f64>();
                if fluid_hit
                    .as_ref()
                    .is_none_or(|fluid| d(fluid.point) >= d(hit.point))
                {
                    return want.is_none_or(|want| want == Fluid::Water);
                }
            }
            fluid_hit.is_some_and(|hit| {
                let f = facts(hit.pos);
                let fluid = f.fluid != Fluid::None && f.fluid_level == 8 && !f.fluid_falling;
                match want {
                    Some(want) => fluid && f.fluid == want,
                    None => fluid || self.is_powder_snow(hit.pos),
                }
            })
        };
        // SolidBucketItem only has useOn: its use is Item.use's PASS.
        if id == "minecraft:powder_snow_bucket" {
            return true;
        }
        if id == "minecraft:bucket" {
            return !self.may_build() || !source(None);
        }
        if horizon_client::items::bucket_contents(id).is_some() {
            return !self.may_build() || self.pick().is_none();
        }
        if id == "minecraft:glass_bottle" {
            // A dragon's breath cloud never counts here: its owner is not
            // synced, so this client's BottleItem.use finds none and PASSes;
            // the packet still goes and the server fills the bottle.
            return !source(Some(Fluid::Water));
        }
        if super::super::boats::any_boat_wood(id).is_some() {
            return self.boat_to_place(held).is_none();
        }
        false
    }
    /// `Item.use` runs on the client too (MultiPlayerGameMode.useItem): an
    /// item held in use until the button lets go starts being used here, so
    /// that letting go sends RELEASE_USE_ITEM (Minecraft.handleKeybinds) and
    /// the server ends the use: a bow or a trident is let go, a crossbow's
    /// charge ends, a shield is lowered, a spyglass stops scoping. Their
    /// sounds, loaded arrows and thrown tridents come from the server.
    /// False when the use FAILs here: a bow or an uncharged crossbow with
    /// nothing to shoot.
    pub(super) fn remote_start_held_use_body(&mut self, held: &ItemStack) -> bool {
        if self.using.is_some() {
            return true;
        }
        let bundle = self.items().get(&held.item).is_some_and(|i| i.bundle);
        let start = match &*held.item {
            // BundleItem.use: always starts; the server drops the contents.
            _ if bundle => true,
            // BowItem.use: with an arrow, or in creative.
            "minecraft:bow" => self.get_projectile(held).is_some() || self.creative,
            // CrossbowItem.use: charging when not charged and with ammunition;
            // a charged one shoots on the server.
            "minecraft:crossbow" => {
                if held.data().is_some_and(|d| !d.charged.is_empty()) {
                    return true;
                }
                self.get_projectile(held).is_some()
            }
            // TridentItem.use: not about to break, Riptide only in water or rain.
            "minecraft:trident" => {
                // FAIL: no dip, and the off hand gets its turn.
                return self.use_trident(held);
            }
            // SpyglassItem.use (its sound is the server's).
            "minecraft:spyglass" => true,
            // InstrumentItem.use: the horn plays here too (`playSound(player, ...)`
            // skips the player's own client), the server's copy tells the others.
            "minecraft:goat_horn" => {
                self.remote_use_instrument(held);
                return true;
            }
            // EnderEyeItem.use: startUsingItem unless an End portal frame is
            // aimed at, with or without a stronghold to throw the eye at.
            "minecraft:ender_eye" => !self.aims_at_end_portal_frame(),
            // Item.use: a kinetic weapon starts being used.
            id if horizon_client::spear::Kinetic::of(id).is_some() => true,
            // ShieldItem.use: anything that blocks attacks, off cooldown.
            // Item.use: a consumable or `blocks_attacks` the server's patch
            // adds starts the use (Consumable.startConsuming, startUsingItem).
            _ => {
                if !self.raise_shield(self.held_slot()) && self.patched_held_use(held) {
                    self.start_using_item();
                }
                return true;
            }
        };
        if start {
            self.start_using_item();
        }
        start || !matches!(&*held.item, "minecraft:bow" | "minecraft:crossbow")
    }
    /// `InstrumentItem.use` on this client: unless the horn is on cooldown,
    /// it is held up, plays its sound at the player and starts its cooldown.
    pub(super) fn remote_use_instrument_body(&mut self, held: &ItemStack) {
        let Some(instrument) = self.res.items().instrument(held) else {
            return;
        };
        let group = horizon_client::cooldowns::group(held, None);
        if self.cooldowns.is_on_cooldown(&group) {
            return;
        }
        self.start_using_item();
        self.play_player_bound_sound(&instrument.sound, 2, instrument.range / 16.0, 1.0, true);
        if instrument.use_duration > 0.0 {
            self.add_cooldown(group, instrument.use_ticks());
        }
    }
    /// Whether `stack` stays in use until the button lets go (bows,
    /// crossbows, tridents, spyglasses, shields, eyes of ender, whose
    /// `getUseDuration` 0 never counts down to the end) or the server ends
    /// it (a mod's item): its use only counts ticks here.
    pub(super) fn held_until_released_body(&self, stack: &ItemStack) -> bool {
        if self.mod_use(stack) {
            return true;
        }
        matches!(
            &*stack.item,
            "minecraft:bow"
                | "minecraft:crossbow"
                | "minecraft:trident"
                | "minecraft:spyglass"
                | "minecraft:ender_eye"
        ) || horizon_client::spear::Kinetic::of(&stack.item).is_some()
            || self.blocks_attacks(stack).is_some()
            || self.patched_held_use(stack)
    }
    /// Whether the server's patch gives `stack`, an item with no consumable
    /// of its own, a `consumable` with a use time or `blocks_attacks` (a
    /// sword made to block): `Item.use` starts a use of it
    /// (`Consumable.startConsuming`, `startUsingItem`) that lasts until the
    /// button lets go or the server stops it, as the client's
    /// `LivingEntity.updateUsingItem` never completes one.
    pub(super) fn patched_held_use_body(&self, stack: &ItemStack) -> bool {
        let (ticks, blocks) = horizon_client::item_sync::patched_use(stack);
        ticks.map_or(blocks, |ticks| ticks > 0)
            && self
                .items()
                .get(&stack.item)
                .is_none_or(|i| i.consumable.is_none())
    }

    /// The rest of Minecraft.startUseItem once the main hand's target passed:
    /// the main hand's item, then the off hand's turn (its block, its item).
    /// The server does each packet's part with that packet's hand only.
    pub(super) fn remote_use_hands_body(&mut self, attempt: mod_api::InteractionAttempt) {
        // Nothing in the crosshair: an empty hand's click is heard, by mods only.
        let miss = mod_api::__private::listened(mod_api::HookId::RightClickEmpty)
            && !self.entity_targeted()
            && self.pick().is_none();
        if miss && self.held().is_none() {
            self.native_right_click_empty(Hand::Main);
        }
        // A successful Item.use always drops the hand (LocalPlayer.itemUsed).
        if self.remote_use_item(Hand::Main) {
            self.heights.item_used(Hand::Main);
            return;
        }
        // The off hand's turn: the event again, then the entity or the block.
        if let Some(swing) = self
            .native_interaction(InteractionKey::Use, Hand::Off, attempt)
            .canceled_swing()
        {
            if swing {
                self.swing_hand(Hand::Off, SwingAnimation::DEFAULT);
            }
            return;
        }
        let off_target = if self.remote_use_targets_entity() {
            self.remote_use_entity(Hand::Off)
        } else {
            self.remote_use_block(Hand::Off)
        };
        if off_target {
            return;
        }
        if miss && self.offhand.is_none() {
            self.native_right_click_empty(Hand::Off);
        }
        if self.remote_use_item(Hand::Off) {
            self.heights.item_used(Hand::Off);
        }
    }
    /// `LivingEntity.updatingUsingItem` on a server client: an item held in
    /// use (`held_until_released`), and a consumable's eating animation and
    /// its crumbs until the server's USE_ITEM_COMPLETE. The server sends the
    /// stack, its hunger and the sounds.
    pub(super) fn remote_tick_using_body(&mut self) {
        let Some((slot, ticks)) = self.using else {
            // A mod's use the game reset (the server stopping the use...).
            self.end_mod_use();
            return;
        };
        let stack = self.stack_in(slot).cloned();
        // The use stops when the hand no longer holds the item it began with
        // (ItemStack.isSameItem).
        let item = stack.as_ref().map(|s| Arc::clone(&s.item));
        if let Some(r) = self.remote.as_mut() {
            if ticks == 0 {
                r.use_item = item;
            } else if r.use_item != item {
                self.using = None;
                self.end_mod_use();
                return;
            }
        }
        // Another hotbar slot picked: LivingEntity.updatingUsingItem sees a
        // different stack in the hand. The off hand keeps its item.
        let in_hand = slot == self.selected || slot == super::super::shield::OFFHAND_SLOT;
        // A mod's item: held until the button lets go, counted for the mod.
        if self.mod_use_tick(slot, ticks, !self.right || !in_hand) {
            return;
        }
        // updatingUsingItem runs here too: a use a mod's tick ran out stays
        // frozen until let go (RELEASE_USE_ITEM) or the server finishes it.
        if let Some(s) = stack.as_ref().filter(|_| in_hand)
            && !self.use_item_ticked(s, ticks)
        {
            return;
        }
        // BrushItem.onUseTick: with no block aimed the use ends (the server
        // ends it too); a stroke shows its dust. Its sounds are the server's.
        if stack
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brush")
        {
            let elapsed = ticks + 1;
            let hit = self.pick().filter(|_| in_hand);
            if let Some(hit) = &hit
                && elapsed % 10 == 5
            {
                self.brush_dust(hit, slot == super::super::shield::OFFHAND_SLOT);
            }
            if hit.is_none() && in_hand {
                self.send_action(6);
            }
            self.using = hit.and(Some((slot, elapsed))).filter(|_| elapsed < 200);
            return;
        }
        // A bundle's 200-tick use ends with the server's entity event 9, not here.
        if stack
            .as_ref()
            .is_some_and(|s| self.items().get(&s.item).is_some_and(|i| i.bundle))
        {
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A drawn bow, a charging crossbow, a raised shield...: held until
        // the button lets go.
        if let Some(s) = stack.as_ref().filter(|s| self.held_until_released(s)) {
            if in_hand && (self.mod_use(s) || self.blocks_attacks(s).is_some()) {
                self.item_on_use_tick(s, ticks);
            }
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A goat horn is held up for its instrument's use duration.
        if let Some(duration) = stack
            .as_ref()
            .and_then(|s| self.res.items().instrument(s))
            .map(|i| i.use_ticks().max(0) as u32)
        {
            self.using = (in_hand && ticks + 1 < duration).then_some((slot, ticks + 1));
            return;
        }
        let consumable = stack.as_ref().and_then(|s| self.consumable(s));
        let (Some(stack), Some(consumable)) = (stack, consumable) else {
            self.using = None;
            return;
        };
        // Button release belongs to the next handleKeybinds phase, which
        // sends RELEASE_USE_ITEM. Do not silently clear a use begun by a
        // quick press/release between ticks.
        if !in_hand {
            self.using = None;
            return;
        }
        // Consumable.startConsuming eats one with no use time at once, without
        // a use (nor a USE_ITEM_COMPLETE).
        if consumable.ticks() <= 0 {
            self.using = None;
            return;
        }
        // Consumable.shouldEmitParticlesAndSounds, as in tick_used_item. The
        // countdown goes on past zero (LivingEntity.updateUsingItem completes
        // only on the server) until the server's USE_ITEM_COMPLETE.
        // getUseItemRemainingTicks: the duration this use started with.
        let remaining = self.use_duration - ticks as i32;
        if consumable.should_emit(remaining) {
            // `emit_consume` plays the eat sound and spawns the bites (when the
            // item has consume particles), as `tick_used_item` does on the
            // player's own server.
            self.emit_consume(&consumable, &stack, 5);
        }
        self.item_on_use_tick(&stack, ticks);
        self.using = Some((slot, ticks + 1));
    }
    /// `Player.handleEntityEvent(9)`: `completeUsingItem` on the client. A consumable's last crumbs, and FoodProperties.onConsume,
    /// which runs on the client too (Consumable.onConsume calls its
    /// ConsumableListeners on both sides): the client's FoodData eats at once,
    /// so a held right click finds `Player.canEat` false instead of starting
    /// another bite before the server's ClientboundSetHealth arrives.
    pub(super) fn remote_complete_using_body(&mut self) {
        let Some((slot, _)) = self.using.take() else {
            return;
        };
        let Some(stack) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return;
        };
        // Consumable.onConsume's emitParticlesAndSounds(16): the last crumbs
        // and bite, which the server plays for the others only.
        if let Some(consumable) = self.consumable(&stack) {
            self.emit_consume(&consumable, &stack, 16);
        }
        if let Some((nutrition, saturation)) = info.food
            && let Some(r) = self.remote.as_mut()
        {
            // FoodData.eat(nutrition, saturation), as finish_consuming does;
            // the next ClientboundSetHealth replaces it.
            let (_, food, sat) = &mut r.health;
            *food = (*food + nutrition as i32).clamp(0, 20);
            *sat = (*sat + saturation).clamp(0.0, *food as f32);
            self.apply_remote_health();
        }
        // applyAfterUseComponentSideEffects, then LivingEntityUseItemEvent.Finish.
        self.remote_use_item_finished(slot, &stack, &info);
    }
}
