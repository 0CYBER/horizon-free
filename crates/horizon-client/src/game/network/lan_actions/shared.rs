// Shared state, helpers and mixed tests stay at the original LAN action scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuOpen {
    Opened,
    Cancelled,
    Refused,
    Stale,
}

/// Per-guest state of the gameplay code that lives outside `Guest`.
#[derive(Default)]
pub(super) struct GuestPlay {
    mining: Option<Mining>,
    destroy_delay: u32,
    using: Option<(usize, u32)>,
    use_duration: i32,
    use_start: super::using_item::UseStart,
    use_delay: u32,
    /// The shield's ItemCooldowns entry: an axe disabled it for this many ticks.
    pub(super) shield_cooldown: u32,
    /// `ServerGamePacketListenerImpl.dropSpamThrottler` (`TickThrottler(20, 1480)`): 20 for each
    /// item a creative player drops from the creative screen, one less each tick.
    creative_drops: u32,
    /// `ServerGamePacketListenerImpl.chatSpamThrottler`: 20 for each chat message, one less each tick.
    pub(super) chat_spam: u32,
    /// `commandSpamThrottler`: 20 for each command, one less each tick.
    pub(super) command_spam: u32,
    /// The recipe viewer's transfer checks (`horizon:recipe_check`): 20 for
    /// each, one less each tick; one from 200 goes unanswered.
    pub(super) recipe_checks: u32,
    /// `ServerCommandSuggestionsProvider`: the latest suggestion request not answered yet (a
    /// newer one replaces it, which then never gets an answer), and whether this tick's one
    /// answer is still free.
    suggestion: Option<Vec<u8>>,
    suggestion_budget: bool,
    /// The guest's `Player.getCooldowns()` (`Game::cooldowns` for its turn).
    pub(super) cooldowns: horizon_client::cooldowns::Cooldowns,
    attack_ticks: u32,
    /// The main hand's item at the guest's last tick (`Game::attack_item`).
    attack_item: Option<std::sync::Arc<str>>,
    swing: horizon_client::first_person::SwingState,
    /// The block being dug and the face the client aimed at.
    digging: Option<([i32; 3], [i32; 3])>,
    /// The server tick the digging started (ServerPlayerGameMode.destroyProgressStart).
    dig_start: u64,
    /// The block that clock is for (ServerPlayerGameMode.destroyPos). A START is the only writer
    /// that names one (death, a new ServerPlayerGameMode, clears it), so an abort, or a STOP that
    /// arrives out of reach, leaves it: the STOP of a dig resumed on the same block is still
    /// measured from the first START's ticks (#4124).
    destroy_pos: Option<[i32; 3]>,
    /// A block the client stopped digging before the server's time was up: it breaks once
    /// its progress since `dig_start` is complete (ServerPlayerGameMode.hasDelayedDestroy).
    delayed: Option<([i32; 3], u64)>,
    last_progress: i32,
    /// Swings the guest's actions started this tick, for the players around
    /// (`LivingEntity.swing` sends ClientboundSwingAnimationPacket).
    swings: u32,
    /// The guest's riding jump charge (`LocalPlayer.jumpRidingTicks`).
    mount: super::mounts::MountState,
    /// The trading screen the guest has open.
    pub(super) merchant: Option<super::lan_trading::GuestMerchant>,
    /// The mount inventory the guest has open (game/network/lan_mount_menu.rs).
    pub(super) mount_window: Option<super::lan_mount_menu::GuestMount>,
    /// The lectern the guest reads (game/network/lan_stations.rs).
    pub(super) lectern: Option<super::lan_stations::GuestLectern>,
    /// The equipment the mods last heard this guest wear (`native_players_equipment`), by
    /// `EquipmentSlot`: main hand, off hand, feet, legs, chest, head.
    pub(super) equipment_heard: Option<Box<[Option<crate::items::ItemStack>; 6]>>,
    /// The slots of the guest's inventories the mods last heard (`native_players_inventory_changes`),
    /// by `NativeInventoryKind`: Player, Ender, Cursor, Crafting.
    pub(super) inventory_heard: Option<Box<[Vec<Option<crate::items::ItemStack>>; 4]>>,
    /// What the guest's body was given for its effects and enchantments.
    potions: super::potions::BodyPotions,
    /// `ServerPlayer.getStats` and the inventory `inventory_changed` last saw.
    pub(super) stats: super::stats::Stats,
    /// `Stats.LEAVE_GAME` is counted: a guest is saved more than once on the
    /// way out, the statistic is awarded once (`Game::award_leave_game`).
    pub(super) left_game: bool,
    last_inventory: u64,
    /// `ServerPlayer.levitationStartPos` and `levitationStartTime`.
    levitation: Option<([f64; 3], u64)>,
    /// The fall, lava ride, Nether trip and blast the guest's advancements track.
    marks: super::advancements::TravelMarks,
    /// `Entity.portalProcess` and `portalCooldown` (game/world/server_travel.rs).
    pub(super) portal: horizon_client::nether_portal::PortalState,
    /// `ServerPlayer.wonGame`: the End poem is showing, the player waits for
    /// its client's PERFORM_RESPAWN (`respawn_requested`).
    pub(super) won_game: bool,
    pub(super) respawn_requested: bool,
    /// `Player.takeXpDelay`: ticks until the next orb can be taken.
    pub(super) xp_delay: u32,
    /// This player entity's age and level-up sound cooldown.
    level_up_sound: horizon_client::entities::LevelUpSound,
    /// What last killed the guest outright (`/kill`): the death message's key.
    death_cause: Option<String>,
    /// The equipment modifiers on the guest's attributes (`apply_equipment`)
    /// and the armour and main hand they were put on for: the hub's player
    /// is a `ServerPlayer` too, whose worn armour counts
    /// (`LivingEntity.collectEquipmentChanges`).
    modifiers: Vec<(String, String)>,
    equipped: Option<Vec<Option<ItemStack>>>,
    craft_grid: Vec<Option<ItemStack>>,
    /// The guest's cast fishing hook (`Player.fishing`).
    pub(super) hook: horizon_client::join_seq::ResidentSlot<super::fishing::Hook>,
    sculk: super::sculk::PlayerEvents,
    /// What `ServerPlayer` last recorded for the guest's scoreboard criteria
    /// (`lastRecordedHealthAndAbsorption` and the others).
    score_recorded: super::scoreboard::Recorded,
}

/// A guest's command waiting for the hub (`LanHost::commands`).
pub(super) struct GuestCommand {
    pub(super) conn: u32,
    pub(super) command: String,
    pub(super) permission: u8,
    /// An integrated server's: no level teleports (yet).
    pub(super) integrated: bool,
    /// A mod's quiet command (game/network/lan_quiet.rs): its output suppressed, as
    /// `CommandSourceStack.withSuppressedOutput` does.
    pub(super) quiet: bool,
}

/// Why a guest's turn didn't start (`as_guest_at`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnRefused {
    /// No guest has that `conn` at that place in the list, or it has no body.
    Gone,
    /// The guest is already taking a turn: its record holds an outer player's state.
    Nested,
}

/// The game's own state while a guest's is swapped in (`enter_guest`), and
/// the guest's records that live outside `Guest`.
pub(super) struct GuestSwap {
    attachments: horizon_mod_runtime::attachments::ResidentAttachments,
    conn: u32,
    i: usize,
    play: GuestPlay,
    player: Option<PlayerMotion>,
    player_save_projection: (Option<i32>, Option<bool>),
    /// The guest's eye height and the game's.
    eye: (f32, f32),
    yaw: f32,
    pitch: f32,
    selected: usize,
    mode: horizon_client::command::args::GameMode,
    creative: bool,
    /// The previous game mode of the turn this one nests in: each player's
    /// own (`ServerPlayerGameMode.previousGameModeForPlayer`).
    previous_mode: Option<horizon_client::command::args::GameMode>,
    /// The abilities of the turn this one nests in.
    abilities: Option<horizon_client::lan::abilities::PlayerAbilities>,
    /// The host's cached interaction ranges, restored after the guest turn.
    entity_reach: f64,
    block_reach: f64,
    /// The host's command permission, swapped for the guest's for its turn.
    permission: u8,
    xp: Experience,
    level_up_sound: horizon_client::entities::LevelUpSound,
    host_keys: horizon_client::held::HeldKeys,
    screen: Screen,
    host_sign: Option<super::sign_edit::SignEditor>,
    host_book: Option<super::books::BookScreen>,
    host_command_block: Option<super::command_blocks::CommandBlockScreen>,
    host_mount_menu: super::mount_menu::MountMenu,
    craft_size: usize,
    craft_table: Option<[i32; 3]>,
    craft_grid: Vec<Option<ItemStack>>,
    container: Option<(i32, i32, i32)>,
    mining: Option<Mining>,
    destroy_delay: u32,
    using: Option<(usize, u32)>,
    use_duration: i32,
    use_start: super::using_item::UseStart,
    use_delay: u32,
    pub(super) shield_cooldown: u32,
    /// The guest's item cooldowns (`Game::cooldowns` for its turn).
    cooldowns: horizon_client::cooldowns::Cooldowns,
    /// The guest's `takeXpDelay` (`Game::xp_delay` for its turn).
    xp_delay: u32,
    attack_ticks: u32,
    attack_item: Option<std::sync::Arc<str>>,
    swing: horizon_client::first_person::SwingState,
    left: bool,
    right: bool,
    riding: Option<u32>,
    host_station: Option<super::stations::Station>,
    sleeping: Option<[i32; 3]>,
    sleep_timer: u32,
    spawn: Option<Vec3>,
    respawn: (
        Option<super::respawn_anchor::RespawnConfig>,
        Option<[f64; 3]>,
    ),
    message: Option<(String, std::time::Instant)>,
    host_ender: Option<(i32, i32, i32)>,
    adv_loaded: bool,
    last_inventory: u64,
    levitation: Option<([f64; 3], u64)>,
    marks: super::advancements::TravelMarks,
    death_cause: Option<String>,
    /// The guest's own damage record, in place while they act.
    combat: super::combat::CombatTracker,
    /// The guest whose turn this one nests in, if any.
    acting: (
        Option<u32>,
        Option<String>,
        Option<u128>,
        Option<u8>,
        Option<u32>,
        Option<i32>,
    ),
    /// Whether the outer turn is a fake player.
    acting_fake: bool,
    /// That turn's other-player building boxes.
    building_boxes: BuildingPlayers,
    /// That turn's `known_movement`.
    known_movement: Vec3,
    /// The packets queued for that guest.
    packets: Vec<play::Clientbound>,
    /// The entity events queued for that guest.
    entity_events: Vec<i8>,
    /// The equipment breaks queued for that guest.
    equipment_breaks: Vec<i8>,
    /// The cooldown packets queued for that guest.
    cooldown_events: Vec<(std::sync::Arc<str>, i32)>,
    /// The player-source sounds queued for that guest.
    player_sounds: Vec<(String, [f64; 3], f32, f32)>,
    block_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// The host's count of swings started, kept while the guest acts.
    swings: u32,
    /// How many projectiles there were: those added since are the guest's.
    projectiles: usize,
    /// The host's equipment modifiers.
    modifiers: Vec<(String, String)>,
}

