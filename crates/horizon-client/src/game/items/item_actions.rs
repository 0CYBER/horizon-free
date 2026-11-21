//! A click that uses the held item at nothing or at an entity (`Minecraft.startUseItem`,
//! `Player.interactOn`), the context a block's use sees, and the mod hooks that cancel a placement
//! or an emptied bucket.
use super::*;

impl Game<'_> {
    /// The hand whose use ended the click, if any.
    pub(super) fn use_item(&mut self) -> Result<Option<Hand>> {
        self.with_player_actor(|game| game.use_item_as_player())
    }

    /// `Minecraft.startUseItem`: the main hand, then the off hand.
    pub(super) fn use_item_as_player(&mut self) -> Result<Option<Hand>> {
        // Spectators only look (MultiPlayerGameMode.useItemOn/useItem pass),
        // but on a server a spectator's use on a block opens the block's
        // menu (ServerPlayerGameMode.useItemOn).
        if self.spectator() {
            if let Some(GuestUse::OnBlock(hand)) = self.guest_use
                && let Some(hit) = self.pick()
            {
                self.with_hand(hand, |game| {
                    let held = game.held().cloned();
                    if matches!(
                        game.right_click_block(&hit, hand, held.as_ref()),
                        native_hooks::BlockClickAnswer::Go { .. }
                    ) {
                        game.spectator_open_menu(hit.pos);
                    }
                });
            }
            return Ok(None);
        }
        // A carrot or warped fungus on a stick boosts the ridden pig or strider.
        // A guest's use packets name one hand and touch no entity: the client
        // sends one per hand and part of its click, and Interact for entities.
        let guest = self.guest_use;
        if let Some(GuestUse::Entity(hand, target)) = guest {
            return Ok(self
                .with_hand(hand, |g| g.interact_entity(Some(target)))?
                .then_some(hand));
        }
        let on_block = matches!(guest, Some(GuestUse::OnBlock(_)));
        if !on_block
            && self.riding.is_some()
            && let Some(item) = self.held().map(|s| s.item.clone())
            && self.use_steering_stick(&item)
        {
            return Ok(Some(Hand::Main));
        }
        if guest.is_none() && self.interact_entity(None)? {
            return Ok(Some(Hand::Main));
        }
        // An EntityHitResult may fall through to Item.use, never to useItemOn.
        let block_hit = match guest {
            Some(GuestUse::OnBlock(_)) => self.pick(),
            Some(GuestUse::Item(_)) => None,
            _ if self.entity_targeted() => None,
            _ => self.pick(),
        };
        let hands = match guest {
            Some(GuestUse::OnBlock(hand) | GuestUse::Item(hand) | GuestUse::Entity(hand, _)) => {
                vec![hand]
            }
            None => Hand::BOTH.to_vec(),
        };
        let entity = guest.is_none() && self.entity_targeted();
        for hand in hands {
            // The off hand's turn starts on the entity too (Minecraft.startUseItem).
            if entity && hand == Hand::Off && self.with_hand(hand, |g| g.interact_entity(None))? {
                return Ok(Some(hand));
            }
            if self.with_hand(hand, |game| game.use_hand(block_hit.as_ref()))? {
                return Ok(Some(hand));
            }
        }
        Ok(None)
    }

    /// The index in `list` of the entity an interact packet's `target` names,
    /// by its server id (`Level.getEntity(id)`), or None for an id this level
    /// does not hold: `handleInteract` finds nothing then either.
    pub(super) fn named_entity<T>(
        list: &[T],
        target: i32,
        id: impl Fn(&T) -> u32,
    ) -> Option<usize> {
        list.iter().position(|e| id(e) as i32 == target)
    }

    /// `Player.interactOn` with the acting hand: whether it took the click. A
    /// guest's interact packet names its entity (`target`), which the helpers
    /// use as it is; the local player picks from the crosshair instead, the
    /// way `Minecraft.hitResult` does. `native_entity_interact_specific` and
    /// `native_interact_entity` hear it first; a claim swings the hand and
    /// ends the click.
    pub(super) fn interact_entity(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        // Native mods, where Player.interactOn starts: before the entity's
        // own interact and the item's interactLivingEntity. The entity is the
        // one the packet named, or the one the crosshair picked.
        let (named, distance) = match target {
            Some(t) => (Some(t.id), None),
            None => match self.pick_entity_hit_at() {
                Some((distance, id)) => (id, Some(distance)),
                None => (None, None),
            },
        };
        // The mod hooks hear no natural fence knot (#13036): its click
        // keeps the knot's own path below.
        if let Some(id) = named
            && let Some(entity) = self
                .level_entity(id)
                .filter(|t| !matches!(t, commands::Target::Knot(_)))
        {
            // The point clicked, measured from the entity's position: a
            // guest's packet carries it, and the local player's is where the
            // crosshair's ray met the entity (`EntityHitResult.getLocation()`).
            let location = match target {
                Some(t) => Some(t.location),
                None => distance
                    .zip(self.aim())
                    .zip(self.target_position(entity))
                    .map(|((d, (eye, dir)), at)| {
                        std::array::from_fn(|k| eye[k] + dir[k] * d - at[k])
                    }),
            };
            let kind = self.target_type(entity);
            let stack = self.held().cloned();
            let mut claimed = false;
            if let Some(location) = location {
                self.native_entity_interact_specific(
                    id,
                    &kind,
                    stack.as_ref(),
                    location,
                    &mut claimed,
                );
            }
            if claimed {
                self.swing_arm();
                return Ok(true);
            }
            let mut result = None;
            let dimension = self.dim.id();
            self.native_entity_interact(
                dimension,
                self.hand,
                id,
                &kind,
                stack.as_ref(),
                &mut result,
            );
            if let Some(answer) = result
                && !matches!(
                    answer,
                    mod_api::InteractionResult::Pass | mod_api::InteractionResult::TryWithEmptyHand
                )
            {
                return Ok(self.interaction_consumed(answer));
            }
        }
        // `AbstractNautilus.interact` keeps a clicked nautilus
        // (`setPersistenceRequired`), used or not, before its own handling runs.
        self.nautilus_clicked(named);
        Ok(self.interact_frame(target)
            || self.interact_armor_stand(target)
            || self.interact_cushion(target)
            || self.interact_interaction(named)
            || self.interact_boat(target)
            || self.interact_cart(target)
            || self.interact_mob(target)?
            || self.item_interact_mob(target))
    }

    /// `Interaction.interact`: a click on a server `minecraft:interaction`
    /// records who used it and on which tick, and ends the click there: the
    /// answer is `CONSUME` whether or not it responds, so nothing behind the
    /// box is used. That answer never reads `response`: only a client's own
    /// prediction does, the arm swinging or not (`MultiPlayerGameMode.interact`).
    /// `response` also picks whether an attack is skipped
    /// (`Interaction.skipAttackInteraction`), and an attack is not this path.
    pub(super) fn interact_interaction(&mut self, named: Option<i32>) -> bool {
        let Some(index) = named.and_then(|id| Self::named_entity(&self.others, id, |o| o.id))
        else {
            return false;
        };
        let player = self.acting_player_uuid();
        let timestamp = self.world.game_time;
        self.others.update(index, |o| {
            let Some(state) = o.interaction.as_mut() else {
                return false;
            };
            state.interaction = Some(super::interactions::PlayerAction { player, timestamp });
            true
        })
    }

    /// Asks the mods whether the acting guest may empty `stack` at `pos`
    /// (`native_bucket_empty`): true when one cancelled it. A game's own
    /// player is not asked.
    pub(super) fn bucket_empty_cancelled(&mut self, stack: &ItemStack, pos: [i32; 3]) -> bool {
        let (Some(uuid), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) else {
            return false;
        };
        let mod_fluid = horizon_world::fluid::mod_fluid_of_bucket(&stack.item);
        let fluid = match &*stack.item {
            "minecraft:lava_bucket" => "minecraft:lava",
            "minecraft:powder_snow_bucket" => "minecraft:powder_snow",
            _ => mod_fluid.map_or("minecraft:water", |fluid| fluid.get().id.as_str()),
        };
        let mut cancel = false;
        self.native_bucket_empty(uuid, &name, pos, fluid, &mut cancel);
        cancel
    }

    /// The player as a block's `useItemOn` / `useWithoutItem` sees them.
    pub(super) fn use_context(&self) -> PlayerContext {
        PlayerContext {
            yaw: self.yaw,
            pitch: self.pitch,
            may_build: self.may_build(),
            can_eat: self.creative
                || self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.health.food.food < 20),
            game_master: self.can_use_game_master_blocks(),
            // RespawnConfig.isSamePosition compares the dimension and the block;
            // where an anchor cannot set spawn (`canSetSpawn`), a click on it
            // explodes before that comparison.
            respawn_point: self
                .respawn_config
                .filter(|c| self.anchor_works() && c.dim == self.dim)
                .map(|c| BlockPos::new(c.pos[0], c.pos[1], c.pos[2])),
        }
    }
}

