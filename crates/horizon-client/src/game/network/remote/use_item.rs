use super::*;

impl Game<'_> {
    /// MultiPlayerGameMode.useItem for `hand`, if it holds something: whether
    /// the click ends there, as far as this client can tell (`use_ends_click`).
    pub(super) fn remote_use_item(&mut self, hand: Hand) -> bool {
        // MultiPlayerGameMode.useItem: spectators pass before sending or predicting.
        if self.spectator() {
            return false;
        }
        let Some(held) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        // Client-owned input items use mod_use_start instead of server item use.
        if self.native_input.claimant(&held.item).is_some() {
            return false;
        }
        let seq = self.next_sequence();
        self.send(S::UseItem(play::UseItem {
            hand: hand_id(hand),
            sequence: seq,
            yaw: self.yaw,
            pitch: self.pitch,
        }));
        let Some(info) = self.items().get(&held.item).cloned() else {
            return false;
        };
        // MultiPlayerGameMode.useItem's prediction: an item cooling down
        // passes (its packet still goes).
        let group = horizon_client::cooldowns::group(&held, info.use_cooldown.as_ref());
        if self.cooldowns.is_on_cooldown(&group) {
            return false;
        }
        if let Some(handled) = self.client_item_interaction(hand, &held) {
            return handled;
        }
        // a mod's item used over time starts here too, as the server starts it.
        if self.with_hand(hand, |g| g.start_mod_use(&held)) {
            return true;
        }
        // Item.use: Equippable.swapWithEquipmentSlot runs on this client too, so
        // the armor slot and the hand change at once (the server confirms).
        if info.equip_slot.is_some() && self.with_hand(hand, |g| g.equip_from_hand(&held)) {
            // Equippable.swapWithEquipmentSlot's SUCCESS (SwingSource.CLIENT).
            self.with_hand(hand, Game::swing_arm);
            return true;
        }
        // `ItemStack.use` runs on this client too: a bucket's own ray and its
        // pick/empty contents decide whether the click succeeds (a failed
        // fill or pickup leaves the hand alone), and the blocks it sets are
        // this use's prediction.
        if &*held.item == "minecraft:bucket"
            || horizon_client::items::bucket_contents(&held.item).is_some()
        {
            let (succeeded, changes, sounds) = self.with_hand(hand, |g| g.remote_use_bucket(&held));
            if let Some(changes) = changes {
                self.predict_blocks(&changes, seq);
            }
            // `BucketItem.emptyContents` / evaporate: `playSound(player, …)` on
            // this client; the server leaves the actor out (arismoko/horizon#4684).
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
                    LevelSound::new(&name, at, volume, pitch)
                        .in_source(mod_api::SoundSource::Blocks),
                );
            }
            // BucketItem.use's SUCCESS (SwingSource.CLIENT) on this client.
            if succeeded {
                self.with_hand(hand, Game::swing_arm);
            }
            return succeeded;
        }
        // SpawnEggItem.use on the client succeeds after a SOURCE_ONLY block
        // hit; the server alone tests LiquidBlock and the placement permission.
        if held.item.ends_with("_spawn_egg") {
            let succeeded = self.spawn_egg_allowed(&held.item) && self.bucket_hit().is_some();
            if succeeded {
                self.with_hand(hand, Game::swing_arm);
            }
            return succeeded;
        }
        // KnowledgeBookItem.use on this client: the book is used up either
        // way. With recipes it is SUCCESS (swings, ends the click); with none
        // it is FAIL, and the other hand gets its turn. The recipes go as
        // their NBT on the wire, and the item's own default is an empty list,
        // so a stack whose patch sets them has some.
        if &*held.item == "minecraft:knowledge_book" {
            /// `DataComponents.RECIPES`' registry id (26.3).
            const RECIPES: i32 = 68;
            let has_recipes = held
                .wire_patch()
                .is_some_and(|p| horizon_protocol::item::WirePatch::adds(&p, RECIPES))
                || held
                    .data()
                    .and_then(|d| d.other("minecraft:recipes"))
                    .is_some_and(|t| matches!(t, Tag::List(l) if !l.is_empty()));
            self.with_hand(hand, Game::consume_held);
            if has_recipes {
                self.apply_use_cooldown(&held);
                self.with_hand(hand, Game::swing_arm);
            }
            return has_recipes;
        }
        // Food starts being eaten (Consumable.startConsuming). A food that
        // can't be eaten (full and not always-edible) fails the use
        // (InteractionResult.FAIL): startUseItem tries the other hand, and
        // no cooldown or use animation starts.
        let eating = self.with_hand(hand, |g| g.start_eating(&info));
        if info.food.is_some() && !eating {
            return false;
        }
        // A bucket, a glass bottle or a boat with nothing to fill, empty or
        // place PASSes or FAILs: startUseItem goes on to the off hand (a
        // Riptide trident there charges).
        if self.with_hand(hand, |g| g.use_finds_no_target(&held)) {
            return false;
        }
        // It runs the use here too: ItemStack.use starts the `use_cooldown`
        // of an item with no use time, and the server's
        // ClientboundCooldownPacket confirms it; one held in use starts it at
        // finishUsingItem (remote_complete_using).
        if !info.consumable.as_ref().is_some_and(|c| c.ticks() > 0) {
            self.apply_use_cooldown(&held);
        }
        if self.with_hand(hand, |g| g.remote_use_book(&held)) {
            // WritableBookItem.use's SUCCESS (SwingSource.CLIENT).
            self.with_hand(hand, Game::swing_arm);
            return true;
        }
        // BowItem.use and CrossbowItem.use FAIL with nothing to shoot: no
        // itemUsed (the hand doesn't dip) and the other hand's turn comes.
        if !self.with_hand(hand, |g| g.remote_start_held_use(&held)) {
            return false;
        }
        let gliding = self.player.as_ref().is_some_and(|p| p.fall_flying);
        // FireworkRocketItem.use while gliding: SUCCESS (SwingSource.PREDICTED),
        // so startUseItem swings this hand itself; a vanilla server swings it
        // for the other players only.
        if gliding && &*held.item == "minecraft:firework_rocket" {
            self.with_hand(hand, Game::swing_arm);
        }
        // A stack's own `blocks_attacks` raises it as a shield's does; a
        // use the server's patch gives it is CONSUME too (no swing).
        let blocks = self.blocks_attacks(&held).is_some() || self.patched_held_use(&held);
        let ends = use_ends_click(&info, gliding) || blocks;
        // MultiPlayerGameMode.useItem: `ItemStack.use`'s SUCCESS swings
        // (SwingSource.CLIENT); CONSUME does not — a bow, crossbow, trident
        // or spear starting its charge, and `ItemUtils.startUsingInstantly`'s
        // food, drink, spyglass, shield and goat horn.
        if ends && !blocks && remote_use_swings(&held, info.consumable.is_some()) {
            self.with_hand(hand, Game::swing_arm);
        }
        ends
    }
}