impl GuestPlay {
    /// The guest's `Player.attackStrengthTicker` outside its turn.
    pub(in crate::game) fn attack_ticks(&self) -> u32 {
        self.attack_ticks
    }
    /// The main-hand item of the guest's last tick (`Game.attack_item`).
    pub(in crate::game) fn attack_item(&self) -> Option<&str> {
        self.attack_item.as_deref()
    }
    /// The item in use: its slot and for how many ticks.
    pub(super) fn using(&self) -> Option<(usize, u32)> {
        self.using
    }
    /// `LivingEntity.getUsedItemHand` of the item in use, none when none is.
    pub(super) fn used_hand(&self) -> Option<Hand> {
        super::item_use::hand_of(self.using)
    }
    /// `LivingEntity.useItemRemaining` of the item in use, 0 when none is.
    pub(super) fn use_remaining(&self) -> i32 {
        super::item_use::remaining_of(self.using, self.use_duration)
    }
    /// Sets `useItemRemaining`; false when no item is in use.
    pub(super) fn set_use_remaining(&mut self, remaining: i32) -> bool {
        super::item_use::set_remaining(self.using, &mut self.use_duration, remaining)
    }
    /// The `ServerPlayerGameMode.destroyProgressStart` a STOP on `pos` is measured from: the clock
    /// belongs to the block the START named (`destroyPos`), and only a START writes either, so a
    /// dig resumed after a fall still runs on the first START's ticks (#4124).
    fn dig_start_for(&self, pos: [i32; 3]) -> Option<u64> {
        (self.destroy_pos == Some(pos)).then_some(self.dig_start)
    }
    /// A dig that ends without a break, an abort or a STOP from out of reach: the guest holds
    /// attack no more (`digging` is `as_guest`'s left button and `tick_mining`'s `forced_hit`),
    /// while the clock `destroy_pos` names stays for the STOP that comes when the guest is back
    /// in reach (#4124).
    fn dig_ended(&mut self) {
        self.digging = None;
        self.mining = None;
    }
}

struct ItemRules<'a>(
    &'a horizon_client::items::Items,
    &'a horizon_client::enchant::Enchantments,
    Option<&'a dyn horizon_client::menu::SlotRules>,
);
impl Rules for ItemRules<'_> {
    fn items(&self) -> &horizon_client::items::Items {
        self.0
    }
    fn enchantments(&self) -> &horizon_client::enchant::Enchantments {
        self.1
    }
    fn max_stack(&self, item: &str) -> u32 {
        self.0.get(item).map_or(64, |i| i.max_stack)
    }
    fn slot_rules(&self) -> Option<&dyn horizon_client::menu::SlotRules> {
        self.2
    }
}

/// `Player.isWithinEntityInteractionRange(entity, buffer)` for a guest: the
/// eyes closer than the entity interaction range (3, 5 in creative) plus
/// `buffer` to the entity's box.
pub(super) fn guest_within_entity_range(
    g: &horizon_client::lan::guest::Guest,
    bounds: &horizon_core::geometry::Aabb,
    buffer: f64,
) -> bool {
    let eye = [g.position[0], g.position[1] + g.eye_height(), g.position[2]];
    let (lo, hi) = (bounds.min_position(), bounds.max_position());
    let d = [
        eye[0] - eye[0].clamp(lo.x, hi.x),
        eye[1] - eye[1].clamp(lo.y, hi.y),
        eye[2] - eye[2].clamp(lo.z, hi.z),
    ];
    let range = if g.creative() { 5.0 } else { 3.0 } + buffer;
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range
}
fn direction(face: i32) -> [i32; 3] {
    match face {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

fn building_player(g: &horizon_client::lan::guest::Guest) -> BuildingPlayer {
    BuildingPlayer {
        conn: Some(g.conn),
        dimension: Dim::from_id(&g.dimension),
        bounds: (g.participates_in_player_simulation()
            && g.placed
            && g.loaded
            && !g.dead
            && !g.spectator())
        .then(|| g.bounds()),
        borrowed: false,
    }
}

thread_local! {
    /// The hub's bodies for the turns of one `tick_guests`, built once instead
    /// of for every turn (players x turns a tick). A guest's entry is refreshed
    /// when its turn ends ([`refresh_building_player`]), so the turns after it
    /// see it where it went (a pearl, a death, spectator mode).
    static BUILDING_CACHE: std::cell::RefCell<Option<HubBodies>> =
        const { std::cell::RefCell::new(None) };
}

/// The hub's current bodies, across dimensions. A borrowed ancestor's
/// live body takes precedence over this saved state until its turn returns.
fn building_players(host: &LanHost) -> HubBodies {
    let guests: Vec<_> = host.hub.guests.simulation_iter().collect();
    let kept = BUILDING_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|list| {
                list.len() == guests.len()
                    && list
                        .iter()
                        .zip(&guests)
                        .all(|(p, g)| p.get().conn == Some(g.conn))
            })
            .cloned()
    });
    kept.unwrap_or_else(|| shared(guests.into_iter().map(building_player)))
}

/// Builds the hub's bodies once for the turns to come (`Some`), or drops them.
pub(super) fn building_cache(host: Option<&LanHost>) {
    let bodies = host.map(|host| shared(host.hub.guests.simulation_iter().map(building_player)));
    BUILDING_CACHE.with(|c| *c.borrow_mut() = bodies);
}

/// The cached bodies as they are now in the hub, after `conn`'s turn (or its
/// death step). Every entry, not only the actor's: a turn can move, kill or
/// change another guest too (`/tp`, an attack, an explosion), and the turns
/// after it must see that guest where it is now, as a list built afresh would.
/// Copies into the shared cells, no allocation.
fn refresh_building_player(host: &LanHost, _conn: u32) {
    BUILDING_CACHE.with(|c| {
        if let Some(list) = c.borrow().as_ref()
            && list.len() == host.hub.guests.simulation_iter().count()
        {
            for (p, g) in list.iter().zip(host.hub.guests.simulation_iter()) {
                if p.get().conn == Some(g.conn) {
                    p.set(building_player(g));
                }
            }
        }
    });
}

/// Kept at LAN action scope to preserve its derived internals point identity.
impl Game<'_> {
    pub(super) fn vehicle_has_other_viewers(
        container: Option<(i32, i32, i32)>,
        guests: &[horizon_client::lan::guest::Guest],
        closing_conn: Option<u32>,
        key: (i32, i32, i32),
    ) -> bool {
        container == Some(key)
            || guests.iter().any(|g| {
                Some(g.conn) != closing_conn
                    && g.menu
                        .as_ref()
                        .is_some_and(|m| m.slots == Slots::Block(key))
            })
    }
}

fn captured_block_break(sound: &horizon_client::audio::CapturedSound, blocks: &[[f64; 3]]) -> bool {
    sound.entity.is_none()
        && blocks
            .iter()
            .any(|b| (0..3).all(|k| (b[k] + 0.5 - sound.position[k]).abs() < 1e-6))
}

/// ClientboundSwingAnimationPacket: the hand and the animation (its type and
/// base duration; each client applies the swinger's haste and fatigue).
pub(super) fn swing_packet(id: i32, hand: Hand, animation: SwingAnimation) -> C {
    C::SwingAnimation(play::SwingAnimation {
        id,
        hand: i32::from(hand == Hand::Off),
        kind: i32::from(animation.kind),
        duration: animation.duration,
    })
}

/// The block columns `Entity.isInWall` looks at: `AABB.ofSize` around the
/// eyes, 0.8 of the body's width across (`BlockPos.betweenClosedStream`).
fn eye_columns(at: [f64; 3], body: Dimensions) -> [std::ops::RangeInclusive<i32>; 2] {
    let half = f64::from(body.width * 0.8) / 2.0;
    let range = |v: f64| (v - half).floor() as i32..=(v + half).floor() as i32;
    [range(at[0]), range(at[2])]
}

/// What `ServerPlayerGameMode.handleBlockBreakAction`'s STOP_DESTROY_BLOCK branch does with a
/// guest's STOP.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StopDig {
    /// A block the last START did not name: the STOP is not this dig's.
    Other,
    /// Already broken, by the guest's own tick: only the crack goes.
    Gone,
    /// The dig the START counts is at least 70 % done: the block breaks outright.
    Break,
    /// Less than that: the server finishes it alone from that same START.
    Delay(u64),
}

/// The STOP branch's decision: a STOP is measured from the START that named its block
/// (`destroyPos`, which an abort or a STOP from out of reach leaves alone), whose ticks times
/// `getDestroyProgress` decide between the break and the delayed dig (#4124).
fn stop_dig(play: &GuestPlay, pos: [i32; 3], air: bool, ticks: u64, per_tick: f32) -> StopDig {
    match play.dig_start_for(pos) {
        None => StopDig::Other,
        Some(_) if air => StopDig::Gone,
        Some(start) if per_tick * (ticks - start + 1) as f32 >= 0.7 => StopDig::Break,
        Some(start) => StopDig::Delay(start),
    }
}

/// What `copyRespawnPosition` sets at respawn time, when it is decided here:
/// nothing to decide for a point with a block (`has_config`, copied once the
/// block is found); otherwise the Overworld forced point's block (`spawn`,
/// stored at its bottom centre), or none.
fn copied_respawn_point(has_config: bool, spawn: Option<[f64; 3]>) -> Option<Option<[i32; 3]>> {
    (!has_config).then(|| spawn.map(|s| s.map(|c| c.floor() as i32)))
}

