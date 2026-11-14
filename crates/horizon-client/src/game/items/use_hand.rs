use super::*;

impl Game<'_> {
    /// `Level.playSound(player, ...)`: a guest's client already predicted the
    /// fire-starter sound, so play it locally on the server and forward it to
    /// the other guests without leaving it in the broadcast capture.
    pub(super) fn play_block_use_sound(&mut self, sound: &str, at: [f64; 3], pitch: f32) {
        if self.lan_acting.is_some() {
            self.audio.play_unshared(sound, Some(at), 1.0, pitch);
            self.guest_block_sounds
                .push((sound.to_owned(), at, 1.0, pitch));
        } else {
            self.audio.play(sound, Some(at), 1.0, pitch);
        }
    }

    /// One hand's turn in `startUseItem`: `useItemOn` the block aimed at,
    /// then `useItem`. Whether the click ends here (it acted or failed).
    pub(in crate::game) fn use_hand(&mut self, block_hit: Option<&Hit>) -> Result<bool> {
        if self.guest_use.is_none()
            && let Some(hit) = block_hit
        {
            return self.use_target_then_item(hit);
        }
        self.use_failed = false;
        // isSecondaryUseActive with something in either hand keeps blocks
        // from reacting.
        let sneaking = self.keys.held(Mapping::Sneak);
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        let mut anchor_passed = false;
        // Native mods, where ServerPlayerGameMode.useItemOn starts
        // (RightClickBlock): a cancel answers for the block and the item, and
        // its PASS goes on to Item.use, which a guest's UseItem asks for.
        // A slow tick's line names the phase of a use that took it (#8669): the
        // mods' hook, the block's own use, then the item's.
        use mod_api::InteractionResult as Answer;
        use mod_api::TriState;
        let on_block = matches!(self.guest_use, Some(GuestUse::OnBlock(_)));
        let mut block_hit = block_hit;
        let (mut use_block, mut use_item) = (TriState::Default, TriState::Default);
        let block_phase = horizon_client::spans::guard("use_block");
        if let Some(hit) = block_hit {
            let held = self.held().cloned();
            let answer = horizon_client::spans::time("use_mods", || {
                self.right_click_block(hit, self.hand, held.as_ref())
            });
            match answer {
                native_hooks::BlockClickAnswer::Go {
                    use_block: block,
                    use_item: item,
                } => (use_block, use_item) = (block, item),
                native_hooks::BlockClickAnswer::Canceled(answer)
                    if answer.consumes_action() || answer == Answer::Fail =>
                {
                    if matches!(answer, Answer::Success | Answer::SuccessServer) {
                        self.swing_arm();
                    }
                    self.use_failed = answer == Answer::Fail;
                    return Ok(true);
                }
                native_hooks::BlockClickAnswer::Canceled(_) => {
                    (use_block, use_item) = (TriState::False, TriState::False);
                }
            }
        }
        // ServerPlayerGameMode.useItemOn: the held item's onItemUseFirst, when RightClickBlock didn't deny the item, ahead of the sneak gate and every block behaviour. Any non-PASS answer ends this hand's block interaction.
        if use_item.or(true)
            && let Some(hit) = block_hit
            && let Some(held) = self.held().cloned()
            && let Some(answer) = self.item_use_first(hit, self.hand, &held)
            && answer != Answer::Pass
        {
            if matches!(answer, Answer::Success | Answer::SuccessServer) {
                self.swing_arm();
            }
            self.use_failed = answer == Answer::Fail;
            return Ok(answer.consumes_action() || answer == Answer::Fail);
        }
        // ServerPlayerGameMode.useItemOn: after the spectator and sneak gates, BlockState.useItemOn first. ITEM_USED_ON_BLOCK sees the stack from before it, whichever phase consumes.
        let held = self.held().cloned();
        let empty = ItemStack::new("minecraft:air", 0);
        let before = held.as_ref().unwrap_or(&empty);
        if use_block.or(!(sneaking && hands_full))
            && let Some(hit) = block_hit
        {
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let mut block_answer =
                self.use_item_on_block(hit, self.hand, held.as_ref(), mod_api::UsePhase::Block);
            if block_answer.is_none()
                && mod_api::__private::has_part_hook(mod_api::HookId::UseItemOn)
                && api_values::block_state::block_listens(mod_api::HookId::UseItemOn, state)
                && let Ok(snapshot) = held.try_into_api()
            {
                let click = self.block_click(hit, self.hand, snapshot.as_ref());
                self.native_use_item_on(state, &click, &mut block_answer);
            }
            let mut chains_sign = false;
            if block_answer.is_none() {
                if self.use_on_cauldron(hit.pos)? {
                    self.swing_arm();
                    return Ok(true);
                }
                if let Some(placed) = self.use_on_campfire(hit.pos)? {
                    if placed {
                        self.swing_arm();
                    }
                    return Ok(true);
                }
                if self.use_lectern(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_sign_applicator(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_golem_statue(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if let Some(swings) = self.use_shelf(hit) {
                    self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
                    if swings {
                        self.swing_arm();
                    }
                    return Ok(true);
                }
                if self.use_composter(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_vault(hit.pos, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_pumpkin(hit.pos, hit.face, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_beehive(hit.pos, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_campfire_douse(hit, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                block_answer = self.use_redstone_ore(hit.pos, held.as_ref())?;
                match self.use_respawn_anchor(hit.pos, held.as_ref())? {
                    respawn_anchor::AnchorUse::Charged => {
                        self.swing_arm();
                        return Ok(true);
                    }
                    // The main hand passes; its item is used, then the off hand's glowstone.
                    respawn_anchor::AnchorUse::Pass => anchor_passed = true,
                    respawn_anchor::AnchorUse::Other => {}
                }
                // Hanging signs chain on ceiling undersides and wall edges unless click commands run.
                chains_sign = self.chains_hanging_sign(hit.pos, hit.face, held.as_ref());
            }
            if block_answer.is_none() {
                use horizon_world::level::UseBranch::ItemOn;
                block_answer = self
                    .use_jukebox(hit.pos, state, held.as_ref(), ItemOn)
                    .or_else(|| self.use_decorated_pot(hit.pos, state, held.as_ref(), ItemOn))
                    .or_else(|| self.use_chiseled_bookshelf(hit, state, held.as_ref(), ItemOn));
            }
            // TRY_WITH_EMPTY_HAND: only the main hand's turn uses the block itself (useWithoutItem). The block's item behaviour hears both hands.
            if !anchor_passed && !chains_sign && (self.hand == Hand::Main || block_answer.is_some())
            {
                let face = match hit.face {
                    [0, 1, 0] => Direction::Up,
                    [0, -1, 0] => Direction::Down,
                    [0, 0, -1] => Direction::North,
                    [0, 0, 1] => Direction::South,
                    [-1, 0, 0] => Direction::West,
                    _ => Direction::East,
                };
                let player = self.use_context();
                let block_hit = horizon_core::geometry::BlockHit {
                    block: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                    direction: face,
                    location: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
                    inside: hit.inside,
                };
                let item = held.as_ref().map(|s| s.item.to_string());
                let adds_candle = self
                    .with_level(|b, _, _| b.cake_candle_state(state, item.as_deref()).is_some());
                let pots_flower = self
                    .with_level(|b, _, _| b.potted_flower_state(state, item.as_deref()).is_some());
                let sign_consumes = self.sign_consumes_click(hit.pos);
                use horizon_world::level::{InteractionResult, UseBranch};
                let item_result = match block_answer {
                    Some(answer) => native_hooks::block_use_result(answer),
                    None => self.with_level(|b, level, ticks| {
                        b.block_use_item_on(level, ticks, state, block_hit, player, item.as_deref())
                    }),
                };
                let mut empty_template_ran = false;
                let (result, branch) = if let Some(result) = item_result {
                    (result, UseBranch::ItemOn)
                } else if self.hand == Hand::Main {
                    let mut empty_answer = None;
                    if mod_api::__private::has_part_hook(mod_api::HookId::UseWithoutItem)
                        && api_values::block_state::block_listens(
                            mod_api::HookId::UseWithoutItem,
                            state,
                        )
                        && let Ok(snapshot) = self.held().try_into_api()
                    {
                        let click = self.block_click(hit, self.hand, snapshot.as_ref());
                        self.native_use_without_item(state, &click, &mut empty_answer);
                    }
                    if empty_answer.is_none()
                        && self
                            .res
                            .behaviours()
                            .data(state)
                            .is(horizon_world::level::classes::Class::Fence)
                        && self.use_knot(hit.pos, false)
                    {
                        self.trigger_default_block_use(hit.pos);
                        return Ok(true);
                    }
                    let answer = empty_answer.or_else(|| {
                        self.use_jukebox(hit.pos, state, None, UseBranch::WithoutItem)
                            .or_else(|| {
                                self.use_decorated_pot(hit.pos, state, None, UseBranch::WithoutItem)
                            })
                            .or_else(|| {
                                self.use_chiseled_bookshelf(
                                    hit,
                                    state,
                                    None,
                                    UseBranch::WithoutItem,
                                )
                            })
                    });
                    let result = if let Some(answer) = answer {
                        native_hooks::block_use_result(answer).unwrap_or(InteractionResult::Pass)
                    } else {
                        empty_template_ran = true;
                        let player = self.use_context();
                        self.with_block_use_level(|b, level, ticks| {
                            b.use_without_item(level, ticks, state, block_hit, player)
                        })
                    };
                    (result, UseBranch::WithoutItem)
                } else {
                    (InteractionResult::Pass, UseBranch::ItemOn)
                };
                self.process_level_events()?;
                // CakeBlock.useItemOn: the candle is used up and counted.
                if block_answer.is_none()
                    && adds_candle
                    && result == horizon_world::level::InteractionResult::Success
                    && let Some(used) = held.as_ref()
                {
                    self.award_stat("used", &used.item, 1);
                    self.consume_held();
                }
                // FlowerPotBlock.useItemOn: the plant is potted, used up, and counted.
                if block_answer.is_none()
                    && pots_flower
                    && result == horizon_world::level::InteractionResult::Success
                    && let Some(used) = held.as_ref()
                {
                    self.award_stat("used", &used.item, 1);
                    self.award_custom_stat("pot_flower", 1);
                    self.consume_held();
                    if let (Some(player), Some(name)) =
                        (self.lan_acting_uuid, self.lan_acting_name.clone())
                    {
                        self.native_player_flower_pot_manipulate(
                            player, &name, hit.pos, &used.item,
                        );
                    }
                }
                // These effects belong to the vanilla method, not its Part replacement.
                if empty_template_ran && result == horizon_world::level::InteractionResult::Success
                {
                    let name = self
                        .res
                        .facts()
                        .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                        .name
                        .clone();
                    match &*name {
                        "minecraft:note_block" => self.award_custom_stat("tune_noteblock", 1),
                        "minecraft:bell" => self.award_custom_stat("bell_ring", 1),
                        "minecraft:lever" => self.lever_puff(hit.pos),
                        _ => {}
                    }
                }
                let sign_passes = empty_template_ran
                    && branch == UseBranch::WithoutItem
                    && self.res.facts().get(state).sign_form().is_some()
                    && !sign_consumes;
                if result.consumes_action() && !sign_passes {
                    // ServerPlayerGameMode.useItemOn: ITEM_USED_ON_BLOCK with the stack before the use, or DEFAULT_BLOCK_USE.
                    match branch {
                        horizon_world::level::UseBranch::ItemOn => {
                            self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
                        }
                        horizon_world::level::UseBranch::WithoutItem => {
                            self.trigger_default_block_use(hit.pos);
                        }
                    }
                    if result.should_swing() {
                        // SUCCESS swings on the client too; the beds, light, respawn anchor and sign's own uses are SUCCESS_SERVER.
                        if server_only_block_use(self.res.facts().get(state), branch) {
                            self.swing_arm();
                        } else {
                            self.swing_arm_predicted();
                        }
                    }
                    return Ok(true);
                }
            }
        }
        // RightClickBlock's useItem FALSE: no ItemStack.useOn, so only Item.use is left (a guest's UseItem asks for it).
        if !use_item.or(true) {
            if on_block {
                return Ok(false);
            }
            block_hit = None;
        }
        drop(block_phase);
        let _item_phase = horizon_client::spans::guard("use_item");
        let stack = self.held().cloned();
        let info = stack
            .as_ref()
            .and_then(|s| self.items().get(&s.item))
            .cloned();
        if stack.is_some() && info.is_none() {
            return Ok(false);
        }
        // ServerPlayerGameMode.useItem and useItemOn: an item on cooldown is not used (PASS, so the other hand may still act), unless
        // RightClickBlock's useItem TRUE forces a block click's useOn, even with an empty hand (the broadcast phase can still handle it).
        let forced = on_block && use_item == TriState::True;
        if !forced
            && stack
                .as_ref()
                .zip(info.as_ref())
                .is_none_or(|(stack, info)| {
                    self.cooldowns
                        .is_on_cooldown(&horizon_client::cooldowns::group(
                            stack,
                            info.use_cooldown.as_ref(),
                        ))
                })
        {
            return Ok(false);
        }
        // ItemStack.useOn: adventure's predicate selects Item.useOn before its event and Part, not just before the eventual block placement.
        if let Some(hit) = block_hit
            && !self.may_build()
            && !self.can_place_on_in_adventure(hit.pos)
        {
            if on_block {
                return Ok(false);
            }
            block_hit = None;
        }
        // Native mods, where ItemStack.useOn and Item.use start: the block aimed at had its turn, and the item is off cooldown. A guest's
        // UseItemOn reaches only useOn, its UseItem only use.
        if let Some(hit) = block_hit
            && let Some(answer) = self.use_item_on_block(
                hit,
                self.hand,
                stack.as_ref(),
                mod_api::UsePhase::ItemAfterBlock,
            )
        {
            if answer.consumes_action() {
                self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
                if matches!(answer, Answer::Success | Answer::SuccessServer) {
                    self.swing_arm();
                }
                return Ok(true);
            }
            if answer == Answer::Fail {
                self.use_failed = true;
                return Ok(true);
            }
            if on_block {
                return Ok(false);
            }
            block_hit = None;
        }
        // An empty stack has no item Part or vanilla Item.useOn implementation.
        let (Some(stack), Some(info)) = (stack, info) else {
            return Ok(false);
        };
        if let Some(hit) = block_hit
            && let Some(answer) = self.item_on_interaction(hit, self.hand, &stack)
        {
            if self.interaction_consumed(answer) {
                self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
                return Ok(true);
            }
            if answer == Answer::Fail {
                self.use_failed = true;
                return Ok(true);
            }
        }
        if let Some(used) = self.use_item_fallthrough(&stack, on_block) {
            return Ok(used);
        }
        // LeadItem.useOn follows the block and the item's override, including
        // when sneaking bypassed the block's useWithoutItem.
        if block_hit.is_some() && &*stack.item == "minecraft:lead" && self.interact_knot() {
            return Ok(true);
        }
        // ItemStack.useOn: after the block's own useItemOn/useWithoutItem
        // passed. DebugStickItem.useOn cycles the selected property.
        if let Some(hit) = block_hit
            && let Some(success) = self.debug_stick_use_on(hit.pos)?
        {
            // `Minecraft.startUseItem`: the stick's client-side `useOn` is
            // SUCCESS, so the actor's hand swings; a remote guest swings in
            // its own client, not on the server.
            if self.lan_acting.is_none() {
                self.swing_arm();
            }
            if success {
                // `ServerPlayerGameMode.useItemOn`: a consuming `useOn` triggers ITEM_USED_ON_BLOCK.
                self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
            } else {
                // DebugStickItem.useOn's FAIL.
                self.use_failed = true;
            }
            return Ok(true);
        }
        // TntBlock.useItemOn is the block's own: a sneak with something in hand skips it, and
        // the flint and steel or fire charge lights a fire on the TNT instead.
        if block_hit.is_some()
            && (self.use_bone_meal(&stack)?
                || (!(sneaking && hands_full) && self.ignite_tnt(&stack)?))
        {
            return Ok(true);
        }
        // ShearsItem.useOn: shears crop a growing plant's head to its top age.
        if let Some(hit) = block_hit
            && self.shears_crop(hit.pos, &stack)?
        {
            return self.finish_item_block_use(block_hit, before);
        }
        // A guest's UseItemOn stops at ItemStack.useOn: what Item.use does
        // (throw, drink, eat, raise, equip) waits for its UseItem.
        let spawn_egg = stack.item.ends_with("_spawn_egg");
        use horizon_client::first_person::UseResult;
        let swings = self.swings_started;
        match self.use_tool(&stack, block_hit.is_some())? {
            UseResult::Success { swing } => {
                if (UseResult::Success { swing }).should_swing(swings, self.swings_started) {
                    self.swing_arm();
                }
                return self.finish_item_block_use(block_hit, before);
            }
            UseResult::Fail => {
                self.use_failed = true;
                return Ok(true);
            }
            UseResult::Pass => {}
        }
        // PotionItem.useOn comes before PotionItem.use's drink.
        if let Some(hit) = block_hit
            && self.use_water_bottle_on_block(hit.pos, hit.face, &stack)?
        {
            return Ok(true);
        }
        // a mod's item used over time: its Item.use, once a block item's
        // useOn had its turn.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_mod_use(&stack)
        {
            return Ok(true);
        }
        if (!on_block && self.use_potion_item(&stack)?)
            || ((!on_block || (spawn_egg && block_hit.is_some())) && self.throw_item(&stack)?)
        {
            return Ok(true);
        }
        // BrushItem.useOn: brushing starts on the block aimed at (CONSUME).
        if block_hit.is_some() && &*stack.item == "minecraft:brush" {
            self.start_using_item();
            return Ok(true);
        }
        if !on_block
            && (self.raise_shield(self.held_slot())
                || self.use_fishing_rod(&stack)
            || self.use_crossbow(&stack)
            || self.use_trident(&stack)
            || self.use_spear(&stack)
            || self.boost_firework(&stack)
            || self.use_empty_map(&stack)
                || self.use_book(&stack)
                // KnowledgeBookItem.use: the recipes it names become known.
                || self.use_knowledge_book(&stack)
                // Block items (heads, carved pumpkins) aimed at a block are
                // placed first (useItemOn before useItem).
                || (!(info.block.is_some() && block_hit.is_some())
                    && self.equip_from_hand(&stack)))
        {
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:spyglass" {
            self.use_spyglass();
            return Ok(true);
        }
        // FoodOnAStickItem.use with no boost (not riding its mount, or the
        // mount refused): PASS, but counted as a use of the stick.
        if !on_block
            && matches!(
                &*stack.item,
                "minecraft:carrot_on_a_stick" | "minecraft:warped_fungus_on_a_stick"
            )
        {
            self.award_stat("used", &stack.item, 1);
        }
        // BundleItem.use: startUsingItem, then onUseTick empties it.
        if !on_block && info.bundle {
            self.start_using_item();
            self.swing_arm();
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:goat_horn" {
            self.use_instrument(&stack);
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:bow" {
            // BowItem.use: ArrowNockEvent, then FAIL without ammunition.
            let found_projectile = self.get_projectile(&stack).is_some();
            let has_ammo = found_projectile || self.creative;
            let player = horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid());
            let name = self.acting_player_name().to_owned();
            let (hand, mut cancel) = (self.hand, false);
            self.native_arrow_nock(player, &name, &stack, hand, has_ammo, &mut cancel);
            if has_ammo && !cancel {
                self.start_using_item();
            }
            return Ok(true);
        }
        // Block foods attempt placement before starting consumption.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_eating(&info) {
            return Ok(true);
        }
        // BoatItem.use also performs its own fluid/block raycast.
        if !on_block && self.place_boat(&stack) {
            return Ok(true);
        }
        // ItemStack.useOn passes in adventure mode unless the item's
        // can_place_on matches the clicked block; Item.use then runs as usual.
        if let Some(hit) = block_hit
            && (self.may_build() || self.can_place_on_in_adventure(hit.pos))
            && (self.launch_firework(hit, &stack)
                || self.place_minecart(hit, &stack)
                || self.place_painting(hit, &stack)
                || self.place_item_frame(hit, &stack)
                || self.use_lodestone_compass(hit, &stack, before)
                || self.use_map_on_banner(hit, &stack, before)
                || self.place_armor_stand(hit, &stack)
                || self.place_cushion(hit, &stack)
                || self.place_end_crystal(hit, &stack))
        {
            return Ok(true);
        }
        if self.use_ender_eye(&stack, block_hit.is_some())? {
            return Ok(true);
        }
        // PlaceOnWaterBlockItem.use: the lily pad and frogspawn aim at the block
        // or water source the SOURCE_ONLY ray meets and are placed on the
        // position above it.
        let water_placement = matches!(&*stack.item, "minecraft:lily_pad" | "minecraft:frogspawn");
        let water_hit;
        let hit = if water_placement {
            let Some(h) = self.pick_fluid() else {
                return Ok(false);
            };
            water_hit = Hit {
                pos: [h.pos[0], h.pos[1] + 1, h.pos[2]],
                ..h
            };
            &water_hit
        } else {
            let Some(hit) = block_hit else {
                return Ok(false);
            };
            hit
        };
        if info.block.is_none() {
            return Ok(false);
        }
        if water_placement && !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
            // The fluid-ray placement has its own target, selected by Item.use.
            return Ok(!on_block && self.start_eating(&info));
        }
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        // BlockItem.place through the vanilla placement rules.
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face,
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: false,
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.keys.held(Mapping::Sneak),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        let outcome = if self.player_place_canceled(&ctx) {
            PlaceOutcome::Failed
        } else {
            self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx))
        };
        let PlaceOutcome::Placed { pos, state } = outcome else {
            self.process_level_events()?;
            // BlockItem.useOn: a consumable that could not be placed is used,
            // which ItemStack.useOn counts as a use of the item.
            if self.start_eating(&info) {
                self.award_stat("used", &stack.item, 1);
            } else {
                // BlockItem.place's FAIL.
                self.use_failed = true;
            }
            return Ok(true);
        };
        let target = [pos.x, pos.y, pos.z];
        let game_master = self.can_use_game_master_blocks();
        let state = self.lectern_placed(target, state, &stack, game_master)?;
        let state = self.apply_item_block_state(target, state, &stack)?;
        let key = (target[0], target[1], target[2]);
        if !self.world.block_entities.stateful_entity(state.0)
            && let Some(entity) = self.new_block_entity_for_state(state)
        {
            self.world.block_entities.insert(key, entity);
        }
        if self.world.block_entities.get(&key).is_some() {
            self.apply_placed_item(target, &stack, game_master);
            // Block.setPlacedBy.
            match self.world.block_entities.get(&key) {
                Some(BlockEntity::CommandBlock(_)) => {
                    let data = stack.data().is_some_and(|d| {
                        d.other(horizon_client::block_entity_item::BLOCK_ENTITY_DATA)
                            .is_some()
                    });
                    self.command_block_placed(key, !data);
                }
                Some(BlockEntity::StructureBlock(_)) => self.structure_block_placed(key),
                Some(BlockEntity::JigsawBlock(_)) => self.jigsaw_block_placed(key),
                Some(BlockEntity::TestBlock(_)) => self.test_block_placed(key),
                Some(BlockEntity::Jukebox(_)) => self.jukebox_placed(target, &stack),
                Some(BlockEntity::Sign(sign)) if !sign.waxed && sign.front.has_editable_text() => {
                    self.open_text_edit(key, true)
                }
                _ => {}
            }
        }
        let placer = self.native_acting_player();
        self.block_item_placed(&stack, placer, pos, &ctx);
        // BlockItem.place's SUCCESS: the client swings with its own prediction.
        self.swing_arm_predicted();
        // BlockItem.place's sound is among the level events;
        // SolidBucketItem.getPlaceSound replaces it with the bucket's own.
        let solid_bucket = &*stack.item == "minecraft:powder_snow_bucket";
        if solid_bucket {
            for event in &mut self.level_events {
                if let LevelEvent::Sound { pos: at, sound, .. } = event
                    && *at == pos
                {
                    *sound = "minecraft:item.bucket.empty_powder_snow".into();
                }
            }
        }
        self.process_level_events()?;
        // BlockItem.place: the used statistic and the placed_block trigger.
        self.award_stat("used", &stack.item, 1);
        self.trigger_block("minecraft:placed_block", target, &stack);
        self.check_wither_spawn(target)?;
        self.check_golem_spawn(target)?;
        self.consume_held();
        // SolidBucketItem.useOn: BucketItem.getEmptySuccessItem in hand.
        if solid_bucket && !self.creative {
            *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
        }
        // ItemStack.useOn applies the pre-use stack's component side effects
        // after the successful placement and its hand replacement.
        self.apply_use_cooldown(&stack);
        Ok(true)
    }
}

impl Game<'_> {
    pub(super) fn apply_consume_effect(
        &mut self,
        consumed: &horizon_client::items::ItemStack,
        effect: &horizon_client::items::ConsumeEffect,
        mob: Option<mod_api::LivingRef>,
    ) -> Option<bool> {
        use horizon_client::items::ConsumeEffect;
        Some(match effect {
            ConsumeEffect::ApplyEffects {
                effects,
                probability,
            } => {
                // ApplyStatusEffectsConsumeEffect draws from the user's random,
                // even at probability 1. The level random stands in for a player's.
                let draw = match mob {
                    Some(living) => {
                        let commands::Target::Mob(id) = self.native_living_target(living)? else {
                            return None;
                        };
                        self.mobs
                            .iter_mut()
                            .find(|m| m.id == id)?
                            .random
                            .next_float()
                    }
                    None => self.world.random.next_float(),
                };
                if draw >= *probability {
                    return Some(false);
                }
                let mut changed = false;
                for eff in effects {
                    let Some(def) = horizon_client::brewing::effect(&eff.id) else {
                        continue;
                    };
                    if let Some(living) = mob {
                        let effect = mod_api::ActiveEffect::new(
                            mod_api::__private::effect_key_by_index(def.index)
                                .expect("effects resolved at load"),
                            eff.duration,
                            eff.amplifier,
                            eff.ambient,
                            eff.show_particles,
                            eff.show_icon,
                        );
                        changed |= self.living_add_effect(living, &effect);
                    } else {
                        let mut instance = horizon_client::effects::EffectInstance::new(
                            def.id,
                            eff.duration,
                            eff.amplifier,
                        );
                        instance.ambient = eff.ambient;
                        instance.visible = eff.show_particles;
                        instance.show_icon = eff.show_icon;
                        changed |= self.add_player_effect(instance);
                    }
                }
                changed
            }
            ConsumeEffect::ClearAllEffects => match mob {
                Some(living) => self.living_remove_all_effects(living),
                None => self.remove_player_effects(None),
            },
            ConsumeEffect::RemoveEffects(effects) => {
                let mut changed = false;
                for effect in effects {
                    changed |= match mob {
                        Some(living) => horizon_client::brewing::effect(effect)
                            .and_then(|def| mod_api::__private::effect_key_by_index(def.index))
                            .is_some_and(|key| self.living_remove_effect(living, key)),
                        None => horizon_client::brewing::effect(effect)
                            .is_some_and(|def| self.remove_player_effects(Some(def.index))),
                    };
                }
                changed
            }
            ConsumeEffect::TeleportRandomly(teleport) => {
                if mob.is_some() {
                    return None;
                }
                self.teleport_randomly(teleport, consumed)
            }
            ConsumeEffect::PlaySound(sound) => {
                let at = match mob {
                    Some(living) => {
                        let commands::Target::Mob(id) = self.native_living_target(living)? else {
                            return None;
                        };
                        self.mobs.iter().find(|m| m.id == id).map(|m| m.position)?
                    }
                    None => self.player.as_ref()?.position,
                };
                let b = horizon_core::position::BlockPos::containing(at);
                let at = [b.x, b.y, b.z].map(|v| f64::from(v) + 0.5);
                let level = self.level(self.dim.id())?;
                let source = match mob {
                    Some(living) => self.entity_sound_source(living.entity())?,
                    None => mod_api::SoundSource::Players,
                };
                self.native_queue_sound(level, sound, at, source, 1.0, 1.0, None)?;
                true
            }
            ConsumeEffect::Native { kind, data } => {
                consume_effect_dispatch::dispatch(kind, || self.native_consume_effect(kind, data))
            }
        })
    }
    /// BowItem.releaseUsing: power from charge time, arrows at power * 3.
    pub(super) fn release_bow(&mut self, stack: &ItemStack, ticks: u32) {
        let ammo = self.get_projectile(stack);
        let mut charge = i32::try_from(ticks).unwrap_or(i32::MAX);
        let mut canceled = false;
        self.native_arrow_loose(stack, ammo.is_some(), &mut charge, &mut canceled);
        let power = item_use::bow_release_power(ammo.is_some(), charge, canceled);
        let (Some(power), Some(ammo)) = (power, ammo) else {
            return;
        };
        let arrow_slot = ammo.slot;
        let weapon = Some(stack.clone());
        let Some(eye) = self.eye_position() else {
            return;
        };
        // BowItem.shootProjectile: shootFromRotation(power * 3, inaccuracy 1).
        let p = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let ammo = ammo.stack;
        let (power_bonus, infinity) = self.bow_enchantments(&ammo.item);
        let effect = dispenser::tipped_effect(&ammo).or_else(|| {
            (&*ammo.item == "minecraft:spectral_arrow").then_some(("minecraft:glowing", 200))
        });
        let kind = horizon_client::projectiles::ProjectileKind::Arrow {
            base_damage: 2.0 + power_bonus,
            critical: power >= 1.0,
            effect,
        };
        let mut arrow = horizon_client::projectiles::Projectile::new(
            kind,
            p,
            Some(horizon_client::mobs::Who::Player),
        );
        // ProjectileWeaponItem.useAmmo: creative and Infinity draw without
        // using the ammo, so the arrow is CREATIVE_ONLY; else setOwner(player)
        // lets it be picked up.
        if let Some(a) = arrow.arrow.as_mut() {
            a.set_item(&horizon_client::arrows::drawn_ammo(
                &ammo,
                self.creative || infinity,
            ));
            a.set_owner(Some(horizon_client::mobs::Who::Player));
            a.fired_from_weapon = weapon;
        }
        let (velocity, on_ground) = self.player.as_ref().map_or((Vec3::ZERO, true), |pl| {
            (self.known_player_movement(), pl.ground.on_ground)
        });
        arrow.shoot_from_rotation(
            self.pitch,
            self.yaw,
            0.0,
            power * 3.0,
            1.0,
            velocity,
            on_ground,
        );
        self.spawn_projectile(arrow, &ammo);
        let (player, name, projectile) = (
            horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid()),
            self.acting_player_name().to_owned(),
            self.next_entity,
        );
        self.native_entity_shoot_bow(player, &name, projectile, power * 3.0);
        // ArrowState posts projectile_shoot on the arrow's first tick.
        let pitch = 1.0 / (self.world.random.next_float() * 0.4 + 1.2) + power * 0.5;
        // BowItem.releaseUsing: `playSound(null, getX(), getY(), getZ(), ...)`, at
        // the feet: an eye-height source a few cm from the listener pans hard.
        let feet = self
            .player
            .as_ref()
            .map_or(eye, |pl| [pl.position.x, pl.position.y, pl.position.z]);
        self.audio
            .play("minecraft:entity.arrow.shoot", Some(feet), 1.0, pitch);
        self.award_stat("used", "minecraft:bow", 1);
        if !self.creative
            && !infinity
            && let Some(i) = arrow_slot
        {
            let cell = self.stack_in_mut(i);
            if let Some(s) = cell.as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *cell = None;
                }
            }
        }
        self.damage_held(1);
    }
}
