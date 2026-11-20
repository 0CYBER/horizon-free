//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `players` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait PlayersHost {
    /// Side: both.
    /// `ItemCooldowns.getCooldownGroup(stack)`: the group `stack`'s
    /// cooldown is filed under, the group its `use_cooldown` component names
    /// (its own, else its item's default), else its item id.
    #[access(Metadata)]
    fn item_cooldown_group(&self, stack: &ItemStack) -> String;
    /// Side: both.
    /// Resolve a real online player by UUID, never a fake. A fake is reached
    /// through the handle returned by its factory. During a nested player swap,
    /// outer players whose records are displaced return None.
    #[access(WorldOnly)]
    fn player(&self, uuid: u128) -> Option<PlayerRef>;
}
/// The server host's methods of the `players` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait PlayersServerHost {
    /// Side: server.
    /// Create or retrieve a connectionless player for the level currently in place.
    #[access(WorldOnly)]
    fn fake_player(&mut self, level: LevelRef, uuid: u128, name: &str) -> Option<PlayerRef>;
    /// Side: server.
    /// The most players who may be on at once (`PlayerList.getMaxPlayers`,
    /// `max-players`), as the server list shows it and logins are refused
    /// beyond. None where no server is open to players (a client, or a server
    /// nobody can join): the one reason there is none.
    #[access(WorldOnly)]
    fn server_max_players(&self) -> Option<u32>;
    /// Side: server.
    /// Sets the player limit from the next login and server list ping on
    /// (Paper's `Server.setMaxPlayers`); players already on stay, even when
    /// they are more than the new limit. `server.properties` is not changed,
    /// so a restart goes back to its `max-players`. Writes nothing and refuses
    /// with `PlayerLimitRefusal::NotOpen` where no server is open to players,
    /// and with `TooHigh { maximum }` for a limit above `i32::MAX`, as vanilla
    /// keeps the limit in an `int` (Paper's `setMaxPlayers` takes one). Zero is
    /// taken: it lets nobody else on.
    #[access(WorldOnly)]
    fn server_set_max_players(&mut self, max_players: u32) -> Result<(), PlayerLimitRefusal>;
    /// Side: server.
    /// The profile of the player with this UUID, as the server knows it,
    /// never asked of Mojang: an online player's whole profile (the properties
    /// the player list gives, so with those `player_set_profile_property` set),
    /// else the user cache's name (`usercache.json`, a dedicated server's),
    /// without properties. None when neither knows them.
    #[access(WorldOnly)]
    fn profile_by_uuid(&self, uuid: u128) -> Option<GameProfile>;
    /// Side: server.
    /// The profile of the player with this name, any case, as
    /// `profile_by_uuid` knows it: an online player's, else an unexpired
    /// user cache entry. Never asked of Mojang, and an unknown name is not
    /// made an offline player.
    #[access(WorldOnly)]
    fn profile_by_name(&self, name: &str) -> Option<GameProfile>;
}
/// The client host's methods of the `players` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait PlayersClientHost {
    /// Side: client.
    /// For a client mod: a push added to the local player's velocity, in blocks a tick, as an
    /// explosion's knockback is (the window's player moves itself, as vanilla's client does). Each component is
    /// clamped to -4..4; a spectator, or a value that is not a number, is left alone.
    #[access(WorldOnly)]
    fn push_player(&mut self, kick: [f32; 3]);
    /// Side: client.
    /// For a client mod: chooses the local player's hotbar slot, `slot` (0 to 8), as
    /// `Inventory.setSelectedSlot` does for a number key. It sends nothing itself: the game
    /// sends the carried-item packet at the next tick's start at the latest, and before the
    /// packets of an attack, a use, an interact, a release, a drop and a continued block break
    /// (not before a block break start). Refused above 8 with `OutOfRange`, without a client
    /// world with `NoWorld`, and for a spectator with `Refused` (its number keys open the
    /// spectator menu), leaving the selection as it was.
    #[access(WorldOnly)]
    fn select_hotbar_slot(&mut self, slot: u8) -> Result<(), HotbarSlotError>;
    /// Side: client.
    /// For a client mod: the local player's collision exclusions, the client half of
    /// `EntityRef::set_collision_exclusions`, with the same boxes, rules and refusals (a client
    /// with no player of its own answers `Stale`). The client moves its player itself, so a mod
    /// sets the same boxes on both sides: the engine sends neither set to the other.
    #[access(WorldOnly)]
    fn set_collision_exclusions(
        &mut self,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError>;
    /// Moves the window's player through `passage` (arismoko/horizon#3775), one its server told it of, as
    /// Immersive Portals' client does: its feet, where it was a tick ago (so the frames
    /// between two ticks do not jump), its velocity and its look come out at the far side,
    /// turned with the passage, and the server is told the passage and where the feet were.
    /// Nothing else of the player changes: on ground, fall distance, sprinting, what it
    /// uses. The server checks the move to there as any move, then answers with a position
    /// packet that changes nothing (all relative and zero), so the player keeps its velocity;
    /// as any position packet does, it stops mining (a passage crossed while mining aborts
    /// the dig). A refused crossing puts the player back where the server has it
    /// ([`Self::last_passage_refusal`] says why). One crossing at a time: the next waits
    /// for the server's answer. Refused, with nothing moved or sent, when not connected to
    /// a server, with no player, while riding, for a passage the window does not hold, when
    /// the feet are more than 20 blocks from the opening, and when a mod's
    /// `client_packet_send` cancels the crossing's payload.
    #[access(WorldOnly)]
    fn cross_passage(&mut self, passage: PassageId) -> Result<(), PassageCrossError>;
    /// Side: client.
    /// The server's refusal of the window's last crossing ([`Self::cross_passage`]) and the
    /// passage it named, until the next crossing; None when it was not refused or no answer
    /// came yet.
    #[access(WorldOnly)]
    fn last_passage_refusal(&self) -> Option<(PassageId, PassageCrossRefusal)>;
}
/// The handles' reads of the `players` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait PlayersBackend {
    /// Side: server.
    /// Original UUID while its clone hook is active, otherwise None.
    #[access(WorldOnly)]
    fn original_player_uuid(&self, original_player: OriginalPlayerRef) -> Option<u128>;
    /// Side: server.
    /// Original position before the data copy, otherwise None.
    #[access(WorldOnly)]
    fn original_player_position(&self, original_player: OriginalPlayerRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// Original dimension before the data copy, otherwise None.
    #[access(WorldOnly)]
    fn original_player_dimension(&self, original_player: OriginalPlayerRef) -> Option<Identifier>;
    /// Side: server.
    /// Original level before the data copy, otherwise None.
    #[access(WorldOnly)]
    fn original_player_level(&self, original_player: OriginalPlayerRef) -> Option<LevelRef>;
    /// Side: server.
    /// Original inventory slot: 0..35 main, 100..103 feet through head, -106 offhand.
    /// None for an empty or invalid slot or an expired clone hook.
    #[access(WorldOnly)]
    fn original_player_inventory_slot(
        &self,
        original_player: OriginalPlayerRef,
        slot: i32,
    ) -> Option<ItemStack>;
    /// Side: server.
    /// Original health at death or End return; None after the clone hook.
    #[access(WorldOnly)]
    fn original_player_health(&self, original_player: OriginalPlayerRef) -> Option<f32>;
    /// Side: server.
    /// Original experience at death or End return; None after the clone hook.
    #[access(WorldOnly)]
    fn original_player_experience(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<PlayerExperience>;
    /// Side: server.
    /// Original active effects at death or End return; None after the clone hook.
    #[access(WorldOnly)]
    fn original_player_effects(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<Vec<ActiveEffect>>;
    /// Side: server.
    /// Read one registered custom book's authoritative flags. A fresh book
    /// returns the default pair; an unavailable player or unregistered key
    /// returns None. This never reads a vanilla ordinal or pending client intent.
    #[access(WorldOnly)]
    fn player_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings>;
    /// Side: server.
    /// The IP address the player connects from, without the port (behind a
    /// Velocity proxy, the address the proxy forwards). Displaced connected
    /// players keep their address; an acting player with no known connection
    /// has an empty address. None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_address(&self, player: PlayerRef) -> Option<String>;
    /// Side: server.
    /// The host and port the player's client typed to connect (the handshake's address: the
    /// name a server with several, or a host a mod announces a pack URL under, was reached by),
    /// where `player_address` gives the IP they connect from. A transfer's is the one the
    /// transferring server gave. The host is cut at its first NUL, where a proxy appends its own
    /// data. None for a stale or unavailable player, a player with no connection of their own
    /// (a fake player, the single-player owner) and a client that typed nothing.
    #[access(WorldOnly)]
    fn player_connect_address(&self, player: PlayerRef) -> Option<ConnectAddress>;
    /// Side: server.
    /// The stable name of a connected or acting player. None for a stale or
    /// unavailable player; displaced online players retain their identity.
    #[access(WorldOnly)]
    fn player_name(&self, player: PlayerRef) -> Option<String>;
    /// Side: server.
    /// `ServerPlayer.getGameProfile`: the UUID, name and login properties (skin
    /// `textures` included) of a connected or acting player. An offline-mode
    /// player has no properties, and `player_set_profile_property` does not
    /// change them: what a mod sets is published to other players and read by
    /// `profile_by_uuid` and `profile_by_name`. None for a stale or unavailable
    /// player.
    #[access(WorldOnly)]
    fn player_profile(&self, player: PlayerRef) -> Option<GameProfile>;
    /// Side: both.
    /// The level the online player is in (`ServerPlayer.level`): on an integrated
    /// server or dedicated server, the player's current dimension; on a client,
    /// the client's level. None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_level(&self, player: PlayerRef) -> Option<LevelRef>;
    /// Side: both.
    /// The dimension the online player is in (`ServerPlayer.level().dimension()`):
    /// `minecraft:overworld`, `minecraft:the_nether`, ... None for a stale
    /// or unavailable player.
    #[access(WorldOnly)]
    fn player_dimension(&self, player: PlayerRef) -> Option<Identifier>;
    /// Side: both.
    /// `Inventory.getSelectedSlot`: the hotbar slot the player holds, 0..9. On a client,
    /// [`crate::ClientHost::select_hotbar_slot`] chooses it.
    #[access(WorldOnly)]
    fn player_selected_slot(&self, player: PlayerRef) -> Option<usize>;
    /// Side: both.
    /// A player's current supported menu. Refuses crafting tables, merchants,
    /// workstations, lecterns, mounts and menus with hidden slots; also refuses
    /// clients, stale players and players whose state is swapped out.
    #[access(WorldOnly)]
    fn player_container_menu(&self, player: PlayerRef) -> Option<MenuRef>;
    /// Side: server.
    /// Every group the player waits on and its ticks left, ordered by
    /// group.
    #[access(WorldOnly)]
    fn player_item_cooldowns(&self, player: PlayerRef) -> Option<Vec<ItemCooldown>>;
    /// Side: server.
    /// `ItemCooldowns.isOnCooldown`: whether `group` still has time to run.
    #[access(WorldOnly)]
    fn player_has_item_cooldown(&self, player: PlayerRef, group: &str) -> Option<bool>;
    /// Side: server.
    /// `ItemCooldowns.getCooldownPercent(group, 0)`: what is left of the
    /// group's cooldown, from 1 (just started) to 0 (none).
    #[access(WorldOnly)]
    fn player_item_cooldown_percent(&self, player: PlayerRef, group: &str) -> Option<f32>;
    /// Side: server.
    /// `Player.getAttackStrengthScale(partial)`: the charge the player's next
    /// `Player.attack` reads, from 0 (just swung) to 1 (full), where `partial`
    /// (vanilla reads 0.5) is how far between ticks the read is. It is
    /// `(attackStrengthTicker + partial) / getCurrentItemAttackStrengthDelay()`,
    /// the delay from the attack speed attribute and the main-hand item.
    /// Inside `attack_entity` it reads the charge that blow uses: the hook
    /// runs before `Player.attack` resets the ticker. 1.0 under 1.8.9 rules,
    /// which have no cooldown. None for a stale or unavailable player.
    #[access(WorldOnly)]
    /// Non-finite partial ticks return `None`; finite partials are not restricted.
    fn player_attack_strength_scale(&self, player: PlayerRef, partial: f32) -> Option<f32>;
    /// Side: server.
    /// `Player.takeXpDelay`: the ticks before the player can take the next
    /// experience orb.
    #[access(WorldOnly)]
    fn player_exp_cooldown(&self, player: PlayerRef) -> Option<u32>;
    /// Side: both.
    /// The dimension and block of the player's last death
    /// (`Player.getLastDeathLocation`).
    #[access(WorldOnly)]
    fn player_last_death_location(&self, player: PlayerRef) -> Option<GlobalPos>;
    /// Side: both.
    /// `FoodData.needsFood`: the food level is below 20.
    #[access(WorldOnly)]
    fn player_needs_food(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `FoodData.hasEnoughFood`: the food level is above 6, enough to
    /// sprint.
    #[access(WorldOnly)]
    fn player_has_enough_food(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.canEat(always)`: an invulnerable player (creative), `always`
    /// (food that can always be eaten) or a food level below 20.
    #[access(WorldOnly)]
    fn player_can_eat(&self, player: PlayerRef, always: bool) -> Option<bool>;
    /// Side: both.
    /// `Player.experienceLevel`, `experienceProgress` and `totalExperience`.
    #[access(WorldOnly)]
    fn player_experience(&self, player: PlayerRef) -> Option<PlayerExperience>;
    /// Side: both.
    /// `Player.getXpNeededForNextLevel`: the points the current level needs
    /// to fill.
    #[access(WorldOnly)]
    fn player_xp_needed_for_next_level(&self, player: PlayerRef) -> Option<i32>;
    /// Side: server.
    /// The player's permission level, 0 to 4 (`Player.permissions()`,
    /// `PlayerList.getProfilePermissions`): an op's level, the
    /// single-player owner's 4 with commands allowed. A displaced connected
    /// player keeps their permission level even while their body is unavailable.
    /// None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_permission_level(&self, player: PlayerRef) -> Option<u8>;
    /// Side: server.
    /// `Player.hasInfiniteMaterials`: the `instabuild` ability, which
    /// creative mode gives.
    #[access(WorldOnly)]
    fn player_has_infinite_materials(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `Player.canUseGameMasterBlocks`: creative and a permission level of
    /// at least 2, as command blocks, structure blocks and jigsaws ask.
    #[access(WorldOnly)]
    fn player_can_use_game_master_blocks(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `ServerPlayerGameMode.getPreviousGameModeForPlayer`: the mode the
    /// player last left (`previousPlayerGameType`, saved with the player), or
    /// `Some(None)` when they never changed mode. None for a stale or
    /// unavailable player.
    #[access(WorldOnly)]
    fn player_previous_game_mode(&self, player: PlayerRef) -> Option<Option<GameMode>>;
    /// Side: both.
    /// `Player.isSecondaryUseActive`: whether the player uses items and
    /// blocks as with sneak held, as vanilla reads it from the shift-key
    /// flag (the same read as `EntityRef::is_shift_key_down`). None for a
    /// stale or unavailable player.
    #[access(WorldOnly)]
    fn player_is_secondary_use_active(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// Whether a live player handle names a connectionless actor.
    #[access(WorldOnly)]
    fn player_is_fake(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `Player.mayUseItemAt(pos, face, stack)`: whether the player may use `stack` at `pos`, the
    /// block it would be placed in when clicked on its `face`. A player who may build (survival,
    /// creative) always may; an adventure or spectator player only with a stack whose
    /// `can_place_on` component matches the clicked block, the one `pos` lies against. The game's
    /// own use runs the same check (`ItemStack.useOn`, buckets). None for a stale or unavailable
    /// player, and for one in another level than the one in place.
    #[access(WorldOnly)]
    fn player_may_use_item_at(
        &self,
        player: PlayerRef,
        pos: [i32; 3],
        face: BlockFace,
        stack: &ItemStack,
    ) -> Option<bool>;
    /// Side: server.
    /// `ServerPlayer.canHarmPlayer(other)`: whether `player` may hurt `other`: the `pvp` game
    /// rule is on, and their teams allow it (allies on a team without friendly fire cannot,
    /// `Player.canHarmPlayer`). Attacks, arrows and tridents ask the same. None when either
    /// handle is stale or unavailable.
    #[access(WorldOnly)]
    fn player_can_harm(&self, player: PlayerRef, other: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `ServerPlayer.getRespawnConfig`: where the player comes back after dying: a bed, a
    /// respawn anchor or a `/spawnpoint`, with the yaw they face and whether it is forced.
    /// `Some(None)` when they have none and respawn at the world spawn. None for a stale or
    /// unavailable player.
    #[access(WorldOnly)]
    fn player_respawn_config(&self, player: PlayerRef) -> Option<Option<PlayerRespawnConfig>>;
    /// Side: server.
    /// `ServerPlayer.isTextFilteringEnabled`: the text-filtering option the player's client
    /// announced (`ClientInformation.textFilteringEnabled`). False for the player running the
    /// server and for an acting player with no connection, as for a client that sent none. None
    /// for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_text_filtering(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// Whether `player_set_sleep_ignored` left the player out of the night's sleep count. False
    /// for a player never ignored. None for a stale handle or a player who isn't online.
    #[access(WorldOnly)]
    fn player_sleep_ignored(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// Whether the player counts in the night's sleep count (`SleepStatus.update`): not a
    /// spectator, not a fake player, and not left out through `player_set_sleep_ignored` while
    /// out of bed. None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_counts_for_sleep(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `Player.getSleepTimer` (`sleepCounter`): the ticks the player has been in bed, 100 once
    /// asleep long enough (`isSleepingLongEnough`, what the night skip waits for), then
    /// counting up 101 to 109 and back to 0 once they are up. None for a stale or unavailable
    /// player.
    #[access(WorldOnly)]
    fn player_sleep_timer(&self, player: PlayerRef) -> Option<u32>;
    /// Side: both.
    /// `Inventory.items`: the non-empty main inventory stacks in slot order, including
    /// their components. None for a stale or unavailable player; on clients only the
    /// local player's inventory is available.
    #[access(WorldOnly)]
    fn player_inventory(&self, player: PlayerRef) -> Option<Vec<ItemStack>>;
    /// Side: server.
    /// The player's non-empty inventory slots by `Inventory`'s numbers, with their
    /// components: 0 to 35 the main inventory (0 to 8 the hotbar), 36 feet, 37 legs, 38
    /// chest, 39 head, 40 the off hand, 41 body and 42 saddle
    /// (`Inventory.EQUIPMENT_SLOT_MAPPING`), lowest first. The numbers
    /// `player_set_inventory_slot` takes, and for 0 to 35 the `Slot` of a saved player's
    /// `Inventory` list. None for a player that is not online (stale handle) and on a client.
    #[access(WorldOnly)]
    fn player_inventory_slots(&self, player: PlayerRef) -> Option<Vec<(u32, ItemStack)>>;
    /// Side: server.
    /// The player's non-empty ender chest slots (`Player.getEnderChestInventory`, 27 slots
    /// numbered 0 to 26 as the `Slot` of a saved player's `EnderItems`), lowest first. None for
    /// a stale or unavailable player and on a client.
    #[access(WorldOnly)]
    fn player_ender_chest(&self, player: PlayerRef) -> Option<Vec<(u32, ItemStack)>>;
    /// Side: server.
    /// The compound mods keep on a server player (NeoForge's `Entity.getPersistentData`),
    /// saved in their file as `NeoForgeData` beside the keys the game keeps; empty when there
    /// is none. None for a stale or unavailable player and on a client.
    #[access(WorldOnly)]
    fn player_persistent_data(&self, player: PlayerRef) -> Option<Compound>;
    /// Side: both.
    /// `ServerPlayerGameMode.getGameModeForPlayer`; Paper `HumanEntity#getGameMode`.
    /// None for stale, offline or unavailable players (including displaced outer players).
    #[access(WorldOnly)]
    fn player_game_mode(&self, player: PlayerRef) -> Option<GameMode>;
    /// Side: both.
    /// `Player.isCreative`; Paper `HumanEntity#getGameMode`.
    #[access(WorldOnly)]
    fn player_is_creative(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.isSpectator`; Paper `HumanEntity#getGameMode`.
    #[access(WorldOnly)]
    fn player_is_spectator(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.mayBuild`; Paper `HumanEntity#getGameMode`.
    #[access(WorldOnly)]
    fn player_may_build(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `FoodData.getFoodLevel/getSaturationLevel/getExhaustionLevel`; Paper
    /// `HumanEntity#getFoodLevel/getSaturation/getExhaustion`. None until the
    /// guest's body exists, as well as for stale or unavailable players.
    #[access(WorldOnly)]
    fn player_food(&self, player: PlayerRef) -> Option<PlayerFood>;
    /// Side: both.
    /// `Player.getAbilities`; Paper `Player#getAllowFlight/isFlying/getFlySpeed/getWalkSpeed`.
    #[access(WorldOnly)]
    fn player_abilities(&self, player: PlayerRef) -> Option<PlayerAbilities>;
    /// Side: both.
    /// `IPlayerExtension.mayFly` (NeoForge): whether the player may start flying, by game mode
    /// or ability. Read this rather than `player_abilities`' `may_fly`, which a creative-flight
    /// attribute would not show; Horizon has no such attribute yet, so today the two agree.
    /// None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_may_fly(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// Whether `entity` is within the player's reach by the server's rule
    /// `reach` ([`EntityReach`]): from the player's eyes to the closest point
    /// of the entity's box, with the rule's own buffer, the range from the
    /// player's `minecraft:entity_interaction_range` (3.0, 5.0 in creative)
    /// and, for an attack, the main hand's `attack_range` component. With the
    /// eyes inside the box the distance is 0; there is no ray and no other
    /// fallback. This is the server's check, not the client's pick. None for a
    /// stale player or entity, one whose box Horizon doesn't keep, or one in
    /// another level. On a client, `player` must be the local player: the range and main hand
    /// are its synced copy, its eye position is predicted, and the target's box is its client copy.
    #[access(WorldOnly)]
    fn player_within_entity_reach(
        &self,
        player: PlayerRef,
        entity: EntityRef,
        reach: EntityReach,
    ) -> Option<bool>;
    /// Side: server.
    /// The value of statistic `stat_type` for `key` (`minecraft:stone`,
    /// `minecraft:jump`...), as `ServerStatsCounter.getValue`: 0 when never
    /// counted.
    #[access(WorldOnly)]
    fn player_stat(&self, player: PlayerRef, stat_type: StatType, key: &str) -> Option<i32>;
    /// Side: server.
    /// Whether the player's recipe book knows `recipe`
    /// (`ServerRecipeBook.contains`).
    #[access(WorldOnly)]
    fn player_has_discovered_recipe(&self, player: PlayerRef, recipe: &str) -> Option<bool>;
    /// Side: both.
    /// The recipes in the player's recipe book (`ServerRecipeBook.recipes`), as qualified ids in
    /// order (`minecraft:oak_planks`). None for a player it can't reach.
    #[access(WorldOnly)]
    fn player_discovered_recipes(&self, player: PlayerRef) -> Option<Vec<String>>;
    /// Side: both.
    /// The brand the player's client announced when it connected (`ServerPlayer.clientBrand`),
    /// empty when it sent none. Displaced connected players keep theirs; an acting player with
    /// no known connection has an empty one. None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_client_brand(&self, player: PlayerRef) -> Option<String>;
    /// Side: server.
    /// The plugin channels the player's client announced in `minecraft:register`, less those
    /// it withdrew in `minecraft:unregister`, during configuration and play, in announcement
    /// order (Bukkit's `Player.getListeningPluginChannels`). Malformed entries are skipped and
    /// at most 128 channels of up to 64 bytes are kept. Empty for a client that announced
    /// none and for an acting player with no known connection; None for a stale or
    /// unavailable player.
    #[access(WorldOnly)]
    fn player_announced_channels(&self, player: PlayerRef) -> Option<Vec<String>>;
    /// Side: server.
    /// Whether `viewer`'s client is shown the player `target` by the rules players are tracked
    /// with (`ServerPlayer.broadcastToPlayer`; Paper's `Player.canSee`): a spectator is seen
    /// only by spectators, and by them only while it watches through its own camera, and a
    /// player hidden with `player_hide` is seen only by players of at least that level.
    /// Distance and chunks are not part of it, so a player out of range is still seen. A player
    /// sees themselves. Refuses with `PlayerWorldRefusal`: `Client`, `Stale` (a handle of the
    /// two no longer identifies a player in the server's list) and `NoConnection` (a fake
    /// player is no one's viewer or target).
    #[access(WorldOnly)]
    fn player_sees(&self, viewer: PlayerRef, target: PlayerRef)
    -> Result<bool, PlayerWorldRefusal>;
    /// Side: server.
    /// All of the player's statistics as `ServerStatsCounter.toJson` writes their
    /// `stats/<uuid>.json`: `{"stats":{"minecraft:mined":{"minecraft:stone":3},...},
    /// "DataVersion":n}`, types and keys namespaced, empty types left out. None for a stale
    /// or unavailable player and on a client.
    #[access(WorldOnly)]
    fn player_statistics(&self, player: PlayerRef) -> Option<String>;
    /// Side: server.
    /// The player's latency in milliseconds, as the Tab list shows it
    /// (`ServerCommonPacketListenerImpl.latency`): keep-alive round trips averaged as vanilla
    /// does (`(old * 3 + new) / 4`), 0 until the first answer. 0 for a fake player and an
    /// acting player with no known connection; None for a stale or unavailable player.
    #[access(WorldOnly)]
    fn player_latency(&self, player: PlayerRef) -> Option<i32>;
    /// Side: server.
    /// The permission level `player_hide` hides the player from (players below it don't see
    /// them), None when they are not hidden. Kept by UUID, so it reads the same for a player
    /// who rejoined hidden. The outer None is a stale world epoch or a client.
    #[access(WorldOnly)]
    fn player_hidden(&self, player: PlayerRef) -> Option<Option<u8>>;
    /// Side: server.
    /// The nickname `player_set_nickname` shows in place of the player's name, None when they
    /// have none. The outer None is a stale or offline player, or no server list.
    #[access(WorldOnly)]
    fn player_nickname(&self, player: PlayerRef) -> Option<Option<String>>;
    /// Side: both.
    /// `Player.hasCorrectToolForDrops(state)`: true when `state` needs no tool, else whether
    /// the stack in the player's selected hotbar slot is a correct tool for it
    /// (`BlockStates::is_correct_tool_for_drops`). Vanilla's own rule: mods'
    /// `can_harvest_block` and `player_harvest_check` are not asked, so a listener of them
    /// may call it. On a client, for the local player only. None for an unknown state, a
    /// stale or unavailable player.
    #[access(WorldOnly)]
    fn player_has_correct_tool_for_drops(&self, player: PlayerRef, state: BlockState)
    -> Option<bool>;
}
/// The handles' client reads of the `players` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait PlayersClientBackend {
    /// Side: client.
    /// The local player's last authoritative custom-book pair. None before
    /// negotiation, while resynchronizing, or for a nonlocal/stale handle.
    #[access(WorldOnly)]
    fn player_synced_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings>;
    /// Side: client.
    /// The local player's predicted breaking state, including Idle. None for
    /// stale, nonlocal or fake handles and outside a logged-in client world.
    #[access(WorldOnly)]
    fn player_block_breaking(&self, player: PlayerRef) -> Option<BlockBreakingState>;
    /// Side: client.
    /// The local player's existing cave ambience mood, normally 0 through 1.
    /// None for stale, nonlocal or fake handles and outside a client world.
    #[access(WorldOnly)]
    fn player_ambient_mood(&self, player: PlayerRef) -> Option<f32>;
}
/// Client-only custom book intent, reached through player handles.
// hidden: mods call PlayerRef methods, never the generated backend trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait PlayersClientBackendMut {
    /// Side: client.
    /// Request a new pair for the local player's negotiated book. True means
    /// queued, not accepted; only an authoritative reply changes saved state.
    #[access(WorldOnly)]
    fn player_request_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool;
}
/// The handles' writes of the `players` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait PlayersBackendMut {
    /// Side: server.
    /// Set a registered custom book's authoritative pair and synchronize an
    /// accepted change. Returns false for an unavailable player, unknown key
    /// or exhausted storage. An unchanged pair succeeds without notification.
    #[access(WorldOnly)]
    fn player_set_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool;
    /// Side: server.
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_tell(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Disconnects that player with `reason`, as `/kick` does
    /// (`ServerGamePacketListenerImpl.disconnect`): their client shows it, the log's "lost
    /// connection" line names it, `player_kicked` hears it and a dedicated server records a
    /// `kick` event. The connection closes when the server next polls it, so the calling hook
    /// finishes first; works for the acting player inside their own hook, and without the
    /// world published. A fake (client-less) player leaves through the same removal path:
    /// the leave line is broadcast once and `player_logged_out` fires once at the next poll.
    /// False for a stale or unavailable player, the single-player owner (`/kick` refuses the
    /// owner) and a player whose disconnect is already pending (a second call, also from
    /// `player_kicked`: the first reason stands).
    /// A second disconnect on a guest already leaving is a no-op and keeps the first reason.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_disconnect(&mut self, player: PlayerRef, reason: Text)
    -> Result<bool, TextFormError>;
    /// Side: server.
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. False when the server list is unavailable, the handle is
    /// stale or the player isn't online.
    #[access(WorldOnly)]
    fn player_set_sleep_ignored(&mut self, player: PlayerRef, ignored: bool) -> bool;
    /// Side: server.
    /// `ServerPlayer.setRespawnPosition`, as `/spawnpoint` sets it: the player's respawn point
    /// becomes `config`, or is forgotten for None (they then respawn at the world spawn). The
    /// yaw wraps to -180..180 (`RespawnData.of`) and the `player_set_spawn` hook may cancel the
    /// change, which refuses it. Kept with the player's record. False for a stale or unavailable
    /// player, a dead one, a dimension the game doesn't have, a yaw that is not finite and a
    /// canceled change.
    #[access(WorldOnly)]
    fn player_set_respawn_config(
        &mut self,
        player: PlayerRef,
        config: Option<PlayerRespawnConfig>,
    ) -> bool;
    /// Side: server.
    /// Sets the player's `sleepCounter`: 0 to 109, the values the game reaches (see
    /// `player_sleep_timer`). A player in bed at 100 counts as slept long enough for the night
    /// skip. False for a value past 109, a stale or unavailable player, a fake player and a
    /// player in another level than the one in place.
    #[access(WorldOnly)]
    fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (`Messages::text_allowed`, its length,
    /// other players' names). False when the server list is unavailable,
    /// the handle is stale or the player isn't online.
    #[access(WorldOnly)]
    fn player_set_nickname(&mut self, player: PlayerRef, nickname: Option<&str>) -> bool;
    /// Side: server.
    /// Shows `sidebar` on that player's screen in place of the one shown before, or takes it
    /// away (None): an objective only their client holds, displayed in the sidebar slot, its
    /// lines as scores with fixed or blank number formats (`ClientboundSetObjectivePacket`,
    /// `SetDisplayObjective`, `SetScore`). The server's scoreboard, `/scoreboard` and the other
    /// players don't see it; it covers a sidebar objective the world shows, for this player,
    /// and is gone when they leave. False when they aren't on.
    #[access(WorldOnly)]
    fn player_set_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Shows `name` for that player in everyone's Tab list (Paper's `Player.playerListName`,
    /// NeoForge's `ServerPlayer.refreshTabListName`), colours and all; None shows the name the
    /// game would (their nickname or real name). The `player_tab_list_name_format` hook still
    /// has the last word. Only the Tab list: chat, name tags and commands keep the real name.
    /// Kept by UUID until a mod sets it again or the server stops (also if they leave and come
    /// back). False when the server list is unavailable, the handle is stale or the player
    /// isn't online.
    #[access(WorldOnly)]
    fn player_set_list_name(
        &mut self,
        player: PlayerRef,
        name: Option<Text>,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Hides that player (a vanish) from the players below permission
    /// `level`, or shows them again (None): from the next tick those players
    /// lose them from the Tab list and the world (their client stops
    /// tracking the player, as it does a spectator), and don't read their
    /// joining or leaving; players of `level` or more still see them, and
    /// a player made an operator meanwhile sees them again. Kept by UUID, even
    /// after this handle disconnects (the player rejoins hidden), until a mod sets it again
    /// or the server stops. Works inside a player's hook too; false for a stale
    /// world epoch and on a client.
    #[access(WorldOnly)]
    fn player_hide(&mut self, player: PlayerRef, level: Option<u8>) -> bool;
    /// Side: server.
    /// Opens a block entity's menu for a connected player (`ServerPlayer.openMenu`).
    /// Returns Stale for an invalidated entity; Ok(false) for a wrong-level player,
    /// non-provider, cancelled open, or an unconnected player such as a fake player.
    #[access(WorldOnly)]
    fn player_open_menu(
        &mut self,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Opens content mod menu type `menu` for a connected player from anywhere, with no
    /// block (`ServerPlayer.openMenu(provider, extraDataWriter)`): an item's use, an entity,
    /// a command. The menu replaces the one they had open. Its container slots show empty
    /// and their clicks go to the mods (`native_menu_clicked`), as `open_menu`'s do. `extra`
    /// (at most 32 KiB) is what the client's `screen_extra_data` reads, and `pos`, when
    /// given, its `screen_pos`. The old menu closes on the server as a close does (its
    /// `removed`, `player_container_close`, the cursor stack and crafting grids back to the
    /// inventory) before the new one is built; `replace` says whether the client is first sent
    /// a close for it, as for `open_menu`. `MenuReplace::InPlace` sends none: the client keeps
    /// its screen and mouse until the new screen replaces it. The window id, or Ok(None) when
    /// a mod cancelled the open. Errors: `NotModMenu`, `PlayerUnavailable` (stale, fake or
    /// vanilla client), `TooLarge`, or `Title` for a title that could not cross; a cancelled or
    /// refused open leaves the old menu as it was.
    #[access(WorldOnly)]
    fn player_open_mod_menu(
        &mut self,
        player: PlayerRef,
        menu: MenuKey,
        title: Text,
        extra: Vec<u8>,
        pos: Option<[i32; 3]>,
        replace: MenuReplace,
    ) -> Result<Option<i32>, OpenMenuError>;
    /// Side: server.
    /// Backs the container slots of the mod menu `window` that `player_open_mod_menu` opened
    /// for the player with storage that is not a block entity (arismoko/horizon#3385; NeoForge's
    /// `StackCopySlot` and `ResourceHandlerSlot`): `places[n]` is what slot `n` shows. From then
    /// on a click or shift-click on those slots, and an `ItemPlace::MenuSlot` write, go through
    /// that storage's own handler, journalled; the client sees ordinary synced slots, mirrored
    /// after each click and each write through an `ItemPlace` of that storage, never each tick.
    /// A slot past `places` stays the mods' own (`native_menu_clicked`). The menu closes once a
    /// place stops resolving (the backpack left the hand, the entity is gone) or a held stack is
    /// no longer the one bound when the slots were backed, checked before each click and at
    /// each change notice. A `MenuSlotPlace::HeldItem` locks the inventory slot holding that
    /// stack while the menu is open, so it can't go into itself. Backing again replaces the
    /// places; a slot no longer backed (an empty list, a shorter one) is emptied, never left
    /// holding the copy it showed. A [`MenuSlotBackRefusal`], and nothing changes, for a player
    /// not online or a window that is not their open menu, a menu that is not a blockless mod
    /// menu, more places than its container slots, or a place that doesn't resolve now.
    #[access(WorldOnly)]
    fn player_back_mod_menu_slots(
        &mut self,
        player: PlayerRef,
        window: i32,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal>;
    /// Side: server.
    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    #[access(WorldOnly)]
    fn player_send_map(&mut self, player: PlayerRef, map_id: i32, colors: &[u8]) -> bool;
    /// Side: server.
    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. A speed that is not finite refuses the write
    /// (vanilla's setters take any float). Returns false for a
    /// stale or unavailable player, a displaced outer player, and a speed
    /// that is not finite.
    #[access(WorldOnly)]
    fn player_set_abilities(&mut self, player: PlayerRef, abilities: PlayerAbilities) -> bool;
    /// Side: server.
    /// A fake player acts as a player does, through the game's own path for the action
    /// (`ServerGamePacketListenerImpl.handleUseItemOn` / `handleUseItem` / `handleInteract`'s
    /// ATTACK, and `ServerPlayerGameMode.destroyBlock`), as its packet would run it: reach,
    /// protection and the border, then the action. Once it runs, an effect a fake can't take
    /// part in (a menu, sleep, a ride, another dimension) is stopped where it would start, and
    /// the answer says which; the action's own side effects (the swing, the hooks before it,
    /// vanilla's result) have still happened then. The fake is found once, and its turn
    /// starts at that place in the list.
    #[access(WorldOnly)]
    fn player_perform_action(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused>;
    /// Side: server.
    /// A machine's one use on the persistent actor, with its private state put back even
    /// when the action refuses. Only the existing guest turn runs gameplay and its hooks.
    #[access(WorldOnly)]
    fn player_perform_action_scoped(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused>;
    /// Side: server.
    /// `ItemCooldowns.addCooldown(group, ticks)`: starts (or restarts) the
    /// group's cooldown, and the player's client draws it on every item of the
    /// group (`ServerItemCooldowns.onCooldownStarted`).
    #[access(WorldOnly)]
    fn player_set_item_cooldown(&mut self, player: PlayerRef, group: &str, ticks: i32) -> bool;
    /// Side: server.
    /// `ItemCooldowns.removeCooldown(group)`: ends the group's cooldown now,
    /// on the client too (`ServerItemCooldowns.onCooldownEnded`).
    #[access(WorldOnly)]
    fn player_remove_item_cooldown(&mut self, player: PlayerRef, group: &str) -> bool;
    /// Side: server.
    /// Sets `Player.takeXpDelay`: no orb is taken for `ticks` ticks. The
    /// change is not reported to `player_exp_cooldown_change`.
    #[access(WorldOnly)]
    fn player_set_exp_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// Sets the player's `WardenSpawnTracker.cooldownTicks`: sculk shriekers
    /// they set off don't warn for `ticks` ticks. The level's players share
    /// one tracker here, the integrated server owner's (the shriekers read it
    /// for whoever sets them off), so this is every player's.
    /// False, changing nothing, for anyone but the tracker's owner until each player has their own.
    #[access(WorldOnly)]
    fn player_set_warden_warning_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// Fires the mod's criteria trigger `trigger` (`mymod:pipe_connected`)
    /// for `player`: vanilla's
    /// `SimpleCriterionTrigger.trigger`. Each criterion of theirs with that
    /// trigger, not yet granted, of an advancement not yet done, is tested
    /// by `test` and then by its `player` predicate; those that pass are
    /// granted, and an advancement they complete is awarded with its toast,
    /// chat line and rewards, as the game's own triggers award.
    ///
    /// The acting player is reached inside their hook; another live player
    /// is reached in their own level, including during a guest's turn. An
    /// outer player displaced by a nested turn is unavailable. `Ok(())`
    /// means the player was reached, whether or not anything was granted.
    /// A stale/offline/displaced handle, a remote client, or a `minecraft:`
    /// trigger or empty id is refused with a typed reason.
    ///
    /// `test` is lent to this call only. A failed test stops further testing
    /// and reports `Test`; earlier matching criteria may already be granted.
    /// Wasm predicates run synchronously, including callbacks into the group
    /// firing the trigger, within that call's remaining execution and memory limits.
    #[access(WorldOnly)]
    fn player_trigger_criterion(
        &mut self,
        player: PlayerRef,
        trigger: &Identifier,
        test: Arc<dyn CriterionTest>,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `default_block_use`, without a tool.
    /// The engine fires `any_block_use` once when the enclosing block use consumes.
    /// It sees the hand after the use; a replacement stack is visible, unlike vanilla's original object.
    /// Evaluates vanilla predicates and rewards in the player's own level; never reads inventory.
    /// `Ok(())` means reached, even if no criterion matched. Refuses remote clients
    /// (`NotServer`), stale/offline/displaced players (`PlayerUnavailable`), and
    /// unloaded positions (`UnloadedChunk`).
    #[access(WorldOnly)]
    fn player_trigger_block_use(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `item_used_on_block` with `original`.
    /// `original` is copied after a before-block PASS and before block/item effects.
    /// The stack is not read from inventory. The engine fires `any_block_use`
    /// once when the enclosing block use consumes, including a consuming cancel.
    /// Call this helper only when the mod also intends the inner criterion.
    /// An invalid stack returns `Stack(StackWriteError)` before the trigger fires.
    /// Evaluates vanilla predicates and rewards in the player's own level.
    /// `Ok(())` means reached, even if nothing matched. Refuses remote clients
    /// (`NotServer`), stale/offline/displaced players (`PlayerUnavailable`), and
    /// unloaded positions (`UnloadedChunk`).
    #[access(WorldOnly)]
    fn player_trigger_item_used_on_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        original: &ItemStack,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server.
    /// `Player.setLastDeathLocation`: stores where the player died, or
    /// forgets it for None. The client hears of it with the player's next
    /// respawn, as in vanilla. False in the cases a read gives None.
    #[access(WorldOnly)]
    fn player_set_last_death_location(
        &mut self,
        player: PlayerRef,
        location: Option<GlobalPos>,
    ) -> bool;
    /// Side: server.
    /// `FoodData.eat(nutrition, saturationModifier)`: adds `nutrition` to
    /// the food level (0 to 20) and `nutrition * modifier * 2` to the
    /// saturation, which stays at most the food level.
    #[access(WorldOnly)]
    fn player_feed(&mut self, player: PlayerRef, nutrition: i32, saturation_modifier: f32) -> bool;
    /// Side: server.
    /// `Player.causeFoodExhaustion`: adds exhaustion (at most 40), as
    /// sprinting or jumping does. An invulnerable player (creative) or a
    /// spectator gets none, and the call still returns true.
    #[access(WorldOnly)]
    fn player_add_exhaustion(&mut self, player: PlayerRef, amount: f32) -> bool;
    /// Side: server.
    /// `FoodData.setFoodLevel`: sets the food level as it is, unclamped,
    /// as vanilla does.
    #[access(WorldOnly)]
    fn player_set_food_level(&mut self, player: PlayerRef, level: i32) -> bool;
    /// Side: server.
    /// `FoodData.setSaturation`: sets the saturation as it is, unclamped.
    #[access(WorldOnly)]
    fn player_set_saturation(&mut self, player: PlayerRef, saturation: f32) -> bool;
    /// Side: server.
    /// Sets the exhaustion as it is (Paper's `HumanEntity#setExhaustion`;
    /// vanilla's `FoodData` only adds to it).
    #[access(WorldOnly)]
    fn player_set_exhaustion(&mut self, player: PlayerRef, exhaustion: f32) -> bool;
    /// Side: server.
    /// `ServerPlayer.setExperiencePoints`: sets the progress toward the next
    /// level to `points` of the points it needs, kept below a full bar. The
    /// level and the total are kept.
    #[access(WorldOnly)]
    fn player_set_experience_points(&mut self, player: PlayerRef, points: i32) -> bool;
    /// Side: server.
    /// `ServerPlayer.setExperienceLevels`: sets the level, keeping the
    /// progress and the total.
    #[access(WorldOnly)]
    fn player_set_experience_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
    /// Side: server.
    /// Changes a player's game mode as `ServerPlayer.setGameMode` and
    /// `/gamemode` do, including the cancellable change hook and ability
    /// updates. Returns false for a stale or unavailable player, an active
    /// outer guest, no change, or a canceled change.
    #[access(WorldOnly)]
    fn player_set_game_mode(&mut self, player: PlayerRef, mode: GameMode) -> bool;
    /// Side: server.
    /// `Inventory.setSelectedSlot`: makes `slot` (0 to 8) the hotbar slot the player holds and
    /// tells their client (`ClientboundSetHeldSlot`), as `/clear`-style server code does. A new
    /// slot ends a main-hand use in progress, as the client's own switch does, and asks no
    /// `player_switch_hotbar_slot_pre`: that hook is for the player's own input. Returns false for
    /// a slot past 8, a stale or unavailable player, and a dead one; true when the player already
    /// held `slot`.
    #[access(WorldOnly)]
    fn player_set_selected_slot(&mut self, player: PlayerRef, slot: usize) -> bool;
    /// Side: server.
    /// `Player.drop(stack, false, thrown_by_player)`: `stack` is thrown from the player's eyes
    /// along their view as an item entity, with the main hand swinging and `item_toss` asked
    /// first. Nothing is taken from an inventory: the stack is the caller's, taken out through
    /// the item places first, which is what separates it from giving or dropping into the
    /// inventory. `thrown_by_player` names the player as the item's thrower, as the drop key
    /// does. `Ok(None)` when the stack is in the world; otherwise `Ok(Some(stack))` comes back, untouched, for
    /// the caller to put back: an empty stack, a stale or dead player, or a toss a mod canceled
    /// (`item_toss`), so a refused drop never loses items. Invalid API stack conversion returns
    /// `StackWriteError`, as other checked stack writes do.
    #[access(WorldOnly)]
    fn player_drop_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
        thrown_by_player: bool,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Remove a fake without treating it as a disconnect or saving it.
    #[access(WorldOnly)]
    fn player_remove_fake(&mut self, player: PlayerRef) -> bool;
    /// Side: server.
    /// `Player.awardStat(stat, amount)`: adds `amount`, as the game does
    /// for its own counts, so `player_stat_award` hears it and may rewrite or cancel
    /// it, and scoreboard criteria follow. False for a player it can't
    /// reach.
    #[access(WorldOnly)]
    fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
        amount: i32,
    ) -> bool;
    /// Side: server.
    /// `Player.resetStat`: sets the statistic back to 0, which scoreboard
    /// criteria follow. False for a player it can't reach.
    #[access(WorldOnly)]
    fn player_reset_stat(&mut self, player: PlayerRef, stat_type: StatType, key: &str) -> bool;
    /// Side: server.
    /// `ServerPlayer.awardRecipes`, as `/recipe give` does: the recipes
    /// unlock in the player's book, with their toasts and the
    /// `player_recipe_discovered` hook, and the client hears of them.
    /// Unknown ids are skipped. The number of recipe displays added, None
    /// for a player it can't reach.
    #[access(WorldOnly)]
    fn player_award_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32>;
    /// Side: server.
    /// `ServerPlayer.resetRecipes`, as `/recipe take` does. The number of
    /// recipe displays taken out, None for a player it can't reach.
    #[access(WorldOnly)]
    fn player_reset_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32>;
    /// Side: server.
    /// `ExperienceOrb.playerTouch`: Mending repairs the player's damaged
    /// items with `points` first (`ExperienceOrb.repairPlayerItems`), then
    /// only the rest goes to their experience
    /// (`Player.giveExperiencePoints`), with the level-up sound. No orb
    /// pickup sound. False, and nothing given, as the other writes; `points`
    /// above `i32::MAX` is refused before Mending repairs anything.
    #[access(WorldOnly)]
    fn player_give_experience_points(&mut self, player: PlayerRef, points: u32) -> bool;
    /// Side: server.
    /// `Player.giveExperienceLevels`: adds `levels`, or takes them off when
    /// negative, as the anvil and the enchanting table charge with. Below 0
    /// the level, the total and the progress toward the next level are all 0.
    #[access(WorldOnly)]
    fn player_give_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
    /// Side: server.
    /// `StopSoundCommand` for one player: their client stops the playing
    /// sounds that match `sound` (an event id) and `source`; `None` matches
    /// any. True means sent.
    #[access(WorldOnly)]
    fn player_stop_sound(
        &mut self,
        player: PlayerRef,
        sound: Option<&str>,
        source: Option<SoundSource>,
    ) -> bool;
    /// Side: server.
    /// `PlaySoundCommand` for one player: plays sound event `sound` at the
    /// player, for them only, under `source`'s volume slider. The seed is the
    /// level random's `nextLong`, drawn once per call. True means sent, not
    /// heard.
    #[access(WorldOnly)]
    fn player_play_sound(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// The calling mod's chunk areas for this player, in place of the ones it asked before; an
    /// empty slice withdraws them, and other mods' areas stay. The engine keeps each area's
    /// chunks loaded with a ticket of its own (`Loaded`, or `Ticking` as a player's simulation
    /// ticket), generates what is missing, and sends a level besides the player's own to a client
    /// that can view it. A chunk no area covers any more is forgotten 52 ticks later. An area
    /// counts as no player for spawning, sleeping, raids or any other rule. Areas are never saved
    /// and end when the player leaves. A call that breaks a limit changes nothing and names it.
    #[access(WorldOnly)]
    fn player_set_chunk_areas(
        &mut self,
        player: PlayerRef,
        areas: &[ChunkArea],
    ) -> Result<(), ChunkAreaRefusal>;
    /// Side: server.
    /// `ServerPlayerGameMode.destroyBlock`: breaks the block at `pos` in the player's level as
    /// this player breaks it, as FTB Ultimine does for each block of a vein. The player's own
    /// checks hold: a spectator, an adventure player whose tool can't break it, a held item that
    /// can't break blocks (a sword in creative), a game master block without the permission and
    /// a dedicated server's spawn protection leave it. Then `break_block` hears it and may cancel
    /// it, and the break runs as the player's own: the block's effects, events, particles and
    /// sound, `player_destroyed_block`, the loot rolled for the held item and its enchantments,
    /// `block_drops` with this player as the breaker, the item's wear, the mined statistic and
    /// the exhaustion. A vein miner takes each block's drops and experience in `block_drops` and
    /// sets them down where it wants with `LevelRef::pop_resource` and
    /// `LevelRef::award_experience`; what else the break sets loose (a torch losing its support,
    /// a container's contents) still drops in the world. With `into_inventory`, the drops left
    /// after `block_drops` go to the player's inventory instead, and what doesn't fit drops from
    /// them as `Inventory.placeItemBackInInventory` drops it. The player needn't reach or look at
    /// the block. Breaking from `break_block` or `block_drops` makes them hear these breaks too:
    /// a mod keeps its own guard, and a break started from inside eight others is refused (a
    /// Horizon safeguard that the reference does not have). True when the block was removed; false, with nothing broken, for a stale handle, a player who
    /// isn't here or is dead, air, an unloaded block, a block no survival player can break
    /// (bedrock), a canceled break, and off a server.
    #[access(WorldOnly)]
    fn player_destroy_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        into_inventory: bool,
    ) -> bool;
    /// Side: both.
    /// Simulate one client movement tick for this server player without changing the player. Refuses stale, absent, dead, spectator or riding players and invalid input.
    #[access(WorldOnly)]
    fn player_simulate_move(
        &mut self,
        player: PlayerRef,
        input: PlayerInput,
    ) -> Option<SimulatedMove>;
    /// Side: server.
    /// Respawns a dead player as their client's Respawn button does
    /// (`ServerGamePacketListenerImpl.handleClientCommand`'s `PERFORM_RESPAWN`, then
    /// `PlayerList.respawn`): at their respawn point, else the world spawn, with
    /// `player_respawned` heard. It is a call for outside every player's hook, and it
    /// returns once the player is alive again. Each refusal changes nothing and is named by
    /// `RespawnRefusal`, checked in this order: `Client`, `InPlayerHook` (a player's state is
    /// swapped in; call it from `server_tick`), `Stale` (no player on this server behind the
    /// handle), `NoConnection` (a fake player, or the world's own player in a single-player
    /// world) and `Alive` (health above 0 and the game not won: the Respawn button
    /// ignores exactly that player; one who has won the game and is watching the End poem is
    /// respawned even alive, on the server's next tick, as the button does).
    #[access(WorldOnly)]
    fn player_respawn(&mut self, player: PlayerRef) -> Result<(), RespawnRefusal>;
    /// Side: server.
    /// The play packets the player's connection sent and received since the last call while
    /// packets were counted (`count_packets`), by direction (sent first) then id; clears them.
    /// Keep-alives and the disconnect are counted; packets of the login and configuration
    /// phases are not. Empty while nothing was counted. None for a stale or unavailable
    /// player, one with no connection (a fake player), and on a client.
    #[access(WorldOnly)]
    fn player_take_packet_counts(&mut self, player: PlayerRef) -> Option<Vec<PacketCount>>;
    /// Side: server.
    /// Acknowledges the player's block actions up to `sequence`
    /// (`ServerGamePacketListenerImpl.ackBlockChangesUpTo`), for a mod that handled one itself,
    /// such as an editor's own message: the client's predicted blocks settle on the server's
    /// states once it gets `ClientboundBlockChangedAckPacket`, which goes after the next tick's
    /// block changes. Pending acknowledgements take the maximum: a lower sequence after a
    /// higher one has no effect. Any nonnegative sequence is accepted, so a value too high can
    /// end pending client predictions early. Refuses, queueing nothing, with
    /// `PlayerWorldRefusal`: `Client`, `Stale`, `NoConnection` and `NegativeSequence`.
    #[access(WorldOnly)]
    fn player_ack_block_changes(
        &mut self,
        player: PlayerRef,
        sequence: i32,
    ) -> Result<(), PlayerWorldRefusal>;
    /// Side: server.
    /// Shows that player alone `state` at `pos` in their level (`ClientboundBlockUpdatePacket`
    /// to their client; Paper's `Player.sendBlockChange`), at once. The world is not changed:
    /// the real block comes back when it or its chunk is next sent, as in vanilla, so a mod
    /// sends it again while it still wants it shown. The visual override is not remembered;
    /// it does not itself acknowledge or clear pending client predictions.
    /// Refuses, sending nothing, with `PlayerWorldRefusal`: `Client`, `Stale`, `NoConnection`, `UnknownState` and
    /// `PositionNotSendable` (y outside -2048..=2047, x or z outside -33554432..33554432).
    #[access(WorldOnly)]
    fn player_show_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Result<(), PlayerWorldRefusal>;
    /// Side: server.
    /// Writes `state` at `pos` in `level` for the player, for a state the mod already chose
    /// (an editor's or a schematic's). It is `level_set_block` with a placement's flags
    /// (`NEIGHBORS | CLIENTS`, plus `KNOWN_SHAPE` when `update_neighbour_shapes` is false; without
    /// it the neighbours' shapes update, so fences, walls and stairs connect), so the block entity
    /// follows that write's rules, and then `Block.setPlacedBy` of the block that stands there, run in
    /// the player's turn (a bed places its head, a door its upper half). Nothing else of vanilla's `BlockItem.place`
    /// or `useOn` runs: no check that the block may be placed there (survival, entity
    /// obstruction, build permission, spawn protection), no `placed_block` advancement trigger or
    /// statistic, no place sound, no game event, no item used or consumed, and `setPlacedBy` is
    /// not given the player as placer. The player is only whose turn `setPlacedBy` runs in, so it is
    /// not "as player" in the sense of a use-on. The state is written as given. Refuses, writing
    /// nothing, with `PlayerWorldRefusal`: `Client`, `Stale`, `Dead`, `StaleLevel`, `OtherLevel`
    /// (not the level in place, where the player is), `UnknownState`, `OutOfWorld`, `NotLoaded`
    /// and `Unchanged` (that state already stands there, or the write was refused as
    /// `level_set_block` returns false).
    #[access(WorldOnly)]
    fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        update_neighbour_shapes: bool,
    ) -> Result<(), PlayerWorldRefusal>;
    /// Numbers: 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand,
    /// 41 body, 42 saddle. ItemPlace uses 100..103 for armour and -106 for offhand;
    /// InventoryMenu has no 41 or 42.
    /// `Inventory.setItem(slot, stack)` by `player_inventory_slots`' numbers: `stack` (None
    /// empties it) goes into that slot as `/item replace entity <player> <slot>` puts it,
    /// without the slot's item check, so a worn slot takes any item; the equip sound and
    /// equipment changes follow for 36 to 42. The client is sent the slot with the tick's
    /// inventory sync (`ContainerSetSlot`), as vanilla's `broadcastChanges`. Each refusal
    /// writes nothing and is named by `PlayerDataRefusal`: `Stale` (no online player behind
    /// the handle), `Client`, `Dead`, `SlotOutOfRange` (a slot past 42, never a panic) and
    /// `Stack` (an item the world lacks, a count or damage the item does not allow).
    /// An open transaction journals this write; an abort restores the previous value.
    /// Equipment effects wait for the outermost commit.
    #[access(WorldOnly)]
    fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// `PlayerEnderChestContainer.setItem(slot, stack)`: `stack` (None empties it) in ender
    /// chest slot `slot` (0 to 26). A player looking into their ender chest is sent the slot
    /// with the open menu's sync. Refuses as `player_set_inventory_slot` does, with
    /// `SlotOutOfRange { last: 26 }` for a slot past 26 (never a panic).
    /// An open transaction journals this write; an abort restores the previous value.
    #[access(WorldOnly)]
    fn player_set_ender_chest_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Gives `stack` as `/give` does, components and all: `Inventory.add` fills matching
    /// stacks, then empty slots (the selected one first); what does not fit is thrown from
    /// the player's eyes with no pickup delay and only they may pick it up; a stack that went
    /// in whole plays the pickup sound. Refuses, with nothing given, with `Stale`, `Client`,
    /// `Dead` or `Stack`, as `player_set_inventory_slot` does.
    /// An open transaction journals this write; an abort restores the previous value.
    /// Overflow drops and pickup sounds wait for the outermost commit.
    #[access(WorldOnly)]
    fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Replaces the compound mods keep on a server player (`player_persistent_data`) with
    /// `data`; an empty compound takes it away. It is saved with the player and never sent to
    /// a client; changing it raises no notification, so a mod that wants to hear of changes
    /// keeps its own. On a death respawn and on a player's first exit from the End (before
    /// the credits) only the `PlayerPersisted` sub-compound is carried to the new player, as
    /// NeoForge does, and every other key is lost; dimension changes, later End exits and
    /// teleports keep the whole compound. For ports: new mods use attachments. Refuses,
    /// writing nothing: `Stale`, `Client`, `Dead`, and `TooDeep` for data that would sit 512
    /// levels deep or more in the player's file.
    /// An open transaction journals this write; an abort restores the previous value.
    #[access(WorldOnly)]
    fn player_set_persistent_data(
        &mut self,
        player: PlayerRef,
        data: Compound,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Sets property `name` of the player's game profile (`textures` for
    /// a skin: Mojang's base64 value and its signature, which the client
    /// checks on an online-mode server), replacing one of the same name.
    /// Other players see it once the player is sent to them again
    /// (`player_refresh`). False for a stale or unavailable player, a fake
    /// player, an empty name, a name, value or signature longer than the
    /// protocol allows (64, 32767 and 1024 UTF-16 code units, so a character past
    /// the basic plane counts twice), and a name the profile does not have yet
    /// when it already holds 16 properties (replacing one of its own names
    /// still works). The integrated server's owner keeps the account's other
    /// properties, of which a profile lists at most 16 in all (the mod's win).
    /// The player's login properties (`player_profile`) stay as they are.
    #[access(WorldOnly)]
    fn player_set_profile_property(
        &mut self,
        player: PlayerRef,
        name: &str,
        value: &str,
        signature: Option<&str>,
    ) -> bool;
    /// Side: server.
    /// Sends the player to the other players again, as Paper's
    /// `hidePlayer` then `showPlayer` does: over the next two ticks they
    /// leave every other player's Tab list and world and come back with
    /// their profile as it is now, so a new skin shows. The player's own
    /// view keeps the skin their client has until they change level or join
    /// again. A player a mod hid (`player_hide`) stays hidden from whom it
    /// hid them. False for a stale or unavailable player and a fake player.
    #[access(WorldOnly)]
    fn player_refresh(&mut self, player: PlayerRef) -> bool;
    /// Side: server.
    /// Replaces all of the player's statistics with `json` (the text `player_statistics`
    /// gives, or another server's `stats/<uuid>.json`), as `StatsCounter.setValue` for each
    /// (Paper's `Player.setStatistic`): a statistic `json` lacks goes back to 0, scoreboard
    /// objectives on a statistic's criterion follow, and the client gets the changed values
    /// the next time it asks (its statistics screen). `player_stat_award` is not heard.
    /// False, with nothing changed, for text that is not a JSON object, a stale, unavailable
    /// or dead player and on a client.
    #[access(WorldOnly)]
    fn player_set_statistics(&mut self, player: PlayerRef, json: &str) -> bool;
    /// Side: server.
    /// Sets a server player's velocity in blocks a tick and tells their
    /// client (`Entity.setDeltaMovement` with `hurtMarked`, which sends
    /// `ClientboundSetEntityMotionPacket` about the player themselves; Paper's
    /// `Player.setVelocity`). The client owns a player's motion, so it moves
    /// by the new value from its next tick. `entity_push` adds to the
    /// velocity instead; `entity_set_velocity` refuses players. False for a
    /// stale or unavailable player, a displaced outer player, and a value
    /// that is not finite.
    #[access(WorldOnly)]
    fn player_set_velocity(&mut self, player: PlayerRef, velocity: [f64; 3]) -> bool;
}