/// ServerPlayer.swingAndResetAttackStrength resets only when a use starts a swing.
fn reset_use_attack_strength(ticker: &mut u32, before: u32, after: u32) {
    if before != after {
        *ticker = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{copied_respawn_point, eye_columns, reset_use_attack_strength};
    use crate::game::api_values::TryIntoApi;
    use horizon_client::pose::Pose;
    #[test]
    fn a_block_break_suppresses_its_positional_duplicate_but_not_an_entity_sound() {
        let mut sound = horizon_client::audio::CapturedSound {
            event: "minecraft:block.stone.break".into(),
            position: [1.5, 64.5, -2.5],
            volume: 1.0,
            pitch: 1.0,
            source: 4,
            entity: None,
            except_entity: None,
        };
        let broken = [[1.0, 64.0, -3.0]];
        assert!(super::captured_block_break(&sound, &broken));
        sound.entity = Some(37);
        assert!(!super::captured_block_break(&sound, &broken));
        sound.entity = None;
        sound.position[0] += 0.25;
        assert!(!super::captured_block_break(&sound, &broken));
    }

    #[test]
    fn use_swing_resets_attack_charge_only_when_started() {
        let mut ticker = 20;
        reset_use_attack_strength(&mut ticker, 3, 4);
        // A sword's attack speed is 1.6: vanilla's (ticker + 0.5) / 12.5.
        assert_eq!((ticker as f32 + 0.5) / 12.5, 0.04);

        ticker = 8;
        reset_use_attack_strength(&mut ticker, 4, 4);
        assert_eq!(ticker, 8); // No swing, or a swing rejected in its first half.
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn menu_rules_match_guest_packets_and_prediction() {
        use horizon_client::{
            chunk::Chunk,
            containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModMenuType},
            items::ItemStack,
            lan::menu::Slots,
            menu::{Input, Kind, Menu},
        };
        use horizon_protocol::packets::play::{ContainerClick, Serverbound};
        use mod_api::{Hooks, MenuPart, MenuSlotRulesEvent, ModEntry, Mods, Registrar};
        use std::sync::Arc;

        struct Part;
        #[mod_api::horizon_part]
        impl MenuPart for Part {
            fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
                if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                    == "minecraft:granite"
                    && event.stack.count() < 8
                {
                    event.may_place = false;
                }
                if event.slot == 0 {
                    if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                        == "minecraft:diorite"
                        && event.stack.count() >= 8
                    {
                        event.max_stack = 3;
                    }
                    if matches!(
                        mod_api::__private::item_key_id(event.stack.item()).unwrap(),
                        "minecraft:andesite" | "minecraft:stone"
                    ) {
                        event.max_stack = 3;
                    }
                    if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                        == "minecraft:dirt"
                    {
                        event.may_place = false;
                    }
                }
            }
        }
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.menu_part("minecraft:generic_9x3", Part);
                r.menu_part("test:menu", Part);
            }
        }
        let mods: Mods = vec![ModEntry::new("menu_rules", Box::new(Mod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let pos = (1, 64, 1);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                let menu_type = Box::leak(Box::new(ModMenuType {
                    id: "test:menu",
                    slots: 27,
                    data_slots: 0,
                    results: Vec::new(),
                    crafting: None,
                    links: None,
                }));
                for is_mod in [false, true] {
                    game.world.block_entities.insert(
                        pos,
                        if is_mod {
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "test:block".into(),
                                ModBlockEntityFields {
                                    items: vec![None; 27],
                                    ..Default::default()
                                },
                            )))
                        } else {
                            BlockEntity::Chest(vec![None; 27])
                        },
                    );
                    let host = game.lan.as_mut().unwrap();
                    host.hub.open_container(
                        12,
                        "minecraft:generic_9x3",
                        horizon_core::nbt::Tag::String("test".into()),
                        pos,
                        Slots::Block(pos),
                        vec![None; 27],
                        None,
                    );
                    host.hub.guests.update(0, |guest| {
                        let opened = guest.menu.as_mut().unwrap();
                        if is_mod {
                            opened.registry_id = "test:menu".into();
                            opened.mod_type = Some(menu_type);
                        }
                    });
                    let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                    for (item, existing, expected) in [
                        ("minecraft:dirt", 0, 0),
                        ("minecraft:stone", 0, 3),
                        ("minecraft:diorite", 2, 3),
                    ] {
                        let stack = ItemStack::new(item, 8);
                        game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                            (existing > 0).then(|| ItemStack::new(item, existing));
                        let mut prediction = Menu::new(if is_mod {
                            Kind::Mod(menu_type)
                        } else {
                            Kind::Chest(3)
                        });
                        prediction.set_registry_id(Some(if is_mod {
                            "test:menu"
                        } else {
                            "minecraft:generic_9x3"
                        }));
                        prediction
                            .load_slot(0, (existing > 0).then(|| ItemStack::new(item, existing)));
                        prediction.carried = Some(stack.clone());
                        prediction
                            .click(&game.menu_env(), 0, 0, Input::Pickup)
                            .unwrap();
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| guest.carried = Some(stack));
                        game.lan_guest_packet(
                            12,
                            Serverbound::ContainerClick(ContainerClick {
                                container: window,
                                state: 0,
                                slot: 0,
                                button: 0,
                                input: 0,
                                changed: Vec::new(),
                                carried: None,
                            }),
                        );
                        let actual = game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .map_or(0, |s| s.count);
                        assert_eq!(actual, expected, "{item} mod={is_mod}");
                        assert_eq!(prediction.slot(0).map_or(0, |s| s.count), actual);
                        assert_eq!(
                            actual + prediction.carried.as_ref().map_or(0, |s| s.count),
                            existing + 8
                        );
                        assert_eq!(
                            prediction.carried.as_ref().map_or(0, |s| s.count),
                            game.lan.as_ref().unwrap().hub.guests[0]
                                .carried
                                .as_ref()
                                .map_or(0, |s| s.count)
                        );
                    }
                    for (item, existing, second, carried, creative, expected) in [
                        ("minecraft:granite", 0, 0, 8, false, (4, 4, 0)),
                        ("minecraft:andesite", 8, 0, 8, false, (3, 4, 9)),
                        ("minecraft:stone", 2, 0, 64, true, (3, 64, 0)),
                        ("minecraft:stone", 2, 63, 64, true, (3, 64, 62)),
                    ] {
                        let slots = game.world.block_entities.get_mut(&pos).unwrap().slots_mut();
                        slots[0] = (existing > 0).then(|| ItemStack::new(item, existing));
                        slots[1] = (second > 0).then(|| ItemStack::new(item, second));
                        let mut prediction = Menu::new(if is_mod {
                            Kind::Mod(menu_type)
                        } else {
                            Kind::Chest(3)
                        });
                        prediction.set_registry_id(Some(if is_mod {
                            "test:menu"
                        } else {
                            "minecraft:generic_9x3"
                        }));
                        prediction.load_slot(0, slots[0].clone());
                        prediction.load_slot(1, slots[1].clone());
                        prediction.carried =
                            Some(horizon_client::items::ItemStack::new(item, carried));
                        prediction.creative = creative;
                        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                            guest.carried =
                                Some(horizon_client::items::ItemStack::new(item, carried));
                            guest.game_mode = if creative {
                                horizon_client::lan::guest::CREATIVE
                            } else {
                                0
                            };
                        });
                        let kind = if creative { 2 << 2 } else { 0 };
                        for (slot, button) in
                            [(-999, kind), (0, kind | 1), (1, kind | 1), (-999, kind | 2)]
                        {
                            prediction
                                .click(&game.menu_env(), slot, button, Input::QuickCraft)
                                .unwrap();
                            game.lan_guest_packet(
                                12,
                                Serverbound::ContainerClick(ContainerClick {
                                    container: window,
                                    state: 0,
                                    slot: slot as i16,
                                    button: button as i8,
                                    input: 5,
                                    changed: Vec::new(),
                                    carried: None,
                                }),
                            );
                        }
                        let slots = game.world.block_entities.get(&pos).unwrap().slots();
                        let result = (
                            slots[0].as_ref().map_or(0, |s| s.count),
                            slots[1].as_ref().map_or(0, |s| s.count),
                            game.lan.as_ref().unwrap().hub.guests[0]
                                .carried
                                .as_ref()
                                .map_or(0, |s| s.count),
                        );
                        let predicted = (
                            prediction.slot(0).map_or(0, |s| s.count),
                            prediction.slot(1).map_or(0, |s| s.count),
                            prediction.carried.as_ref().map_or(0, |s| s.count),
                        );
                        assert_eq!(result, expected, "{item} mod={is_mod}");
                        assert_eq!(predicted, expected, "{item} mod={is_mod}");
                        if !creative {
                            assert_eq!(result.0 + result.1 + result.2, existing + carried);
                        }
                    }
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.game_mode = 0);
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                        Some(horizon_client::items::ItemStack::new("minecraft:dirt", 2));
                    let mut prediction = Menu::new(if is_mod {
                        Kind::Mod(menu_type)
                    } else {
                        Kind::Chest(3)
                    });
                    prediction.set_registry_id(Some(if is_mod {
                        "test:menu"
                    } else {
                        "minecraft:generic_9x3"
                    }));
                    prediction.load_slot(
                        0,
                        Some(horizon_client::items::ItemStack::new("minecraft:dirt", 2)),
                    );
                    prediction.set_player_item(
                        9,
                        Some(horizon_client::items::ItemStack::new("minecraft:dirt", 4)),
                    );
                    prediction
                        .click(&game.menu_env(), 27, 0, Input::QuickMove)
                        .unwrap();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory[9] =
                            Some(horizon_client::items::ItemStack::new("minecraft:dirt", 4))
                    });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 27,
                            button: 0,
                            input: 1,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .count,
                        2
                    );
                    assert_eq!(prediction.slot(0).unwrap().count, 2);
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory.fill(None);
                        guest.inventory[0] =
                            Some(horizon_client::items::ItemStack::new("minecraft:stone", 8));
                    });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 54,
                            button: 0,
                            input: 2,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[0].as_ref().unwrap().count, 8);
                    assert_eq!(guest.inventory.iter().flatten().count(), 1);
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                        Some(horizon_client::items::ItemStack::new("minecraft:dirt", 2));
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory.fill(None);
                        guest.inventory[0] =
                            Some(horizon_client::items::ItemStack::new("minecraft:stone", 8));
                    });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button: 0,
                            input: 2,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .count,
                        3
                    );
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[0].as_ref().unwrap().count, 5);
                    assert_eq!(guest.inventory[1].as_ref().unwrap().count, 2);
                    // A component-limited displaced stack fills to 16, then leaves 7 elsewhere.
                    let mut limited = ItemStack::new("minecraft:dirt", 8);
                    limited.data_mut().set_other(
                        "minecraft:max_stack_size",
                        Some(horizon_core::nbt::Tag::Int(16)),
                    );
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                        Some(limited.clone());
                    limited.count = 15;
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory.fill(None);
                        guest.inventory[1] = Some(limited);
                        guest.selected = 1;
                        guest.inventory[0] =
                            Some(horizon_client::items::ItemStack::new("minecraft:stone", 8));
                    });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button: 0,
                            input: 2,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[1].as_ref().unwrap().count, 16);
                    assert_eq!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .filter(|s| &*s.item == "minecraft:dirt")
                            .map(|s| s.count)
                            .sum::<u32>(),
                        23
                    );
                    game.guest_close_all(12);
                }
            })
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn crafting_guest_self_swap_keeps_eight_items() {
        use horizon_client::chunk::Chunk;
        use horizon_protocol::packets::play::{ContainerClick, Serverbound};
        use std::sync::Arc;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let table = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafting_table")
                .unwrap();
            game.world.set(1, 64, 1, table).unwrap();
            game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                guest.inventory[0] =
                    Some(horizon_client::items::ItemStack::new("minecraft:stone", 8));
            });
            game.lan_open_crafting(12, [1, 64, 1]);
            let window = game
                .lan
                .as_ref()
                .unwrap()
                .menus
                .get(&12)
                .unwrap()
                .crafting
                .table
                .as_ref()
                .unwrap()
                .0;
            game.lan_guest_packet(
                12,
                Serverbound::ContainerClick(ContainerClick {
                    container: window,
                    state: 0,
                    slot: 37,
                    button: 0,
                    input: 2,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.inventory[0].as_ref().unwrap().count, 8);
            assert_eq!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum::<u32>(),
                8
            );
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn block_menu_handle_and_use_each_count_the_guest_once() {
        use horizon_client::chunk::Chunk;
        use mod_api::BlockUpdateFlags;
        use std::sync::Arc;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let pos = (0, 64, 0);
            assert_eq!(
                game.level_set_block(
                    level,
                    [pos.0, pos.1, pos.2],
                    chest,
                    BlockUpdateFlags::NEIGHBORS
                ),
                Ok(true)
            );
            // A handle starts here, without an earlier use's `open_menu`.
            assert_eq!(game.open_block_menu(12, pos), super::MenuOpen::Opened);
            assert_eq!(game.lid_openers.get(&pos), Some(&1));
            game.guest_close_all(12);
            assert!(!game.lid_openers.contains_key(&pos));
            // A block use starts before leaving the guest turn; its hand-off
            // must not start the same opener a second time.
            assert!(
                game.as_guest(12, |game| game
                    .open_block_effect_menu([pos.0, pos.1, pos.2]))
                    .is_some()
            );
            assert!(game.lan.as_ref().unwrap().hub.guests[0].menu.is_some());
            assert_eq!(game.lid_openers.get(&pos), Some(&1));
            game.guest_close_all(12);
            assert!(!game.lid_openers.contains_key(&pos));
            struct StopMenu;
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for StopMenu {
                fn container_opened(
                    &self,
                    _: &mut dyn mod_api::ServerHost,
                    _: &mut mod_api::ContainerOpenedEvent<'_>,
                ) -> mod_api::Flow {
                    mod_api::Flow::Cancel
                }
            }
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new("stop_menu", Box::new(StopMenu))].into(),
                || {
                    assert!(
                        game.as_guest(12, |game| game
                            .open_block_effect_menu([pos.0, pos.1, pos.2]))
                            .is_some()
                    );
                    assert!(game.lan.as_ref().unwrap().hub.guests[0].menu.is_none());
                    assert!(!game.lid_openers.contains_key(&pos));
                },
            );
            {
                let value = 3;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            assert_eq!(game.open_block_menu(12, pos), super::MenuOpen::Opened);
            assert!(!game.lid_openers.contains_key(&pos));
            game.guest_close_all(12);
            assert!(!game.lid_openers.contains_key(&pos));
            // The incoming double-count bug kept a shulker open after its last close.
            {
                let value = 0;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            let shulker = game
                .pack
                .assets
                .registry
                .default_state("minecraft:shulker_box")
                .unwrap();
            let at = [pos.0, pos.1, pos.2];
            assert_eq!(
                game.level_set_block(level, at, shulker, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            assert!(
                game.as_guest(12, |game| game.open_block_effect_menu(at))
                    .is_some()
            );
            assert_eq!(game.be_anims.shulkers[&at].open_count, 1);
            game.guest_close_all(12);
            assert_eq!(game.be_anims.shulkers[&at].open_count, 0);
            assert_eq!(game.open_block_menu(12, pos), super::MenuOpen::Opened);
            assert_eq!(game.be_anims.shulkers[&at].open_count, 1);
            game.guest_close_all(12);
            assert_eq!(game.be_anims.shulkers[&at].open_count, 0);
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn block_menu_click_keeps_spent_action_on_cancellation() {
        use horizon_client::{chunk::Chunk, lan::guest::GuestKind};
        use mod_api::BlockUpdateFlags;
        use std::sync::Arc;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            assert!(!game.lan_open_container(12, (2, 64, 0)));
            let mut guest = game.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = GuestKind::Fake;
            game.lan.as_mut().unwrap().hub.guests.insert(0, guest);
            assert!(game.lan_open_container(12, (0, 64, 0)));
            assert!(game.lan.as_ref().unwrap().hub.guests[0].menu.is_none());
            let mut guest = game.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = GuestKind::Network;
            game.lan.as_mut().unwrap().hub.guests.insert(0, guest);
            assert!(game.lan_open_container(12, (0, 64, 0)));
            assert!(game.lan.as_ref().unwrap().hub.guests[0].menu.is_some());
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn block_parts_pick_and_use_on_the_server_and_for_a_guest() {
        use super::super::{lan::LanSettings, native_entity_refs::tests::with_game};
        use horizon_client::{
            chunk::Chunk,
            lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
        };
        use horizon_protocol::packets::{configuration::ClientInformation, play};
        use mod_api::InteractionResult as Answer;
        use mod_api::{
            self, BlockPart, CloneItemStackEvent, Hooks, Host, ItemPart, ItemStack, Registrar,
            ServerHost, UseItemOnEvent, UseOnEvent, UseWithoutItemEvent,
        };
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicI32, AtomicUsize},
        };

        struct Picker(Arc<Mutex<Vec<bool>>>);
        #[mod_api::horizon_part]
        impl BlockPart for Picker {
            fn clone_item_stack(
                &self,
                _: &dyn Host,
                event: &mut CloneItemStackEvent<'_>,
            ) -> Option<ItemStack> {
                self.0.lock().unwrap().push(event.include_data);
                event.include_data.then(|| {
                    mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:diamond").unwrap(),
                        1,
                    )
                    .unwrap()
                })
            }
        }
        #[derive(Clone)]
        struct User {
            answer: Arc<Mutex<Option<Answer>>>,
            replace_with: Arc<Mutex<Option<horizon_world::registry::BlockStateId>>>,
            empty_answer: Arc<Mutex<Option<Answer>>>,
            empty_food: Arc<Mutex<Option<i32>>>,
            empty_clicks: Arc<Mutex<Vec<(horizon_world::registry::BlockStateId, u128)>>>,
            clicks: Arc<Mutex<Vec<(u128, mod_api::client::first_person::Hand)>>>,
            item_uses: Arc<AtomicUsize>,
        }
        #[mod_api::horizon_part]
        impl BlockPart for User {
            fn use_item_on(
                &self,
                host: &mut dyn ServerHost,
                event: &mut UseItemOnEvent<'_>,
            ) -> Option<Answer> {
                let replacement = self.replace_with.lock().unwrap().take();
                if let Some(state) = replacement {
                    let level = host.level("minecraft:overworld").unwrap();
                    assert_eq!(
                        level.set_block(
                            host,
                            event.click.pos,
                            state.try_into_api().unwrap(),
                            mod_api::BlockUpdateFlags::KNOWN_SHAPE,
                        ),
                        Ok(true)
                    );
                }
                self.clicks
                    .lock()
                    .unwrap()
                    .push((event.click.player, event.click.hand));
                *self.answer.lock().unwrap()
            }
            fn use_without_item(
                &self,
                host: &mut dyn ServerHost,
                event: &mut UseWithoutItemEvent<'_>,
            ) -> Option<Answer> {
                assert_eq!(event.click.hand, mod_api::client::first_person::Hand::Main);
                if let Some(level) = self.empty_food.lock().unwrap().take() {
                    let player = host.player(event.click.player).unwrap();
                    assert!(player.set_food_level(host, level));
                }
                self.empty_clicks.lock().unwrap().push((
                    crate::game::api_values::TryIntoEngine::try_into_engine(&event.state, &())
                        .unwrap(),
                    event.click.player,
                ));
                *self.empty_answer.lock().unwrap()
            }
        }
        // The held items' own `Item.useOn`, counted, taking the click.
        #[mod_api::horizon_part]
        impl ItemPart for User {
            fn use_on(&self, _: mod_api::HostSide<'_>, _: &mut UseOnEvent<'_>) -> Option<Answer> {
                self.item_uses
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(Answer::SuccessServer)
            }
        }
        struct Mod {
            picks: Arc<Mutex<Vec<bool>>>,
            user: User,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block_key = r.block_key("minecraft:stone");
                r.block_part(block_key, Picker(self.picks.clone()));
                let block_key = r.block_key("minecraft:beehive");
                r.block_part(block_key, Picker(self.picks.clone()));
                let block_key = r.block_key("minecraft:chest");
                r.block_part(block_key, Picker(self.picks.clone()));
                let block_key = r.block_key("minecraft:lever");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:cauldron");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:jukebox");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:decorated_pot");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:redstone_ore");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:chiseled_bookshelf");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:cake");
                r.block_part(block_key, self.user.clone());
                let item = r.item_key("minecraft:stick");
                r.item_part(item, self.user.clone());
                let item = r.item_key("minecraft:water_bucket");
                r.item_part(item, self.user.clone());
                let item = r.item_key("minecraft:stone");
                r.item_part(item, self.user.clone());
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let user = User {
            answer: Arc::new(Mutex::new(Some(Answer::Success))),
            replace_with: Arc::new(Mutex::new(None)),
            empty_answer: Arc::new(Mutex::new(None)),
            empty_food: Arc::new(Mutex::new(None)),
            empty_clicks: Arc::new(Mutex::new(Vec::new())),
            clicks: Arc::new(Mutex::new(Vec::new())),
            item_uses: Arc::new(AtomicUsize::new(0)),
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "picker",
            Box::new(Mod {
                picks: seen.clone(),
                user: user.clone(),
            }),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let stone = state("minecraft:stone");
                let crop = state("minecraft:wheat");
                let chest = state("minecraft:chest");
                game.world.set(0, 64, 0, stone).unwrap();
                game.world.set(1, 64, 0, crop).unwrap();
                let settings = LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                };
                let net = NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 2)
                };
                let config = HubConfig {
                    default_game_mode: 1,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 2,
                    integrated: true,
                    levels: game.level_shapes,
                };
                game.lan_host(settings, net, config).unwrap();
                let (tx, _rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn: 12,
                        uuid: 12,
                        name: "guest12".into(),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    12,
                    [0.5, 64.0, 2.0],
                    10,
                );
                guest.placed = true;
                guest.game_mode = 1;
                game.lan.as_mut().unwrap().hub.guests.push(guest);

                // BlockState.useItemOn replaces the template. Only TryWithEmptyHand
                // reaches useWithoutItem, which toggles this lever on the main hand.
                use mod_api::client::first_person::Hand;
                use std::sync::atomic::Ordering::SeqCst;
                let lever = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:lever")
                    .unwrap();
                let hit = super::super::Hit {
                    pos: [0, 64, 4],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 4.5],
                    inside: false,
                };
                game.world.set(0, 64, 4, lever).unwrap();
                game.inventory[game.selected] =
                    Some(horizon_client::items::ItemStack::new("minecraft:stick", 1));
                for (answer, item_uses, powered, swings) in [
                    (Answer::Success, 0, false, 1),
                    (Answer::SuccessServer, 0, false, 1),
                    (Answer::Consume, 0, false, 0),
                    (Answer::Pass, 1, false, 1),
                    (Answer::Fail, 2, false, 1),
                    (Answer::TryWithEmptyHand, 2, true, 1),
                ] {
                    *user.answer.lock().unwrap() = Some(answer);
                    game.swing = Default::default();
                    let before = game.swings_started;
                    let empty_before = user.empty_clicks.lock().unwrap().len();
                    assert!(game.use_hand(Some(&hit)).unwrap());
                    assert_eq!(
                        user.empty_clicks.lock().unwrap().len() - empty_before,
                        usize::from(answer == Answer::TryWithEmptyHand),
                    );
                    assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 1);
                    assert_eq!(game.swings_started - before, swings);
                    assert_eq!(user.item_uses.load(SeqCst), item_uses);
                    assert_eq!(game.world.block(0, 64, 4) != lever, powered);
                }
                let calls = user.clicks.lock().unwrap().len();
                game.world.set(0, 64, 4, stone).unwrap();
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(user.item_uses.load(SeqCst), 3);
                assert_eq!(user.clicks.lock().unwrap().len(), calls);

                // A block-specific useItemOn is replaced too, before its bucket exchange.
                let cauldron = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cauldron")
                    .unwrap();
                let full = game
                    .with_props(
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:water_cauldron")
                            .unwrap(),
                        &[("level", "3")],
                    )
                    .unwrap();
                let cauldron_hit = super::super::Hit {
                    pos: [5, 64, 4],
                    point: [5.5, 65.0, 4.5],
                    ..hit
                };
                game.creative = false;
                for (answer, expected, bucket, item_calls) in [
                    (Some(Answer::Success), cauldron, "minecraft:water_bucket", 3),
                    (None, full, "minecraft:bucket", 3),
                    (Some(Answer::Pass), cauldron, "minecraft:water_bucket", 4),
                ] {
                    game.world.set(5, 64, 4, cauldron).unwrap();
                    game.inventory[game.selected] = Some(horizon_client::items::ItemStack::new(
                        "minecraft:water_bucket",
                        1,
                    ));
                    *user.answer.lock().unwrap() = answer;
                    assert!(game.use_hand(Some(&cauldron_hit)).unwrap());
                    assert_eq!(game.world.block(5, 64, 4), expected);
                    let held = game.inventory[game.selected].as_ref().unwrap();
                    assert_eq!((&*held.item, held.count), (bucket, 1));
                    assert_eq!(user.item_uses.load(SeqCst), item_calls);
                }

                // TRY_WITH_EMPTY_HAND still ejects a jukebox's disc, not just its
                // level-only success result: the game owns that side effect.
                let jukebox = game
                    .with_props(
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:jukebox")
                            .unwrap(),
                        &[("has_record", "true")],
                    )
                    .unwrap();
                game.world.set(6, 64, 4, jukebox).unwrap();
                game.world.block_entities.insert(
                    (6, 64, 4),
                    horizon_client::containers::BlockEntity::Jukebox(
                        horizon_client::containers::Jukebox {
                            items: [Some(horizon_client::items::ItemStack::new(
                                "minecraft:music_disc_13",
                                1,
                            ))],
                            ticks: 0,
                            playing: false,
                            song_clock_present: false,
                            loaded_clock: false,
                        },
                    ),
                );
                let jukebox_hit = super::super::Hit {
                    pos: [6, 64, 4],
                    point: [6.5, 65.0, 4.5],
                    ..hit
                };
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                assert!(game.use_hand(Some(&jukebox_hit)).unwrap());
                assert!(matches!(
                    game.world.block_entities.get(&(6, 64, 4)),
                    Some(horizon_client::containers::BlockEntity::Jukebox(j)) if j.record().is_some()
                ));
                *user.empty_answer.lock().unwrap() = None;
                let calls = user.clicks.lock().unwrap().len();
                assert!(game.use_hand(Some(&jukebox_hit)).unwrap());
                assert_eq!(user.clicks.lock().unwrap().len(), calls + 1);
                assert!(matches!(
                    game.world.block_entities.get(&(6, 64, 4)),
                    Some(horizon_client::containers::BlockEntity::Jukebox(j)) if j.record().is_none()
                ));
                assert!(
                    game.dropped
                        .iter()
                        .any(|item| &*item.stack.item == "minecraft:music_disc_13")
                );
                assert_eq!(user.item_uses.load(SeqCst), 4);

                // ServerPlayerGameMode uses the state it read before useItemOn,
                // even when that method replaces the block before asking for empty use.
                let floor_lever = game.with_props(lever, &[("face", "floor")]).unwrap();
                let powered = game
                    .with_props(floor_lever, &[("powered", "true")])
                    .unwrap();
                let cobblestone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cobblestone")
                    .unwrap();
                let replaced_hit = super::super::Hit {
                    pos: [8, 64, 4],
                    point: [8.5, 65.0, 4.5],
                    ..hit
                };
                game.world.set(8, 63, 4, stone).unwrap();
                game.world.set(8, 64, 4, floor_lever).unwrap();
                *user.replace_with.lock().unwrap() = Some(cobblestone);
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                assert_eq!(game.world.block(8, 64, 4), powered);
                assert_eq!(user.item_uses.load(SeqCst), 4);

                assert_eq!(
                    user.empty_clicks
                        .lock()
                        .unwrap()
                        .last()
                        .map(|(state, _)| *state),
                    Some(floor_lever)
                );

                // Every supplied empty result replaces the lever; TRY is not retried.
                let mut item_uses = 4;
                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    game.world.set(8, 64, 4, floor_lever).unwrap();
                    *user.empty_answer.lock().unwrap() = Some(answer);
                    let calls = user.empty_clicks.lock().unwrap().len();
                    let before = game.swings_started;
                    assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                    assert_eq!(
                        game.swings_started - before,
                        u32::from(answer != Answer::Consume)
                    );
                    assert_eq!(game.world.block(8, 64, 4), floor_lever);
                    assert_eq!(user.empty_clicks.lock().unwrap().len(), calls + 1);
                    if matches!(
                        answer,
                        Answer::Pass | Answer::Fail | Answer::TryWithEmptyHand
                    ) {
                        item_uses += 1;
                    }
                    assert_eq!(user.item_uses.load(SeqCst), item_uses);
                }

                // The template's TRY reaches the part too; the off hand never does.
                *user.answer.lock().unwrap() = None;
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                assert_eq!(game.world.block(8, 64, 4), floor_lever);
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                game.hand = Hand::Off;
                game.offhand = Some(horizon_client::items::ItemStack::new("minecraft:stick", 1));
                let calls = user.empty_clicks.lock().unwrap().len();
                assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                assert_eq!(user.empty_clicks.lock().unwrap().len(), calls);
                item_uses += 1;
                assert_eq!(user.item_uses.load(SeqCst), item_uses);
                game.hand = Hand::Main;
                game.offhand = None;

                // Ore's item-method PASS must not be mistaken for TRY.
                let ore = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:redstone_ore")
                    .unwrap();
                game.world.set(11, 64, 4, ore).unwrap();
                game.inventory[game.selected] =
                    Some(horizon_client::items::ItemStack::new("minecraft:stone", 1));
                let ore_hit = super::super::Hit {
                    pos: [11, 64, 4],
                    point: [11.5, 65.0, 4.5],
                    ..hit
                };
                *user.answer.lock().unwrap() = None;
                let calls = user.empty_clicks.lock().unwrap().len();
                assert!(game.use_hand(Some(&ore_hit)).unwrap());
                assert_eq!(user.empty_clicks.lock().unwrap().len(), calls);
                item_uses += 1;
                assert_eq!(user.item_uses.load(SeqCst), item_uses);
                assert_eq!(
                    game.world.block(11, 64, 4),
                    game.with_props(ore, &[("lit", "true")]).unwrap()
                );
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                game.inventory[game.selected] =
                    Some(horizon_client::items::ItemStack::new("minecraft:stick", 1));

                // The hook precedes the pot's wobble and the bookshelf's removal.
                let pot = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:decorated_pot")
                    .unwrap();
                game.world.set(10, 64, 4, pot).unwrap();
                let pot_hit = super::super::Hit {
                    pos: [10, 64, 4],
                    point: [10.5, 65.0, 4.5],
                    ..hit
                };
                assert!(game.use_hand(Some(&pot_hit)).unwrap());
                assert!(!game.be_anims.pots.contains_key(&pot_hit.pos));
                *user.empty_answer.lock().unwrap() = None;
                assert!(game.use_hand(Some(&pot_hit)).unwrap());
                assert!(game.be_anims.pots.contains_key(&pot_hit.pos));
                let shelf = game
                    .with_props(
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:chiseled_bookshelf")
                            .unwrap(),
                        &[("facing", "north"), ("slot_0_occupied", "true")],
                    )
                    .unwrap();
                game.world.set(9, 64, 4, shelf).unwrap();
                let mut books = Box::<horizon_client::containers::ChiseledBookshelf>::default();
                books.items[0] = Some(horizon_client::items::ItemStack::new("minecraft:book", 1));
                game.world.block_entities.insert(
                    (9, 64, 4),
                    horizon_client::containers::BlockEntity::ChiseledBookshelf(books),
                );
                let shelf_hit = super::super::Hit {
                    pos: [9, 64, 4],
                    face: [0, 0, -1],
                    point: [9.75, 64.75, 4.0],
                    inside: false,
                };
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                assert!(game.use_hand(Some(&shelf_hit)).unwrap());
                assert!(matches!(
                    game.world.block_entities.get(&(9, 64, 4)),
                    Some(horizon_client::containers::BlockEntity::ChiseledBookshelf(b)) if b.items[0].is_some()
                ));
                *user.empty_answer.lock().unwrap() = None;
                assert!(game.use_hand(Some(&shelf_hit)).unwrap());
                assert!(matches!(
                    game.world.block_entities.get(&(9, 64, 4)),
                    Some(horizon_client::containers::BlockEntity::ChiseledBookshelf(b)) if b.items[0].is_none()
                ));

                // The real guest packet reaches empty use with the guest's UUID.
                game.world.set(0, 64, 4, lever).unwrap();
                {
                    let value = Some(horizon_client::items::ItemStack::new("minecraft:stick", 1));
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.inventory[0] = value);
                }
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                game.lan_guest_packet(
                    12,
                    play::Serverbound::UseItemOn(play::UseItemOn {
                        hand: 0,
                        pos: hit.pos,
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence: 1,
                    }),
                );
                assert_eq!(user.clicks.lock().unwrap().last(), Some(&(12, Hand::Main)));
                assert_eq!(user.empty_clicks.lock().unwrap().last(), Some(&(lever, 12)));
                assert_eq!(user.item_uses.load(SeqCst), item_uses);
                assert_eq!(game.world.block(0, 64, 4), lever);

                // A delegating Part's food write precedes vanilla's canEat check.
                let cake = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cake")
                    .unwrap();
                let bitten = game.with_props(cake, &[("bites", "1")]).unwrap();
                *user.empty_answer.lock().unwrap() = None;
                for (food_write, expected, food, item_calls) in [
                    (None, bitten, 12, item_uses),
                    (Some(20), cake, 20, item_uses + 1),
                ] {
                    game.world.set(0, 64, 4, cake).unwrap();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.game_mode = 0;
                        guest.food = 10;
                    });
                    *user.empty_food.lock().unwrap() = food_write;
                    game.lan_guest_packet(
                        12,
                        play::Serverbound::UseItemOn(play::UseItemOn {
                            hand: 0,
                            pos: hit.pos,
                            face: 1,
                            cursor: [0.5, 1.0, 0.5],
                            inside: false,
                            world_border: false,
                            sequence: 2,
                        }),
                    );
                    assert_eq!(game.world.block(0, 64, 4), expected);
                    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].food, food);
                    assert_eq!(user.item_uses.load(SeqCst), item_calls);
                }
                {
                    let value = 1;
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.game_mode = value);
                }
                game.world.set(0, 64, 4, stone).unwrap();
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [0, 64, 0],
                        include_data: true,
                    }),
                );
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    &*guest.inventory[guest.selected].as_ref().unwrap().item,
                    "minecraft:diamond"
                );
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [0, 64, 0],
                        include_data: false,
                    }),
                );
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    &*guest.inventory[guest.selected].as_ref().unwrap().item,
                    "minecraft:stone"
                );
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [1, 64, 0],
                        include_data: true,
                    }),
                );
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    &*guest.inventory[guest.selected].as_ref().unwrap().item,
                    "minecraft:wheat_seeds"
                );
                assert_eq!(*seen.lock().unwrap(), [true, false]);

                // addBlockDataToItem runs on the part's item: a chest picked with
                // its data gives the part's diamond carrying the chest's contents.
                game.world.set(3, 64, 0, chest).unwrap();
                let mut slots = vec![None; 27];
                slots[0] = Some(horizon_client::items::ItemStack::new("minecraft:apple", 2));
                game.world.block_entities.insert(
                    (3, 64, 0),
                    horizon_client::containers::BlockEntity::Chest(slots),
                );
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [3, 64, 0],
                        include_data: true,
                    }),
                );
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                let picked = guest.inventory[guest.selected].as_ref().unwrap();
                assert_eq!(&*picked.item, "minecraft:diamond");
                let Some(horizon_core::nbt::Tag::Compound(components)) =
                    horizon_client::item_data::encode_components(picked)
                else {
                    panic!("the chest's data is on the diamond");
                };
                assert!(components.contains_key(&"minecraft:container".into()));
                assert_eq!(*seen.lock().unwrap(), [true, false, true]);

                let hive = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:beehive")
                    .unwrap();
                let hive = game.with_props(hive, &[("honey_level", "1")]).unwrap();
                game.world.set(2, 64, 0, hive).unwrap();
                game.world.block_entities.insert(
                    (2, 64, 0),
                    horizon_client::containers::BlockEntity::Beehive(Box::default()),
                );
                game.creative = true;
                game.break_block([2, 64, 0]).unwrap();
                assert!(
                    game.dropped
                        .iter()
                        .any(|item| &*item.stack.item == "minecraft:beehive")
                );
                assert_eq!(*seen.lock().unwrap(), [true, false, true]);
            })
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn block_part_use_item_on_consumes_once() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;
        use mod_api::InteractionResult as Answer;
        use mod_api::{self, BlockPart, Hooks, Registrar, ServerHost, UseItemOnEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering::SeqCst},
        };

        #[derive(Clone)]
        struct BlockUse(Arc<AtomicUsize>);
        #[mod_api::horizon_part]
        impl BlockPart for BlockUse {
            fn use_item_on(
                &self,
                _: &mut dyn ServerHost,
                _: &mut UseItemOnEvent<'_>,
            ) -> Option<Answer> {
                self.0.fetch_add(1, SeqCst);
                Some(Answer::Success)
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for BlockUse {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block_key = r.block_key("minecraft:lever");
                r.block_part(block_key, self.clone());
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "block_use",
            Box::new(BlockUse(calls.clone())),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let lever = game
                    .with_props(
                        registry.default_state("minecraft:lever").unwrap(),
                        &[("face", "floor")],
                    )
                    .unwrap();
                game.world.set(0, 63, 4, stone).unwrap();
                game.world.set(0, 64, 4, lever).unwrap();
                let hit = super::super::Hit {
                    pos: [0, 64, 4],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 4.5],
                    inside: false,
                };
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(game.world.block(0, 64, 4), lever);
                assert_eq!(calls.load(SeqCst), 1);
            });
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn replacing_empty_note_block_use_does_not_award_vanilla_stats() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;
        use mod_api::InteractionResult as Answer;
        use mod_api::{self, BlockPart, Hooks, Registrar, ServerHost, UseWithoutItemEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering::SeqCst},
        };

        #[derive(Clone)]
        struct Replacement(Arc<AtomicBool>);
        #[mod_api::horizon_part]
        impl BlockPart for Replacement {
            fn use_without_item(
                &self,
                _: &mut dyn ServerHost,
                _: &mut UseWithoutItemEvent<'_>,
            ) -> Option<Answer> {
                self.0.load(SeqCst).then_some(Answer::Success)
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for Replacement {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block_key = r.block_key("minecraft:note_block");
                r.block_part(block_key, self.clone());
            }
        }
        let replace = Arc::new(AtomicBool::new(true));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "empty_note_replacement",
            Box::new(Replacement(replace.clone())),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let note = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:note_block")
                    .unwrap();
                game.world.set(0, 64, 4, note).unwrap();
                let hit = super::super::Hit {
                    pos: [0, 64, 4],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 4.5],
                    inside: false,
                };
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(game.world.block(0, 64, 4), note);
                assert_eq!(game.stats.get("custom", "minecraft:tune_noteblock"), 0);

                // Delegating still runs the actual method and awards its stat.
                replace.store(false, SeqCst);
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_ne!(game.world.block(0, 64, 4), note);
                assert_eq!(game.stats.get("custom", "minecraft:tune_noteblock"), 1);
            });
        });
    }

    #[test]
    #[ignore = "requires prepared game resources"]
    fn fake_body_does_not_count_toward_sleep() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.lan_acting_fake = false;
            let real = game.sleep_counts();
            assert_eq!(real.0, 1);
            game.lan_acting_fake = true;
            assert_eq!(game.sleep_counts(), (0, 0, 0));
            game.lan_acting_fake = false;
            assert_eq!(game.sleep_counts(), real);
        });
    }

    /// `copyRespawnPosition` copies an Overworld forced point, not none; a
    /// point with a block is left to its block check.
    #[test]
    fn respawn_copies_the_forced_point() {
        assert_eq!(
            copied_respawn_point(false, Some([12.5, 64.0, -3.5])),
            Some(Some([12, 64, -4]))
        );
        assert_eq!(copied_respawn_point(false, None), Some(None));
        assert_eq!(copied_respawn_point(true, Some([0.5, 64.0, 0.5])), None);
    }

    /// #4124: a STOP is measured from the START that named its block (`destroyPos`) — an abort, or
    /// a STOP from out of reach, ends the dig the guest's button held but leaves that clock — and
    /// breaks the block at 70 % of the dig, or the server finishes it alone from the same START.
    /// A guest that reaches a block only at a jump's apex keeps its dig this way, and no other
    /// block or new body inherits the clock.
    #[test]
    fn a_stop_after_a_fall_is_measured_from_the_starts_clock() {
        use super::{GuestPlay, Mining, StopDig, stop_dig};
        let pos = [12, 70, -5];
        let mut play = GuestPlay {
            digging: Some((pos, [0, 1, 0])),
            mining: Some(Mining {
                pos,
                progress: 0.5,
                per_tick: 0.0,
                ticks: 8,
            }),
            destroy_pos: Some(pos),
            dig_start: 1000,
            ..GuestPlay::default()
        };
        // The landing: the dig the button held is gone, the clock the START set is not.
        play.dig_ended();
        assert!(play.digging.is_none() && play.mining.is_none());
        // 13 ticks of 0.05: 65 % of the dig, so the server finishes it from tick 1000 itself.
        assert_eq!(
            stop_dig(&play, pos, false, 1012, 0.05),
            StopDig::Delay(1000)
        );
        // 16 ticks: 80 %, the STOP breaks the block outright.
        assert_eq!(stop_dig(&play, pos, false, 1015, 0.05), StopDig::Break);
        // A block the START did not name never gets the clock, and a broken one only its crack.
        assert_eq!(
            stop_dig(&play, [12, 71, -5], false, 1015, 0.05),
            StopDig::Other
        );
        assert_eq!(stop_dig(&play, pos, true, 1015, 0.05), StopDig::Gone);
        // A respawn's new ServerPlayerGameMode starts from no clock at all.
        play.destroy_pos = None;
        assert_eq!(stop_dig(&play, pos, false, 1015, 0.05), StopDig::Other);
    }

    /// #3684: a player shrunk to `minecraft:scale` 0.1 who stands 0.05 from a
    /// wall has eyes 0.048 wide, which don't reach into it: they don't
    /// suffocate there, as a full-sized player 0.3 from the wall doesn't.
    #[test]
    fn a_small_player_checks_only_the_blocks_at_their_small_eyes() {
        let at = [205.95, 100.0, 530.5];
        assert_eq!(
            eye_columns(at, Pose::Standing.dimensions(0.1)),
            [205..=205, 530..=530]
        );
        assert_eq!(
            eye_columns(at, Pose::Standing.dimensions(1.0)),
            [205..=206, 530..=530]
        );
        assert_eq!(
            eye_columns([205.7, 100.0, 530.5], Pose::Standing.dimensions(1.0)),
            [205..=205, 530..=530]
        );
    }
}