#[cfg(test)]
mod interaction_click_tests {
    //! `Interaction.interact` on a server: a click on a `minecraft:interaction`
    //! writes the `interaction` record and ends there, so nothing behind its box
    //! is used, and `response` does not change that answer.
    use super::*;

    /// Where a guest's interact packet says it clicked.
    fn target(id: u32) -> InteractTarget {
        InteractTarget {
            id: id as i32,
            location: [0.0, 1.0, 0.0],
        }
    }

    fn interaction(id: u32, response: bool) -> crate::game::commands::OtherEntity {
        crate::game::commands::OtherEntity {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            kind: "minecraft:interaction".into(),
            position: horizon_core::geometry::Vec3::new(4.5, 70.0, 4.5),
            yaw: 0.0,
            pitch: 0.0,
            item_transformation: None,
            interaction: Some(super::interactions::InteractionState {
                width: 2.0,
                height: 1.5,
                response,
                attack: None,
                interaction: None,
            }),
            knot_check: None,
            eye: None,
        }
    }

    /// The use record the server holds for the `index`th entity, if any.
    fn record(game: &Game<'_>, index: usize) -> Option<super::interactions::PlayerAction> {
        game.others[index].interaction?.interaction
    }

    #[test]
    fn a_click_on_a_server_interaction_records_it_and_ends_there() {
        if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
            return;
        }
        crate::game::native_entity_refs::tests::with_game(|game| {
            let uuid = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
            game.lan_acting_uuid = Some(uuid);
            game.world.game_time = 4321;
            game.inventory[0] = Some(ItemStack::new("minecraft:stone", 5));
            game.others.push(interaction(900, false));
            game.others.push(interaction(901, true));
            let player = horizon_client::lan::playerdata::uuid_ints(uuid);
            // A click that reached the box: the click is taken there, not passed on to the
            // held item, and the record names the player and the tick. The click is set at
            // the level of `use_item_as_player` and `interact_entity`, where a guest's own
            // interact packet arrives: the box that lets a live game's picked click through
            // to `Player.interactOn` is open PR 19033, so the picker is not yet a path here.
            game.guest_use = Some(GuestUse::Entity(Hand::Main, target(900)));
            assert!(matches!(
                game.use_item_as_player().unwrap(),
                Some(Hand::Main)
            ));
            assert_eq!(
                game.held().map(|s| (s.item.as_ref(), s.count)),
                Some(("minecraft:stone", 5)),
                "the item behind the box was not used"
            );
            assert_eq!(
                record(game, 0),
                Some(super::interactions::PlayerAction {
                    player,
                    timestamp: 4321
                })
            );
            // A responding interaction answers the same: `response` is the client's own
            // swing, never the server's answer.
            game.world.game_time = 4322;
            assert!(game.interact_entity(Some(target(901))).unwrap());
            assert_eq!(
                record(game, 1),
                Some(super::interactions::PlayerAction {
                    player,
                    timestamp: 4322
                })
            );
        });
    }
}