/// A `ServerboundSelectBundleItemPacket` slot as the hub's `select_bundle_item` reads it.
/// With a crafting table or a station open (`own` slots of its own first), its
/// inventory part is the inventory menu's main inventory and hotbar from slot 9, in
/// the same order; its own slots hold no bundle the hub can reach.
fn bundle_inventory_slot(slot: usize, own: Option<usize>) -> Option<usize> {
    match own {
        Some(own) => slot.checked_sub(own).filter(|s| *s < 36).map(|s| s + 9),
        None => Some(slot),
    }
}

/// Where the server aims a guest's attack or interact on a mob. The packet names only the
/// entity, so the server picked the mob's centre; a slab, trapdoor, stair or fence between the
/// eyes and that centre then hid a mob the guest's own crosshair hit through the gap (#5772).
/// This returns the centre when it is visible, else the visible point of the box nearest to it (the one nearer the eyes when two are equally near)
/// (`occluded` tells whether a block outline hides a point from the eyes); the centre again
/// when the whole box is hidden.
fn visible_aim_point(
    eye: [f64; 3],
    lo: [f64; 3],
    hi: [f64; 3],
    occluded: impl Fn([f64; 3]) -> bool,
) -> [f64; 3] {
    let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
    if !occluded(centre) {
        return centre;
    }
    let margin = [0, 1, 2].map(|k| ((hi[k] - lo[k]) / 2.0).min(0.02));
    let across = |k: usize, steps: usize| {
        (0..=steps)
            .map(|i| {
                lo[k] + margin[k] + (hi[k] - lo[k] - 2.0 * margin[k]) * i as f64 / steps as f64
            })
            .collect::<Vec<f64>>()
    };
    let (xs, ys, zs) = (across(0, 2), across(1, 4), across(2, 2));
    let mut candidates = Vec::with_capacity(xs.len() * ys.len() * zs.len());
    for &x in &xs {
        for &y in &ys {
            for &z in &zs {
                candidates.push([x, y, z]);
            }
        }
    }
    let nearness = |p: &[f64; 3]| (0..3).map(|k| (p[k] - centre[k]).powi(2)).sum::<f64>();
    let from_eye = |p: &[f64; 3]| (0..3).map(|k| (p[k] - eye[k]).powi(2)).sum::<f64>();
    candidates.sort_by(|a, b| {
        nearness(a)
            .total_cmp(&nearness(b))
            .then(from_eye(a).total_cmp(&from_eye(b)))
            .then(a[1].total_cmp(&b[1]))
            .then(a[0].total_cmp(&b[0]))
            .then(a[2].total_cmp(&b[2]))
    });
    candidates
        .into_iter()
        .find(|p| !occluded(*p))
        .unwrap_or(centre)
}

/// The first target `(id, removed, bounds)` whose box meets the swept box,
/// skipping removed ones and the vehicle the player rides.
fn first_spin_target(
    swept: Aabb,
    riding: Option<u32>,
    mut targets: impl Iterator<Item = (u32, bool, Aabb)>,
) -> Option<usize> {
    targets.position(|(id, removed, b)| !removed && Some(id) != riding && b.intersects(swept))
}

#[cfg(test)]
mod hotbar_tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use horizon_protocol::packets::play;
    use mod_api::{
        Flow, Hooks, ModEntry, PlayerSwitchHotbarSlotPostEvent, PlayerSwitchHotbarSlotPreEvent,
        ServerHost,
    };
    use std::sync::{Arc, Mutex};

    /// What the hooks heard, as (pre or post, uuid, entity id, old, new).
    type Heard = Arc<Mutex<Vec<(&'static str, u128, i32, i32, i32)>>>;

    /// Hears both hooks and refuses a switch to slot 8.
    struct Hotbar(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Hotbar {
        fn player_switch_hotbar_slot_pre(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerSwitchHotbarSlotPreEvent,
        ) -> Flow {
            let row = ("pre", e.player, e.player_entity_id, e.old_slot, e.new_slot);
            self.0.lock().unwrap().push(row);
            if e.new_slot == 8 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn player_switch_hotbar_slot_post(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerSwitchHotbarSlotPostEvent,
        ) {
            let row = ("post", e.player, e.player_entity_id, e.old_slot, e.new_slot);
            self.0.lock().unwrap().push(row);
        }
    }

    /// `PlayerSwitchHotbarSlotEvent`: pre, then post once the slot changed, both naming the
    /// player by UUID and entity id; the same slot again fires neither, and a cancelled pre
    /// keeps the old slot with no post.
    #[test]
    #[ignore = "requires prepared game resources"]
    fn a_hotbar_switch_fires_pre_then_post_and_pre_can_keep_the_slot() {
        let heard = Heard::default();
        let mods = vec![ModEntry::new("hotbar", Box::new(Hotbar(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.lan.as_mut().unwrap().hub.guests.push(guest(1));
                let switch = |game: &mut super::Game<'_>, slot| {
                    game.lan_guest_packet(
                        1,
                        play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot }),
                    );
                    game.lan.as_ref().unwrap().hub.guests[0].selected
                };
                assert_eq!(switch(game, 3), 3);
                assert_eq!(switch(game, 3), 3);
                assert_eq!(switch(game, 8), 3);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("pre", 123, 20, 0, 3),
                        ("post", 123, 20, 0, 3),
                        ("pre", 123, 20, 3, 8),
                    ]
                );
            });
        });
    }
}

#[cfg(test)]
mod respawn_position_tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use horizon_client::chunk::Chunk;
    use mod_api::{Hooks, LevelDimension, ModEntry, PlayerRespawnPositionEvent, ServerHost};
    use std::sync::{Arc, Mutex};

    /// What the hook heard: the player's UUID, the original level and point,
    /// and whether the block was missing.
    type Heard = Arc<Mutex<Vec<(u128, LevelDimension, [f64; 3], bool)>>>;

    /// Moves every respawn ten blocks east, turns the player north, and
    /// keeps no respawn point for next time.
    struct East(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for East {
        fn player_respawn_position(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerRespawnPositionEvent,
        ) {
            let row = (
                e.player.uuid(),
                e.original_level,
                e.original_position,
                e.missing_respawn_block,
            );
            self.0.lock().unwrap().push(row);
            e.position[0] += 10.0;
            e.yaw = 180.0;
            e.copy_original_spawn_position = false;
        }
    }

    /// `PlayerRespawnPositionEvent` on a server: a guest whose forced point
    /// needs no block respawns there, the mod hears that point with the
    /// player's handle, and its new point, facing and copy choice are what
    /// the player gets.
    #[test]
    #[ignore = "requires prepared game resources"]
    fn a_mod_moves_a_respawn_and_drops_the_point() {
        let heard = Heard::default();
        let mods = vec![ModEntry::new("east", Box::new(East(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let mut player = guest(1);
                player.dead = true;
                player.spawn = Some([5.5, 64.0, 5.5]);
                game.lan.as_mut().unwrap().hub.guests.push(player);
                game.guest_respawn(1);
                let heard = heard.lock().unwrap();
                let [(uuid, level, original, missing)] = heard[..] else {
                    panic!("heard {heard:?}");
                };
                assert_eq!(
                    (uuid, level, missing),
                    (123, LevelDimension::Overworld, false)
                );
                let g = &game.lan.as_ref().unwrap().hub.guests[0];
                assert!(!g.dead);
                assert_eq!(g.position, [original[0] + 10.0, original[1], original[2]]);
                assert_eq!(g.yaw, 180.0);
                assert_eq!((g.spawn, &g.respawn_elsewhere), (None, &None));
            });
        });
    }

    /// Hears each respawn's facing and changes nothing.
    struct Listen(Arc<Mutex<Vec<([f64; 3], f32, f32)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listen {
        fn player_respawn_position(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerRespawnPositionEvent,
        ) {
            self.0.lock().unwrap().push((*e.position, e.yaw, e.pitch));
        }
    }

    /// A mod that only listens changes nothing: the guest respawns at its
    /// point facing the respawn direction, not the way it died, and keeps
    /// the point.
    #[test]
    #[ignore = "requires prepared game resources"]
    fn a_listening_mod_leaves_the_respawn_as_it_was() {
        let heard = Arc::default();
        let mods = vec![ModEntry::new(
            "listen",
            Box::new(Listen(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let mut player = guest(1);
                player.dead = true;
                (player.yaw, player.pitch) = (90.0, 30.0);
                player.spawn = Some([5.5, 64.0, 5.5]);
                game.lan.as_mut().unwrap().hub.guests.push(player);
                game.guest_respawn(1);
                let heard = heard.lock().unwrap();
                let [(found, yaw, pitch)] = heard[..] else {
                    panic!("heard {heard:?}");
                };
                assert_eq!((yaw, pitch), (0.0, 0.0), "told the respawn facing");
                let g = &game.lan.as_ref().unwrap().hub.guests[0];
                assert!(!g.dead);
                assert_eq!(g.position, found);
                assert_eq!((g.yaw, g.pitch), (0.0, 0.0));
                assert_eq!(g.spawn, Some([5.5, 64.0, 5.5]));
            });
        });
    }
}

#[cfg(test)]
mod pickup_xp_tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use horizon_client::entities::ExperienceOrb;
    use horizon_core::geometry::Vec3;

    /// Listens to `pickup_xp`, canceling it when told to.
    struct Listen(bool);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listen {
        fn pickup_xp(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::PickupXpEvent<'_>,
        ) -> mod_api::Flow {
            if self.0 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// A guest takes the orb it stands in: the orb's id left in the level and the
    /// guest's total experience after.
    fn take_orb() -> (bool, i32) {
        let mut out = (false, 0);
        with_game(|game| {
            open_to_lan(game);
            let mut g = guest(1);
            g.loaded = true;
            g.position = [0.5, 70.0, 0.5];
            game.lan.as_mut().unwrap().hub.guests.push(g);
            let p = Vec3::new(0.5, 70.0, 0.5);
            game.orbs.push(ExperienceOrb {
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                id: 40,
                position: p,
                previous: p,
                velocity: Vec3::ZERO,
                value: 7,
                age: 0,
                on_ground: true,
                count: 1,
                ticks: 0,
                air: 300,
                health: 5,
                no_gravity: true,
                following: None,
                fall_distance: 0.0,
            });
            game.guest_orbs(0);
            let left = game.orbs.iter().any(|o| o.id == 40);
            out = (left, game.lan.as_ref().unwrap().hub.guests[0].xp_total);
        });
        out
    }

    /// `PlayerXpEvent.PickupXp` through the shared `listened` probe: a pickup goes the
    /// same with no listener and with one that changes nothing, and a cancel keeps the orb.
    #[test]
    #[ignore = "needs prepared resources"]
    fn pickup_xp_is_the_same_with_and_without_a_listener() {
        let none = take_orb();
        assert_eq!(none, (false, 7));
        let mods = vec![mod_api::ModEntry::new("listen", Box::new(Listen(false)))].into();
        mod_platform::testing::with_mods(mods, || assert_eq!(take_orb(), none));
        let mods = vec![mod_api::ModEntry::new("cancel", Box::new(Listen(true)))].into();
        mod_platform::testing::with_mods(mods, || assert_eq!(take_orb(), (true, 0)));
    }
}

#[cfg(test)]
mod respawn_world_point_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::packets::{configuration::ClientInformation, play::Clientbound};
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    /// PlayerList.respawn sends world respawn data, independently of the personal landing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn guest_respawn_keeps_world_information_distinct_from_personal_landing() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "horizon-respawn-point-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        game.world_root = scratch.clone();
        let world = [0, 100, 0];
        let personal = [5.5, 101.0, 5.5];
        game.level.world_spawn = Some(world);
        game.spawn = Some(Vec3::new(0.5, 100.0, 0.5));
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "respawn-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 330,
                name: "RespawnGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [0.5, 100.0, 0.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.spawn = Some(personal);
        guest.health = 0.0;
        guest.dead = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        game.guest_respawn(2);
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.uuid, 330);
        assert_eq!(guest.position, personal);
        assert_eq!(guest.health, 20.0);
        assert!(!guest.dead);
        let mut packets = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => packets.push(body),
                Out::Packets(bodies) => packets.extend(bodies),
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            }
        }
        let mut sequence = Vec::new();
        let mut points = Vec::new();
        for body in packets {
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            match Clientbound::decode(id, r.rest()).unwrap() {
                Clientbound::Respawn(_) => sequence.push(84),
                Clientbound::SetDefaultSpawnPosition(p) => {
                    sequence.push(99);
                    points.push((
                        p.pos.dimension,
                        p.pos.pos,
                        p.yaw.to_bits(),
                        p.pitch.to_bits(),
                    ));
                }
                _ => {}
            }
        }
        assert_eq!(sequence, [84, 99, 99]);
        assert_eq!(points, vec![("minecraft:overworld".into(), world, 0, 0); 2]);
        drop(game);
        std::fs::remove_dir_all(scratch).unwrap();
    }
}

#[cfg(test)]
mod swing_echo_tests {
    use super::ItemStack;
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::chunk::Chunk;
    use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_core::position::ChunkPos;
    use horizon_protocol::packets::{
        configuration::ClientInformation,
        play::{self, Clientbound, Serverbound},
    };
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    /// handleUseItemOn swings the player's arm, but sends the swing to the
    /// player's own client only when the client did not predict it: a block
    /// placed (SUCCESS) is the client's own swing, a bed's use
    /// (SUCCESS_SERVER) is not (#9887).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn only_a_server_only_use_swings_back_to_the_player() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("horizon-swing-echo-unit-{}", std::process::id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("setblock 6 200 4 red_bed[facing=north,part=foot]"));
        assert!(game.run_command("setblock 6 200 3 red_bed[facing=north,part=head]"));
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "swing-echo-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 331,
                name: "SwingGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [4.5, 200.0, 4.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 16));
        guest.selected = 0;
        let mut host = game.lan.take().unwrap();
        host.hub.guests.push(guest);
        let id = host.hub.guests[0].entity_id;
        let swings_back = || {
            let mut n = 0;
            for out in rx.try_iter() {
                let bodies = match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => panic!("unexpected disconnect"),
                };
                for body in bodies {
                    let mut r = horizon_protocol::buf::Reader::new(&body);
                    let packet_id = r.varint().unwrap();
                    if let Ok(Clientbound::SwingAnimation(p)) =
                        Clientbound::decode(packet_id, r.rest())
                        && p.id == id
                    {
                        n += 1;
                    }
                }
            }
            n
        };
        let use_on = |pos: [i32; 3], cursor: [f32; 3], sequence: i32| {
            Serverbound::UseItemOn(play::UseItemOn {
                hand: 0,
                pos,
                face: 1,
                cursor,
                inside: false,
                world_border: false,
                sequence,
            })
        };
        game.lan = Some(host);
        // BlockItem.place: SUCCESS, the client's own swing.
        game.lan_guest_packet(2, use_on([4, 199, 6], [0.5, 1.0, 0.5], 1));
        assert_eq!(game.world.block(4, 200, 6).0, stone, "the stone is placed");
        assert_eq!(swings_back(), 0, "a placement is not swung back");
        // The swing ends before the next use can start one.
        for _ in 0..8 {
            game.lan_swings();
        }
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.selected = 1);
        // BedBlock.useWithoutItem: SUCCESS_SERVER, the server's swing only.
        game.lan_guest_packet(2, use_on([6, 200, 4], [0.5, 0.5625, 0.5], 2));
        assert_eq!(swings_back(), 1, "a bed's use is swung back");
    }
}

/// `Entity.touchingUnloadedChunk`: the box at `at` inflated by one block
/// reaches a chunk that is not loaded (`Level.hasChunksAt` over the floor of
/// its minimum and the ceiling of its maximum, as section coordinates).
fn touching_unloaded_chunk(
    at: [f64; 3],
    body: Dimensions,
    loaded: impl Fn(i32, i32) -> bool,
) -> bool {
    let half = f64::from(body.width) / 2.0 + 1.0;
    let section = |v: f64| (v as i32) >> 4;
    let (x0, x1) = (
        section((at[0] - half).floor()),
        section((at[0] + half).ceil()),
    );
    let (z0, z1) = (
        section((at[2] - half).floor()),
        section((at[2] + half).ceil()),
    );
    !(x0..=x1).all(|x| (z0..=z1).all(|z| loaded(x, z)))
}

#[cfg(test)]
mod guest_close_listener_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::{
        chunk::Chunk,
        containers::BlockEntity,
        lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
        sculk::Info,
    };
    use horizon_core::position::ChunkPos;
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    fn selected(game: &Game<'_>) -> Option<Info> {
        let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&(9, 100, 4)) else {
            panic!("actual command-created sensor must exist");
        };
        sensor.vibration.selector.current.map(|(info, _)| info)
    }
    fn reset_listener(game: &mut Game<'_>) {
        assert!(game.run_command("setblock 9 100 4 air strict"));
        assert!(game.run_command("setblock 9 100 4 sculk_sensor strict"));
        game.sculk_prepare();
        game.sculk_sync_index();
        assert!(selected(game).is_none());
    }
    fn open(game: &mut Game<'_>, conn: u32, sequence: i32) -> i32 {
        game.lan_guest_packet(
            conn,
            S::UseItemOn(play::UseItemOn {
                hand: 0,
                pos: [6, 100, 4],
                face: 1,
                cursor: [0.5, 1.0, 0.5],
                inside: false,
                world_border: false,
                sequence,
            }),
        );
        let guest = game.lan.as_mut().unwrap().hub.guest(conn).unwrap();
        assert!(guest.ack_up_to.get() >= sequence);
        assert_eq!(guest.health, 20.0);
        assert_eq!(guest.position, [4.5, 100.0, 4.5]);
        let menu = guest
            .menu
            .as_ref()
            .expect("actual packet must open the chest menu");
        assert_eq!(menu.pos, Some((6, 100, 4)));
        assert_eq!(menu.size, 27);
        menu.window
    }
    fn close(game: &mut Game<'_>, conn: u32, window: i32) {
        game.lan_guest_packet(
            conn,
            S::ContainerClose(play::ContainerClose { container: window }),
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
    fn ordinary_guest_final_close_posts_to_actual_listener() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0 15 15"));
        assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
        assert!(game.run_command("setblock 6 100 4 chest"));
        assert!(game.run_command("setblock 9 100 4 sculk_sensor strict"));
        game.sculk_prepare();
        game.sculk_sync_index();
        assert!(selected(&game).is_none());
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "guest-close-test".into(), 4)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 4,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();

        let mut receivers = Vec::new();
        for conn in 1..=3 {
            let (tx, rx) = mpsc::channel();
            receivers.push(rx);
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: u128::from(conn) + 11200,
                    name: format!("CloseGuest{conn}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32 + 100,
                [4.5, 100.0, 4.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.game_mode = if conn == 3 { 3 } else { 0 };
            guest.on_ground = true;
            assert_eq!(guest.health, 20.0);
            game.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let clock = game.world.game_time;
        let mut missing = Vec::new();
        let window = open(&mut game, 1, 1);
        assert_eq!(game.lid_openers.get(&(6, 100, 4)), Some(&1));
        assert_eq!(selected(&game).unwrap().event, "minecraft:container_open");
        reset_listener(&mut game);
        close(&mut game, 1, window + 20);
        assert_eq!(
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .as_ref()
                .unwrap()
                .window,
            window
        );
        assert_eq!(game.lid_openers.get(&(6, 100, 4)), Some(&1));
        assert!(selected(&game).is_none());
        close(&mut game, 1, window);
        assert!(
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .is_none()
        );
        assert!(!game.lid_openers.contains_key(&(6, 100, 4)));
        match selected(&game) {
            Some(info) => {
                assert_eq!(info.event, "minecraft:container_close");
                assert_eq!(info.pos, [6.5, 100.5, 4.5]);
            }
            None => missing.push("single final close"),
        }
        reset_listener(&mut game);
        let first = open(&mut game, 1, 2);
        assert_eq!(selected(&game).unwrap().event, "minecraft:container_open");
        reset_listener(&mut game);
        let last = open(&mut game, 2, 1);
        assert_eq!(game.lid_openers.get(&(6, 100, 4)), Some(&2));
        assert!(selected(&game).is_none());
        close(&mut game, 1, first);
        assert!(
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guest(2)
                .unwrap()
                .menu
                .is_some()
        );
        assert_eq!(game.lid_openers.get(&(6, 100, 4)), Some(&1));
        assert!(selected(&game).is_none());
        close(&mut game, 2, last);
        assert!(!game.lid_openers.contains_key(&(6, 100, 4)));
        match selected(&game) {
            Some(info) => {
                assert_eq!(info.event, "minecraft:container_close");
                assert_eq!(info.pos, [6.5, 100.5, 4.5]);
            }
            None => missing.push("shared final close"),
        }
        reset_listener(&mut game);
        let spectator = open(&mut game, 3, 1);
        assert!(!game.lid_openers.contains_key(&(6, 100, 4)));
        assert!(selected(&game).is_none());
        close(&mut game, 3, spectator);
        assert!(
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guest(3)
                .unwrap()
                .menu
                .is_none()
        );
        assert!(selected(&game).is_none());
        assert!(game.run_command("setblock 6 100 4 air strict"));
        assert!(
            game.run_command("summon oak_chest_boat 6.5 100 4.5 {UUID:[I;112,0,0,9],NoGravity:1b}")
        );
        let boat = game.boats.last().unwrap().id;
        // Chest boats open for a sneaking guest; a plain empty-hand click boards.
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(1, |g| g.sneaking = true);
        game.lan_guest_packet(
            1,
            S::Interact(play::Interact {
                id: boat as i32,
                hand: 0,
                location: [0.0; 3],
                secondary: true,
            }),
        );
        let vehicle_window = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(1)
            .unwrap()
            .menu
            .as_ref()
            .expect("actual vehicle interaction must open menu")
            .window;
        reset_listener(&mut game);
        close(&mut game, 1, vehicle_window);
        assert!(
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .is_none()
        );
        assert_eq!(selected(&game).unwrap().event, "minecraft:container_close");
        assert_eq!(selected(&game).unwrap().pos, [6.5, 100.0, 4.5]);
        for guest in &game.lan.as_mut().unwrap().hub.guests {
            assert_eq!(guest.uuid, u128::from(guest.conn) + 11200);
            assert_eq!(guest.health, 20.0);
            assert_eq!(guest.position, [4.5, 100.0, 4.5]);
        }
        assert_eq!(game.world.game_time, clock);
        assert!(
            missing.is_empty(),
            "actual ordinary guest final close must be heard: {missing:?}"
        );
        drop(receivers);
    }
}

#[cfg(test)]
mod item_on_block_cooldown_tests {
    use super::ItemStack;
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::chunk::Chunk;
    use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_core::position::ChunkPos;
    use horizon_protocol::packets::{
        configuration::ClientInformation,
        play::{self, Clientbound, Serverbound},
    };
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    // Actual registered guest packet route, with real loaded block facts.
    fn placement_control(check_start: Option<bool>) {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("horizon-use-cooldown-unit-{}", std::process::id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "use-cooldown-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 331,
                name: "SwingGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [4.5, 200.0, 4.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 16));
        guest.selected = 0;

        game.lan.as_mut().unwrap().hub.guests.push(guest);
        assert_eq!(game.lan.as_mut().unwrap().hub.guests[0].uuid, 331);
        assert!(
            game.lan.as_mut().unwrap().hub.guests[0].placed
                && game.lan.as_mut().unwrap().hub.guests[0].loaded
        );
        assert_eq!(game.lan.as_mut().unwrap().hub.guests[0].game_mode, 0);
        let mut planks = ItemStack::new("minecraft:oak_planks", 5);
        planks.data_mut().set_other(
            "minecraft:use_cooldown",
            Some(horizon_core::nbt::Tag::compound([
                ("seconds", horizon_core::nbt::Tag::Float(30.0f32.to_bits())),
                (
                    "cooldown_group",
                    horizon_core::nbt::Tag::String("probe:group".into()),
                ),
            ])),
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.inventory[0] = Some(planks));
        let use_on = |x, sequence| {
            Serverbound::UseItemOn(play::UseItemOn {
                hand: 0,
                pos: [x, 199, 6],
                face: 1,
                cursor: [0.5, 1.0, 0.5],
                inside: false,
                world_border: false,
                sequence,
            })
        };
        assert!(game.world.is_air(game.world.block(5, 200, 6)));
        if check_start.is_none() {
            let mut stick = game.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .unwrap()
                .clone();
            stick.item = "minecraft:stick".into();
            stick.count = 1;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.inventory[0] = Some(stick));
            game.lan_guest_packet(2, use_on(5, 1));
            assert!(
                game.world.is_air(game.world.block(5, 200, 6)),
                "failed stick use"
            );
            assert!(
                !game.lan.as_mut().unwrap().play[&2]
                    .cooldowns
                    .is_on_cooldown("probe:group")
            );
            let mut planks = game.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .unwrap()
                .clone();
            planks.item = "minecraft:oak_planks".into();
            planks.count = 5;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.inventory[0] = Some(planks));
        }
        game.lan_guest_packet(2, use_on(5, 2));
        let oak = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_planks")
            .unwrap();
        assert_eq!(game.world.block(5, 200, 6), oak, "actual first placement");
        assert_eq!(
            game.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .unwrap()
                .count,
            4
        );
        assert!(
            game.lan.as_mut().unwrap().play.contains_key(&2),
            "real acting guest context installed"
        );
        if check_start == Some(true) {
            assert!(
                game.lan.as_mut().unwrap().play[&2]
                    .cooldowns
                    .is_on_cooldown("probe:group"),
                "successful use starts the actual guest's group"
            );
            assert!(rx.try_iter().any(|out| {
                let bodies = match out { Out::Packet(body) => vec![body], Out::Packets(bodies) => bodies, Out::Disconnect(_) => panic!("disconnect") };
                bodies.iter().any(|body| {
                    let mut r = horizon_protocol::buf::Reader::new(body);
                    matches!(Clientbound::decode(r.varint().unwrap(),r.rest()), Ok(Clientbound::Cooldown(p)) if p.group == "probe:group" && p.duration == 600)
                })
            }), "existing helper's actual cooldown packet");
        } else if check_start == Some(false) {
            game.lan_guest_packet(2, use_on(6, 2));
            assert!(
                game.world.is_air(game.world.block(6, 200, 6)),
                "same-group second placement is rejected"
            );
            assert_eq!(
                game.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                    .as_ref()
                    .unwrap()
                    .count,
                4
            );
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn failed_stick_use_keeps_group_available_for_placement() {
        placement_control(None);
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn actual_guest_placement_starts_its_cooldown() {
        placement_control(Some(true));
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn actual_guest_second_placement_is_rejected() {
        placement_control(Some(false));
    }
}

#[cfg(test)]
#[path = "tests/aim_point_tests.rs"]
mod aim_point_tests;

#[cfg(test)]
#[path = "tests/boat_holder_movement_tests.rs"]
mod boat_holder_movement_tests;

#[cfg(test)]
#[path = "tests/bundle_slot_tests.rs"]
mod bundle_slot_tests;

#[cfg(test)]
#[path = "tests/crack_feedback_tests.rs"]
mod crack_feedback_tests;

#[cfg(test)]
#[path = "tests/menu_still_valid_tests.rs"]
mod menu_still_valid_tests;

#[cfg(test)]
#[path = "tests/projectile_lifecycle_tests.rs"]
mod projectile_lifecycle_tests;

#[cfg(test)]
#[path = "tests/spin_target_tests.rs"]
mod spin_target_tests;

#[cfg(test)]
#[path = "tests/spawn_protection_tests.rs"]
mod spawn_protection_tests;

#[cfg(test)]
#[path = "tests/unloaded_chunk_tests.rs"]
mod unloaded_chunk_tests;
